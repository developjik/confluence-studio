//! Confluence Cloud REST client (v2 API + v1 attachments).
//!
//! Endpoints are resolved against the normalized site origin:
//! <base_url>/wiki/api/v2/... (a "/wiki" suffix on the configured base_url is
//! stripped to avoid duplication). Pagination follows _links.next cursors.

use crate::confluence::convert;
use crate::confluence::models::{
    Attachment, ConfluenceAuth, PageBrief, PageDetail, PageStorage, PageTreeNode, PageVersion,
    SpaceSummary, TestResult,
};
use reqwest::Method;
use serde::{Deserialize, Serialize};

/// User-facing errors (converted to a plain string at the Tauri boundary).
#[derive(Debug, thiserror::Error)]
pub enum ConfluenceError {
    #[error("인증 실패(이메일/API 토큰 확인)")]
    Auth,
    #[error("페이지/스페이스 없음")]
    NotFound,
    #[error("Confluence 오류 ({status}): {summary}")]
    Api { status: u16, summary: String },
    #[error("네트워크 오류: {0}")]
    Network(String),
    #[error("응답 파싱 오류: {0}")]
    Parse(String),
}

pub type Result<T> = std::result::Result<T, ConfluenceError>;

/// Normalize a configured base URL to the site origin (no trailing "/", no "/wiki").
fn normalize_base(url: &str) -> String {
    let mut s = url.trim().trim_end_matches('/').to_string();
    if let Some(stripped) = s.strip_suffix("/wiki") {
        s = stripped.trim_end_matches('/').to_string();
    }
    s
}

pub struct ConfluenceClient {
    base: String,
    email: String,
    token: String,
    http: reqwest::Client,
}

impl ConfluenceClient {
    pub fn new(auth: ConfluenceAuth) -> Self {
        Self {
            base: normalize_base(&auth.base_url),
            email: auth.email,
            token: auth.token,
            http: reqwest::Client::new(),
        }
    }

    /// Builder-style timeout override.
    pub fn with_timeout(mut self, secs: u64) -> Self {
        if let Ok(client) = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(secs))
            .build()
        {
            self.http = client;
        }
        self
    }

    fn request(&self, method: Method, url: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, url)
            .basic_auth(&self.email, Some(&self.token))
    }

    async fn send(&self, builder: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        let resp = builder
            .send()
            .await
            .map_err(|e| ConfluenceError::Network(e.to_string()))?;
        Self::check_status(resp).await
    }

    async fn check_status(resp: reqwest::Response) -> Result<reqwest::Response> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        let summary: String = body.chars().take(300).collect();
        match status.as_u16() {
            401 | 403 => Err(ConfluenceError::Auth),
            404 => Err(ConfluenceError::NotFound),
            other => Err(ConfluenceError::Api {
                status: other,
                summary,
            }),
        }
    }

    /// Resolve a _links value (absolute URL or site-relative path).
    fn resolve(&self, link: &str) -> String {
        if link.starts_with("http://") || link.starts_with("https://") {
            link.to_string()
        } else if link.starts_with('/') {
            format!("{}{}", self.base, link)
        } else {
            format!("{}/{}", self.base, link)
        }
    }

    fn v2(&self, path: &str) -> String {
        format!("{}/wiki/api/v2{}", self.base, path)
    }

    /// GET a paginated endpoint, following _links.next until exhausted.
    async fn paginate(&self, first_url: String) -> Result<Vec<serde_json::Value>> {
        let mut items = Vec::new();
        let mut next = Some(first_url);
        while let Some(url) = next {
            let resp = self.send(self.request(Method::GET, &url)).await?;
            let v: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| ConfluenceError::Parse(e.to_string()))?;
            if let Some(arr) = v.get("results").and_then(|r| r.as_array()) {
                items.extend(arr.iter().cloned());
            }
            let next_link = v
                .pointer("/_links/next")
                .and_then(|n| n.as_str())
                .map(|l| self.resolve(l));
            if next_link.as_deref() == Some(url.as_str()) {
                break; // guard against self-referential cursors
            }
            next = next_link;
        }
        Ok(items)
    }

    fn parse_items<T: for<'de> Deserialize<'de>>(items: Vec<serde_json::Value>) -> Result<Vec<T>> {
        items
            .into_iter()
            .map(|v| serde_json::from_value(v).map_err(|e| ConfluenceError::Parse(e.to_string())))
            .collect()
    }

    /// Connectivity + credential check (GET /spaces?limit=1).
    pub async fn test(&self) -> Result<TestResult> {
        let url = format!("{}?limit=1", self.v2("/spaces"));
        let resp = self.send(self.request(Method::GET, &url)).await?;
        let server_time = resp
            .headers()
            .get(reqwest::header::DATE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        Ok(TestResult {
            ok: true,
            site_name: None,
            server_time,
        })
    }

    /// All spaces (cursor pagination, limit=50 per page).
    pub async fn list_spaces(&self) -> Result<Vec<SpaceSummary>> {
        let items = self
            .paginate(format!("{}?limit=50", self.v2("/spaces")))
            .await?;
        Self::parse_items(items)
    }

    async fn get_briefs(&self, path: &str) -> Result<Vec<PageBrief>> {
        let items = self.paginate(format!("{}?limit=50", self.v2(path))).await?;
        Self::parse_items(items)
    }

    /// Full page tree of a space. The returned root is a synthetic node for the
    /// space itself (id = spaceId, title = "", status = "space"); the top-level
    /// pages are its children.
    pub async fn page_tree(&self, space_id: &str) -> Result<PageTreeNode> {
        let tops = self
            .get_briefs(&format!("/spaces/{}/pages", space_id))
            .await?;
        let mut children = Vec::with_capacity(tops.len());
        for brief in tops {
            children.push(self.build_tree_node(brief).await?);
        }
        Ok(PageTreeNode {
            id: space_id.to_string(),
            title: String::new(),
            status: "space".to_string(),
            children,
        })
    }

    /// Recursive tree builder (boxed future: async fn cannot recurse directly).
    fn build_tree_node(
        &self,
        brief: PageBrief,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<PageTreeNode>> + Send + '_>>
    {
        Box::pin(async move {
            let briefs = self
                .get_briefs(&format!("/pages/{}/children", brief.id))
                .await?;
            let mut children = Vec::with_capacity(briefs.len());
            for b in briefs {
                children.push(self.build_tree_node(b).await?);
            }
            Ok(PageTreeNode {
                id: brief.id,
                title: brief.title,
                status: brief.status,
                children,
            })
        })
    }

    async fn get_page_json(&self, page_id: &str) -> Result<ApiPage> {
        let url = format!(
            "{}?body-format=storage",
            self.v2(&format!("/pages/{}", page_id))
        );
        let resp = self.send(self.request(Method::GET, &url)).await?;
        resp.json()
            .await
            .map_err(|e| ConfluenceError::Parse(e.to_string()))
    }

    /// Page with the storage body converted to markdown.
    pub async fn get_page(&self, page_id: &str) -> Result<PageDetail> {
        let p = self.get_page_json(page_id).await?;
        Ok(self.to_detail(p, String::new()))
    }

    /// Raw storage-format page (export support).
    pub async fn get_page_storage(&self, page_id: &str) -> Result<PageStorage> {
        let p = self.get_page_json(page_id).await?;
        Ok(PageStorage {
            storage: p
                .body
                .and_then(|b| b.storage)
                .and_then(|s| s.value)
                .unwrap_or_default(),
            id: p.id,
            title: p.title,
            version: p.version.and_then(|v| v.number).unwrap_or(0),
            parent_id: p.parent_id,
        })
    }

    /// All descendants of a page, in the order returned by the API.
    pub async fn get_descendants(&self, page_id: &str) -> Result<Vec<PageBrief>> {
        self.get_briefs(&format!("/pages/{}/descendants", page_id))
            .await
    }

    /// Create a page (markdown is converted to storage XHTML).
    pub async fn create_page(
        &self,
        space_id: &str,
        parent_id: Option<&str>,
        title: &str,
        markdown: &str,
    ) -> Result<PageDetail> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            representation: &'a str,
            value: String,
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct CreatePage<'a> {
            space_id: &'a str,
            status: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            parent_id: Option<&'a str>,
            title: &'a str,
            body: Body<'a>,
        }
        let payload = CreatePage {
            space_id,
            status: "current",
            parent_id,
            title,
            body: Body {
                representation: "storage",
                value: convert::markdown_to_storage(markdown),
            },
        };
        let resp = self
            .send(
                self.request(Method::POST, &self.v2("/pages"))
                    .json(&payload),
            )
            .await?;
        let p: ApiPage = resp
            .json()
            .await
            .map_err(|e| ConfluenceError::Parse(e.to_string()))?;
        Ok(self.to_detail(p, markdown.to_string()))
    }

    /// Update a page. version_number is the CURRENT version; version+1 is sent.
    pub async fn update_page(
        &self,
        page_id: &str,
        title: &str,
        markdown: &str,
        version_number: u32,
    ) -> Result<PageDetail> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            representation: &'a str,
            value: String,
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct UpdatePage<'a> {
            id: &'a str,
            status: &'a str,
            title: &'a str,
            body: Body<'a>,
            version: PageVersion,
        }
        let payload = UpdatePage {
            id: page_id,
            status: "current",
            title,
            body: Body {
                representation: "storage",
                value: convert::markdown_to_storage(markdown),
            },
            version: PageVersion {
                number: version_number + 1,
                message: None,
            },
        };
        let url = self.v2(&format!("/pages/{}", page_id));
        let resp = self
            .send(self.request(Method::PUT, &url).json(&payload))
            .await?;
        let p: ApiPage = resp
            .json()
            .await
            .map_err(|e| ConfluenceError::Parse(e.to_string()))?;
        Ok(self.to_detail(p, markdown.to_string()))
    }

    pub async fn delete_page(&self, page_id: &str) -> Result<()> {
        let url = self.v2(&format!("/pages/{}", page_id));
        self.send(self.request(Method::DELETE, &url)).await?;
        Ok(())
    }

    /// Attachments of a content id (v1 API, cursor pagination, limit=200).
    pub async fn list_attachments(&self, content_id: &str) -> Result<Vec<Attachment>> {
        let url = format!(
            "{}/wiki/rest/api/content/{}/child/attachment?limit=200",
            self.base, content_id
        );
        let items = self.paginate(url).await?;
        let raw: Vec<ApiAttachment> = Self::parse_items(items)?;
        Ok(raw
            .into_iter()
            .map(|a| Attachment {
                id: a.id.unwrap_or_default(),
                title: a.title.unwrap_or_default(),
                download_url: a.links.and_then(|l| l.download).unwrap_or_default(),
                media_type: a
                    .extensions
                    .as_ref()
                    .and_then(|e| e.get("mediaType"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
            .collect())
    }

    /// Download attachment bytes (download_url may be site-relative).
    pub async fn download_attachment(&self, download_url: &str) -> Result<Vec<u8>> {
        let url = self.resolve(download_url);
        let resp = self.send(self.request(Method::GET, &url)).await?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ConfluenceError::Network(e.to_string()))?;
        Ok(bytes.to_vec())
    }

    fn to_detail(&self, p: ApiPage, md_fallback: String) -> PageDetail {
        let (version_number, updated_at) = match p.version {
            Some(v) => (v.number.unwrap_or(0), v.created_at),
            None => (0, None),
        };
        let storage = p
            .body
            .as_ref()
            .and_then(|b| b.storage.as_ref())
            .and_then(|s| s.value.clone());
        let markdown = storage
            .map(|s| convert::storage_to_markdown(&s))
            .unwrap_or(md_fallback);
        let web_url = p
            .links
            .as_ref()
            .and_then(|l| l.webui.clone())
            .map(|w| self.web_url(&w))
            .unwrap_or_default();
        PageDetail {
            id: p.id,
            space_id: p.space_id.unwrap_or_default(),
            parent_id: p.parent_id,
            title: p.title,
            status: p.status,
            version: version_number,
            markdown,
            web_url,
            created_at: p.created_at,
            updated_at,
        }
    }

    fn web_url(&self, webui: &str) -> String {
        if webui.starts_with("http") {
            webui.to_string()
        } else if webui.starts_with("/wiki") {
            format!("{}{}", self.base, webui)
        } else {
            format!("{}/wiki{}", self.base, webui)
        }
    }
}

// ---------------------------------------------------------------------------
// internal API DTOs (lenient: everything optional/defaults)
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ApiPage {
    id: String,
    status: String,
    title: String,
    space_id: Option<String>,
    parent_id: Option<String>,
    created_at: Option<String>,
    version: Option<ApiVersion>,
    body: Option<ApiBody>,
    #[serde(rename = "_links")]
    links: Option<ApiLinks>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ApiVersion {
    number: Option<u32>,
    #[allow(dead_code)]
    message: Option<String>,
    created_at: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ApiBody {
    storage: Option<ApiStorage>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ApiStorage {
    value: Option<String>,
    #[allow(dead_code)]
    representation: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ApiLinks {
    #[allow(dead_code)]
    webui: Option<String>,
    #[allow(dead_code)]
    download: Option<String>,
    #[allow(dead_code)]
    next: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ApiAttachment {
    id: Option<String>,
    title: Option<String>,
    extensions: Option<serde_json::Value>,
    #[serde(rename = "_links")]
    links: Option<ApiLinks>,
}

#[cfg(test)]
mod tests {
    use super::normalize_base;

    #[test]
    fn normalizes_base_url() {
        assert_eq!(
            normalize_base("https://x.atlassian.net"),
            "https://x.atlassian.net"
        );
        assert_eq!(
            normalize_base("https://x.atlassian.net/"),
            "https://x.atlassian.net"
        );
        assert_eq!(
            normalize_base("https://x.atlassian.net/wiki"),
            "https://x.atlassian.net"
        );
        assert_eq!(
            normalize_base("https://x.atlassian.net/wiki/"),
            "https://x.atlassian.net"
        );
        assert_eq!(
            normalize_base("  https://x.atlassian.net/wiki/  "),
            "https://x.atlassian.net"
        );
    }
}
