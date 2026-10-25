//! [`DamonClient`] — protocol v2 WebSocket thin client.
//!
//! One connection multiplexes every session: requests carry a client-assigned
//! id resolved through a pending map, pushes fan out over a broadcast channel,
//! and a dedicated receive task routes frames. The handle is a cheap `Arc`
//! clone, so the Tauri command layer can share one connection.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, broadcast, oneshot, watch};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::protocol::{
    BackendStatus, DamonError, Hello, PermissionBehavior, Push, Request, Response, SessionInfo,
    SessionSummary, TurnResult,
};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Default per-call timeout. Generous on purpose: `turn.start` resolves only
/// when the turn ends.
pub const DEFAULT_CALL_TIMEOUT_SECS: u64 = 300;

/// How long `connect` waits for the hello push (non-fatal on timeout).
const HELLO_TIMEOUT: Duration = Duration::from_secs(3);

/// `/health` probe timeout — discovery must fail fast when the daemon is dead.
const HEALTH_PROBE_TIMEOUT: Duration = Duration::from_millis(1500);

/// Broadcast capacity for streamed events (timeline deltas can be chatty).
const EVENT_CHANNEL_CAPACITY: usize = 1024;

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// A live daemon endpoint, validated through `/health`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonEndpoint {
    /// Ready-to-use WebSocket URL (`ws://127.0.0.1:{port}/ws`).
    pub url: String,
    pub port: u16,
    pub pid: Option<u32>,
    pub version: Option<String>,
    pub tls: bool,
    /// `/health` status word ("ok").
    pub status: String,
    /// Backends the daemon currently offers.
    pub backends: Vec<BackendStatus>,
}

/// `~/.damon/daemon.json` — a hint file only; always validated via `/health`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveryFile {
    port: u16,
    #[serde(default)]
    pid: Option<u32>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    tls: bool,
}

/// `/health` response (loopback shape).
#[derive(Debug, Clone, Deserialize)]
struct HealthInfo {
    #[serde(default = "default_status")]
    status: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    backends: Vec<String>,
}

fn default_status() -> String {
    "ok".to_string()
}

/// Find the daemon via `~/.damon/daemon.json`, proving liveness through
/// `http://127.0.0.1:{port}/health`. `None` when the file is missing, stale,
/// or the daemon does not answer.
pub async fn discover() -> Option<DaemonEndpoint> {
    let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
    discover_under(&home).await
}

/// [`discover`] with an explicit home directory (test seam).
pub async fn discover_under(home: &Path) -> Option<DaemonEndpoint> {
    let file = home.join(".damon").join("daemon.json");
    let raw = tokio::fs::read_to_string(file).await.ok()?;
    let discovery: DiscoveryFile = serde_json::from_str(&raw).ok()?;
    probe(&discovery).await
}

async fn probe(discovery: &DiscoveryFile) -> Option<DaemonEndpoint> {
    let client = reqwest::Client::builder()
        .timeout(HEALTH_PROBE_TIMEOUT)
        .build()
        .ok()?;
    let response = client
        .get(format!("http://127.0.0.1:{}/health", discovery.port))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let health: HealthInfo = response.json().await.ok()?;
    if health.status != "ok" {
        return None;
    }
    let scheme = if discovery.tls { "wss" } else { "ws" };
    Some(DaemonEndpoint {
        url: format!("{scheme}://127.0.0.1:{}/ws", discovery.port),
        port: discovery.port,
        pid: discovery.pid,
        version: discovery.version.clone().or(health.version),
        tls: discovery.tls,
        status: health.status,
        backends: health
            .backends
            .into_iter()
            .map(|id| BackendStatus {
                id,
                available: true,
                capabilities: None,
            })
            .collect(),
    })
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Connected thin client. Cheap to clone; all clones share one socket, one
/// pending-request map and one event broadcast. Drop the last clone (or call
/// [`DamonClient::close`]) to tear the connection down.
#[derive(Clone)]
pub struct DamonClient {
    inner: Arc<Inner>,
}

struct Inner {
    shared: Arc<Shared>,
    sink: Mutex<SplitSink<Ws, Message>>,
    reader: StdMutex<Option<JoinHandle<()>>>,
}

struct Shared {
    pending: StdMutex<HashMap<u64, oneshot::Sender<Result<Value, DamonError>>>>,
    events: broadcast::Sender<Push>,
    hello: watch::Sender<Option<Hello>>,
    next_id: AtomicU64,
    call_timeout_secs: AtomicU64,
    closed: AtomicBool,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(reader) = self.reader.lock().unwrap().take() {
            reader.abort();
        }
    }
}

impl DamonClient {
    /// Open a WebSocket to `url` (`ws://…/ws`), optionally with an
    /// `Authorization: Bearer <token>` header. Waits up to 3s for the hello
    /// push (non-fatal — the daemon may just be slow).
    pub async fn connect(url: &str, token: Option<String>) -> Result<Self, DamonError> {
        let mut request = url
            .into_client_request()
            .map_err(|e| DamonError::ConnectFailed(format!("{url}: {e}")))?;
        if let Some(token) = token.as_deref().filter(|t| !t.is_empty()) {
            let bearer = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|e| DamonError::ConnectFailed(format!("invalid bearer token: {e}")))?;
            request.headers_mut().insert("Authorization", bearer);
        }
        let (ws, _response) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| DamonError::ConnectFailed(format!("{url}: {e}")))?;
        let (sink, stream) = ws.split();

        let (events, _guard) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        let (hello_tx, hello_rx) = watch::channel(None);
        let shared = Arc::new(Shared {
            pending: StdMutex::new(HashMap::new()),
            events,
            hello: hello_tx,
            next_id: AtomicU64::new(0),
            call_timeout_secs: AtomicU64::new(DEFAULT_CALL_TIMEOUT_SECS),
            closed: AtomicBool::new(false),
        });
        let reader = tokio::spawn(receive_loop(stream, shared.clone()));
        let client = Self {
            inner: Arc::new(Inner {
                shared,
                sink: Mutex::new(sink),
                reader: StdMutex::new(Some(reader)),
            }),
        };
        client.wait_for_hello(hello_rx).await;
        Ok(client)
    }

    /// Discover the daemon and connect in one step.
    pub async fn discover_and_connect(token: Option<String>) -> Result<Self, DamonError> {
        let endpoint = discover().await.ok_or(DamonError::DaemonNotRunning)?;
        Self::connect(&endpoint.url, token).await
    }

    async fn wait_for_hello(&self, mut rx: watch::Receiver<Option<Hello>>) {
        if rx.borrow_and_update().is_some() {
            return;
        }
        let _ = tokio::time::timeout(HELLO_TIMEOUT, async {
            loop {
                if rx.changed().await.is_err() {
                    return;
                }
                if rx.borrow_and_update().is_some() {
                    return;
                }
            }
        })
        .await;
    }

    // -- raw RPC -----------------------------------------------------------

    /// Issue a request and await its response. RPC errors surface as typed
    /// [`DamonError`] variants.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, DamonError> {
        let shared = &self.inner.shared;
        if shared.closed.load(Ordering::SeqCst) {
            return Err(DamonError::ConnectionClosed(
                "연결이 이미 종료되었습니다".into(),
            ));
        }
        let id = shared.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let payload = serde_json::to_string(&Request {
            id,
            method: method.to_string(),
            params,
        })
        .map_err(|e| DamonError::Protocol(format!("요청 직렬화 실패 ({method}): {e}")))?;

        let (tx, rx) = oneshot::channel();
        shared.pending.lock().unwrap().insert(id, tx);
        {
            let mut sink = self.inner.sink.lock().await;
            if let Err(e) = sink.send(Message::Text(payload.into())).await {
                shared.pending.lock().unwrap().remove(&id);
                return Err(DamonError::ConnectionClosed(format!(
                    "요청 전송 실패 ({method}): {e}"
                )));
            }
        }

        let timeout = Duration::from_secs(
            shared
                .call_timeout_secs
                .load(Ordering::Relaxed)
                .clamp(1, u32::MAX as u64),
        );
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(DamonError::ConnectionClosed(
                "응답 대기 채널이 닫혔습니다".into(),
            )),
            Err(_) => {
                shared.pending.lock().unwrap().remove(&id);
                Err(DamonError::RequestTimeout {
                    method: method.to_string(),
                    secs: timeout.as_secs(),
                })
            }
        }
    }

    // -- typed wrappers ----------------------------------------------------

    /// `session.create {backend?, cwd?}` → `{sessionId, backend, …}`.
    pub async fn create_session(
        &self,
        backend: Option<&str>,
        cwd: Option<&str>,
    ) -> Result<SessionInfo, DamonError> {
        let mut params = json!({});
        if let Some(backend) = backend {
            params["backend"] = json!(backend);
        }
        if let Some(cwd) = cwd {
            params["cwd"] = json!(cwd);
        }
        let result = self.call("session.create", params).await?;
        parse("session.create", result)
    }

    /// `session.list {limit?}` → session rows (pinned first).
    pub async fn list_sessions(
        &self,
        limit: Option<u32>,
    ) -> Result<Vec<SessionSummary>, DamonError> {
        let params = match limit {
            Some(limit) => json!({ "limit": limit }),
            None => json!({}),
        };
        let result = self.call("session.list", params).await?;
        #[derive(Deserialize)]
        struct ListResult {
            #[serde(default)]
            sessions: Vec<SessionSummary>,
        }
        parse::<ListResult>("session.list", result).map(|r| r.sessions)
    }

    /// `turn.start {sessionId, prompt}` → `{turnId, stopReason, usage?}`.
    /// Resolves when the turn ends; stream events ride [`DamonClient::subscribe_events`].
    pub async fn turn_start(
        &self,
        session_id: &str,
        prompt: &str,
    ) -> Result<TurnResult, DamonError> {
        let result = self
            .call(
                "turn.start",
                json!({ "sessionId": session_id, "prompt": prompt }),
            )
            .await?;
        parse("turn.start", result)
    }

    /// `turn.cancel {sessionId}` → whether a live turn was cancelled.
    pub async fn cancel_turn(&self, session_id: &str) -> Result<bool, DamonError> {
        let result = self
            .call("turn.cancel", json!({ "sessionId": session_id }))
            .await?;
        #[derive(Deserialize)]
        struct Cancelled {
            #[serde(default)]
            cancelled: bool,
        }
        parse::<Cancelled>("turn.cancel", result).map(|r| r.cancelled)
    }

    /// `permission.respond {sessionId, requestId, response: {behavior}}`.
    pub async fn respond_permission(
        &self,
        session_id: &str,
        request_id: &str,
        behavior: PermissionBehavior,
    ) -> Result<(), DamonError> {
        self.call(
            "permission.respond",
            json!({
                "sessionId": session_id,
                "requestId": request_id,
                "response": { "behavior": behavior }
            }),
        )
        .await?;
        Ok(())
    }

    /// `permission.respond` with a full response object (answers, message,
    /// interrupt, …) for advanced ask kinds.
    pub async fn respond_permission_raw(
        &self,
        session_id: &str,
        request_id: &str,
        response: Value,
    ) -> Result<Value, DamonError> {
        self.call(
            "permission.respond",
            json!({
                "sessionId": session_id,
                "requestId": request_id,
                "response": response
            }),
        )
        .await
    }

    /// `session.delete {sessionId}` → whether the session was deleted.
    pub async fn delete_session(&self, session_id: &str) -> Result<bool, DamonError> {
        let result = self
            .call("session.delete", json!({ "sessionId": session_id }))
            .await?;
        #[derive(Deserialize)]
        struct Deleted {
            #[serde(default)]
            deleted: bool,
        }
        parse::<Deleted>("session.delete", result).map(|r| r.deleted)
    }

    /// `session.rename {sessionId, title}` → whether the title changed.
    pub async fn rename_session(&self, session_id: &str, title: &str) -> Result<bool, DamonError> {
        let result = self
            .call(
                "session.rename",
                json!({ "sessionId": session_id, "title": title }),
            )
            .await?;
        #[derive(Deserialize)]
        struct Renamed {
            #[serde(default)]
            renamed: bool,
        }
        parse::<Renamed>("session.rename", result).map(|r| r.renamed)
    }

    /// `backend.list` → backend roster with availability.
    pub async fn backend_list(&self) -> Result<Vec<BackendStatus>, DamonError> {
        let result = self.call("backend.list", json!({})).await?;
        #[derive(Deserialize)]
        struct Backends {
            #[serde(default)]
            backends: Vec<BackendStatus>,
        }
        parse::<Backends>("backend.list", result).map(|r| r.backends)
    }

    // -- events / lifecycle --------------------------------------------------

    /// Subscribe to server pushes (`session.event`, …). Every clone can hold
    /// its own receiver; a slow consumer lags (and drops) rather than blocking
    /// the receive loop.
    pub fn subscribe_events(&self) -> broadcast::Receiver<Push> {
        self.inner.shared.events.subscribe()
    }

    /// The daemon's hello info, once received.
    pub fn hello(&self) -> Option<Hello> {
        self.inner.shared.hello.borrow().clone()
    }

    /// Whether the connection is still usable.
    pub fn is_closed(&self) -> bool {
        self.inner.shared.closed.load(Ordering::SeqCst)
    }

    /// Override the per-call response timeout (seconds).
    pub fn set_call_timeout(&self, secs: u64) {
        self.inner
            .shared
            .call_timeout_secs
            .store(secs.max(1), Ordering::Relaxed);
    }

    /// Close the connection: best-effort WS close frame, then tear the
    /// receive loop down and fail every pending waiter.
    pub async fn close(&self) {
        {
            let mut sink = self.inner.sink.lock().await;
            let _ = sink.send(Message::Close(None)).await;
            let _ = sink.close().await;
        }
        self.inner.shared.closed.store(true, Ordering::SeqCst);
        self.fail_pending(DamonError::ConnectionClosed(
            "클라이언트에서 연결을 닫았습니다".into(),
        ));
        if let Some(reader) = self.inner.reader.lock().unwrap().take() {
            reader.abort();
        }
    }

    fn fail_pending(&self, error: DamonError) {
        for (_, waiter) in self.inner.shared.pending.lock().unwrap().drain() {
            let _ = waiter.send(Err(error.clone()));
        }
    }
}

fn parse<T: serde::de::DeserializeOwned>(method: &str, result: Value) -> Result<T, DamonError> {
    serde_json::from_value(result)
        .map_err(|e| DamonError::Protocol(format!("{method} 응답 파싱 실패: {e}")))
}

/// Receive loop: route frames until the socket dies, then fail waiters.
async fn receive_loop(stream: SplitStream<Ws>, shared: Arc<Shared>) {
    let mut stream = stream;
    while let Some(frame) = stream.next().await {
        let text = match frame {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => continue,
        };
        route(&shared, &text);
    }
    shared.closed.store(true, Ordering::SeqCst);
    for (_, waiter) in shared.pending.lock().unwrap().drain() {
        let _ = waiter.send(Err(DamonError::ConnectionClosed(
            "데몬이 연결을 닫았습니다".into(),
        )));
    }
}

/// Route one inbound frame: response → pending waiter, hello → watch,
/// push → broadcast. Unrecognized frames are ignored (never fatal).
fn route(shared: &Shared, text: &str) {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return;
    };
    if value.get("id").is_some() && (value.get("result").is_some() || value.get("error").is_some())
    {
        if let Ok(response) = serde_json::from_value::<Response>(value)
            && let Some(waiter) = shared.pending.lock().unwrap().remove(&response.id)
        {
            let outcome = match response.error {
                Some(error) => Err(DamonError::from_rpc(error)),
                None => Ok(response.result.unwrap_or(Value::Null)),
            };
            let _ = waiter.send(outcome);
        }
        return;
    }
    if value.get("hello").is_some() {
        if let Ok(hello) = serde_json::from_value::<Hello>(value) {
            let _ = shared.hello.send(Some(hello));
        }
        return;
    }
    if value.get("event").is_some()
        && let Ok(push) = serde_json::from_value::<Push>(value)
    {
        let _ = shared.events.send(push);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_file_parses_camel_case_keys() {
        let file: DiscoveryFile = serde_json::from_str(
            r#"{"port": 9470, "pid": 42, "version": "0.8.0", "tls": false, "configPath": "/tmp/damon.toml"}"#,
        )
        .unwrap();
        assert_eq!(file.port, 9470);
        assert_eq!(file.pid, Some(42));
        assert!(!file.tls);

        // missing port → unusable
        assert!(serde_json::from_str::<DiscoveryFile>(r#"{"pid": 1}"#).is_err());
    }

    #[test]
    fn health_info_defaults() {
        let health: HealthInfo = serde_json::from_str(r#"{"backends": ["mock"]}"#).unwrap();
        assert_eq!(health.status, "ok");
        assert_eq!(health.backends, vec!["mock".to_string()]);
    }
}
