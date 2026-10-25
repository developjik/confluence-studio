//! Damon daemon (protocol v2) WebSocket thin client.
//!
//! - [`protocol`]: wire frame model (request/response/push/hello, StreamEvent,
//!   typed error-code mapping).
//! - [`client`]: [`DamonClient`] — discovery, connection, multiplexed calls and
//!   the event broadcast.

pub mod client;
pub mod protocol;

pub use client::{DaemonEndpoint, DamonClient};
pub use protocol::{
    BackendStatus, DamonError, DamonPush, Hello, HelloInfo, PermissionBehavior, PermissionRequest,
    Push, Request, Response, RpcError, SessionInfo, SessionSummary, StreamEvent, TurnResult,
    error_code, stream_event_type, timeline_kind,
};
