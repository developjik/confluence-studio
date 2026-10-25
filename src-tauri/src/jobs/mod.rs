//! Export jobs: store, scheduler, native export engine, run history.
//! Contract: docs/ARCHITECTURE.md § "Module: jobs".
//!
//! Semantics ported from the original Python app
//! (`confluence-export-desktop/src/confluence_sync_tool`): schedule
//! occurrences, misfire=skip, concurrency=skip_if_running, atomic JSON
//! writes and `sha256:`-prefixed content hashes.

pub mod export;
pub mod history;
pub mod models;
pub mod scheduler;
pub mod store;

pub use export::{
    ATTACHMENTS_DIR_NAME, AttachmentRef, ExportOutcome, ExportProgress, PageData, PageSource,
    RunGuard, RunStatus, RunningExports, STATE_DIR_NAME, run_export,
};
pub use history::{append_run, list_runs};
pub use models::{ExportOptions, Job, Schedule, ScheduleKind};
pub use scheduler::{Scheduler, TickState};
pub use store::JobStore;

use thiserror::Error;

/// Module-wide error type. `Display` messages are user-facing (Korean),
/// matching the Golden rules in docs/ARCHITECTURE.md.
#[derive(Debug, Error)]
pub enum JobsError {
    #[error("파일 접근 오류: {0}")]
    Io(#[from] std::io::Error),
    #[error("설정 파일을 해석할 수 없습니다: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Export(String),
}

impl JobsError {
    /// Build a free-form export error (used by `PageSource` implementations).
    pub fn export(message: impl Into<String>) -> Self {
        Self::Export(message.into())
    }
}

pub type JobsResult<T> = Result<T, JobsError>;

/// Shared filesystem helpers: atomic writes via tmp file + fsync + rename
/// (port of `fs_policy.atomic_write_text`).
pub(crate) mod fs_util {
    use std::io;
    use std::path::Path;

    fn tmp_path(path: &Path) -> std::path::PathBuf {
        path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()))
    }

    /// Synchronous atomic write (small config/state files).
    pub(crate) fn atomic_write_bytes(path: &Path, data: &[u8]) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = tmp_path(path);
        let result = (|| {
            std::fs::write(&tmp, data)?;
            if let Ok(file) = std::fs::File::open(&tmp) {
                // fsync the file contents before the rename becomes visible.
                let _ = file.sync_all();
            }
            std::fs::rename(&tmp, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }

    pub(crate) fn atomic_write_text(path: &Path, text: &str) -> io::Result<()> {
        atomic_write_bytes(path, text.as_bytes())
    }

    /// Async atomic write used from `run_export` (keeps the async runtime
    /// free of blocking I/O per the architecture golden rules).
    pub(crate) async fn atomic_write_bytes_async(path: &Path, data: Vec<u8>) -> io::Result<()> {
        use tokio::io::AsyncWriteExt;

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tmp = tmp_path(path);
        let result = async {
            let mut file = tokio::fs::File::create(&tmp).await?;
            file.write_all(&data).await?;
            file.sync_all().await?;
            drop(file);
            tokio::fs::rename(&tmp, path).await
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&tmp).await;
        }
        result
    }
}
