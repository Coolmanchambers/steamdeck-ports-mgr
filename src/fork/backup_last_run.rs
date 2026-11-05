//! What the last "Back up saves now" run left out (review W6). The backup zip only holds the ports it could copy, and its
//! date alone made the checkup say "Backed up today" when some ports failed. Each run writes `last-run.json` next to the
//! zips in `Saves Backup > Global Backups`: the date, when it ended, the ports it could not back up (their shown names),
//! and which zip was the newest when it ended. The checkup's Save backups line reads it and says "Last backup missed X"
//! while that zip is still the newest (a later backup, or one copied in, ends it).
//!
//! Nothing else reads the file: the backup lists, Restore saves and the cloud copy look only at `.zip` files.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const FILE_NAME: &str = "last-run.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastRun {
    /// The date the run used for its zip's name ("2026-11-04").
    pub date: String,
    /// When the run ended, seconds since 1970.
    pub at: u64,
    /// The ports it could not back up, by the name shown.
    pub missed: Vec<String>,
    /// The newest zip in the folder when the run ended (its file name and modified time in seconds), `None` when there
    /// was none: the record speaks for the newest backup while that is still the newest.
    pub newest: Option<(String, u64)>,
}

pub fn path_in(global_dir: &Path) -> PathBuf {
    global_dir.join(FILE_NAME)
}

/// The newest `.zip` in the folder by modified time (the name breaks a tie), as its file name and time in seconds.
pub fn newest_zip(global_dir: &Path) -> Option<(String, u64)> {
    std::fs::read_dir(global_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")))
        .filter_map(|e| {
            let meta = e.metadata().ok().filter(std::fs::Metadata::is_file)?;
            let secs = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
            Some((secs, e.file_name().to_string_lossy().into_owned()))
        })
        .max()
        .map(|(secs, name)| (name, secs))
}

/// Writes the record (atomically, like the app's other state files), noting the newest zip there now. The folder is made
/// when it is not there.
pub fn write(global_dir: &Path, date: &str, at: u64, missed: Vec<String>) -> std::io::Result<LastRun> {
    std::fs::create_dir_all(global_dir)?;
    let run = LastRun { date: date.to_string(), at, missed, newest: newest_zip(global_dir) };
    let newest = run.newest.as_ref().map(|(name, mtime)| json!({"name": name, "mtime": mtime}));
    let v = json!({"version": 1, "date": run.date, "at": run.at, "missed": run.missed, "newest_zip": newest});
    crate::core::files::write_atomic(&path_in(global_dir), v.to_string().as_bytes())?;
    Ok(run)
}

/// The record, or `None` when there is none or it cannot be read (then nothing is said about a missed port).
pub fn read(global_dir: &Path) -> Option<LastRun> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path_in(global_dir)).ok()?).ok()?;
    let newest = match v.get("newest_zip") {
        Some(Value::Null) => None,
        Some(z) => Some((z.get("name")?.as_str()?.to_string(), z.get("mtime")?.as_u64()?)),
        None => return None,
    };
    Some(LastRun {
        date: v.get("date").and_then(Value::as_str).unwrap_or_default().to_string(),
        at: v.get("at").and_then(Value::as_u64)?,
        missed: v.get("missed").and_then(Value::as_array).map(|a| a.iter().filter_map(|n| n.as_str().map(str::to_string)).collect()).unwrap_or_default(),
        newest,
    })
}

/// The ports the newest backup run missed: the record's names while the newest zip is the one it noted. Read only.
pub fn missed_by_newest(global_dir: &Path) -> Vec<String> {
    match read(global_dir) {
        Some(run) if run.newest == newest_zip(global_dir) => run.missed,
        _ => Vec::new(),
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

    fn zip(dir: &Path, name: &str, secs: u64) {
        let path = dir.join(name);
        std::fs::write(&path, b"PK").unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(UNIX_EPOCH + Duration::from_secs(secs)).unwrap();
    }

    #[test]
    fn the_record_round_trips_and_a_missing_or_broken_one_says_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let global = tmp.path().join("Global Backups");
        assert_eq!(read(&global), None);
        assert!(missed_by_newest(&global).is_empty());
        let run = write(&global, "2026-11-04", 1_000, vec!["Dusklight".into(), "Ship of Harkinian".into()]).unwrap();
        assert_eq!(run.newest, None, "no zip there");
        assert_eq!(read(&global), Some(run));
        std::fs::write(path_in(&global), "{ not json").unwrap();
        assert_eq!(read(&global), None);
        assert!(!global.join("last-run.json.tmp").exists());
        assert_eq!(newest_zip(&global), None, "the record itself is not a backup");
    }

    #[test]
    fn the_missed_ports_count_only_while_the_zip_it_noted_is_the_newest() {
        let tmp = tempfile::tempdir().unwrap();
        let global = tmp.path().join("Global Backups");
        std::fs::create_dir_all(&global).unwrap();
        // The run made no zip at all: an older one is the newest, and stays so.
        zip(&global, "2026-11-01.zip", 500);
        write(&global, "2026-11-04", 1_000, vec!["Dusklight".into()]).unwrap();
        assert_eq!(missed_by_newest(&global), ["Dusklight"]);
        // Its own zip, whatever its time (a clock off by hours changes nothing).
        zip(&global, "2026-11-04.zip", 99_000);
        write(&global, "2026-11-04", 1_000, vec!["Dusklight".into()]).unwrap();
        assert_eq!(missed_by_newest(&global), ["Dusklight"]);
        // A newer backup after it, even a minute later: no longer the newest run.
        zip(&global, "2026-11-04-2.zip", 99_030);
        assert!(missed_by_newest(&global).is_empty());
        // The next run missed nothing.
        write(&global, "2026-11-05", 2_000, Vec::new()).unwrap();
        assert!(missed_by_newest(&global).is_empty());
    }
}
