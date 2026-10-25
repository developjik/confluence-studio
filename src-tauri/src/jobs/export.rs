//! Native export engine: tree fetch (via the injected `PageSource`),
//! markdown rendering, `NN - Title.md` layout, sha256 skip_unchanged,
//! cleanup_stale, attachment downloads and cancellation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

use super::JobsResult;
use super::fs_util;
use super::history;
use super::models::Job;

/// Hidden directory inside the export target holding state and history.
pub const STATE_DIR_NAME: &str = ".confluence-studio";
/// Attachment folder directly under the export target.
pub const ATTACHMENTS_DIR_NAME: &str = "attachments";
const STATE_FILE_NAME: &str = "state.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgress {
    pub message: String,
    pub done: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentRef {
    pub id: String,
    /// File name including extension, e.g. `screenshot.png`.
    pub title: String,
    #[serde(default)]
    pub media_type: String,
    #[serde(default)]
    pub download_url: String,
}

/// One page of the export tree, produced by a `PageSource`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PageData {
    pub id: String,
    pub title: String,
    /// Directory chain from the space name down to the immediate parent
    /// page (empty for a bare root). Also used for the breadcrumb line.
    pub parent_title_path: Vec<String>,
    /// Confluence storage-format (XHTML) body.
    pub storage: String,
    #[serde(default)]
    pub attachments: Vec<AttachmentRef>,
    #[serde(default)]
    pub version: Option<u32>,
    #[serde(default)]
    pub web_url: Option<String>,
}

/// Data source abstraction so the engine is testable with mocks and
/// decoupled from the Confluence client (ports the DI seam of `engine.py`).
pub trait PageSource {
    /// Flattened export tree in stable order (a parent always precedes its
    /// children; sibling order determines the `NN` numbering).
    fn fetch_tree(&mut self, job: &Job) -> JobsResult<Vec<PageData>>;
    fn fetch_attachment(&mut self, att: &AttachmentRef) -> JobsResult<Vec<u8>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Success,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExportOutcome {
    pub run_id: String,
    pub job_id: String,
    pub status: RunStatus,
    pub pages_written: u32,
    pub pages_skipped: u32,
    pub attachments_written: u32,
    pub files_removed: u32,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Running-job registry (cancel_job_run contract)
// ---------------------------------------------------------------------------

/// Per-job running set backing the `run_job` / `cancel_job_run` commands:
/// `try_start` enforces skip_if_running, `cancel` flips the run's cancel
/// flag, and the guard releases the slot when the run finishes.
#[derive(Clone, Default)]
pub struct RunningExports {
    inner: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

/// Ownership token for one running export; dropping it frees the slot.
pub struct RunGuard {
    job_id: String,
    flag: Arc<AtomicBool>,
    inner: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

impl RunningExports {
    pub fn new() -> Self {
        Self::default()
    }

    /// `None` when the job is already running (skip_if_running).
    pub fn try_start(&self, job_id: &str) -> Option<RunGuard> {
        let mut map = self.inner.lock();
        if map.contains_key(job_id) {
            return None;
        }
        let flag = Arc::new(AtomicBool::new(false));
        map.insert(job_id.to_string(), flag.clone());
        Some(RunGuard {
            job_id: job_id.to_string(),
            flag,
            inner: self.inner.clone(),
        })
    }

    pub fn is_running(&self, job_id: &str) -> bool {
        self.inner.lock().contains_key(job_id)
    }

    /// Request cancellation; returns whether a run was active.
    pub fn cancel(&self, job_id: &str) -> bool {
        let map = self.inner.lock();
        match map.get(job_id) {
            Some(flag) => {
                flag.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }
}

impl RunGuard {
    /// Cancel flag to pass to `run_export`.
    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.flag
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        let mut map = self.inner.lock();
        match map.get(&self.job_id) {
            Some(current) if Arc::ptr_eq(current, &self.flag) => {
                map.remove(&self.job_id);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Filename sanitizing (port of fs_policy.sanitize_component, max 120 chars,
// Windows-invalid characters and reserved names handled)
// ---------------------------------------------------------------------------

static INVALID_CHARS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"[<>:"/\\|?*\x00-\x1f]"#).unwrap());
static WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];
const MAX_COMPONENT_LEN: usize = 120;

pub fn sanitize_component(value: &str) -> String {
    let text = INVALID_CHARS.replace_all(value, "_");
    let text = WHITESPACE.replace_all(&text, " ");
    let text = text.trim_matches([' ', '.']);
    let mut text = if text.is_empty() { "_untitled" } else { text }.to_string();
    let stem = text.split('.').next().unwrap_or("").to_uppercase();
    if RESERVED_NAMES.contains(&stem.as_str()) {
        text.push('_');
    }
    if text.chars().count() > MAX_COMPONENT_LEN {
        let truncated: String = text.chars().take(MAX_COMPONENT_LEN).collect();
        text = truncated.trim_end_matches([' ', '.']).to_string();
        if text.is_empty() {
            text = "_untitled".to_string();
        }
    }
    text
}

// ---------------------------------------------------------------------------
// Minimal storage → markdown fallback
// ---------------------------------------------------------------------------

/// INTEGRATION POINT (for the integration pass): switch this function body
/// to `crate::confluence::convert::storage_to_markdown(storage)` now that
/// the confluence module provides it, and drop the local fallback. The
/// fallback performs a paragraph-level strip (tags removed, block
/// boundaries kept) so the jobs module and its tests stay independent of
/// the confluence module while it lands (per the module contract).
fn storage_to_markdown(storage: &str) -> String {
    let mut stripped = String::with_capacity(storage.len());
    let mut in_tag = false;
    for ch in storage.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => stripped.push(c),
            _ => {}
        }
    }
    // Entity decoding: &amp; is decoded last so "&amp;lt;" ends up as the
    // literal text "&lt;" instead of being double-decoded to "<".
    let decoded = stripped
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    // Normalize whitespace: trim line ends and collapse 3+ blank lines.
    let mut text = String::with_capacity(decoded.len());
    let mut blank = 0;
    for line in decoded.split('\n') {
        let line = line.trim_end();
        if line.is_empty() {
            blank += 1;
            if blank <= 1 {
                text.push('\n');
            }
        } else {
            blank = 0;
            text.push_str(line);
            text.push('\n');
        }
    }
    text.trim().to_string()
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// Stable content signature of a page: everything that determines the
/// rendered file *except* the run-varying `exported_at` timestamp, so that
/// skip_unchanged can detect unchanged pages across runs. (The exported_at
/// in an on-disk file therefore reflects the run that last wrote it.)
fn page_signature(page: &PageData, markdown: &str) -> String {
    let mut signature = String::new();
    signature.push_str(&page.id);
    signature.push('\u{0}');
    signature.push_str(&page.title);
    signature.push('\u{0}');
    if let Some(version) = page.version {
        signature.push_str(&version.to_string());
    }
    signature.push('\u{0}');
    if let Some(url) = &page.web_url {
        signature.push_str(url);
    }
    signature.push('\u{0}');
    signature.push_str(&page.parent_title_path.join("/"));
    signature.push('\u{0}');
    signature.push_str(markdown);
    sha256_hex(signature.as_bytes())
}

fn render_page_file(
    job: &Job,
    page: &PageData,
    markdown: &str,
    exported_at: DateTime<Utc>,
) -> String {
    let mut out = String::new();
    if job.export_options.front_matter {
        out.push_str("---\n");
        out.push_str(&format!("title: {:?}\n", page.title));
        out.push_str(&format!("page_id: {}\n", page.id));
        if let Some(url) = &page.web_url {
            out.push_str(&format!("url: {:?}\n", url));
        }
        if let Some(version) = page.version {
            out.push_str(&format!("version: {}\n", version));
        }
        out.push_str(&format!("exported_at: {}\n", exported_at.to_rfc3339()));
        out.push_str("---\n\n");
    }
    if job.export_options.page_breadcrumbs && !page.parent_title_path.is_empty() {
        out.push_str(&format!("> {}\n\n", page.parent_title_path.join(" > ")));
    }
    out.push_str(markdown);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------------------
// Export state (skip_unchanged / cleanup_stale bookkeeping)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
enum StateKind {
    #[serde(rename = "page")]
    Page,
    #[serde(rename = "attachment")]
    Attachment,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct StateEntry {
    kind: StateKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    page_id: Option<String>,
    /// Pages: `page_signature` (run-independent); attachments: sha256 of the
    /// file bytes.
    hash: String,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StateFile {
    schema_version: u32,
    /// relative path ('/'-separated) → entry
    #[serde(default)]
    entries: HashMap<String, StateEntry>,
}

impl StateFile {
    fn load(state_dir: &Path) -> Self {
        let path = state_dir.join(STATE_FILE_NAME);
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    fn persist(&self, state_dir: &Path) -> std::io::Result<()> {
        let file = StateFile {
            schema_version: 1,
            entries: self.entries.clone(),
        };
        let mut text = serde_json::to_string_pretty(&file).map_err(io_from_serde)?;
        text.push('\n');
        fs_util::atomic_write_text(&state_dir.join(STATE_FILE_NAME), &text)
    }
}

fn io_from_serde(err: serde_json::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string())
}

fn relative_path(parts: &[String], file_name: &str) -> String {
    let mut segments: Vec<String> = parts.iter().map(|s| sanitize_component(s)).collect();
    segments.push(file_name.to_string());
    segments.join("/")
}

/// Delete now-empty parent directories of `path` up to (excluding) `root`.
async fn prune_empty_dirs(path: &Path, root: &Path) {
    let mut current = path.parent().map(Path::to_path_buf);
    while let Some(dir) = current {
        if dir == root || dir == Path::new("/") {
            break;
        }
        match tokio::fs::remove_dir(&dir).await {
            Ok(()) => current = dir.parent().map(Path::to_path_buf),
            Err(_) => break,
        }
    }
}

// ---------------------------------------------------------------------------
// Export engine
// ---------------------------------------------------------------------------

/// Run one export to completion. Never panics on source/IO errors: failures
/// are captured in the returned `ExportOutcome` (status `failed`), which is
/// also appended to the run history. `cancel` is polled between pages and
/// before each attachment download.
pub async fn run_export(
    job: &Job,
    source: &mut dyn PageSource,
    progress: &mut dyn FnMut(ExportProgress),
    cancel: &AtomicBool,
) -> ExportOutcome {
    let started_at = Utc::now();
    let suffix: String = uuid::Uuid::new_v4().simple().to_string()[..6].to_string();
    let run_id = format!(
        "{}-{}-{}",
        started_at.format("%Y%m%d-%H%M%S"),
        job.id,
        suffix
    );
    let mut outcome = ExportOutcome {
        run_id,
        job_id: job.id.clone(),
        status: RunStatus::Success,
        pages_written: 0,
        pages_skipped: 0,
        attachments_written: 0,
        files_removed: 0,
        started_at,
        finished_at: started_at,
        error: None,
    };

    let target = PathBuf::from(&job.target_dir);
    let state_dir = target.join(STATE_DIR_NAME);
    if let Err(err) = tokio::fs::create_dir_all(&state_dir).await {
        let msg = format!("대상 폴더를 만들 수 없습니다: {err}");
        return finish_export(outcome, Some(msg), None, &target).await;
    }

    progress(ExportProgress {
        message: "페이지 트리를 가져오는 중…".to_string(),
        done: 0,
        total: 0,
    });
    let pages = match source.fetch_tree(job) {
        Ok(pages) => pages,
        Err(err) => {
            let msg = err.to_string();
            return finish_export(outcome, Some(msg), None, &target).await;
        }
    };

    let prior_state = StateFile::load(&state_dir);
    let mut live: HashMap<String, StateEntry> = HashMap::new();
    let mut sibling_numbering: HashMap<Vec<String>, u32> = HashMap::new();
    let total = pages.len() as u32;
    let mut status = RunStatus::Success;
    let mut error: Option<String> = None;

    for (index, page) in pages.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            status = RunStatus::Cancelled;
            break;
        }

        // "NN - Title.md" — NN is the 1-based position among siblings sharing
        // the same parent chain, assigned in fetch order.
        let number = {
            let counter = sibling_numbering
                .entry(page.parent_title_path.clone())
                .or_insert(0);
            *counter += 1;
            *counter
        };
        let file_stem = format!("{:02} - {}", number, sanitize_component(&page.title));
        let rel = relative_path(&page.parent_title_path, &format!("{file_stem}.md"));
        let abs = target.join(&rel);

        let markdown = storage_to_markdown(&page.storage);
        let content = render_page_file(job, page, &markdown, started_at);
        let hash = page_signature(page, &markdown);

        let unchanged = job.export_options.skip_unchanged
            && prior_state
                .entries
                .get(&rel)
                .is_some_and(|e| e.hash == hash)
            && tokio::fs::try_exists(&abs).await.unwrap_or(false);

        if unchanged {
            outcome.pages_skipped += 1;
        } else {
            let bytes = content.into_bytes();
            if let Err(err) = fs_util::atomic_write_bytes_async(&abs, bytes).await {
                status = RunStatus::Failed;
                error = Some(format!("파일 저장 실패 ({}): {err}", page.title));
                break;
            }
            outcome.pages_written += 1;
        }
        live.insert(
            rel,
            StateEntry {
                kind: StateKind::Page,
                page_id: Some(page.id.clone()),
                hash,
                updated_at: started_at,
            },
        );

        if job.include_attachments {
            for att in &page.attachments {
                if cancel.load(Ordering::SeqCst) {
                    status = RunStatus::Cancelled;
                    break;
                }
                let att_name = format!(
                    "{} - {}",
                    sanitize_component(&att.id),
                    sanitize_component(&att.title)
                );
                let att_rel = relative_path(&[ATTACHMENTS_DIR_NAME.to_string()], &att_name);
                let att_abs = target.join(&att_rel);
                let bytes = match source.fetch_attachment(att) {
                    Ok(bytes) => bytes,
                    // A single failing attachment does not fail the export.
                    Err(err) => {
                        progress(ExportProgress {
                            message: format!("첨부 {} 건너뜀: {err}", att.title),
                            done: index as u32 + 1,
                            total,
                        });
                        continue;
                    }
                };
                let hash = sha256_hex(&bytes);
                let unchanged = job.export_options.skip_unchanged
                    && prior_state
                        .entries
                        .get(&att_rel)
                        .is_some_and(|e| e.hash == hash)
                    && tokio::fs::try_exists(&att_abs).await.unwrap_or(false);
                if !unchanged {
                    if let Err(err) = fs_util::atomic_write_bytes_async(&att_abs, bytes).await {
                        status = RunStatus::Failed;
                        error = Some(format!("첨부 저장 실패 ({}): {err}", att.title));
                        break;
                    }
                    outcome.attachments_written += 1;
                }
                live.insert(
                    att_rel,
                    StateEntry {
                        kind: StateKind::Attachment,
                        page_id: Some(page.id.clone()),
                        hash,
                        updated_at: started_at,
                    },
                );
            }
            if status != RunStatus::Success {
                break;
            }
        }

        progress(ExportProgress {
            message: format!("'{}' 내보냄", page.title),
            done: index as u32 + 1,
            total,
        });
    }

    if status == RunStatus::Success && job.export_options.cleanup_stale {
        // Remove files we previously wrote whose page/attachment no longer
        // appears in this run (remote deleted or moved), then drop their
        // state entries. Files we never tracked are left untouched.
        let stale: Vec<String> = prior_state
            .entries
            .keys()
            .filter(|rel| !live.contains_key(*rel))
            .cloned()
            .collect();
        for rel in stale {
            let abs = target.join(&rel);
            if abs.strip_prefix(&target).is_err() {
                continue; // defensive: never delete outside the target
            }
            if tokio::fs::remove_file(&abs).await.is_ok() {
                outcome.files_removed += 1;
                prune_empty_dirs(&abs, &target).await;
            }
        }
    }

    let mut final_state = prior_state;
    if status == RunStatus::Success {
        final_state.entries = live;
    } else {
        // Cancelled/failed runs keep prior entries for pages not reached.
        for (rel, entry) in live {
            final_state.entries.insert(rel, entry);
        }
    }

    outcome.status = status;
    finish_export(outcome, error, Some(final_state), &target).await
}

/// Stamp finish time, persist state (when provided) and append the run to
/// the history — both inside `spawn_blocking` so the async runtime stays
/// free of file I/O — then return the outcome. Failed and cancelled runs
/// are recorded in the history too.
async fn finish_export(
    mut outcome: ExportOutcome,
    error: Option<String>,
    state: Option<StateFile>,
    target: &Path,
) -> ExportOutcome {
    if error.is_some() {
        outcome.status = RunStatus::Failed;
    }
    outcome.error = error;
    outcome.finished_at = Utc::now();

    let state_dir = target.join(STATE_DIR_NAME);
    let target = target.to_path_buf();
    let history_outcome = outcome.clone();
    let _ = tokio::task::spawn_blocking(move || {
        if let Some(state) = state
            && let Err(err) = state.persist(&state_dir)
        {
            eprintln!("state.json 저장 실패: {err}");
        }
        if let Err(err) = history::append_run(&target, &history_outcome) {
            eprintln!("실행 이력 저장 실패: {err}");
        }
    })
    .await;
    outcome
}
