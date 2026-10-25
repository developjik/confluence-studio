//! Integration tests for the jobs module (store, models normalization,
//! scheduler occurrences/tick, export engine, run history).
//! All tests are prefixed `jobs_` so `cargo test jobs` runs the whole suite.
//! Everything runs against real temporary directories.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, TimeZone, Utc};
use chrono_tz::Asia::Seoul;
use confluence_studio_lib::jobs::{
    append_run, list_runs, run_export, AttachmentRef, ExportOptions, ExportOutcome, Job, JobStore,
    PageData, PageSource, RunStatus, RunningExports, Schedule, ScheduleKind, Scheduler, TickState,
    JobsError, JobsResult,
};
use tokio::sync::{mpsc, watch};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "cs-jobs-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn make_job(target: &Path) -> Job {
    Job {
        id: "job-1".to_string(),
        name: "잡".to_string(),
        space_id: "SP".to_string(),
        root_page_id: None,
        target_dir: target.to_string_lossy().into_owned(),
        include_attachments: false,
        enabled: true,
        export_options: ExportOptions::default(),
        schedule: Schedule::default(),
    }
}

fn scheduled_job(times: Vec<String>, kind: ScheduleKind) -> Job {
    let mut job = make_job(Path::new("/tmp/cs-unused"));
    job.schedule = Schedule {
        enabled: true,
        kind,
        times,
        ..Schedule::default()
    };
    job
}

fn page(id: &str, title: &str, parents: &[&str], storage: &str) -> PageData {
    PageData {
        id: id.to_string(),
        title: title.to_string(),
        parent_title_path: parents.iter().map(|s| s.to_string()).collect(),
        storage: storage.to_string(),
        attachments: Vec::new(),
        version: Some(7),
        web_url: Some(format!("https://example.atlassian.net/wiki/{id}")),
    }
}

#[derive(Default)]
struct MockSource {
    pages: Vec<PageData>,
    attachments: HashMap<String, Vec<u8>>,
    fail_tree: bool,
    fail_attachments: bool,
}

impl PageSource for MockSource {
    fn fetch_tree(&mut self, _job: &Job) -> JobsResult<Vec<PageData>> {
        if self.fail_tree {
            return Err(JobsError::export("트리 조회 실패(테스트)"));
        }
        Ok(self.pages.clone())
    }

    fn fetch_attachment(&mut self, att: &AttachmentRef) -> JobsResult<Vec<u8>> {
        if self.fail_attachments {
            return Err(JobsError::export("첨부 조회 실패(테스트)"));
        }
        self.attachments
            .get(&att.id)
            .cloned()
            .ok_or_else(|| JobsError::export(format!("첨부 없음: {}", att.id)))
    }
}

fn no_progress() -> impl FnMut(confluence_studio_lib::jobs::ExportProgress) {
    |_| {}
}

fn stub_outcome(job_id: &str) -> ExportOutcome {
    let now = Utc::now();
    ExportOutcome {
        run_id: format!("run-{job_id}"),
        job_id: job_id.to_string(),
        status: RunStatus::Success,
        pages_written: 0,
        pages_skipped: 0,
        attachments_written: 0,
        files_removed: 0,
        started_at: now,
        finished_at: now,
        error: None,
    }
}

fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
}

/// "HH:MM" (Asia/Seoul) of the instant `seconds` ago from `now`.
fn hhmm_seoul_ago(now: DateTime<Utc>, seconds: i64) -> String {
    (now - ChronoDuration::seconds(seconds))
        .with_timezone(&Seoul)
        .format("%H:%M")
        .to_string()
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

#[test]
fn jobs_store_load_missing_dir_is_empty() {
    let tmp = TempDir::new();
    let store = JobStore::load(&tmp.path().join("app-data"));
    assert!(store.list().is_empty());
}

#[test]
fn jobs_store_save_generates_uuid_and_persists_camelcase() {
    let tmp = TempDir::new();
    let data_dir = tmp.path().join("app-data");
    let mut store = JobStore::load(&data_dir);

    let mut job = make_job(tmp.path());
    job.id = String::new();
    job.schedule.times = vec!["13:00".into(), "9:00".into(), "09:00".into()];
    let stored = store.save(&job).unwrap();

    assert!(!stored.id.is_empty(), "빈 id에 uuid가 생성되어야 함");
    assert_eq!(stored.schedule.times, vec!["09:00", "13:00"]);

    let text = std::fs::read_to_string(data_dir.join("jobs.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    let saved = &value["jobs"][0];
    assert_eq!(saved["id"], stored.id);
    assert_eq!(saved["spaceId"], "SP");
    assert_eq!(saved["targetDir"], job.target_dir);
    assert!(saved.get("rootPageId").is_none(), "None 필드는 생략");
    assert_eq!(saved["exportOptions"]["skipUnchanged"], true);
    assert_eq!(saved["schedule"]["kind"], "daily");
    assert_eq!(saved["schedule"]["timezone"], "Asia/Seoul");

    let reloaded = JobStore::load(&data_dir);
    assert_eq!(reloaded.list().len(), 1);
    assert_eq!(reloaded.list()[0].id, stored.id);
}

#[test]
fn jobs_store_save_upserts_by_id_and_delete_removes() {
    let tmp = TempDir::new();
    let data_dir = tmp.path().join("app-data");
    let mut store = JobStore::load(&data_dir);

    let mut job = make_job(tmp.path());
    job.id = "j-1".into();
    store.save(&job).unwrap();
    job.name = "수정됨".into();
    store.save(&job).unwrap();
    assert_eq!(store.list().len(), 1);
    assert_eq!(store.list()[0].name, "수정됨");

    assert!(store.delete("j-1").unwrap());
    assert!(!store.delete("j-1").unwrap());
    assert!(store.list().is_empty());
    assert!(JobStore::load(&data_dir).list().is_empty());
}

#[test]
fn jobs_store_quarantines_corrupt_jobs_json() {
    let tmp = TempDir::new();
    let data_dir = tmp.path().join("app-data");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(data_dir.join("jobs.json"), "{ not json").unwrap();

    let store = JobStore::load(&data_dir);
    assert!(store.list().is_empty());

    let quarantined: Vec<_> = std::fs::read_dir(&data_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("jobs.json.corrupt-"))
        .collect();
    assert_eq!(quarantined.len(), 1, "손상 파일은 격리되어야 함");
}

// ---------------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------------

#[test]
fn jobs_schedule_normalized_sorts_dedupes_and_falls_back() {
    let schedule = Schedule {
        times: vec!["13:00".into(), "9:00".into(), "09:00".into(), "25:00".into(), "junk".into()],
        ..Schedule::default()
    };
    assert_eq!(schedule.normalized().times, vec!["09:00", "13:00"]);

    let empty = Schedule {
        times: vec!["nope".into()],
        ..Schedule::default()
    };
    assert_eq!(empty.normalized().times, vec!["09:00"]);
}

#[test]
fn jobs_schedule_normalized_fixes_invalid_fields() {
    let schedule = Schedule {
        days_of_week: vec![9, 0, 3, 3, 1],
        day_of_month: Some(32),
        timezone: "Not/AZone".into(),
        ..Schedule::default()
    };
    let normalized = schedule.normalized();
    assert_eq!(normalized.days_of_week, vec![1, 3]);
    assert_eq!(normalized.day_of_month, None);
    assert_eq!(normalized.timezone, "Asia/Seoul");

    let valid = Schedule {
        day_of_month: Some(31),
        timezone: "America/New_York".into(),
        ..Schedule::default()
    };
    let normalized = valid.normalized();
    assert_eq!(normalized.day_of_month, Some(31));
    assert_eq!(normalized.timezone, "America/New_York");
}

#[test]
fn jobs_models_serde_camelcase_lenient_and_kind_alias() {
    // camelCase 직렬화 + 기본값
    let json = r#"{"id":"j","name":"n","spaceId":"s","targetDir":"t"}"#;
    let job: Job = serde_json::from_str(json).unwrap();
    assert!(job.enabled);
    assert_eq!(job.export_options.max_workers, 8);
    assert!(!job.schedule.enabled);
    assert_eq!(job.schedule.kind, ScheduleKind::Daily);

    let value = serde_json::to_value(&job).unwrap();
    assert_eq!(value["spaceId"], "s");
    assert_eq!(value["exportOptions"]["frontMatter"], true);
    assert_eq!(value["schedule"]["daysOfWeek"], serde_json::json!([1, 2, 3, 4, 5]));
    assert_eq!(value["schedule"]["dayOfMonth"], serde_json::Value::Null);

    // 스네이크 별칭(multiple_daily)과 알 수 없는 kind 폴백
    let aliased: Job = serde_json::from_str(
        r#"{"id":"j","name":"n","spaceId":"s","targetDir":"t","schedule":{"kind":"multiple_daily","enabled":true}}"#,
    )
    .unwrap();
    assert_eq!(aliased.schedule.kind, ScheduleKind::MultipleDaily);

    let unknown: Job = serde_json::from_str(
        r#"{"id":"j","name":"n","spaceId":"s","targetDir":"t","schedule":{"kind":"bogus"}}"#,
    )
    .unwrap();
    assert_eq!(unknown.schedule.kind, ScheduleKind::Daily);
}

// ---------------------------------------------------------------------------
// Scheduler: occurrences_between (pure logic)
// ---------------------------------------------------------------------------

#[test]
fn jobs_occurrences_daily_kst_boundary() {
    let job = scheduled_job(vec!["09:00".into()], ScheduleKind::Daily);
    // 09:00 KST = 00:00 UTC. start는 미포함, end는 포함.
    let start = utc(2026, 5, 5, 0, 30);
    let end = utc(2026, 5, 6, 0, 0);
    let occurrences = Scheduler::occurrences_between(&job, start, end);
    assert_eq!(occurrences, vec![utc(2026, 5, 6, 0, 0)]);

    // 경계 안쪽(08:59:59~09:00:01 KST = 23:59:59Z ~ 00:00:01Z)
    let start = Utc.with_ymd_and_hms(2026, 5, 4, 23, 59, 59).unwrap();
    let end = Utc.with_ymd_and_hms(2026, 5, 5, 0, 0, 1).unwrap();
    let occurrences = Scheduler::occurrences_between(&job, start, end);
    assert_eq!(occurrences, vec![utc(2026, 5, 5, 0, 0)]);
}

#[test]
fn jobs_occurrences_multiple_daily_sorted_times() {
    let job = scheduled_job(
        vec!["18:00".into(), "13:00".into(), "9:00".into()],
        ScheduleKind::MultipleDaily,
    );
    let occurrences = Scheduler::occurrences_between(&job, utc(2026, 5, 5, 0, 0), utc(2026, 5, 6, 0, 0));
    // 09:00 KST == start(제외), 13:00/18:00 KST 포함, 다음날 09:00 == end(포함)
    assert_eq!(
        occurrences,
        vec![utc(2026, 5, 5, 4, 0), utc(2026, 5, 5, 9, 0), utc(2026, 5, 6, 0, 0)]
    );
}

#[test]
fn jobs_occurrences_weekdays_skips_weekend() {
    let job = scheduled_job(vec!["09:00".into()], ScheduleKind::Weekdays);
    // 2026-05-04(월) ~ 05-11(월), 09:00 KST = 00:00 UTC
    let occurrences = Scheduler::occurrences_between(&job, utc(2026, 5, 4, 0, 0), utc(2026, 5, 11, 0, 0));
    assert_eq!(
        occurrences,
        vec![
            utc(2026, 5, 5, 0, 0),
            utc(2026, 5, 6, 0, 0),
            utc(2026, 5, 7, 0, 0),
            utc(2026, 5, 8, 0, 0),
            utc(2026, 5, 11, 0, 0),
        ]
    );
}

#[test]
fn jobs_occurrences_weekly_selected_days_only() {
    let mut job = scheduled_job(vec!["09:00".into()], ScheduleKind::Weekly);
    job.schedule.days_of_week = vec![1, 5]; // 월, 금
    let occurrences = Scheduler::occurrences_between(&job, utc(2026, 5, 4, 0, 0), utc(2026, 5, 11, 0, 0));
    assert_eq!(occurrences, vec![utc(2026, 5, 8, 0, 0), utc(2026, 5, 11, 0, 0)]);

    // 빈 days_of_week → 발생 없음 (원본 규칙)
    job.schedule.days_of_week = vec![];
    assert!(Scheduler::occurrences_between(&job, utc(2026, 5, 4, 0, 0), utc(2026, 5, 11, 0, 0)).is_empty());
}

#[test]
fn jobs_occurrences_monthly_31_skips_short_months() {
    let mut job = scheduled_job(vec!["09:00".into()], ScheduleKind::Monthly);
    job.schedule.timezone = "UTC".into();
    job.schedule.day_of_month = Some(31);
    let occurrences = Scheduler::occurrences_between(&job, utc(2026, 6, 1, 0, 0), utc(2026, 8, 1, 0, 0));
    assert_eq!(occurrences, vec![utc(2026, 7, 31, 9, 0)]);

    job.schedule.day_of_month = None;
    assert!(Scheduler::occurrences_between(&job, utc(2026, 6, 1, 0, 0), utc(2026, 8, 1, 0, 0)).is_empty());
}

#[test]
fn jobs_occurrences_disabled_enabled_or_paused_yield_none() {
    let mut job = scheduled_job(vec!["09:00".into()], ScheduleKind::Daily);
    job.enabled = false;
    assert!(Scheduler::occurrences_between(&job, utc(2026, 5, 1, 0, 0), utc(2026, 5, 8, 0, 0)).is_empty());

    job.enabled = true;
    job.schedule.enabled = false;
    assert!(Scheduler::occurrences_between(&job, utc(2026, 5, 1, 0, 0), utc(2026, 5, 8, 0, 0)).is_empty());

    job.schedule.enabled = true;
    job.schedule.paused = true;
    assert!(Scheduler::occurrences_between(&job, utc(2026, 5, 1, 0, 0), utc(2026, 5, 8, 0, 0)).is_empty());
}

#[test]
fn jobs_occurrences_timezone_changes_instant() {
    let mut utc_job = scheduled_job(vec!["09:00".into()], ScheduleKind::Daily);
    utc_job.schedule.timezone = "UTC".into();
    let window = (utc(2026, 5, 5, 8, 0), utc(2026, 5, 5, 10, 0));
    assert_eq!(
        Scheduler::occurrences_between(&utc_job, window.0, window.1),
        vec![utc(2026, 5, 5, 9, 0)]
    );

    let seoul_job = scheduled_job(vec!["09:00".into()], ScheduleKind::Daily); // Asia/Seoul
    assert!(Scheduler::occurrences_between(&seoul_job, window.0, window.1).is_empty());
}

#[test]
fn jobs_occurrences_dst_gap_and_ambiguous_skipped() {
    // 2026-03-08 America/New_York: 02:00→03:00 전환(02:30 없음)
    let mut job = scheduled_job(vec!["02:30".into()], ScheduleKind::Daily);
    job.schedule.timezone = "America/New_York".into();
    let occurrences = Scheduler::occurrences_between(&job, utc(2026, 3, 7, 12, 0), utc(2026, 3, 9, 12, 0));
    // 03-08 02:30은 존재하지 않음 → 건너뜀, 03-09 02:30 EDT = 06:30Z
    assert_eq!(occurrences, vec![utc(2026, 3, 9, 6, 30)]);

    // 2026-11-01 America/New_York: 01:30 애매(ambiguous) → 건너뜀
    let mut job = scheduled_job(vec!["01:30".into()], ScheduleKind::Daily);
    job.schedule.timezone = "America/New_York".into();
    let occurrences = Scheduler::occurrences_between(&job, utc(2026, 10, 31, 12, 0), utc(2026, 11, 2, 12, 0));
    assert_eq!(occurrences, vec![utc(2026, 11, 2, 6, 30)]);
}

#[test]
fn jobs_next_occurrence_after_now() {
    let job = scheduled_job(vec!["09:00".into()], ScheduleKind::Daily);
    // 2026-05-05 18:30 KST 기준 다음 발생은 05-06 09:00 KST
    let next = Scheduler::next_occurrence(&job, utc(2026, 5, 5, 9, 30));
    assert_eq!(next, Some(utc(2026, 5, 6, 0, 0)));
}

// ---------------------------------------------------------------------------
// Scheduler: tick / run_loop
// ---------------------------------------------------------------------------

fn tick_runner(counter: Arc<AtomicUsize>) -> impl Fn(Job, DateTime<Utc>) -> std::pin::Pin<Box<dyn std::future::Future<Output = ExportOutcome> + Send>>
       + Clone
       + Send
       + Sync
       + 'static {
    move |_job, _scheduled| {
        let counter = counter.clone();
        Box::pin(async move {
            counter.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(400)).await;
            stub_outcome("job-1")
        })
    }
}

#[tokio::test]
async fn jobs_tick_once_fires_due_job_once_across_ticks() {
    let now = Utc::now();
    let job = scheduled_job(vec![hhmm_seoul_ago(now, 30)], ScheduleKind::Daily);
    let jobs = Arc::new(Mutex::new(vec![job]));
    let counter = Arc::new(AtomicUsize::new(0));
    let runner = tick_runner(counter.clone());
    let mut state = TickState::new(Arc::new(RunningExports::new()));

    for _ in 0..3 {
        let mut load = || jobs.lock().unwrap().clone();
        state.tick_once(now, &mut load, &runner).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 1, "동일 발생은 한 번만 실행");
}

#[tokio::test]
async fn jobs_tick_once_skips_misfired_occurrence() {
    let now = Utc::now();
    // 10분 전 발생 → grace(120s) 초과 → misfire skip
    let job = scheduled_job(vec![hhmm_seoul_ago(now, 600)], ScheduleKind::Daily);
    let jobs = Arc::new(Mutex::new(vec![job]));
    let counter = Arc::new(AtomicUsize::new(0));
    let runner = tick_runner(counter.clone());
    let mut state = TickState::new(Arc::new(RunningExports::new()));

    let mut load = || jobs.lock().unwrap().clone();
    state.tick_once(now, &mut load, &runner).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 0, "misfire는 skip");
}

#[tokio::test]
async fn jobs_tick_once_skip_if_running() {
    let now = Utc::now();
    // 두 발생 모두 grace 안쪽(30s/90s 전), 러너는 400ms 점유
    let job = scheduled_job(
        vec![hhmm_seoul_ago(now, 90), hhmm_seoul_ago(now, 30)],
        ScheduleKind::Daily,
    );
    let jobs = Arc::new(Mutex::new(vec![job]));
    let counter = Arc::new(AtomicUsize::new(0));
    let runner = tick_runner(counter.clone());
    let mut state = TickState::new(Arc::new(RunningExports::new()))
        .with_misfire_grace(ChronoDuration::minutes(10));

    let mut load = || jobs.lock().unwrap().clone();
    state.tick_once(now, &mut load, &runner).await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 1, "실행 중인 잡은 skip_if_running");
}

#[tokio::test]
async fn jobs_run_loop_fires_due_job_and_stops_on_shutdown() {
    let tmp = TempDir::new();
    let now = Utc::now();
    let mut job = scheduled_job(vec![hhmm_seoul_ago(now, 30)], ScheduleKind::Daily);
    job.target_dir = tmp.path().to_string_lossy().into_owned();
    let jobs = Arc::new(Mutex::new(vec![job]));
    let counter = Arc::new(AtomicUsize::new(0));
    let (tx, mut rx) = mpsc::channel::<()>(4);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let counter_for_runner = counter.clone();
    let task = tokio::spawn(Scheduler::run_loop(
        move || jobs.lock().unwrap().clone(),
        move |job, _scheduled| {
            let tx = tx.clone();
            let counter = counter_for_runner.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                let _ = tx.send(()).await;
                stub_outcome(&job.id)
            }
        },
        Duration::from_millis(50),
        shutdown_rx,
        Arc::new(RunningExports::new()),
    ));

    let fired = tokio::time::timeout(Duration::from_secs(20), rx.recv()).await;
    assert!(fired.is_ok_and(|v| v.is_some()), "스케줄된 실행이 발생해야 함");

    shutdown_tx.send(true).unwrap();
    let stopped = tokio::time::timeout(Duration::from_secs(5), task).await;
    assert!(stopped.is_ok(), "shutdown 시 루프가 종료되어야 함");
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

// ---------------------------------------------------------------------------
// Export engine
// ---------------------------------------------------------------------------

fn tree_fixture() -> Vec<PageData> {
    vec![
        page("p1", "Home", &["Docs"], "<p>Hello <strong>world</strong></p>"),
        page("p2", "Setup Guide", &["Docs", "Home"], "<p>Setup</p>"),
        page("p3", "Advanced", &["Docs", "Home"], "<p>Adv</p>"),
        page("p4", "Troubleshooting", &["Docs", "Home", "Setup Guide"], "<p>Trouble</p>"),
    ]
}

#[tokio::test]
async fn jobs_export_writes_tree_numbering_front_matter_breadcrumbs() {
    let tmp = TempDir::new();
    let job = make_job(tmp.path());
    let mut source = MockSource {
        pages: tree_fixture(),
        ..MockSource::default()
    };

    let outcome = run_export(&job, &mut source, &mut no_progress(), &AtomicBool::new(false)).await;

    assert_eq!(outcome.status, RunStatus::Success);
    assert_eq!(outcome.pages_written, 4);
    assert_eq!(outcome.pages_skipped, 0);
    assert!(outcome.error.is_none());

    let root = tmp.path();
    let home = std::fs::read_to_string(root.join("Docs/01 - Home.md")).unwrap();
    assert!(home.starts_with("---\n"), "front matter가 파일 맨 앞");
    assert!(home.contains("title: \"Home\""));
    assert!(home.contains("page_id: p1"));
    assert!(home.contains("version: 7"));
    assert!(home.contains("exported_at: "));
    assert!(home.contains("> Docs\n"), "breadcrumb 경로 줄");
    assert!(home.contains("Hello world"), "파라그래프 텍스트 보존");

    assert!(root.join("Docs/Home/01 - Setup Guide.md").is_file());
    assert!(root.join("Docs/Home/02 - Advanced.md").is_file());
    let trouble = std::fs::read_to_string(root.join("Docs/Home/Setup Guide/01 - Troubleshooting.md")).unwrap();
    assert!(trouble.contains("> Docs > Home > Setup Guide"));

    // state.json + run_history.json 기록
    assert!(root.join(".confluence-studio/state.json").is_file());
    let history = list_runs(root);
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, RunStatus::Success);
    assert_eq!(history[0].job_id, job.id);
    assert_eq!(history[0].run_id, outcome.run_id);
}

#[tokio::test]
async fn jobs_export_skip_unchanged_on_second_run() {
    let tmp = TempDir::new();
    let job = make_job(tmp.path());
    let mut source = MockSource {
        pages: tree_fixture(),
        ..MockSource::default()
    };
    let cancel = AtomicBool::new(false);

    let first = run_export(&job, &mut source, &mut no_progress(), &cancel).await;
    assert_eq!(first.pages_written, 4);

    let second = run_export(&job, &mut source, &mut no_progress(), &cancel).await;
    assert_eq!(second.status, RunStatus::Success);
    assert_eq!(second.pages_written, 0);
    assert_eq!(second.pages_skipped, 4);

    // 한 페이지만 바뀌면 그 페이지만 다시 씀
    source.pages[1].storage = "<p>Setup changed</p>".into();
    let third = run_export(&job, &mut source, &mut no_progress(), &cancel).await;
    assert_eq!(third.pages_written, 1);
    assert_eq!(third.pages_skipped, 3);
    let updated = std::fs::read_to_string(tmp.path().join("Docs/Home/01 - Setup Guide.md")).unwrap();
    assert!(updated.contains("Setup changed"));
}

#[tokio::test]
async fn jobs_export_cleanup_stale_removes_deleted_pages() {
    let tmp = TempDir::new();
    let job = make_job(tmp.path());
    let cancel = AtomicBool::new(false);

    let mut source = MockSource {
        pages: vec![
            page("a", "Alpha", &["Docs"], "<p>A</p>"),
            page("b", "Beta", &["Docs", "Home"], "<p>B</p>"),
        ],
        ..MockSource::default()
    };
    let first = run_export(&job, &mut source, &mut no_progress(), &cancel).await;
    assert_eq!(first.pages_written, 2);
    let beta_path = tmp.path().join("Docs/Home/01 - Beta.md");
    assert!(beta_path.is_file());

    // 원격에서 Beta가 사라진 상태로 재실행
    source.pages.retain(|p| p.id != "b");
    let second = run_export(&job, &mut source, &mut no_progress(), &cancel).await;
    assert_eq!(second.status, RunStatus::Success);
    assert_eq!(second.files_removed, 1);
    assert!(!beta_path.exists(), "사라진 페이지 파일 삭제");
    assert!(!tmp.path().join("Docs/Home").exists(), "빈 디렉터리 정리");
    assert!(tmp.path().join("Docs/01 - Alpha.md").is_file());

    let state_text = std::fs::read_to_string(tmp.path().join(".confluence-studio/state.json")).unwrap();
    let state: serde_json::Value = serde_json::from_str(&state_text).unwrap();
    assert_eq!(state["entries"].as_object().unwrap().len(), 1);
}

#[tokio::test]
async fn jobs_export_cancel_stops_between_pages() {
    let tmp = TempDir::new();
    let job = make_job(tmp.path());
    let mut source = MockSource {
        pages: vec![
            page("p1", "One", &["Docs"], "<p>1</p>"),
            page("p2", "Two", &["Docs"], "<p>2</p>"),
        ],
        ..MockSource::default()
    };
    let cancel = AtomicBool::new(false);
    let cancel_ptr = &cancel;
    let mut progress = move |p: confluence_studio_lib::jobs::ExportProgress| {
        if p.done == 1 {
            cancel_ptr.store(true, Ordering::SeqCst);
        }
    };

    let outcome = run_export(&job, &mut source, &mut progress, &cancel).await;
    assert_eq!(outcome.status, RunStatus::Cancelled);
    assert_eq!(outcome.pages_written, 1);
    assert!(tmp.path().join("Docs/01 - One.md").is_file());
    assert!(!tmp.path().join("Docs/02 - Two.md").exists());
    assert_eq!(
        list_runs(tmp.path()).last().map(|r| r.status),
        Some(RunStatus::Cancelled),
        "취소도 이력에 기록"
    );
}

#[tokio::test]
async fn jobs_export_attachments_downloaded_and_skipped_when_unchanged() {
    let tmp = TempDir::new();
    let mut job = make_job(tmp.path());
    job.include_attachments = true;
    let mut source = MockSource {
        pages: vec![PageData {
            attachments: vec![AttachmentRef {
                id: "att1".into(),
                title: "shot.png".into(),
                media_type: "image/png".into(),
                download_url: "/download/att1".into(),
            }],
            ..page("p1", "Home", &["Docs"], "<p>x</p>")
        }],
        attachments: HashMap::from([("att1".to_string(), b"PNGDATA".to_vec())]),
        ..MockSource::default()
    };
    let cancel = AtomicBool::new(false);

    let first = run_export(&job, &mut source, &mut no_progress(), &cancel).await;
    assert_eq!(first.status, RunStatus::Success);
    assert_eq!(first.attachments_written, 1);
    let att_path = tmp.path().join("attachments/att1 - shot.png");
    assert_eq!(std::fs::read(&att_path).unwrap(), b"PNGDATA");

    let second = run_export(&job, &mut source, &mut no_progress(), &cancel).await;
    assert_eq!(second.attachments_written, 0, "변경 없는 첨부는 skip");
    assert_eq!(second.pages_written, 0);
}

#[tokio::test]
async fn jobs_export_attachments_not_fetched_when_disabled() {
    let tmp = TempDir::new();
    let job = make_job(tmp.path()); // include_attachments = false
    let mut source = MockSource {
        pages: vec![PageData {
            attachments: vec![AttachmentRef {
                id: "att1".into(),
                title: "shot.png".into(),
                media_type: "image/png".into(),
                download_url: String::new(),
            }],
            ..page("p1", "Home", &["Docs"], "<p>x</p>")
        }],
        fail_attachments: true,
        ..MockSource::default()
    };

    let outcome = run_export(&job, &mut source, &mut no_progress(), &AtomicBool::new(false)).await;
    assert_eq!(outcome.status, RunStatus::Success);
    assert!(!tmp.path().join("attachments").exists());
}

#[tokio::test]
async fn jobs_export_filename_sanitized() {
    let tmp = TempDir::new();
    let job = make_job(tmp.path());
    let mut source = MockSource {
        pages: vec![
            page("p1", "bad/name:*?\"<>|", &["Docs"], "<p>1</p>"),
            page("p2", "", &["Docs"], "<p>2</p>"),
            page("p3", "CON", &["Docs"], "<p>3</p>"),
            page("p4", &"A".repeat(200), &["Docs"], "<p>4</p>"),
        ],
        ..MockSource::default()
    };

    let outcome = run_export(&job, &mut source, &mut no_progress(), &AtomicBool::new(false)).await;
    assert_eq!(outcome.status, RunStatus::Success);

    let root = tmp.path().join("Docs");
    let names: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"01 - bad_name_______.md".to_string()), "{names:?}");
    assert!(names.contains(&"02 - _untitled.md".to_string()));
    assert!(names.contains(&"03 - CON_.md".to_string()), "Windows 예약명 처리");
    let long = names.iter().find(|n| n.starts_with("04 - ")).unwrap();
    assert!(long.chars().count() <= "04 - ".len() + 120 + 3, "파일명 길이 제한");
}

#[tokio::test]
async fn jobs_export_failed_source_records_failed_history() {
    let tmp = TempDir::new();
    let job = make_job(tmp.path());
    let mut source = MockSource {
        fail_tree: true,
        ..MockSource::default()
    };

    let outcome = run_export(&job, &mut source, &mut no_progress(), &AtomicBool::new(false)).await;
    assert_eq!(outcome.status, RunStatus::Failed);
    assert!(outcome.error.as_deref().unwrap().contains("트리"));
    assert_eq!(outcome.pages_written, 0);

    let history = list_runs(tmp.path());
    assert_eq!(history.len(), 1, "실패도 이력에 기록");
    assert_eq!(history[0].status, RunStatus::Failed);
    assert_eq!(history[0].error, outcome.error);
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

#[test]
fn jobs_history_append_list_caps_at_100() {
    let tmp = TempDir::new();
    assert!(list_runs(tmp.path()).is_empty(), "파일 없으면 빈 목록");

    for i in 0..105 {
        let mut outcome = stub_outcome("job-1");
        outcome.run_id = format!("run-{i:03}");
        append_run(tmp.path(), &outcome).unwrap();
    }

    let runs = list_runs(tmp.path());
    assert_eq!(runs.len(), 100, "최근 100건 유지");
    assert_eq!(runs.first().unwrap().run_id, "run-005", "오래된 항목 제거");
    assert_eq!(runs.last().unwrap().run_id, "run-104");
    assert!(runs.windows(2).all(|w| w[0].started_at <= w[1].started_at || w[0].run_id < w[1].run_id));
}

#[test]
fn jobs_running_exports_registry_skip_and_cancel() {
    let registry = RunningExports::new();
    assert!(!registry.is_running("j1"));
    let guard = registry.try_start("j1").unwrap();
    assert!(registry.is_running("j1"));
    assert!(registry.try_start("j1").is_none(), "실행 중 재시작 거부");
    assert!(!guard.cancel_flag().load(Ordering::SeqCst));
    assert!(registry.cancel("j1"));
    assert!(guard.cancel_flag().load(Ordering::SeqCst));
    assert!(!registry.cancel("j1-other"));

    drop(guard);
    assert!(!registry.is_running("j1"), "가드 해제 시 슬롯 반납");
    assert!(registry.try_start("j1").is_some(), "해제 후 재시작 가능");
}
