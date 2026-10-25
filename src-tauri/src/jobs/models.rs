//! Job / schedule / export-option models (port of `models.py`), typed and
//! camelCase-serialized per the architecture golden rules.

use serde::{Deserialize, Serialize};

pub const DEFAULT_TIMEZONE: &str = "Asia/Seoul";

fn default_true() -> bool {
    true
}

fn default_times() -> Vec<String> {
    vec!["09:00".to_string()]
}

fn default_days_of_week() -> Vec<u32> {
    vec![1, 2, 3, 4, 5]
}

fn default_max_workers() -> u32 {
    8
}

fn default_timezone() -> String {
    DEFAULT_TIMEZONE.to_string()
}

/// Parse a `"HH:MM"` time (lenient about zero padding: `"9:00"` is valid).
pub fn parse_hhmm(value: &str) -> Option<(u32, u32)> {
    let (hour, minute) = value.trim().split_once(':')?;
    if hour.is_empty() || minute.is_empty() || hour.len() > 2 || minute.len() > 2 {
        return None;
    }
    if !hour.bytes().all(|b| b.is_ascii_digit()) || !minute.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hour = hour.parse::<u32>().ok()?;
    let minute = minute.parse::<u32>().ok()?;
    if hour > 23 || minute > 59 {
        None
    } else {
        Some((hour, minute))
    }
}

fn valid_timezone(tz: &str) -> bool {
    tz.parse::<chrono_tz::Tz>().is_ok()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScheduleKind {
    #[serde(alias = "multiple_daily")]
    MultipleDaily,
    Weekdays,
    Weekly,
    Monthly,
    /// Fallback for unknown kinds (ports the original default of "daily").
    #[default]
    #[serde(other)]
    Daily,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub kind: ScheduleKind,
    #[serde(default = "default_times")]
    pub times: Vec<String>,
    /// ISO weekdays (Mon=1..Sun=7).
    #[serde(default = "default_days_of_week")]
    pub days_of_week: Vec<u32>,
    #[serde(default)]
    pub day_of_month: Option<u32>,
    /// IANA timezone name.
    #[serde(default = "default_timezone")]
    pub timezone: String,
    #[serde(default)]
    pub paused: bool,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            enabled: false,
            kind: ScheduleKind::Daily,
            times: default_times(),
            days_of_week: default_days_of_week(),
            day_of_month: None,
            timezone: default_timezone(),
            paused: false,
        }
    }
}

impl Schedule {
    /// Normalized copy: times parsed/sorted/deduped (canonical `HH:MM`,
    /// falling back to 09:00 when nothing valid remains — ports
    /// `schedule_times()`), weekdays filtered to 1..=7 and sorted, an invalid
    /// `day_of_month` dropped, and an unknown timezone reset to the default.
    pub fn normalized(&self) -> Schedule {
        let mut times: Vec<(u32, u32)> = self.times.iter().filter_map(|t| parse_hhmm(t)).collect();
        times.sort_unstable();
        times.dedup();
        let times = if times.is_empty() {
            vec![(9, 0)]
        } else {
            times
        };

        let mut days_of_week: Vec<u32> = self
            .days_of_week
            .iter()
            .copied()
            .filter(|d| (1..=7).contains(d))
            .collect();
        days_of_week.sort_unstable();
        days_of_week.dedup();

        Schedule {
            enabled: self.enabled,
            kind: self.kind,
            times: times
                .iter()
                .map(|(h, m)| format!("{h:02}:{m:02}"))
                .collect(),
            days_of_week,
            day_of_month: self.day_of_month.filter(|d| (1..=31).contains(d)),
            timezone: if valid_timezone(&self.timezone) {
                self.timezone.clone()
            } else {
                DEFAULT_TIMEZONE.to_string()
            },
            paused: self.paused,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    #[serde(default = "default_true")]
    pub skip_unchanged: bool,
    #[serde(default = "default_true")]
    pub cleanup_stale: bool,
    #[serde(default = "default_true")]
    pub page_breadcrumbs: bool,
    #[serde(default = "default_true")]
    pub front_matter: bool,
    #[serde(default = "default_max_workers")]
    pub max_workers: u32,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            skip_unchanged: true,
            cleanup_stale: true,
            page_breadcrumbs: true,
            front_matter: true,
            max_workers: default_max_workers(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub name: String,
    /// Export target space.
    pub space_id: String,
    /// When set, only this page's subtree is exported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_page_id: Option<String>,
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

impl Job {
    /// Normalized copy (trimmed strings, canonical schedule).
    pub fn normalized(&self) -> Job {
        let id = self.id.trim().to_string();
        let root_page_id = self
            .root_page_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        Job {
            id,
            name: self.name.trim().to_string(),
            space_id: self.space_id.trim().to_string(),
            root_page_id,
            target_dir: self.target_dir.trim().to_string(),
            include_attachments: self.include_attachments,
            enabled: self.enabled,
            export_options: self.export_options.clone(),
            schedule: self.schedule.normalized(),
        }
    }
}
