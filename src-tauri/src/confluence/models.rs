//! Public DTOs for the confluence module.
//! All DTOs serialize as camelCase JSON at the Tauri boundary (Golden rule 3).

use serde::{Deserialize, Serialize};

/// Confluence Cloud credentials (in-process only; never serialized to the webview).
/// Lives here but is re-exported as `crate::confluence::ConfluenceAuth`
/// because `crate::credentials` uses that path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfluenceAuth {
    pub base_url: String,
    pub email: String,
    pub token: String,
}

/// Result of `ConfluenceClient::test` (command: test_connection).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_time: Option<String>,
}

/// Space entry (command: list_spaces).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceSummary {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub name: String,
    pub r#type: String,
}

/// Node of a page tree (command: get_page_tree). The root node is a synthetic
/// node for the space itself (id = spaceId, status = "space").
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageTreeNode {
    pub id: String,
    pub title: String,
    pub status: String,
    pub children: Vec<PageTreeNode>,
}

/// Page with markdown body (commands: get_page / create_page / update_page).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageDetail {
    pub id: String,
    pub space_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub title: String,
    pub status: String,
    pub version: u32,
    pub markdown: String,
    pub web_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// Version payload used when updating a page (number = current + 1).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageVersion {
    pub number: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Minimal page descriptor used by trees and descendants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageBrief {
    pub id: String,
    pub title: String,
    pub status: String,
}

/// Raw storage-format page (export support).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageStorage {
    pub storage: String,
    pub id: String,
    pub title: String,
    pub version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
}

/// Attachment metadata (v1 API shape mapped to camelCase).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub title: String,
    pub download_url: String,
    pub media_type: String,
}
