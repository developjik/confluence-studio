//! Persistent job store: `jobs.json` in the app-data dir with atomic writes
//! (port of `storage.JobStore`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::JobsResult;
use super::fs_util;
use super::models::Job;

const JOBS_FILE_NAME: &str = "jobs.json";

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobsFile {
    schema_version: u32,
    jobs: Vec<Job>,
}

impl Default for JobsFile {
    fn default() -> Self {
        Self {
            schema_version: 1,
            jobs: Vec::new(),
        }
    }
}

/// In-memory job list backed by `<app_data_dir>/jobs.json`.
#[derive(Debug)]
pub struct JobStore {
    dir: PathBuf,
    jobs: Vec<Job>,
}

impl JobStore {
    /// Load the store. A missing file yields an empty store. A corrupt file
    /// is quarantined (renamed to `jobs.json.corrupt-<ts>`) so a later save
    /// can never silently destroy the user's data.
    pub fn load(app_data_dir: &Path) -> Self {
        let path = app_data_dir.join(JOBS_FILE_NAME);
        let mut store = Self {
            dir: app_data_dir.to_path_buf(),
            jobs: Vec::new(),
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return store,
            Err(err) => {
                eprintln!("jobs.json 읽기 실패 — 빈 상태로 시작합니다: {err}");
                return store;
            }
        };
        match serde_json::from_str::<JobsFile>(&text) {
            Ok(file) => store.jobs = file.jobs.into_iter().map(|job| job.normalized()).collect(),
            Err(err) => {
                let quarantine = app_data_dir.join(format!(
                    "{JOBS_FILE_NAME}.corrupt-{}",
                    chrono::Utc::now().timestamp()
                ));
                let _ = std::fs::rename(&path, &quarantine);
                eprintln!(
                    "jobs.json 손상 — {}로 이동하고 빈 상태로 시작합니다: {err}",
                    quarantine.display()
                );
            }
        }
        store
    }

    pub fn jobs_path(&self) -> PathBuf {
        self.dir.join(JOBS_FILE_NAME)
    }

    pub fn list(&self) -> Vec<Job> {
        self.jobs.clone()
    }

    pub fn get(&self, job_id: &str) -> Option<Job> {
        self.jobs.iter().find(|job| job.id == job_id).cloned()
    }

    /// Upsert by id (a fresh id is generated when empty) and persist.
    /// Returns the stored (normalized) job.
    pub fn save(&mut self, job: &Job) -> JobsResult<Job> {
        let mut stored = job.normalized();
        if stored.id.is_empty() {
            stored.id = uuid::Uuid::new_v4().to_string();
        }
        match self.jobs.iter_mut().find(|j| j.id == stored.id) {
            Some(slot) => *slot = stored.clone(),
            None => self.jobs.push(stored.clone()),
        }
        self.persist()?;
        Ok(stored)
    }

    /// Remove a job by id. Returns whether it existed (and was removed).
    pub fn delete(&mut self, job_id: &str) -> JobsResult<bool> {
        let before = self.jobs.len();
        self.jobs.retain(|job| job.id != job_id);
        let removed = before != self.jobs.len();
        if removed {
            self.persist()?;
        }
        Ok(removed)
    }

    /// Atomically write `jobs.json` (tmp + rename).
    pub fn persist(&self) -> JobsResult<()> {
        let file = JobsFile {
            schema_version: 1,
            jobs: self.jobs.clone(),
        };
        let mut text = serde_json::to_string_pretty(&file)?;
        text.push('\n');
        fs_util::atomic_write_text(&self.jobs_path(), &text)?;
        Ok(())
    }
}
