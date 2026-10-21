//! Pruning the `shortcuts.vdf.bak-<stamp>` copies that every write leaves behind.
//!
//! Rules, in the order they win:
//! - only files named exactly like our backups are ever looked at (`<file>.bak-YYYYMMDD-HHMMSS`, with an
//!   optional `-<n>` for same-second collisions); anything else in the folder stays,
//! - nothing is pruned when the live file is missing, because then a backup may be the only copy,
//! - the newest `keep` backups stay, and `keep` is at least 1, so the only backup is never deleted,
//! - a backup modified in the last hour stays whatever its rank,
//! - symlinks and folders are never deleted.
//!
//! Age is the file's modified time, which is when the backup was written. Pruning is best effort for
//! the caller: a backup that cannot be deleted is reported, not fatal.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How many backups a normal prune keeps.
pub const DEFAULT_KEEP: usize = 10;
/// A backup younger than this is never deleted.
pub const PROTECT_RECENT: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupFile {
    pub path: PathBuf,
    pub modified: SystemTime,
}

#[derive(Debug, Default)]
pub struct PruneReport {
    pub deleted: Vec<PathBuf>,
    pub kept: usize,
    pub failed: Vec<(PathBuf, io::Error)>,
}

/// `<live file name>.bak-` followed by the stamp `backup_stamp` writes (8 digits, `-`, 6 digits) and
/// an optional collision counter.
fn is_backup_name(name: &str, live_name: &str) -> bool {
    let Some(stamp) = name.strip_prefix(live_name).and_then(|rest| rest.strip_prefix(".bak-")) else { return false };
    let mut parts = stamp.splitn(3, '-');
    let digits = |s: Option<&str>, len: Option<usize>| {
        s.is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && len.is_none_or(|l| s.len() == l))
    };
    let (date, time, counter) = (parts.next(), parts.next(), parts.next());
    digits(date, Some(8)) && digits(time, Some(6)) && (counter.is_none() || digits(counter, None))
}

/// Our backups of `shortcuts_path`, newest first (ties broken by name, later name first).
pub fn list_backups(shortcuts_path: &Path) -> io::Result<Vec<BackupFile>> {
    let Some(live_name) = shortcuts_path.file_name().and_then(|n| n.to_str()) else { return Ok(Vec::new()) };
    let dir = match shortcuts_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let mut found = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if !name.to_str().is_some_and(|n| is_backup_name(n, live_name)) {
            continue;
        }
        let Ok(meta) = fs::symlink_metadata(entry.path()) else { continue };
        if !meta.is_file() {
            continue;
        }
        found.push(BackupFile { path: entry.path(), modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH) });
    }
    found.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| b.path.cmp(&a.path)));
    Ok(found)
}

/// Deletes the backups that the rules in the module doc allow. `now` is a parameter so tests do not
/// depend on the clock.
pub fn prune_backups(shortcuts_path: &Path, keep: usize, now: SystemTime) -> io::Result<PruneReport> {
    let mut report = PruneReport::default();
    if !shortcuts_path.is_file() {
        report.kept = list_backups(shortcuts_path).map(|b| b.len()).unwrap_or(0);
        return Ok(report);
    }
    let backups = list_backups(shortcuts_path)?;
    let keep = keep.max(1);
    for (rank, backup) in backups.iter().enumerate() {
        // A modified time in the future counts as recent: the clock moved, not the file.
        let recent = now.duration_since(backup.modified).map_or(true, |age| age < PROTECT_RECENT);
        if rank < keep || recent {
            report.kept += 1;
            continue;
        }
        match fs::remove_file(&backup.path) {
            Ok(()) => report.deleted.push(backup.path.clone()),
            Err(e) => {
                report.kept += 1;
                report.failed.push((backup.path.clone(), e));
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::FileTimes;

    fn temp_dir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("dpm_steam_backups_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    const NOW_SECS: u64 = 2_000_000_000;

    fn now() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(NOW_SECS)
    }

    /// Creates a file whose modified time is `age` before `now()`.
    fn make(dir: &Path, name: &str, age: Duration) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, name).unwrap();
        let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_times(FileTimes::new().set_modified(now() - age)).unwrap();
        path
    }

    fn hours(h: u64) -> Duration {
        Duration::from_secs(h * 3600)
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut n: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        n.sort();
        n
    }

    #[test]
    fn only_our_backup_names_count() {
        let live = "shortcuts.vdf";
        assert!(is_backup_name("shortcuts.vdf.bak-20261020-120000", live));
        assert!(is_backup_name("shortcuts.vdf.bak-20261020-120000-3", live));
        for bad in [
            "shortcuts.vdf",
            "shortcuts.vdf.bak-T",
            "shortcuts.vdf.bak-20261020-120000-",
            "shortcuts.vdf.bak-20261020-120000-x",
            "shortcuts.vdf.bak-20261020-12000",
            "shortcuts.vdf.bak-20261020-120000-1-2",
            "shortcuts.vdf.bak-20261020",
            "shortcuts.vdf.dpm-tmp",
            "other.vdf.bak-20261020-120000",
            "my notes.bak-20261020-120000",
        ] {
            assert!(!is_backup_name(bad, live), "{bad}");
        }
    }

    #[test]
    fn keeps_the_newest_n_and_deletes_older_ones() {
        let dir = temp_dir("keepn");
        make(&dir, "shortcuts.vdf", hours(0));
        for i in 1..=5u64 {
            make(&dir, &format!("shortcuts.vdf.bak-2026101{i}-100000"), hours(24 * i));
        }
        let report = prune_backups(&dir.join("shortcuts.vdf"), 2, now()).unwrap();
        assert_eq!(report.deleted.len(), 3);
        assert_eq!(report.kept, 2);
        assert_eq!(names(&dir), ["shortcuts.vdf", "shortcuts.vdf.bak-20261011-100000", "shortcuts.vdf.bak-20261012-100000"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_only_backup_is_never_deleted_even_with_keep_zero() {
        let dir = temp_dir("only");
        make(&dir, "shortcuts.vdf", hours(0));
        make(&dir, "shortcuts.vdf.bak-20261001-100000", hours(24 * 400));
        let report = prune_backups(&dir.join("shortcuts.vdf"), 0, now()).unwrap();
        assert!(report.deleted.is_empty());
        assert_eq!(report.kept, 1);
        assert_eq!(names(&dir).len(), 2);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_backup_from_the_last_hour_is_kept_whatever_its_rank() {
        let dir = temp_dir("recent");
        make(&dir, "shortcuts.vdf", hours(0));
        // Ranks 0 and 1 are old by mtime order only if the recent ones sort first, so make the recent
        // ones lose the ranking by giving keep = 1 and several fresh files.
        make(&dir, "shortcuts.vdf.bak-20261019-100000-2", Duration::from_secs(60));
        make(&dir, "shortcuts.vdf.bak-20261019-100000-1", Duration::from_secs(120));
        make(&dir, "shortcuts.vdf.bak-20261019-100000", Duration::from_secs(59 * 60));
        make(&dir, "shortcuts.vdf.bak-20261001-100000", hours(48));
        let report = prune_backups(&dir.join("shortcuts.vdf"), 1, now()).unwrap();
        assert_eq!(report.deleted.len(), 1);
        assert!(report.deleted[0].ends_with("shortcuts.vdf.bak-20261001-100000"));
        assert_eq!(report.kept, 3);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_backup_just_over_an_hour_old_can_go() {
        let dir = temp_dir("edge");
        make(&dir, "shortcuts.vdf", hours(0));
        make(&dir, "shortcuts.vdf.bak-20261019-100000", Duration::from_secs(10));
        make(&dir, "shortcuts.vdf.bak-20261018-100000", PROTECT_RECENT);
        let report = prune_backups(&dir.join("shortcuts.vdf"), 1, now()).unwrap();
        assert_eq!(report.deleted.len(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_modified_time_in_the_future_counts_as_recent() {
        let dir = temp_dir("future");
        make(&dir, "shortcuts.vdf", hours(0));
        make(&dir, "shortcuts.vdf.bak-20261019-100000", Duration::from_secs(10));
        let path = dir.join("shortcuts.vdf.bak-20261018-100000");
        fs::write(&path, "x").unwrap();
        let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_times(FileTimes::new().set_modified(now() + hours(5))).unwrap();
        let report = prune_backups(&dir.join("shortcuts.vdf"), 1, now()).unwrap();
        assert!(report.deleted.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn nothing_is_pruned_when_the_live_file_is_missing() {
        let dir = temp_dir("nolive");
        make(&dir, "shortcuts.vdf.bak-20261001-100000", hours(100));
        make(&dir, "shortcuts.vdf.bak-20261002-100000", hours(90));
        let report = prune_backups(&dir.join("shortcuts.vdf"), 1, now()).unwrap();
        assert!(report.deleted.is_empty());
        assert_eq!(report.kept, 2);
        assert_eq!(names(&dir).len(), 2);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn other_files_folders_and_other_vdf_backups_are_left_alone() {
        let dir = temp_dir("others");
        make(&dir, "shortcuts.vdf", hours(0));
        make(&dir, "shortcuts.vdf.bak-20261001-100000", hours(500));
        make(&dir, "shortcuts.vdf.bak-20261002-100000", hours(400));
        make(&dir, "shortcuts.vdf.bak-by-hand", hours(900));
        make(&dir, "localconfig.vdf.bak-20261001-100000", hours(900));
        make(&dir, "notes.txt", hours(900));
        fs::create_dir(dir.join("shortcuts.vdf.bak-20261003-100000")).unwrap();
        let report = prune_backups(&dir.join("shortcuts.vdf"), 1, now()).unwrap();
        assert_eq!(report.deleted.len(), 1);
        assert_eq!(
            names(&dir),
            [
                "localconfig.vdf.bak-20261001-100000",
                "notes.txt",
                "shortcuts.vdf",
                "shortcuts.vdf.bak-20261002-100000",
                "shortcuts.vdf.bak-20261003-100000",
                "shortcuts.vdf.bak-by-hand",
            ]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_with_a_backup_name_is_not_followed_or_deleted() {
        let dir = temp_dir("symlink");
        make(&dir, "shortcuts.vdf", hours(0));
        let precious = make(&dir, "precious.txt", hours(900));
        make(&dir, "shortcuts.vdf.bak-20261002-100000", hours(10));
        std::os::unix::fs::symlink(&precious, dir.join("shortcuts.vdf.bak-20261001-100000")).unwrap();
        prune_backups(&dir.join("shortcuts.vdf"), 1, now()).unwrap();
        assert!(precious.exists());
        assert!(fs::symlink_metadata(dir.join("shortcuts.vdf.bak-20261001-100000")).is_ok());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn same_age_backups_are_ranked_by_name_so_the_result_is_stable() {
        let dir = temp_dir("ties");
        make(&dir, "shortcuts.vdf", hours(0));
        for name in ["shortcuts.vdf.bak-20261001-100000", "shortcuts.vdf.bak-20261001-100000-1", "shortcuts.vdf.bak-20261001-100000-2"] {
            make(&dir, name, hours(10));
        }
        prune_backups(&dir.join("shortcuts.vdf"), 1, now()).unwrap();
        assert_eq!(names(&dir), ["shortcuts.vdf", "shortcuts.vdf.bak-20261001-100000-2"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_real_save_leaves_a_backup_the_pruner_recognises() {
        use super::super::shortcuts::{self, NewShortcut};
        use super::super::store::{backup_stamp, ShortcutsFile};
        let dir = temp_dir("real");
        let path = dir.join("shortcuts.vdf");
        fs::write(&path, super::super::vdf::fixtures::plain_file()).unwrap();
        let mut file = ShortcutsFile::load(&path).unwrap();
        shortcuts::add(&mut file.doc, &NewShortcut { app_name: "A".into(), exe: "/a".into(), start_dir: "/".into(), ..Default::default() }).unwrap();
        let report = file.save(&|| Ok(Vec::new()), &backup_stamp()).unwrap();
        let listed = list_backups(&path).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, report.backup.unwrap());
        let _ = fs::remove_dir_all(dir);
    }
}
