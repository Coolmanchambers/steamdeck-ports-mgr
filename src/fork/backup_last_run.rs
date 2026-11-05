//! What the last "Back up saves now" run left out (review W6). The backup zip only holds the ports it could copy, and its
//! date alone made the checkup say "Backed up today" when some ports failed. Each run writes `last-run.json` next to the
//! zips in `Saves Backup > Global Backups`: the date, when it ended, and the ports it could not back up (their shown names).
//! The checkup's Save backups line reads it and says "Last backup missed X" while that run is the newest backup.
//!
//! Nothing else reads the file: the backup lists, Restore saves and the cloud copy look only at `.zip` files.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const FILE_NAME: &str = "last-run.json";

/// A zip this much newer than the record was made by something else after that run (the run writes its zip first, then
/// the record): the record no longer speaks for the newest backup.
const NEWER_BY_SECS: u64 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastRun {
    /// The date the run used for its zip's name ("2026-11-04").
    pub date: String,
    /// When the run ended, seconds since 1970.
    pub at: u64,
    /// The ports it could not back up, by the name shown.
    pub missed: Vec<String>,
}

pub fn path_in(global_dir: &Path) -> PathBuf {
    global_dir.join(FILE_NAME)
}

/// Writes the record (atomically, like the app's other state files). The folder is made when it is not there.
pub fn write(global_dir: &Path, run: &LastRun) -> std::io::Result<()> {
    std::fs::create_dir_all(global_dir)?;
    let v = json!({"version": 1, "date": run.date, "at": run.at, "missed": run.missed});
    crate::core::files::write_atomic(&path_in(global_dir), v.to_string().as_bytes())
}

/// The record, or `None` when there is none or it cannot be read (then nothing is said about a missed port).
pub fn read(global_dir: &Path) -> Option<LastRun> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path_in(global_dir)).ok()?).ok()?;
    Some(LastRun {
        date: v.get("date").and_then(Value::as_str).unwrap_or_default().to_string(),
        at: v.get("at").and_then(Value::as_u64)?,
        missed: v.get("missed").and_then(Value::as_array).map(|a| a.iter().filter_map(|n| n.as_str().map(str::to_string)).collect()).unwrap_or_default(),
    })
}

/// The ports the newest backup run missed: the record's names while no zip is newer than it (`newest_zip`: when the
/// newest zip in the folder was made). Read only.
pub fn missed_by_newest(global_dir: &Path, newest_zip: Option<SystemTime>) -> Vec<String> {
    let Some(run) = read(global_dir) else { return Vec::new() };
    let zip_secs = newest_zip.and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs());
    match zip_secs {
        Some(z) if z > run.at + NEWER_BY_SECS => Vec::new(),
        _ => run.missed,
    }
}

/// Now, in seconds since 1970.
pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn run(at: u64, missed: &[&str]) -> LastRun {
        LastRun { date: "2026-11-04".into(), at, missed: missed.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn the_record_round_trips_and_a_missing_or_broken_one_says_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let global = tmp.path().join("Global Backups");
        assert_eq!(read(&global), None);
        assert!(missed_by_newest(&global, None).is_empty());
        write(&global, &run(1_000, &["Dusklight", "Ship of Harkinian"])).unwrap();
        assert_eq!(read(&global), Some(run(1_000, &["Dusklight", "Ship of Harkinian"])));
        std::fs::write(path_in(&global), "{ not json").unwrap();
        assert_eq!(read(&global), None);
        assert!(!global.join("last-run.json.tmp").exists());
    }

    #[test]
    fn the_missed_ports_count_only_while_that_run_is_the_newest_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let global = tmp.path().join("Global Backups");
        write(&global, &run(1_000, &["Dusklight"])).unwrap();
        let at = |s: u64| Some(UNIX_EPOCH + Duration::from_secs(s));
        assert_eq!(missed_by_newest(&global, None), ["Dusklight"], "the run made no zip at all");
        assert_eq!(missed_by_newest(&global, at(990)), ["Dusklight"], "its own zip, written just before the record");
        assert_eq!(missed_by_newest(&global, at(1_030)), ["Dusklight"], "a clock a little off is not a newer backup");
        assert!(missed_by_newest(&global, at(5_000)).is_empty(), "a newer zip came after that run");
        write(&global, &run(6_000, &[])).unwrap();
        assert!(missed_by_newest(&global, at(5_990)).is_empty(), "the next run missed nothing");
    }
}
