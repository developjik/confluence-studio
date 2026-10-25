//! Integration tests for the Damon protocol v2 thin client against an axum
//! WebSocket mock daemon. Contract: docs/ARCHITECTURE.md § "Module: damon".
//!
//! Mock semantics (mirrors protocol-v2.md):
//! - on upgrade: hello push `{"hello": {…}}`
//! - `session.create` → `{sessionId: "s-mock-1", backend: "mock"}` (backend
//!   "nope" → error -32003)
//! - `turn.start` with a plain prompt: `session.event` stream
//!   (turn_started → timeline assistant_message "He" + "llo" → turn_completed),
//!   then `{turnId, stopReason: "completed", usage}`
//! - `turn.start` with prompt "permission": turn_started →
//!   permission_requested {id: "perm-1", …}; the turn response is withheld
//!   until `permission.respond` arrives, then turn_completed + response.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use confluence_studio_lib::damon::client::{DamonClient, discover_under};
use confluence_studio_lib::damon::protocol::{
    DamonError, PermissionBehavior, error_code, timeline_kind,
};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Mock daemon
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct MockState {
    /// Every request frame the mock received, in order.
    requests: Arc<Mutex<Vec<Value>>>,
    /// Authorization header seen on the last /ws upgrade.
    authorization: Arc<Mutex<Option<String>>>,
}

/// Spawn the mock daemon; returns (state, ws_url, port).
async fn spawn_mock() -> (MockState, String, u16) {
    let state = MockState::default();
    let app = Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws_upgrade))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (state, format!("ws://127.0.0.1:{port}/ws"), port)
}

async fn health() -> Json<Value> {
    Json(json!({"status": "ok", "version": "0.8.0-mock", "backends": ["mock"]}))
}

async fn ws_upgrade(
    State(state): State<MockState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    *state.authorization.lock().unwrap() = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    ws.on_upgrade(move |socket| handle_socket(state, socket))
}

fn text(value: &Value) -> Message {
    Message::Text(value.to_string().into())
}

fn ok(id: u64, result: Value) -> Message {
    text(&json!({"id": id, "result": result}))
}

fn err(id: u64, code: i64, message: &str) -> Message {
    text(&json!({"id": id, "error": {"code": code, "message": message}}))
}

fn session_event(session: &str, data: Value) -> Message {
    text(&json!({"event": "session.event", "sessionId": session, "data": data}))
}

async fn handle_socket(state: MockState, mut socket: WebSocket) {
    let _ = socket
        .send(text(&json!({
            "hello": {
                "protocol": 2,
                "daemon": "damond",
                "version": "0.8.0-mock",
                "permissionTimeoutSecs": 300
            }
        })))
        .await;

    // turn.start id whose response waits for a permission.respond.
    let mut pending_permission_turn: Option<u64> = None;

    while let Some(Ok(msg)) = socket.recv().await {
        let Message::Text(frame_text) = msg else {
            continue;
        };
        let Ok(frame) = serde_json::from_str::<Value>(&frame_text) else {
            continue;
        };
        state.requests.lock().unwrap().push(frame.clone());

        let id = frame["id"].as_u64().unwrap_or(0);
        let method = frame["method"].as_str().unwrap_or_default();
        let session = frame["params"]["sessionId"]
            .as_str()
            .unwrap_or("s-mock-1")
            .to_string();

        match method {
            "session.create" => {
                if frame["params"]["backend"] == json!("nope") {
                    let _ = socket
                        .send(err(
                            id,
                            error_code::BACKEND_NOT_REGISTERED,
                            "backend not registered: nope",
                        ))
                        .await;
                } else {
                    let _ = socket
                        .send(ok(
                            id,
                            json!({"sessionId": "s-mock-1", "backend": "mock", "replayed": 0}),
                        ))
                        .await;
                }
            }
            "session.list" => {
                let _ = socket
                    .send(ok(
                        id,
                        json!({"sessions": [
                            {
                                "sessionId": "s-mock-1",
                                "createdAt": 1770000000000i64,
                                "backend": "mock",
                                "title": "Mock session",
                                "cwd": "/tmp",
                                "tags": [],
                                "pinned": true,
                                "archived": false
                            }
                        ]}),
                    ))
                    .await;
            }
            "session.rename" => {
                let _ = socket.send(ok(id, json!({"renamed": true}))).await;
            }
            "session.delete" => {
                let _ = socket.send(ok(id, json!({"deleted": true}))).await;
            }
            "turn.cancel" => {
                let _ = socket.send(ok(id, json!({"cancelled": true}))).await;
            }
            "permission.respond" => {
                let _ = socket.send(ok(id, json!({}))).await;
                if let Some(turn_id) = pending_permission_turn.take() {
                    let _ = socket
                        .send(session_event(
                            &session,
                            json!({"type": "turn_completed", "turn_id": "t1"}),
                        ))
                        .await;
                    let _ = socket
                        .send(ok(
                            turn_id,
                            json!({"turnId": "t1", "stopReason": "completed"}),
                        ))
                        .await;
                }
            }
            "turn.start" => {
                let prompt = frame["params"]["prompt"].as_str().unwrap_or_default();
                let _ = socket
                    .send(session_event(
                        &session,
                        json!({"type": "turn_started", "turn_id": "t1"}),
                    ))
                    .await;
                if prompt == "permission" {
                    let _ = socket
                        .send(session_event(
                            &session,
                            json!({
                                "type": "permission_requested",
                                "turn_id": "t1",
                                "id": "perm-1",
                                "kind": "tool",
                                "name": "Bash",
                                "title": "bash command",
                                "input": {"command": "ls -la"},
                                "actions": [{"id": "a1", "label": "Allow", "behavior": "allow"}]
                            }),
                        ))
                        .await;
                    pending_permission_turn = Some(id);
                } else {
                    let _ = socket
                        .send(session_event(&session, json!({
                            "type": "timeline", "kind": "assistant_message", "text": "He", "turn_id": "t1"
                        })))
                        .await;
                    let _ = socket
                        .send(session_event(&session, json!({
                            "type": "timeline", "kind": "assistant_message", "text": "llo", "turn_id": "t1"
                        })))
                        .await;
                    let _ = socket
                        .send(session_event(&session, json!({
                            "type": "turn_completed", "turn_id": "t1", "usage": {"output_tokens": 2}
                        })))
                        .await;
                    let _ = socket
                        .send(ok(
                            id,
                            json!({
                                "turnId": "t1", "stopReason": "completed",
                                "usage": {"input_tokens": 1, "output_tokens": 2}
                            }),
                        ))
                        .await;
                }
            }
            other => {
                let _ = socket
                    .send(err(
                        id,
                        error_code::METHOD_NOT_FOUND,
                        &format!("unknown method: {other}"),
                    ))
                    .await;
            }
        }
    }
}

/// Connect with the mock's usual token and a short call timeout so a protocol
/// bug fails the test instead of hanging it.
async fn connect(url: &str) -> DamonClient {
    let client = DamonClient::connect(url, Some("test-token".into()))
        .await
        .expect("connect");
    client.set_call_timeout(5);
    client
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn damon_connects_with_bearer_header_and_receives_hello() {
    let (state, url, _port) = spawn_mock().await;
    let client = connect(&url).await;

    let hello = client.hello().expect("hello push after connect");
    assert_eq!(hello.hello.protocol_major(), Some(2));
    assert_eq!(hello.hello.daemon.as_deref(), Some("damond"));
    assert_eq!(hello.hello.version.as_deref(), Some("0.8.0-mock"));
    assert_eq!(hello.hello.permission_timeout_secs, Some(300));
    assert!(!client.is_closed());

    // The upgrade carried the Bearer header.
    assert_eq!(
        *state.authorization.lock().unwrap(),
        Some("Bearer test-token".to_string())
    );
    client.close().await;
    assert!(client.is_closed());
}

#[tokio::test]
async fn damon_streams_turn_events_and_result() {
    let (state, url, _port) = spawn_mock().await;
    let client = connect(&url).await;

    let session = client.create_session(Some("mock"), None).await.unwrap();
    assert_eq!(session.session_id, "s-mock-1");
    assert_eq!(session.backend, "mock");
    assert_eq!(session.replayed, Some(0));

    // Subscribe BEFORE the turn so no stream event can be missed.
    let mut events = client.subscribe_events();
    let collector = tokio::spawn(async move {
        let mut assembled = String::new();
        let (mut saw_start, mut saw_completed) = (false, false);
        while let Ok(push) = events.recv().await {
            let Some(event) = push.stream_event() else {
                continue;
            };
            match event.event_type.as_str() {
                "turn_started" => saw_start = true,
                "turn_completed" => {
                    saw_completed = true;
                    break;
                }
                "timeline" if event.kind.as_deref() == Some(timeline_kind::ASSISTANT_MESSAGE) => {
                    assembled.push_str(event.text.as_deref().unwrap_or(""));
                }
                _ => {}
            }
        }
        (assembled, saw_start, saw_completed)
    });

    let result = client
        .turn_start(&session.session_id, "hello please")
        .await
        .expect("turn result");
    assert_eq!(result.turn_id, "t1");
    assert_eq!(result.stop_reason, "completed");
    assert!(result.is_completed());
    assert_eq!(result.usage.unwrap()["input_tokens"], 1);

    let (assembled, saw_start, saw_completed) = collector.await.unwrap();
    assert_eq!(assembled, "Hello", "streamed chunks He + llo must assemble");
    assert!(saw_start, "turn_started event must arrive");
    assert!(saw_completed, "turn_completed event must arrive");

    // The turn.start request carried both camelCase params.
    {
        let requests = state.requests.lock().unwrap();
        let turn = requests
            .iter()
            .find(|frame| frame["method"] == "turn.start")
            .expect("turn.start frame recorded");
        assert_eq!(turn["params"]["sessionId"], "s-mock-1");
        assert_eq!(turn["params"]["prompt"], "hello please");
    }

    client.close().await;
}

#[tokio::test]
async fn damon_answers_permission_asks_during_turn() {
    let (state, url, _port) = spawn_mock().await;
    let client = connect(&url).await;

    let session = client.create_session(Some("mock"), None).await.unwrap();

    // A responder answers the ask as soon as the event streams in — while
    // turn_start is still awaiting its (withheld) response.
    let mut events = client.subscribe_events();
    let responder = {
        let client = client.clone();
        tokio::spawn(async move {
            while let Ok(push) = events.recv().await {
                let Some(event) = push.stream_event() else {
                    continue;
                };
                if event.event_type == "permission_requested" {
                    let ask = event.permission_request().expect("ask shape");
                    assert_eq!(ask.id, "perm-1");
                    assert_eq!(ask.kind.as_deref(), Some("tool"));
                    assert_eq!(ask.name.as_deref(), Some("Bash"));
                    let session_id = push.session_id.clone().unwrap_or_default();
                    client
                        .respond_permission(&session_id, &ask.id, PermissionBehavior::Allow)
                        .await
                        .expect("respond");
                    break;
                }
            }
        })
    };

    let result = client
        .turn_start(&session.session_id, "permission")
        .await
        .expect("turn completes after the permission answer");
    assert_eq!(result.stop_reason, "completed");
    responder.await.unwrap();

    // The mock only finished the turn once permission.respond arrived.
    {
        let requests = state.requests.lock().unwrap();
        let respond = requests
            .iter()
            .find(|frame| frame["method"] == "permission.respond")
            .expect("permission.respond frame recorded");
        assert_eq!(respond["params"]["sessionId"], "s-mock-1");
        assert_eq!(respond["params"]["requestId"], "perm-1");
        assert_eq!(respond["params"]["response"]["behavior"], "allow");
    }

    client.close().await;
}

#[tokio::test]
async fn damon_maps_typed_error_codes() {
    let (_state, url, _port) = spawn_mock().await;
    let client = connect(&url).await;

    let error = client.create_session(Some("nope"), None).await.unwrap_err();
    match &error {
        DamonError::BackendNotRegistered(message) => {
            assert!(
                message.contains("nope"),
                "raw message rides along: {message}"
            );
        }
        other => panic!("expected BackendNotRegistered, got {other:?}"),
    }
    assert_eq!(error.rpc_code(), Some(error_code::BACKEND_NOT_REGISTERED));
    assert!(
        error.to_string().contains("백엔드"),
        "user-facing Display: {error}"
    );

    client.close().await;
}

#[tokio::test]
async fn damon_session_admin_wrappers_round_trip() {
    let (_state, url, _port) = spawn_mock().await;
    let client = connect(&url).await;

    let sessions = client.list_sessions(Some(10)).await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "s-mock-1");
    assert_eq!(sessions[0].title.as_deref(), Some("Mock session"));
    assert_eq!(sessions[0].backend.as_deref(), Some("mock"));
    assert_eq!(sessions[0].created_at, Some(1770000000000));

    assert!(client.rename_session("s-mock-1", "renamed").await.unwrap());
    assert!(client.delete_session("s-mock-1").await.unwrap());
    assert!(client.cancel_turn("s-mock-1").await.unwrap());
    assert!(!client.is_closed());

    client.close().await;
}

#[tokio::test]
async fn damon_discover_probes_health_and_builds_endpoint() {
    // Live daemon: discovery file + answering /health.
    let (_state, url, port) = spawn_mock().await;
    let live = TempHome::new("live");
    live.write_daemon_json(&json!({
        "port": port, "pid": 42, "version": "0.8.0", "tls": false, "configPath": "/tmp/d.toml"
    }));
    let endpoint = discover_under(&live.path)
        .await
        .expect("endpoint for live daemon");
    assert_eq!(endpoint.url, url);
    assert_eq!(endpoint.port, port);
    assert_eq!(endpoint.pid, Some(42));
    assert_eq!(endpoint.version.as_deref(), Some("0.8.0"));
    assert!(!endpoint.tls);
    assert_eq!(endpoint.status, "ok");
    assert!(
        endpoint
            .backends
            .iter()
            .any(|backend| backend.id == "mock" && backend.available)
    );

    // No discovery file → None.
    let empty = TempHome::new("empty");
    assert!(discover_under(&empty.path).await.is_none());

    // Stale file (nothing listens on the port) → the health probe rejects it.
    let stale = TempHome::new("stale");
    stale.write_daemon_json(&json!({"port": 1, "pid": 9, "version": "0.8.0", "tls": false}));
    assert!(discover_under(&stale.path).await.is_none());

    // Malformed file → None.
    let garbage = TempHome::new("garbage");
    garbage.write_raw_daemon_json("not json");
    assert!(discover_under(&garbage.path).await.is_none());
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Temp home directory cleaned up on drop.
struct TempHome {
    path: PathBuf,
}

impl TempHome {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("cs-damon-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn write_daemon_json(&self, body: &Value) {
        self.write_raw_daemon_json(&body.to_string());
    }

    fn write_raw_daemon_json(&self, body: &str) {
        let dir = self.path.join(".damon");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("daemon.json"), body).unwrap();
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
