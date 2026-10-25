//! Damon wire protocol v2 frame model.
//!
//! Ground truth: damon-agent-core `docs/protocol-v2.md` (verified against the
//! v0.8.0 daemon source). Wire conventions:
//!
//! - request  `{"id": 1, "method": "session.create", "params": {…}}`
//! - response `{"id": 1, "result": {…}}` / `{"id": 1, "error": {code, message, data?}}`
//! - push     `{"event": "session.event", "sessionId": "…", "replay"?, "data": {…}}`
//!   (`sessionId` is camelCase on the wire)
//! - hello    `{"hello": {protocol, daemon, version, permissionTimeoutSecs, …}}`
//!   (sent on connect and as the response to a `hello` request)
//!
//! `session.event` payloads are [`StreamEvent`]s: internally tagged `"type"`
//! (snake_case) with snake_case fields (`turn_id`, …). Envelope DTOs that
//! cross the Tauri boundary keep camelCase keys per the architecture contract.
//!
//! Unknown fields are ignored (and round-tripped where noted); unknown RPC
//! error codes fall back to [`DamonError::Rpc`].

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

// ---------------------------------------------------------------------------
// Envelopes
// ---------------------------------------------------------------------------

/// Request frame sent to the daemon. `params` is omitted when null.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

/// Response frame. Exactly one of `result` / `error` is present; frames whose
/// `id` is not a number (parse-error replies carry `id: null`) are ignored by
/// the client since no caller can be waiting on them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

/// Typed RPC error object (`{code, message, data?}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Server push frame (`{event, sessionId?, replay?, data?}`). Other envelope
/// keys the daemon adds over time (`creationId`, `runId`, `scanId`, …)
/// round-trip untouched through [`Push::extra`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Push {
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Catch-up frames replaying history a late subscriber missed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    /// Unclaimed envelope keys, preserved verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Broadcast-side alias used by the architecture contract.
pub type DamonPush = Push;

impl Push {
    /// Event name of the session stream multiplexer.
    pub const SESSION_EVENT: &'static str = "session.event";

    /// Parsed [`StreamEvent`] when this push is a `session.event` frame with
    /// parseable data.
    pub fn stream_event(&self) -> Option<StreamEvent> {
        if self.event != Self::SESSION_EVENT {
            return None;
        }
        self.data
            .as_ref()
            .and_then(|data| serde_json::from_value(data.clone()).ok())
    }
}

/// Hello frame: sent by the daemon on connect and as the `hello` method's
/// response (the response additionally carries `backends[]`/`methods[]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub hello: HelloInfo,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloInfo {
    /// Protocol major (2). Kept as [`Value`] — tolerant of string-typed builds.
    pub protocol: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backends: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub methods: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_clients: Option<u64>,
    /// Unclaimed fields, preserved verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl HelloInfo {
    /// Protocol major as a number, accepting `"2"`-style strings too.
    pub fn protocol_major(&self) -> Option<u64> {
        self.protocol
            .as_u64()
            .or_else(|| self.protocol.as_str().and_then(|s| s.parse().ok()))
    }
}

// ---------------------------------------------------------------------------
// StreamEvent (session.event payloads)
// ---------------------------------------------------------------------------

/// Well-known `type` tags of [`StreamEvent`].
pub mod stream_event_type {
    pub const THREAD_STARTED: &str = "thread_started";
    pub const TURN_STARTED: &str = "turn_started";
    pub const TURN_COMPLETED: &str = "turn_completed";
    pub const TURN_FAILED: &str = "turn_failed";
    pub const TURN_CANCELED: &str = "turn_canceled";
    pub const TIMELINE: &str = "timeline";
    pub const MODE_CHANGED: &str = "mode_changed";
    pub const MODEL_CHANGED: &str = "model_changed";
    pub const PERMISSION_REQUESTED: &str = "permission_requested";
    pub const PERMISSION_RESOLVED: &str = "permission_resolved";
    pub const ATTENTION_REQUIRED: &str = "attention_required";
    pub const SUBAGENT: &str = "subagent";
}

/// Well-known timeline `kind` tags ([`StreamEvent::kind`]).
pub mod timeline_kind {
    pub const USER_MESSAGE: &str = "user_message";
    pub const ASSISTANT_MESSAGE: &str = "assistant_message";
    pub const REASONING: &str = "reasoning";
    pub const TOOL_CALL: &str = "tool_call";
    pub const TODO: &str = "todo";
    pub const COMPACTION: &str = "compaction";
    pub const ERROR: &str = "error";
    pub const UNKNOWN: &str = "unknown";
}

/// One normalized daemon event: `{turn_id?, type, …}` with a flattened
/// timeline item (`kind`, `text`) when `type == "timeline"`.
///
/// Field names are snake_case on the wire (matching the daemon serializer);
/// camelCase aliases are accepted for forward tolerance. Unknown fields
/// (usage, actions, detail, items, …) round-trip untouched via
/// [`StreamEvent::extra`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamEvent {
    #[serde(
        default,
        rename = "turn_id",
        alias = "turnId",
        skip_serializing_if = "Option::is_none"
    )]
    pub turn_id: Option<String>,
    /// Event tag — see [`stream_event_type`].
    #[serde(rename = "type", alias = "eventType")]
    pub event_type: String,
    /// Timeline item tag when `event_type == "timeline"` — see [`timeline_kind`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Timeline text (`user_message` / `assistant_message` / `reasoning`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Permission ask id (`permission_requested {id, …}`); `requestId` in
    /// `permission.respond` params.
    #[serde(default, rename = "id", skip_serializing_if = "Option::is_none")]
    pub ask_id: Option<String>,
    /// `turn_failed` error / `turn_canceled` reason / `timeline error` message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Unclaimed fields, preserved verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl StreamEvent {
    /// Whether the event carries this `type` tag.
    pub fn is(&self, event_type: &str) -> bool {
        self.event_type == event_type
    }

    /// Whether this is a timeline event of the given `kind`.
    pub fn is_timeline(&self, kind: &str) -> bool {
        self.event_type == stream_event_type::TIMELINE && self.kind.as_deref() == Some(kind)
    }

    /// `turn_completed`/`turn_failed` usage object, when present.
    pub fn usage(&self) -> Option<&Value> {
        self.extra.get("usage")
    }

    /// Parsed permission ask when this event is a `permission_requested`.
    pub fn permission_request(&self) -> Option<PermissionRequest> {
        if self.event_type != stream_event_type::PERMISSION_REQUESTED {
            return None;
        }
        serde_json::to_value(self)
            .ok()
            .and_then(|v| serde_json::from_value(v).ok())
    }
}

/// A `permission_requested` ask (`{id, kind, name, title?, input?, detail?,
/// actions[], suggestions[], questions?}`). Action/answer rendering fields are
/// kept as raw JSON — surfaces render them directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub id: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
    #[serde(default)]
    pub actions: Vec<Value>,
    #[serde(default)]
    pub suggestions: Vec<Value>,
    #[serde(default)]
    pub questions: Vec<Value>,
}

/// Behavior sent with `permission.respond` (`{behavior: …}`). `Always` is the
/// "always allow" upgrade some surfaces offer; the daemon treats it as a
/// suggested-update allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionBehavior {
    Allow,
    Deny,
    Always,
}

// ---------------------------------------------------------------------------
// Method result DTOs (camelCase at the Tauri boundary)
// ---------------------------------------------------------------------------

/// `session.create` result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub session_id: String,
    pub backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replayed: Option<u64>,
}

/// One `session.list` row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub session_id: String,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// Unix milliseconds, when reported.
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub pinned: Option<bool>,
    #[serde(default)]
    pub archived: Option<bool>,
}

/// `turn.start` result — resolves when the turn ends (events stream first).
/// `stop_reason` is one of `completed | canceled | failed | closed | timeout`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnResult {
    pub turn_id: String,
    pub stop_reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detached: Option<bool>,
}

impl TurnResult {
    pub fn is_completed(&self) -> bool {
        self.stop_reason == "completed"
    }
}

/// One `backend.list` entry (also the shape `damon_status` reports).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendStatus {
    pub id: String,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Value>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Stable Damon RPC error codes (protocol v2 wire contract).
pub mod error_code {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32000;
    pub const SESSION_NOT_LIVE: i64 = -32001;
    pub const CAPABILITY_MISSING: i64 = -32002;
    pub const BACKEND_NOT_REGISTERED: i64 = -32003;
    pub const MAX_SESSIONS: i64 = -32004;
    pub const RESPONSE_TOO_LARGE: i64 = -32005;
    pub const TURN_ALREADY_RUNNING: i64 = -32006;
    pub const ASK_NOT_FOUND: i64 = -32008;
    pub const PLAN_CONFLICT: i64 = -32009;
    pub const MCP_CONFLICT: i64 = -32010;
    pub const MEMORY_REJECTED: i64 = -32011;
}

/// Client-side error model. RPC error codes map to typed variants so callers
/// can branch without string-matching; unmapped codes become [`DamonError::Rpc`].
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DamonError {
    #[error("Damon 데몬이 실행 중이 아닙니다 (~/.damon/daemon.json 없음 또는 /health 무응답)")]
    DaemonNotRunning,
    #[error("Damon WebSocket 연결 실패: {0}")]
    ConnectFailed(String),
    #[error("Damon 연결이 끊어졌습니다: {0}")]
    ConnectionClosed(String),
    #[error("Damon 요청 시간 초과 ({method}, {secs}초) — 데몬이 응답하지 않았습니다")]
    RequestTimeout { method: String, secs: u64 },
    #[error("세션이 실행 중이 아닙니다: {0} — session.resume 후 재시도하세요")]
    SessionNotLive(String),
    #[error("백엔드가 이 기능을 지원하지 않습니다: {0}")]
    CapabilityMissing(String),
    #[error(
        "백엔드가 설치/등록되어 있지 않습니다: {0} — backend.list에서 다른 백엔드를 선택하세요"
    )]
    BackendNotRegistered(String),
    #[error(
        "세션 최대 개수(max_sessions)에 도달했습니다 — 다른 세션을 닫거나 삭제한 뒤 다시 시도하세요"
    )]
    MaxSessionsReached,
    #[error("응답이 1MiB 프레임 상한을 초과했습니다 — limit/offset으로 나눠 조회하세요")]
    ResponseTooLarge,
    #[error("이 세션에서 이미 턴이 진행 중입니다 — 종료를 기다리거나 먼저 취소하세요")]
    TurnAlreadyRunning,
    #[error("처리할 권한 요청이 없습니다 (이미 응답했거나 종료됨): {0}")]
    AskNotFound(String),
    #[error("알 수 없는 메서드입니다: {0}")]
    MethodUnknown(String),
    #[error("요청 파라미터가 올바르지 않습니다: {0}")]
    InvalidParams(String),
    #[error("Damon 오류 [{code}]: {message}")]
    Rpc { code: i64, message: String },
    #[error("Damon 프로토콜 오류: {0}")]
    Protocol(String),
}

impl DamonError {
    /// Map a wire error object to the typed model.
    pub fn from_rpc(err: RpcError) -> Self {
        match err.code {
            error_code::SESSION_NOT_LIVE => Self::SessionNotLive(err.message),
            error_code::CAPABILITY_MISSING => Self::CapabilityMissing(err.message),
            error_code::BACKEND_NOT_REGISTERED => Self::BackendNotRegistered(err.message),
            error_code::MAX_SESSIONS => Self::MaxSessionsReached,
            error_code::RESPONSE_TOO_LARGE => Self::ResponseTooLarge,
            error_code::TURN_ALREADY_RUNNING => Self::TurnAlreadyRunning,
            error_code::ASK_NOT_FOUND => Self::AskNotFound(err.message),
            error_code::METHOD_NOT_FOUND => Self::MethodUnknown(err.message),
            error_code::INVALID_PARAMS => Self::InvalidParams(err.message),
            _ => Self::Rpc {
                code: err.code,
                message: err.message,
            },
        }
    }

    /// Wire error code for code-carrying variants (inverse of [`DamonError::from_rpc`]).
    pub fn rpc_code(&self) -> Option<i64> {
        match self {
            Self::SessionNotLive(_) => Some(error_code::SESSION_NOT_LIVE),
            Self::CapabilityMissing(_) => Some(error_code::CAPABILITY_MISSING),
            Self::BackendNotRegistered(_) => Some(error_code::BACKEND_NOT_REGISTERED),
            Self::MaxSessionsReached => Some(error_code::MAX_SESSIONS),
            Self::ResponseTooLarge => Some(error_code::RESPONSE_TOO_LARGE),
            Self::TurnAlreadyRunning => Some(error_code::TURN_ALREADY_RUNNING),
            Self::AskNotFound(_) => Some(error_code::ASK_NOT_FOUND),
            Self::MethodUnknown(_) => Some(error_code::METHOD_NOT_FOUND),
            Self::InvalidParams(_) => Some(error_code::INVALID_PARAMS),
            Self::Rpc { code, .. } => Some(*code),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_omits_null_params() {
        let req = Request {
            id: 1,
            method: "backend.list".into(),
            params: Value::Null,
        };
        let wire = serde_json::to_value(&req).unwrap();
        assert_eq!(wire, json!({"id": 1, "method": "backend.list"}));

        let req = Request {
            id: 2,
            method: "session.create".into(),
            params: json!({"backend": "claude"}),
        };
        let wire = serde_json::to_string(&req).unwrap();
        assert!(wire.contains(r#""method":"session.create""#));
        assert!(wire.contains(r#""backend":"claude""#));
    }

    #[test]
    fn response_parses_result_and_error() {
        let ok: Response =
            serde_json::from_value(json!({"id": 7, "result": {"sessionId": "s1"}})).unwrap();
        assert_eq!(ok.id, 7);
        assert!(ok.error.is_none());
        assert_eq!(ok.result.unwrap()["sessionId"], "s1");

        let err: Response = serde_json::from_value(
            json!({"id": 8, "error": {"code": -32003, "message": "nope", "data": {"backend": "nope"}}}),
        )
        .unwrap();
        let rpc = err.error.unwrap();
        assert_eq!(rpc.code, -32003);
        assert_eq!(rpc.data.unwrap()["backend"], "nope");
    }

    #[test]
    fn push_uses_camel_case_session_id_and_keeps_extras() {
        let push: Push = serde_json::from_value(json!({
            "event": "session.event",
            "sessionId": "s1",
            "replay": true,
            "creationId": "c9",
            "data": {"type": "timeline", "kind": "assistant_message", "text": "Hi"}
        }))
        .unwrap();
        assert_eq!(push.event, "session.event");
        assert_eq!(push.session_id.as_deref(), Some("s1"));
        assert_eq!(push.replay, Some(true));
        assert_eq!(push.extra.get("creationId").unwrap(), "c9");

        let ev = push.stream_event().unwrap();
        assert!(ev.is_timeline(timeline_kind::ASSISTANT_MESSAGE));
        assert_eq!(ev.text.as_deref(), Some("Hi"));

        // Round-trip keeps every envelope key.
        let back = serde_json::to_value(&push).unwrap();
        assert_eq!(back["creationId"], "c9");
        assert_eq!(back["sessionId"], "s1");
    }

    #[test]
    fn hello_parses_protocol_and_timeout() {
        let hello: Hello = serde_json::from_value(json!({
            "hello": {
                "protocol": 2,
                "daemon": "damond",
                "version": "0.8.0",
                "permissionTimeoutSecs": 300,
                "remoteClients": 1
            }
        }))
        .unwrap();
        assert_eq!(hello.hello.protocol_major(), Some(2));
        assert_eq!(hello.hello.daemon.as_deref(), Some("damond"));
        assert_eq!(hello.hello.permission_timeout_secs, Some(300));
        assert_eq!(hello.hello.remote_clients, Some(1));

        // string-typed protocol still reads
        let lax: Hello = serde_json::from_value(json!({"hello": {"protocol": "2"}})).unwrap();
        assert_eq!(lax.hello.protocol_major(), Some(2));
    }

    #[test]
    fn stream_event_accepts_snake_and_camel_turn_id() {
        let ev: StreamEvent = serde_json::from_value(json!({
            "type": "turn_started", "turn_id": "t1"
        }))
        .unwrap();
        assert_eq!(ev.turn_id.as_deref(), Some("t1"));

        let lax: StreamEvent = serde_json::from_value(json!({
            "type": "turn_completed", "turnId": "t1", "usage": {"cost_usd": 0.1}
        }))
        .unwrap();
        assert_eq!(lax.turn_id.as_deref(), Some("t1"));
        assert_eq!(lax.usage().unwrap()["cost_usd"], 0.1);

        // round-trip keeps the wire shape (snake_case, "type" tag)
        let wire = serde_json::to_value(&lax).unwrap();
        assert_eq!(wire["type"], "turn_completed");
        assert_eq!(wire["turn_id"], "t1");
        assert!(wire.get("turnId").is_none());
    }

    #[test]
    fn permission_event_parses_ask() {
        let ev: StreamEvent = serde_json::from_value(json!({
            "type": "permission_requested",
            "turn_id": "t9",
            "id": "perm-1",
            "kind": "tool",
            "name": "Bash",
            "title": "bash prompt",
            "input": {"command": "ls"},
            "actions": [{"id": "a1", "label": "Allow", "behavior": "allow"}]
        }))
        .unwrap();
        let ask = ev.permission_request().unwrap();
        assert_eq!(ask.id, "perm-1");
        assert_eq!(ask.kind.as_deref(), Some("tool"));
        assert_eq!(ask.name.as_deref(), Some("Bash"));
        assert_eq!(ask.actions.len(), 1);

        assert!(
            StreamEvent {
                event_type: "turn_started".into(),
                ..serde_json::from_value::<StreamEvent>(json!({"type": "turn_started"})).unwrap()
            }
            .permission_request()
            .is_none()
        );
    }

    #[test]
    fn typed_error_code_mapping() {
        let map = |code: i64| {
            DamonError::from_rpc(RpcError {
                code,
                message: format!("msg {code}"),
                data: None,
            })
        };

        assert!(matches!(map(-32001), DamonError::SessionNotLive(_)));
        assert!(matches!(map(-32002), DamonError::CapabilityMissing(_)));
        assert!(matches!(map(-32003), DamonError::BackendNotRegistered(_)));
        assert!(matches!(map(-32004), DamonError::MaxSessionsReached));
        assert!(matches!(map(-32005), DamonError::ResponseTooLarge));
        assert!(matches!(map(-32006), DamonError::TurnAlreadyRunning));
        assert!(matches!(map(-32008), DamonError::AskNotFound(_)));
        assert!(matches!(map(-32602), DamonError::InvalidParams(_)));

        let untyped = map(-32000);
        assert_eq!(
            untyped,
            DamonError::Rpc {
                code: -32000,
                message: "msg -32000".into()
            }
        );
        assert_eq!(untyped.rpc_code(), Some(-32000));
        assert_eq!(
            map(-32003).rpc_code(),
            Some(error_code::BACKEND_NOT_REGISTERED)
        );
        assert_eq!(DamonError::DaemonNotRunning.rpc_code(), None);
    }

    #[test]
    fn method_dtos_are_camel_case() {
        let info: SessionInfo = serde_json::from_value(json!({
            "sessionId": "s1", "backend": "mock", "replayed": 3
        }))
        .unwrap();
        assert_eq!(info.session_id, "s1");
        assert_eq!(info.replayed, Some(3));

        let turn: TurnResult = serde_json::from_value(json!({
            "turnId": "t1", "stopReason": "completed", "usage": {"input_tokens": 3}
        }))
        .unwrap();
        assert!(turn.is_completed());
        assert_eq!(turn.turn_id, "t1");

        let row: SessionSummary = serde_json::from_value(json!({
            "sessionId": "s1", "backend": "mock", "title": "T", "createdAt": 1770000000000i64,
            "cwd": "/tmp", "tags": [], "pinned": true, "archived": false
        }))
        .unwrap();
        assert_eq!(row.created_at, Some(1770000000000i64));
        assert_eq!(row.pinned, Some(true));

        let back = serde_json::to_value(&row).unwrap();
        assert_eq!(back["sessionId"], "s1"); // camelCase back out
    }
}
