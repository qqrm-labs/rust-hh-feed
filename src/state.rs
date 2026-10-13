use anyhow::Result;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path};

pub type PostedJobs = HashMap<String, String>;
const DEFAULT_FETCH_WINDOW_HOURS: i64 = 24;
const FETCH_OVERLAP_MINUTES: i64 = 15;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostedJobsState {
    #[serde(default = "default_state_version")]
    pub version: u32,
    #[serde(default)]
    pub last_successful_run_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub jobs: PostedJobs,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StoredPostedJobsState {
    Legacy(PostedJobs),
    Versioned(PostedJobsState),
}

const STATE_VERSION: u32 = 2;

fn default_state_version() -> u32 {
    STATE_VERSION
}

impl Default for PostedJobsState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            last_successful_run_at: None,
            jobs: HashMap::new(),
        }
    }
}

pub fn load_posted_jobs(path: &Path) -> Result<PostedJobsState> {
    if !path.exists() {
        return Ok(PostedJobsState::default());
    }

    let data = fs::read_to_string(path)?;
    let state = match serde_json::from_str::<StoredPostedJobsState>(&data)? {
        StoredPostedJobsState::Legacy(jobs) => PostedJobsState {
            jobs,
            ..PostedJobsState::default()
        },
        StoredPostedJobsState::Versioned(state) => PostedJobsState {
            version: STATE_VERSION,
            ..state
        },
    };
    Ok(state)
}

pub fn save_posted_jobs(path: &Path, state: &PostedJobsState) -> Result<()> {
    let data = serde_json::to_string_pretty(state)?;
    crate::checkpoint::save(path, data.as_bytes())?;
    Ok(())
}

pub fn confirm_posted_jobs(path: &Path, state: &PostedJobsState) -> Result<()> {
    crate::checkpoint::confirm(path, &serde_json::to_vec_pretty(state)?)?;
    Ok(())
}

/// Remove entries older than `retention_days`.
pub fn prune_old_jobs(map: &mut PostedJobs, retention_days: i64) {
    let cutoff = Utc::now().date_naive() - Duration::days(retention_days);
    map.retain(|_, date| {
        NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map(|d| d >= cutoff)
            .unwrap_or(false)
    });
}

pub fn default_fetch_window_start(now: DateTime<Utc>) -> DateTime<Utc> {
    now - Duration::hours(DEFAULT_FETCH_WINDOW_HOURS)
}

pub fn fetch_window_start(
    last_successful_run_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    let overlap = Duration::minutes(FETCH_OVERLAP_MINUTES);
    last_successful_run_at
        .map(|timestamp| timestamp - overlap)
        .unwrap_or_else(|| default_fetch_window_start(now))
}

pub fn resolve_fetch_window_start(
    last_successful_run_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    backfill_hours: Option<i64>,
) -> DateTime<Utc> {
    let state_start = fetch_window_start(last_successful_run_at, now);

    match backfill_hours {
        Some(hours) if hours > 0 => {
            state_start.min(now - Duration::hours(hours) - Duration::minutes(FETCH_OVERLAP_MINUTES))
        }
        _ => state_start,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn load_and_save_cycle() {
        let path = PathBuf::from("test_posted_jobs.json");
        let mut state = PostedJobsState::default();
        state.jobs.insert("1".into(), "2024-07-08".into());
        state.last_successful_run_at = Some(Utc.with_ymd_and_hms(2026, 4, 3, 1, 17, 50).unwrap());
        save_posted_jobs(&path, &state).unwrap();
        let loaded = load_posted_jobs(&path).unwrap();
        assert_eq!(state, loaded);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn load_missing_returns_empty_state() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("missing.json");
        let state = load_posted_jobs(&path).unwrap();
        assert!(state.jobs.is_empty());
        assert!(state.last_successful_run_at.is_none());
        assert_eq!(state.version, STATE_VERSION);
    }

    #[test]
    fn load_legacy_state_returns_jobs_without_success_timestamp() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("legacy.json");
        fs::write(&path, "{\"1\":\"2024-07-08\"}").unwrap();

        let state = load_posted_jobs(&path).unwrap();

        assert_eq!(state.jobs.get("1").map(String::as_str), Some("2024-07-08"));
        assert!(state.last_successful_run_at.is_none());
        assert_eq!(state.version, STATE_VERSION);
    }

    #[test]
    fn load_invalid_json_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.json");
        fs::write(&path, "not json").unwrap();
        let result = load_posted_jobs(&path);
        assert!(result.is_err());
    }

    #[test]
    fn save_fails_on_directory() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("subdir");
        fs::create_dir(&path).unwrap();
        let state = PostedJobsState::default();
        let result = save_posted_jobs(&path, &state);
        assert!(result.is_err());
    }

    #[test]
    fn prune_old_jobs_removes_outdated_entries() {
        let mut map = PostedJobs::new();
        map.insert("old".into(), "2024-01-01".into());
        let recent = Utc::now().date_naive().to_string();
        map.insert("recent".into(), recent.clone());
        prune_old_jobs(&mut map, 30);
        assert!(map.contains_key("recent"));
        assert!(!map.contains_key("old"));
    }

    #[test]
    fn prune_old_jobs_drops_invalid_dates() {
        let mut map = PostedJobs::new();
        map.insert("bad".into(), "not a date".into());
        prune_old_jobs(&mut map, 30);
        assert!(map.is_empty());
    }

    #[test]
    fn fetch_window_start_uses_last_success_with_overlap() {
        let now = Utc.with_ymd_and_hms(2026, 4, 3, 8, 0, 0).unwrap();
        let last_success = Utc.with_ymd_and_hms(2026, 4, 3, 7, 30, 0).unwrap();

        let start = fetch_window_start(Some(last_success), now);

        assert_eq!(start, Utc.with_ymd_and_hms(2026, 4, 3, 7, 15, 0).unwrap());
    }

    #[test]
    fn fetch_window_start_falls_back_to_default_window_without_success_timestamp() {
        let now = Utc.with_ymd_and_hms(2026, 4, 3, 8, 0, 0).unwrap();

        let start = fetch_window_start(None, now);

        assert_eq!(start, Utc.with_ymd_and_hms(2026, 4, 2, 8, 0, 0).unwrap());
    }

    #[test]
    fn resolve_fetch_window_start_uses_earlier_backfill_window() {
        let now = Utc.with_ymd_and_hms(2026, 4, 3, 8, 0, 0).unwrap();
        let last_success = Utc.with_ymd_and_hms(2026, 4, 3, 7, 30, 0).unwrap();

        let start = resolve_fetch_window_start(Some(last_success), now, Some(48));

        assert_eq!(start, Utc.with_ymd_and_hms(2026, 4, 1, 7, 45, 0).unwrap());
    }

    #[test]
    fn resolve_fetch_window_start_uses_backfill_window_even_when_it_only_expands_slightly() {
        let now = Utc.with_ymd_and_hms(2026, 4, 3, 8, 0, 0).unwrap();
        let last_success = Utc.with_ymd_and_hms(2026, 4, 3, 7, 30, 0).unwrap();

        let start = resolve_fetch_window_start(Some(last_success), now, Some(1));

        assert_eq!(start, Utc.with_ymd_and_hms(2026, 4, 3, 6, 45, 0).unwrap());
    }
}
