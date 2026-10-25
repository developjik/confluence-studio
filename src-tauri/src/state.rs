//! Shared application state managed by Tauri (integration layer).
//!
//! Owns the export-job store, the running-export registry used for
//! cancellation, the lazily connected Damon client and the `AppHandle`
//! used to emit backend events. See docs/ARCHITECTURE.md §
//! "Tauri command contract".

use std::sync::Arc;

use parking_lot::Mutex;
use tauri::{AppHandle, Manager};
use tokio::sync::RwLock;

use crate::damon::DamonClient;
use crate::jobs::{JobStore, RunningExports};

/// Global state shared by every Tauri command and background task.
pub struct AppState {
    /// Persistent export-job store backed by `<app_data_dir>/jobs.json`.
    /// Lock sections are short (in-memory list / small JSON write); disk
    /// writes issued from async contexts go through `spawn_blocking`.
    pub jobs: Arc<Mutex<JobStore>>,
    /// Registry of manually started (and scheduler-started) export runs.
    /// `try_start` enforces skip_if_running, the guard's cancel flag is fed
    /// to `run_export`, and `cancel` implements the `cancel_job_run`
    /// command.
    pub running: Arc<RunningExports>,
    /// Guard registry used only inside the scheduler tick loop so a new
    /// scheduled dispatch of a job is skipped while the previous scheduled
    /// dispatch is still running (`skip_if_running`). Runs themselves always
    /// register in [`AppState::running`] so `cancel_job_run` and the
    /// manual/scheduled overlap check share one view.
    pub scheduler_running: Arc<RunningExports>,
    /// Lazily discovered+connected Damon client. `None` — or a client whose
    /// connection has closed — means the next Damon command reconnects.
    pub damon: RwLock<Option<Arc<DamonClient>>>,
    /// Handle used by background tasks to emit events to the webview.
    pub app: AppHandle,
}

impl AppState {
    /// Build the state, loading the job store from the app data directory
    /// (created on demand).
    pub fn init(app: AppHandle) -> Result<Self, tauri::Error> {
        let dir = app.path().app_data_dir()?;
        std::fs::create_dir_all(&dir)?;
        let store = JobStore::load(&dir);
        Ok(Self {
            jobs: Arc::new(Mutex::new(store)),
            running: Arc::new(RunningExports::new()),
            scheduler_running: Arc::new(RunningExports::new()),
            damon: RwLock::new(None),
            app,
        })
    }
}
