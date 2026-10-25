//! Schedule occurrence computation and the background run loop
//! (port of `scheduler.py`).
//!
//! Ported rules:
//! * `daily` / `multiple_daily`: every day.
//! * `weekdays`: Monday–Friday.
//! * `weekly`: ISO weekday in `days_of_week` (empty set never matches).
//! * `monthly`: `day_of_month`, no occurrence in months lacking that day.
//! * Occurrences are judged per matching day × times with `start < t <= end`
//!   in the schedule's timezone (default Asia/Seoul in the original app;
//!   now configurable per job).
//! * Misfire policy: skip occurrences older than the grace window.
//! * Concurrency policy: skip_if_running (per-job running set).

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, TimeZone, Utc};
use chrono_tz::{Asia::Seoul, Tz};
use tokio::sync::watch;

use super::export::{ExportOutcome, RunningExports};
use super::models::{Job, Schedule, ScheduleKind};

pub const DEFAULT_TICK: StdDuration = StdDuration::from_secs(30);
/// Occurrences older than this (relative to the observing tick) are treated
/// as misfired and skipped (original default: 120s).
pub const DEFAULT_MISFIRE_GRACE: Duration = Duration::seconds(120);

pub struct Scheduler;

fn last_day_of_month(year: i32, month: u32) -> u32 {
    let (next_y, next_m) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(next_y, next_m, 1)
        .and_then(|d| d.pred_opt())
        .map(|d| d.day())
        .unwrap_or(31)
}

fn day_matches(schedule: &Schedule, day: NaiveDate) -> bool {
    match schedule.kind {
        ScheduleKind::Daily | ScheduleKind::MultipleDaily => true,
        ScheduleKind::Weekdays => day.weekday().number_from_monday() <= 5,
        ScheduleKind::Weekly => {
            !schedule.days_of_week.is_empty()
                && schedule
                    .days_of_week
                    .contains(&day.weekday().number_from_monday())
        }
        ScheduleKind::Monthly => match schedule.day_of_month {
            Some(target) => {
                target <= last_day_of_month(day.year(), day.month()) && day.day() == target
            }
            None => false,
        },
    }
}

impl Scheduler {
    /// All scheduled occurrences `t` with `start < t <= end` (UTC instants),
    /// computed in the job's timezone. Disabled/paused jobs yield none.
    pub fn occurrences_between(
        job: &Job,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Vec<DateTime<Utc>> {
        if !job.enabled || !job.schedule.enabled || job.schedule.paused {
            return Vec::new();
        }
        if end <= start {
            return Vec::new();
        }
        let schedule = job.schedule.normalized();
        let tz: Tz = schedule.timezone.parse().unwrap_or(Seoul);
        let start_local = start.with_timezone(&tz);
        let end_local = end.with_timezone(&tz);
        let times: Vec<(u32, u32)> = schedule
            .times
            .iter()
            .filter_map(|t| super::models::parse_hhmm(t))
            .collect();

        let mut occurrences = Vec::new();
        let mut day = Some(start_local.date_naive());
        let end_day = end_local.date_naive();
        while let Some(current) = day {
            if day_matches(&schedule, current) {
                for (hour, minute) in &times {
                    let Some(naive) = current.and_hms_opt(*hour, *minute, 0) else {
                        continue;
                    };
                    // DST gap (no such local time) or ambiguous local time
                    // → skip that occurrence.
                    if let LocalResult::Single(local) = tz.from_local_datetime(&naive) {
                        let t: DateTime<Utc> = local.with_timezone(&Utc);
                        if start < t && t <= end {
                            occurrences.push(t);
                        }
                    }
                }
            }
            if current >= end_day {
                break;
            }
            day = current.succ_opt();
        }
        occurrences.sort();
        occurrences
    }

    /// Next occurrence strictly after `now` (port of `next_run_at`).
    pub fn next_occurrence(job: &Job, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        Scheduler::occurrences_between(job, now, now + Duration::days(400))
            .into_iter()
            .next()
    }

    /// Background loop: every `tick` (30s in production) it collects
    /// occurrences from the last day that have not been handled yet, skips
    /// misfired ones (older than the grace window), skips jobs already
    /// running (skip_if_running) and spawns the run so the loop never blocks.
    /// `shutdown` is a `tokio::sync::watch` channel; setting it to `true`
    /// (or dropping the sender) stops the loop.
    pub async fn run_loop<J, F, Fut>(
        mut load_jobs: J,
        runner: F,
        tick: StdDuration,
        mut shutdown: watch::Receiver<bool>,
        running: Arc<RunningExports>,
    ) where
        J: FnMut() -> Vec<Job> + Send + 'static,
        F: Fn(Job, DateTime<Utc>) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = ExportOutcome> + Send + 'static,
    {
        let tick = tick.max(StdDuration::from_millis(10));
        let mut state = TickState::new(running);
        if *shutdown.borrow() {
            return;
        }
        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    match changed {
                        Ok(()) if !*shutdown.borrow() => continue,
                        _ => break,
                    }
                }
                _ = tokio::time::sleep(tick) => {
                    let now = Utc::now();
                    state.tick_once(now, &mut load_jobs, &runner).await;
                }
            }
        }
    }
}

/// Mutable state carried across scheduler ticks.
pub struct TickState {
    /// `(jobId|scheduledRfc3339)` keys already handled — prevents duplicate
    /// runs of one occurrence across ticks. Pruned to the last two days so
    /// memory stays bounded (small improvement over the original app).
    due_keys: HashMap<String, DateTime<Utc>>,
    running: Arc<RunningExports>,
    misfire_grace: Duration,
}

impl TickState {
    pub fn new(running: Arc<RunningExports>) -> Self {
        Self {
            due_keys: HashMap::new(),
            running,
            misfire_grace: DEFAULT_MISFIRE_GRACE,
        }
    }

    pub fn with_misfire_grace(mut self, grace: Duration) -> Self {
        self.misfire_grace = grace;
        self
    }

    /// One scheduler tick at `now`. Runs due jobs in spawned background
    /// tasks; the returned future resolves once dispatching is done (it does
    /// not wait for the spawned runs). Export history is recorded by
    /// `run_export` itself.
    pub async fn tick_once<J, F, Fut>(&mut self, now: DateTime<Utc>, load_jobs: &mut J, runner: &F)
    where
        J: FnMut() -> Vec<Job>,
        F: Fn(Job, DateTime<Utc>) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = ExportOutcome> + Send + 'static,
    {
        self.due_keys
            .retain(|_, scheduled| now.signed_duration_since(*scheduled) < Duration::days(2));

        for job in load_jobs() {
            if !job.enabled || !job.schedule.enabled || job.schedule.paused {
                continue;
            }
            let occurrences = Scheduler::occurrences_between(&job, now - Duration::days(1), now);
            for scheduled in occurrences {
                let key = format!("{}|{}", job.id, scheduled.to_rfc3339());
                if self.due_keys.insert(key, scheduled).is_some() {
                    continue; // already dispatched on an earlier tick
                }
                // misfire = skip
                if now.signed_duration_since(scheduled) > self.misfire_grace {
                    continue;
                }
                // concurrency = skip_if_running
                let Some(guard) = self.running.try_start(&job.id) else {
                    continue;
                };
                let runner = runner.clone();
                let job = job.clone();
                tokio::spawn(async move {
                    let _outcome = runner(job, scheduled).await;
                    drop(guard); // releases the running slot
                });
            }
        }
    }
}
