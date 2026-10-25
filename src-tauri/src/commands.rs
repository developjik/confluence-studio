//! Tauri command wiring (integration layer).
//!
//! Implements the command/event contract of docs/ARCHITECTURE.md §
//! "Tauri command contract" on top of the `confluence`, `jobs`, `damon`
//! and `credentials` modules. Commands are thin: they build clients,
//! forward to module APIs and emit the documented events.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{broadcast, watch};

use crate::confluence::{ConfluenceClient, ConfluenceError, PageDetail, PageTreeNode, TestResult};
use crate::credentials::{self, ConnectionInfo};
use crate::damon::{DamonClient, DamonError, PermissionBehavior, Push};
use crate::jobs::{
    AttachmentRef, ExportOptions, ExportOutcome, ExportProgress, Job, PageData, PageSource,
    RunGuard, RunStatus, Schedule, Scheduler, append_run, list_runs, run_export,
};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Event names + payload DTOs (backend → frontend emit)
// ---------------------------------------------------------------------------

pub const EVENT_EXPORT_PROGRESS: &str = "export://progress";
pub const EVENT_EXPORT_RUN_FINISHED: &str = "export://run-finished";
pub const EVENT_DAMON_EVENT: &str = "damon://event";
pub const EVENT_DAMON_TURN_DONE: &str = "damon://turn-done";
pub const EVENT_CONFLUENCE_CHANGED: &str = "confluence://changed";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgressEvent {
    pub job_id: String,
    pub run_id: String,
    pub message: String,
    pub done: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRunFinishedEvent {
    pub job_id: String,
    pub run_id: String,
    pub status: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfluenceChangedEvent {
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Run a blocking keychain operation off the async runtime.
async fn keyring<T, F>(f: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, credentials::CredentialError> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Load full credentials (keychain) and build a fresh Confluence client —
/// the contract requires a per-call client.
async fn confluence_client() -> Result<ConfluenceClient, String> {
    let auth = keyring(credentials::load_full).await?;
    Ok(ConfluenceClient::new(auth))
}

fn emit_confluence_changed(
    app: &AppHandle,
    kind: &'static str,
    page_id: Option<String>,
    space_id: Option<String>,
) {
    let _ = app.emit(
        EVENT_CONFLUENCE_CHANGED,
        ConfluenceChangedEvent {
            kind,
            page_id,
            space_id,
        },
    );
}

/// Run id in the `run_export` format (`YYYYMMDD-HHMMSS-<jobId>-<hex6>`).
/// Used for the `run_job` return value and progress events; the history
/// entry minted inside `run_export` keeps its own id of the same shape.
fn new_run_id(job_id: &str) -> String {
    let suffix: String = uuid::Uuid::new_v4().simple().to_string()[..6].to_string();
    format!(
        "{}-{}-{}",
        Utc::now().format("%Y%m%d-%H%M%S"),
        job_id,
        suffix
    )
}

fn synthetic_failed_outcome(job_id: &str, run_id: &str, message: String) -> ExportOutcome {
    let now = Utc::now();
    ExportOutcome {
        run_id: run_id.to_string(),
        job_id: job_id.to_string(),
        status: RunStatus::Failed,
        pages_written: 0,
        pages_skipped: 0,
        attachments_written: 0,
        files_removed: 0,
        started_at: now,
        finished_at: now,
        error: Some(message),
    }
}

fn summarize_outcome(outcome: &ExportOutcome) -> String {
    let label = match outcome.status {
        RunStatus::Success => "성공",
        RunStatus::Failed => "실패",
        RunStatus::Cancelled => "취소됨",
    };
    let mut text = format!(
        "{label}: 페이지 {}개 작성, {}개 건너뜀",
        outcome.pages_written, outcome.pages_skipped
    );
    if outcome.attachments_written > 0 {
        text.push_str(&format!(", 첨부 {}", outcome.attachments_written));
    }
    if outcome.files_removed > 0 {
        text.push_str(&format!(", 정리 {}", outcome.files_removed));
    }
    if let Some(err) = &outcome.error {
        text.push_str(&format!(" — {err}"));
    }
    text
}

async fn record_history(target_dir: &str, outcome: &ExportOutcome) {
    let dir = PathBuf::from(target_dir);
    let outcome = outcome.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || append_run(&dir, &outcome)).await;
}

// ---------------------------------------------------------------------------
// Export execution (shared by run_job and the scheduler)
// ---------------------------------------------------------------------------

/// Run one export in the background (the caller already holds the
/// `RunGuard` from `AppState::running`). Streams `export://progress` and
/// finishes with `export://run-finished`. `run_export` records the history
/// entry itself; start-up failures are recorded here so they stay visible.
async fn execute_export(
    state: Arc<AppState>,
    job: Job,
    guard: RunGuard,
    run_id: String,
) -> ExportOutcome {
    let app = state.app.clone();
    let mut progress = {
        let app = app.clone();
        let job_id = job.id.clone();
        let run_id = run_id.clone();
        move |p: ExportProgress| {
            let _ = app.emit(
                EVENT_EXPORT_PROGRESS,
                ExportProgressEvent {
                    job_id: job_id.clone(),
                    run_id: run_id.clone(),
                    message: p.message,
                    done: p.done,
                    total: p.total,
                },
            );
        }
    };

    let outcome = match credentials::load_full() {
        Ok(auth) => {
            let mut source =
                ConfluencePageSource::new(ConfluenceClient::new(auth), job.include_attachments);
            run_export(&job, &mut source, &mut progress, guard.cancel_flag()).await
        }
        Err(err) => {
            let outcome = synthetic_failed_outcome(
                &job.id,
                &run_id,
                format!("연결 정보를 불러올 수 없습니다: {err}"),
            );
            record_history(&job.target_dir, &outcome).await;
            outcome
        }
    };

    emit_run_finished(&app, &outcome);
    outcome
}

fn emit_run_finished(app: &AppHandle, outcome: &ExportOutcome) {
    let status = serde_json::to_value(outcome.status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "failed".to_string());
    let _ = app.emit(
        EVENT_EXPORT_RUN_FINISHED,
        ExportRunFinishedEvent {
            job_id: outcome.job_id.clone(),
            run_id: outcome.run_id.clone(),
            status,
            summary: summarize_outcome(outcome),
        },
    );
}

/// Start one export on a dedicated runtime thread. `run_export`'s progress
/// callback parameter is a non-Send `&mut dyn FnMut`, which makes its future
/// non-Send — it cannot be spawned on the shared async runtime, so each run
/// gets its own small multi-thread runtime on a dedicated thread (which
/// also satisfies the `block_in_place` bridge used by
/// `ConfluencePageSource`). Events and history still flow from there.
fn start_export(state: Arc<AppState>, job: Job, guard: RunGuard, run_id: String) {
    let thread_state = state.clone();
    let thread_job = job.clone();
    let thread_run_id = run_id.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("export-{}", job.id))
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build();
            match runtime {
                Ok(runtime) => {
                    runtime.block_on(execute_export(
                        thread_state,
                        thread_job,
                        guard,
                        thread_run_id,
                    ));
                }
                Err(err) => {
                    eprintln!("export 실행 런타임 생성 실패: {err}");
                    emit_run_finished(
                        &thread_state.app,
                        &synthetic_failed_outcome(
                            &thread_job.id,
                            &thread_run_id,
                            format!("export 런타임 생성 실패: {err}"),
                        ),
                    );
                }
            }
        });
    if spawned.is_err() {
        let outcome = synthetic_failed_outcome(
            &job.id,
            &run_id,
            "백그라운드 export 스레드를 시작할 수 없습니다".to_string(),
        );
        emit_run_finished(&state.app, &outcome);
    }
}

/// Spawn the scheduler background loop (30s tick). Scheduled runs register
/// in `AppState::running` (shared with manual runs and `cancel_job_run`),
/// while the tick-local `scheduler_running` registry implements
/// skip_if_running between scheduled dispatches.
pub fn spawn_scheduler(state: Arc<AppState>) {
    // Keep the shutdown sender alive for the whole app lifetime (dropping
    // it would stop the loop); the process exit stops it in practice.
    static SHUTDOWN: OnceLock<watch::Sender<bool>> = OnceLock::new();

    let jobs = state.jobs.clone();
    let load_jobs = move || jobs.lock().list();

    let runner_state = state.clone();
    let runner = move |job: Job, _scheduled: DateTime<Utc>| {
        let state = runner_state.clone();
        async move {
            let Some(guard) = state.running.try_start(&job.id) else {
                // A manual (or other) run of this job is already active —
                // skip silently like the original skip_if_running.
                return synthetic_failed_outcome(
                    &job.id,
                    "",
                    "이미 실행 중인 잡입니다 (건너뜀)".to_string(),
                );
            };
            let run_id = new_run_id(&job.id);
            start_export(state.clone(), job.clone(), guard, run_id.clone());
            // Dispatched — the run reports progress/outcome via events and
            // history from its own thread; tick_once ignores this value.
            synthetic_failed_outcome(&job.id, &run_id, String::new())
        }
    };

    let (tx, rx) = watch::channel(false);
    let _ = SHUTDOWN.set(tx);
    tauri::async_runtime::spawn(Scheduler::run_loop(
        load_jobs,
        runner,
        Duration::from_secs(30),
        rx,
        state.scheduler_running.clone(),
    ));
}

// ---------------------------------------------------------------------------
// PageSource: Confluence-backed export data source
// ---------------------------------------------------------------------------

/// `PageSource` over the real Confluence client. The trait is synchronous
/// (mockable by design), so the async HTTP calls are driven through
/// `block_in_place` on the multi-threaded Tauri runtime.
struct ConfluencePageSource {
    client: ConfluenceClient,
    include_attachments: bool,
}

impl ConfluencePageSource {
    fn new(client: ConfluenceClient, include_attachments: bool) -> Self {
        Self {
            client,
            include_attachments,
        }
    }

    fn cf(err: ConfluenceError) -> crate::jobs::JobsError {
        crate::jobs::JobsError::export(err.to_string())
    }
}

fn find_node<'a>(node: &'a PageTreeNode, id: &str) -> Option<&'a PageTreeNode> {
    if node.id == id {
        return Some(node);
    }
    node.children.iter().find_map(|c| find_node(c, id))
}

impl PageSource for ConfluencePageSource {
    fn fetch_tree(&mut self, job: &Job) -> crate::jobs::JobsResult<Vec<PageData>> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                // Space name heads the export directory chain.
                let spaces = self.client.list_spaces().await.map_err(Self::cf)?;
                let space_name = spaces
                    .iter()
                    .find(|s| s.id == job.space_id)
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| job.space_id.clone());

                let tree = self
                    .client
                    .page_tree(&job.space_id)
                    .await
                    .map_err(Self::cf)?;

                // Explicit stack of (node, parent chain): iterative DFS keeps
                // parents before children and siblings in order.
                let mut stack: Vec<(&PageTreeNode, Vec<String>)> = Vec::new();
                match job
                    .root_page_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    Some(root_id) => {
                        let node = find_node(&tree, root_id).ok_or_else(|| {
                            crate::jobs::JobsError::export(format!(
                                "루트 페이지({root_id})를 스페이스 트리에서 찾을 수 없습니다"
                            ))
                        })?;
                        // Bare root: exported at the target root.
                        stack.push((node, Vec::new()));
                    }
                    None => {
                        // Synthetic space root: export its children under
                        // the space-name folder.
                        for child in tree.children.iter().rev() {
                            stack.push((child, vec![space_name.clone()]));
                        }
                    }
                }

                let mut pages = Vec::new();
                while let Some((node, path)) = stack.pop() {
                    let storage = self
                        .client
                        .get_page_storage(&node.id)
                        .await
                        .map_err(Self::cf)?;
                    let attachments = if self.include_attachments {
                        self.client
                            .list_attachments(&node.id)
                            .await
                            .map_err(Self::cf)?
                            .into_iter()
                            .map(|a| AttachmentRef {
                                id: a.id,
                                title: a.title,
                                media_type: a.media_type,
                                download_url: a.download_url,
                            })
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    };
                    pages.push(PageData {
                        id: node.id.clone(),
                        title: node.title.clone(),
                        parent_title_path: path.clone(),
                        storage: storage.storage,
                        attachments,
                        version: Some(storage.version),
                        web_url: None,
                    });
                    let mut child_path = path;
                    child_path.push(node.title.clone());
                    for child in node.children.iter().rev() {
                        stack.push((child, child_path.clone()));
                    }
                }
                Ok(pages)
            })
        })
    }

    fn fetch_attachment(&mut self, att: &AttachmentRef) -> crate::jobs::JobsResult<Vec<u8>> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(self.client.download_attachment(&att.download_url))
        })
        .map_err(Self::cf)
    }
}

// ---------------------------------------------------------------------------
// Commands: connection
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn save_connection(base_url: String, email: String, token: String) -> Result<(), String> {
    keyring(move || credentials::save(&base_url, &email, &token)).await
}

#[tauri::command]
pub async fn get_connection() -> Result<Option<ConnectionInfo>, String> {
    keyring(credentials::load_info).await
}

#[tauri::command]
pub async fn test_connection() -> Result<TestResult, String> {
    let client = confluence_client().await?;
    client.test().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn clear_connection() -> Result<(), String> {
    keyring(credentials::clear).await
}

// ---------------------------------------------------------------------------
// Commands: spaces / pages
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_spaces() -> Result<Vec<crate::confluence::SpaceSummary>, String> {
    let client = confluence_client().await?;
    client.list_spaces().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_page_tree(space_id: String) -> Result<PageTreeNode, String> {
    let client = confluence_client().await?;
    client.page_tree(&space_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_page(page_id: String) -> Result<PageDetail, String> {
    let client = confluence_client().await?;
    client.get_page(&page_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_page(
    app: AppHandle,
    space_id: String,
    parent_id: Option<String>,
    title: String,
    markdown: String,
) -> Result<PageDetail, String> {
    let client = confluence_client().await?;
    let detail = client
        .create_page(&space_id, parent_id.as_deref(), &title, &markdown)
        .await
        .map_err(|e| e.to_string())?;
    emit_confluence_changed(
        &app,
        "created",
        Some(detail.id.clone()),
        Some(detail.space_id.clone()),
    );
    Ok(detail)
}

#[tauri::command]
pub async fn update_page(
    app: AppHandle,
    page_id: String,
    title: String,
    markdown: String,
    version: u32,
) -> Result<PageDetail, String> {
    let client = confluence_client().await?;
    let detail = client
        .update_page(&page_id, &title, &markdown, version)
        .await
        .map_err(|e| e.to_string())?;
    emit_confluence_changed(
        &app,
        "updated",
        Some(detail.id.clone()),
        Some(detail.space_id.clone()),
    );
    Ok(detail)
}

#[tauri::command]
pub async fn delete_page(app: AppHandle, page_id: String) -> Result<(), String> {
    let client = confluence_client().await?;
    client
        .delete_page(&page_id)
        .await
        .map_err(|e| e.to_string())?;
    emit_confluence_changed(&app, "deleted", Some(page_id), None);
    Ok(())
}

// ---------------------------------------------------------------------------
// Commands: jobs / scheduling
// ---------------------------------------------------------------------------

/// `save_job` payload — mirrors [`Job`] but tolerates a missing/empty `id`
/// (the store mints a uuid for new jobs), matching the frontend `JobInput`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobInputDto {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub space_id: String,
    #[serde(default)]
    pub root_page_id: Option<String>,
    #[serde(default)]
    pub target_dir: String,
    #[serde(default)]
    pub include_attachments: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub export_options: ExportOptions,
    #[serde(default)]
    pub schedule: Schedule,
}

fn default_true() -> bool {
    true
}

impl From<JobInputDto> for Job {
    fn from(dto: JobInputDto) -> Self {
        Job {
            id: dto.id.trim().to_string(),
            name: dto.name,
            space_id: dto.space_id,
            root_page_id: dto
                .root_page_id
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            target_dir: dto.target_dir,
            include_attachments: dto.include_attachments,
            enabled: dto.enabled,
            export_options: dto.export_options,
            schedule: dto.schedule,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunJobResult {
    pub run_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelJobRunResult {
    pub cancelled: bool,
}

#[tauri::command]
pub async fn list_jobs(state: State<'_, Arc<AppState>>) -> Result<Vec<Job>, String> {
    Ok(state.jobs.lock().list())
}

#[tauri::command]
pub async fn save_job(state: State<'_, Arc<AppState>>, job: JobInputDto) -> Result<Job, String> {
    let job: Job = job.into();
    let store = state.jobs.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut store = store.lock();
        store.save(&job).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn delete_job(state: State<'_, Arc<AppState>>, job_id: String) -> Result<(), String> {
    let store = state.jobs.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut store = store.lock();
        store.delete(&job_id).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(())
}

#[tauri::command]
pub async fn run_job(
    state: State<'_, Arc<AppState>>,
    job_id: String,
) -> Result<RunJobResult, String> {
    let job = {
        let store = state.jobs.lock();
        store
            .get(&job_id)
            .ok_or_else(|| "잡을 찾을 수 없습니다".to_string())?
    };
    let guard = state
        .running
        .try_start(&job_id)
        .ok_or_else(|| "이미 실행 중인 잡입니다".to_string())?;
    let run_id = new_run_id(&job_id);
    start_export(state.inner().clone(), job, guard, run_id.clone());
    Ok(RunJobResult { run_id })
}

#[tauri::command]
pub async fn cancel_job_run(
    state: State<'_, Arc<AppState>>,
    job_id: String,
) -> Result<CancelJobRunResult, String> {
    Ok(CancelJobRunResult {
        cancelled: state.running.cancel(&job_id),
    })
}

#[tauri::command]
pub async fn list_job_runs(
    state: State<'_, Arc<AppState>>,
    job_id: String,
    limit: Option<u32>,
) -> Result<Vec<ExportOutcome>, String> {
    let target = {
        let store = state.jobs.lock();
        let job = store
            .get(&job_id)
            .ok_or_else(|| "잡을 찾을 수 없습니다".to_string())?;
        PathBuf::from(job.target_dir.clone())
    };
    let mut runs = tauri::async_runtime::spawn_blocking(move || list_runs(&target))
        .await
        .map_err(|e| e.to_string())?;
    runs.retain(|r| r.job_id == job_id);
    runs.reverse(); // newest first for the history table
    if let Some(limit) = limit {
        runs.truncate(limit as usize);
    }
    Ok(runs)
}

// ---------------------------------------------------------------------------
// Commands: Damon
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DamonBackendDto {
    pub id: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DamonStatusDto {
    pub running: bool,
    pub url: Option<String>,
    pub version: Option<String>,
    pub backends: Vec<DamonBackendDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DamonSessionDto {
    pub session_id: String,
    pub title: String,
    pub backend: String,
    pub created_at: String,
    pub cwd: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DamonCreateSessionDto {
    pub session_id: String,
    pub backend: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DamonCancelledDto {
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DamonDeletedDto {
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DamonRenamedDto {
    pub renamed: bool,
}

/// Connect to the daemon on demand (discover → connect) and cache the
/// client. A cached-but-closed client is replaced. Every new connection
/// gets exactly one subscription pump emitting `damon://event` /
/// `damon://turn-done`.
async fn ensure_damon(state: &Arc<AppState>) -> Result<Arc<DamonClient>, String> {
    {
        let guard = state.damon.read().await;
        if let Some(client) = guard.as_ref()
            && !client.is_closed()
        {
            return Ok(client.clone());
        }
    }
    let mut guard = state.damon.write().await;
    if let Some(client) = guard.as_ref()
        && !client.is_closed()
    {
        return Ok(client.clone());
    }
    let client = DamonClient::discover_and_connect(None)
        .await
        .map_err(|e| e.to_string())?;
    let client = Arc::new(client);
    spawn_damon_pump(state.app.clone(), client.clone());
    *guard = Some(client.clone());
    Ok(client)
}

fn millis_to_iso(ms: i64) -> String {
    chrono::DateTime::<Utc>::from_timestamp_millis(ms)
        .map(|d| d.to_rfc3339())
        .unwrap_or_default()
}

/// Forward one daemon push to the webview.
///
/// Wire shims for the frontend contract (src/types.ts `DamonStreamEvent`):
/// the protocol ask id (`id`) is mirrored as `requestId`, and a
/// `turn_failed` `message` as `reason` — ChatPanel reads those key names.
fn forward_damon_push(app: &AppHandle, push: &Push) {
    if push.event != Push::SESSION_EVENT {
        return;
    }
    let Some(session_id) = push.session_id.clone().filter(|s| !s.is_empty()) else {
        return;
    };
    let mut event = match push.stream_event() {
        Some(ev) => serde_json::to_value(&ev).unwrap_or(Value::Null),
        None => push.data.clone().unwrap_or(Value::Null),
    };
    if let Some(obj) = event.as_object_mut() {
        if let Some(id) = obj.get("id").cloned() {
            obj.entry("requestId").or_insert(id);
        }
        if let Some(message) = obj.get("message").cloned() {
            obj.entry("reason").or_insert(message);
        }
    }

    let _ = app.emit(
        EVENT_DAMON_EVENT,
        json!({ "sessionId": session_id, "event": event }),
    );

    // Terminal turn events → damon://turn-done.
    if let Some(event_type) = event.get("type").and_then(Value::as_str) {
        let stop_reason = match event_type {
            "turn_completed" => Some("completed"),
            "turn_failed" => Some("failed"),
            "turn_canceled" => Some("canceled"),
            _ => None,
        };
        if let Some(stop_reason) = stop_reason {
            let turn_id = event
                .get("turn_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let _ = app.emit(
                EVENT_DAMON_TURN_DONE,
                json!({ "sessionId": session_id, "turnId": turn_id, "stopReason": stop_reason }),
            );
        }
    }
}

fn spawn_damon_pump(app: AppHandle, client: Arc<DamonClient>) {
    let mut rx = client.subscribe_events();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(push) => forward_damon_push(&app, &push),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[tauri::command]
pub async fn damon_status(state: State<'_, Arc<AppState>>) -> Result<DamonStatusDto, String> {
    // Housekeeping: forget a dead cached connection so the next Damon
    // command reconnects.
    let dead = state
        .damon
        .read()
        .await
        .as_ref()
        .is_some_and(|c| c.is_closed());
    if dead {
        state.damon.write().await.take();
    }
    // Status is health-based (does not force a WS connection).
    match crate::damon::client::discover().await {
        Some(endpoint) => Ok(DamonStatusDto {
            running: true,
            url: Some(endpoint.url),
            version: endpoint.version,
            backends: endpoint
                .backends
                .into_iter()
                .map(|b| DamonBackendDto {
                    id: b.id,
                    available: b.available,
                })
                .collect(),
        }),
        None => Ok(DamonStatusDto {
            running: false,
            url: None,
            version: None,
            backends: Vec::new(),
        }),
    }
}

#[tauri::command]
pub async fn damon_list_sessions(
    state: State<'_, Arc<AppState>>,
    limit: Option<u32>,
) -> Result<Vec<DamonSessionDto>, String> {
    let client = ensure_damon(&state).await?;
    let sessions = client
        .list_sessions(limit)
        .await
        .map_err(|e| e.to_string())?;
    Ok(sessions
        .into_iter()
        .map(|s| DamonSessionDto {
            session_id: s.session_id,
            title: s.title.unwrap_or_default(),
            backend: s.backend.unwrap_or_default(),
            created_at: s.created_at.map(millis_to_iso).unwrap_or_default(),
            cwd: s.cwd.unwrap_or_default(),
        })
        .collect())
}

#[tauri::command]
pub async fn damon_create_session(
    state: State<'_, Arc<AppState>>,
    backend: Option<String>,
    title: Option<String>,
) -> Result<DamonCreateSessionDto, String> {
    let client = ensure_damon(&state).await?;
    let info = client
        .create_session(backend.as_deref(), None)
        .await
        .map_err(|e| e.to_string())?;
    // session.create has no title param in protocol v2; apply it as a rename.
    if let Some(title) = title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        let _ = client.rename_session(&info.session_id, title).await;
    }
    Ok(DamonCreateSessionDto {
        session_id: info.session_id,
        backend: info.backend,
    })
}

#[tauri::command]
pub async fn damon_prompt(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    session_id: String,
    prompt: String,
) -> Result<(), String> {
    let client = ensure_damon(&state).await?;
    // Fire-and-forget per the contract: the command returns immediately and
    // the turn streams via damon://event / damon://turn-done.
    tauri::async_runtime::spawn(async move {
        // 히스토리에서 고른 세션은 살아있는 백엔드 프로세스가 아닐 수 있다.
        // -32001(session not live)이면 session.resume으로 재부착 후 1회 재시도한다.
        let result = match client.turn_start(&session_id, &prompt).await {
            Err(DamonError::SessionNotLive(_)) => {
                eprintln!("damon 세션 미활성 — resume 후 재시도: {session_id}");
                match client
                    .call("session.resume", json!({ "sessionId": session_id }))
                    .await
                {
                    Ok(_) => client.turn_start(&session_id, &prompt).await,
                    Err(resume_err) => Err(resume_err),
                }
            }
            other => other,
        };
        match result {
            Ok(result) => {
                // completed/failed/canceled stop reasons also arrive as push
                // events (the pump emits turn-done); surface terminal-only
                // reasons (closed/timeout/…) here so the UI never hangs.
                if !matches!(
                    result.stop_reason.as_str(),
                    "completed" | "failed" | "canceled"
                ) {
                    let _ = app.emit(
                        EVENT_DAMON_TURN_DONE,
                        json!({
                            "sessionId": session_id,
                            "turnId": result.turn_id,
                            "stopReason": result.stop_reason,
                        }),
                    );
                }
            }
            Err(err) => {
                // A timeout means the turn may still complete later (the
                // pump will emit turn-done); any other failure would strand
                // the streaming UI, so synthesize a terminal event.
                if !matches!(err, DamonError::RequestTimeout { .. }) {
                    eprintln!("damon turn 실패: {err}");
                    let _ = app.emit(
                        EVENT_DAMON_TURN_DONE,
                        json!({
                            "sessionId": session_id,
                            "turnId": "",
                            "stopReason": "failed",
                        }),
                    );
                }
            }
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn damon_cancel_turn(
    state: State<'_, Arc<AppState>>,
    session_id: String,
) -> Result<DamonCancelledDto, String> {
    let client = ensure_damon(&state).await?;
    let cancelled = client
        .cancel_turn(&session_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(DamonCancelledDto { cancelled })
}

#[tauri::command]
pub async fn damon_permission_respond(
    state: State<'_, Arc<AppState>>,
    session_id: String,
    request_id: String,
    behavior: PermissionBehavior,
) -> Result<(), String> {
    let client = ensure_damon(&state).await?;
    client
        .respond_permission(&session_id, &request_id, behavior)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn damon_delete_session(
    state: State<'_, Arc<AppState>>,
    session_id: String,
) -> Result<DamonDeletedDto, String> {
    let client = ensure_damon(&state).await?;
    let deleted = client
        .delete_session(&session_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(DamonDeletedDto { deleted })
}

#[tauri::command]
pub async fn damon_rename_session(
    state: State<'_, Arc<AppState>>,
    session_id: String,
    title: String,
) -> Result<DamonRenamedDto, String> {
    let client = ensure_damon(&state).await?;
    let renamed = client
        .rename_session(&session_id, &title)
        .await
        .map_err(|e| e.to_string())?;
    Ok(DamonRenamedDto { renamed })
}
