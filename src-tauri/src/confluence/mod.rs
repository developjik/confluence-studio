//! Confluence Cloud REST client, models and storage<->markdown conversion.
//! Contract: docs/ARCHITECTURE.md § "Module: confluence". Filled in by the module pass.
//! (ConfluenceAuth may move into models.rs — it MUST stay reachable as
//!  crate::confluence::ConfluenceAuth because crate::credentials uses it.)

use serde::{Deserialize, Serialize};

/// Confluence Cloud credentials (in-process only; never serialized to the webview).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfluenceAuth {
    pub base_url: String,
    pub email: String,
    pub token: String,
}
