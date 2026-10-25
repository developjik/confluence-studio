//! Run history: `<target>/.confluence-studio/run_history.json`, keeping the
//! most recent 100 runs (append-order, oldest first).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::JobsResult;
use super::export::{ExportOutcome, STATE_DIR_NAME};
use super::fs_util;

const RUN_HISTORY_FILE: &str = "run_history.json";
const MAX_RUNS: usize = 100;

fn history_path(target_dir: &Path) -> PathBuf {
    target_dir.join(STATE_DIR_NAME).join(RUN_HISTORY_FILE)
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryFile {
    schema_version: u32,
    #[serde(default)]
    runs: Vec<ExportOutcome>,
}

/// Append a run to the history and trim it to the last 100 entries.
pub fn append_run(target_dir: &Path, outcome: &ExportOutcome) -> JobsResult<()> {
    let path = history_path(target_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Tolerate a corrupt history file rather than failing the export.
    let mut file: HistoryFile = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    file.schema_version = 1;
    file.runs.push(outcome.clone());
    let overflow = file.runs.len().saturating_sub(MAX_RUNS);
    if overflow > 0 {
        file.runs.drain(..overflow);
    }
    let mut text = serde_json::to_string_pretty(&file)?;
    text.push('\n');
    fs_util::atomic_write_text(&path, &text)?;
    Ok(())
}

/// Recorded runs for a target directory (append order: oldest first).
/// Missing/corrupt file yields an empty list.
pub fn list_runs(target_dir: &Path) -> Vec<ExportOutcome> {
    let path = history_path(target_dir);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<HistoryFile>(&text).ok())
        .map(|file| file.runs)
        .unwrap_or_default()
}
