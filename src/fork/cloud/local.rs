//! Which files in `Saves Backup` are candidates for the cloud copy.
//!
//! Only zips in the three places the app writes backups: `Global Backups/<date>.zip`,
//! `Update Backups/<folder>/<stamp>.zip` and `Restore Backups/<folder>/<stamp>.zip`. Never `Pending
//! Restore`, never a scratch folder (`_backup_*`, `_update_backup_*`, `_restore_*`, `_cloud_download_*`), never
//! a link, an empty file or a hidden file. A zip is only taken once it reads back (its central directory is at
//! the end, so a half-written one does not) and has not changed for `min_age` seconds; one that is not ready
//! is counted as deferred so the caller looks again soon.

use crate::fork::save_restore::zip_index::read_index;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub const GLOBAL_DIR: &str = "Global Backups";
pub const UPDATE_DIR: &str = "Update Backups";
pub const RESTORE_DIR: &str = "Restore Backups";

/// A zip that does not read back and is older than this is called broken and skipped for good.
const BROKEN_AFTER_SECS: u64 = 10 * 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalZip {
    /// Path under `Saves Backup`, `/` separators.
    pub rel: String,
    pub path: PathBuf,
    pub size: u64,
    pub mtime: u64,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Scan {
    pub zips: Vec<LocalZip>,
    /// Not ready yet (too new, or not readable yet): look again soon.
    pub deferred: usize,
    /// Old and unreadable: never uploaded.
    pub broken: usize,
}

fn plain_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with(['.', '_']) && !name.chars().any(|c| c.is_control() || c == '/' || c == '\\')
}

fn is_real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_dir())
}

fn entries(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|e| Some((e.file_name().into_string().ok()?, e.path()))).collect();
    out.sort();
    out
}

fn consider(scan: &mut Scan, rel: String, path: PathBuf, now: u64, min_age: u64) {
    let Ok(meta) = fs::symlink_metadata(&path) else { return };
    if !meta.file_type().is_file() || meta.len() == 0 {
        return;
    }
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    // A time in the future counts as new: the clock moved, not the file.
    let age = now.saturating_sub(mtime);
    let too_new = mtime > now || age < min_age;
    if too_new {
        scan.deferred += 1;
    } else if read_index(&path).is_err() {
        if age < BROKEN_AFTER_SECS {
            scan.deferred += 1;
        } else {
            scan.broken += 1;
        }
    } else {
        scan.zips.push(LocalZip { rel, path, size: meta.len(), mtime });
    }
}

pub fn scan(saves_backup_dir: &Path, now: u64, min_age: u64) -> Scan {
    let mut scan = Scan::default();
    let zip_name = |name: &str| plain_name(name) && name.len() > 4 && name.ends_with(".zip");
    let global = saves_backup_dir.join(GLOBAL_DIR);
    if is_real_dir(&global) {
        for (name, path) in entries(&global).into_iter().filter(|(n, _)| zip_name(n)) {
            consider(&mut scan, format!("{GLOBAL_DIR}/{name}"), path, now, min_age);
        }
    }
    for top in [UPDATE_DIR, RESTORE_DIR] {
        let top_dir = saves_backup_dir.join(top);
        if !is_real_dir(&top_dir) {
            continue;
        }
        for (folder, folder_path) in entries(&top_dir).into_iter().filter(|(n, p)| plain_name(n) && is_real_dir(p)) {
            for (name, path) in entries(&folder_path).into_iter().filter(|(n, _)| zip_name(n)) {
                consider(&mut scan, format!("{top}/{folder}/{name}"), path, now, min_age);
            }
        }
    }
    scan
}

#[cfg(test)]
pub(crate) mod testing {
    use std::io::Write;
    use std::path::Path;

    /// A tiny but valid zip: one stored entry, so `read_index` accepts it.
    pub fn tiny_zip(entry: &str, data: &[u8]) -> Vec<u8> {
        let crc = crc32(data);
        let name = entry.as_bytes();
        let mut out = Vec::new();
        out.extend_from_slice(&0x04034b50u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(data);
        let cd_offset = out.len() as u32;
        out.extend_from_slice(&0x02014b50u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&[0; 12]);
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(name);
        let cd_size = out.len() as u32 - cd_offset;
        out.extend_from_slice(&0x06054b50u32.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in data {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        !crc
    }

    pub fn write_zip(path: &Path, entry: &str, data: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(&tiny_zip(entry, data)).unwrap();
    }

    pub fn set_mtime(path: &Path, unix: u64) {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(unix)).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dpm_cloud_local_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    const NOW: u64 = 2_000_000_000;

    fn old(path: &Path) {
        set_mtime(path, NOW - 3_600);
    }

    #[test]
    fn the_three_backup_kinds_are_found_with_their_relative_paths() {
        let dir = temp_dir("kinds");
        for rel in ["Global Backups/2026-10-24.zip", "Update Backups/soh/2026-10-24_10-00-00.zip", "Restore Backups/soh/2026-10-24_11-00-00.zip"] {
            write_zip(&dir.join(rel), "a.sav", b"data");
            old(&dir.join(rel));
        }
        let scan = scan(&dir, NOW, 60);
        let rels: Vec<&str> = scan.zips.iter().map(|z| z.rel.as_str()).collect();
        assert_eq!(rels, ["Global Backups/2026-10-24.zip", "Update Backups/soh/2026-10-24_10-00-00.zip", "Restore Backups/soh/2026-10-24_11-00-00.zip"]);
        assert_eq!(scan.deferred + scan.broken, 0);
        assert!(scan.zips.iter().all(|z| z.size > 0 && z.mtime == NOW - 3_600));
    }

    #[test]
    fn pending_restore_and_scratch_folders_are_ignored() {
        let dir = temp_dir("scratch");
        for rel in [
            "Pending Restore/soh/save_folder/x.zip",
            "_backup_abc/Global Backups/x.zip",
            "Update Backups/_update_backup_abc/x.zip",
            "Restore Backups/_restore_abc/x.zip",
            "_cloud_download_abc/x.zip",
            "Global Backups/.hidden.zip",
            "Global Backups/_scratch.zip",
            "Update Backups/soh/notes.txt",
            "Global Backups/x.zip.tmp",
            "Update Backups/soh/deeper/more/x.zip",
        ] {
            write_zip(&dir.join(rel), "a", b"data");
            old(&dir.join(rel));
        }
        let s = scan(&dir, NOW, 60);
        assert!(s.zips.is_empty(), "{:?}", s.zips);
    }

    #[test]
    fn a_zip_changed_a_moment_ago_waits_and_is_counted() {
        let dir = temp_dir("new");
        let p = dir.join("Global Backups/new.zip");
        write_zip(&p, "a", b"data");
        set_mtime(&p, NOW - 5);
        let s = scan(&dir, NOW, 60);
        assert!((s.zips.len(), s.deferred) == (0, 1), "{s:?}");
        let s = scan(&dir, NOW, 3);
        assert!((s.zips.len(), s.deferred) == (1, 0));
        set_mtime(&p, NOW + 500);
        assert_eq!(scan(&dir, NOW, 0).deferred, 1, "a time in the future is new");
    }

    #[test]
    fn a_half_written_zip_waits_and_an_old_unreadable_one_is_broken() {
        let dir = temp_dir("half");
        let half = dir.join("Update Backups/soh/half.zip");
        fs::create_dir_all(half.parent().unwrap()).unwrap();
        fs::write(&half, b"PK\x03\x04 not finished").unwrap();
        set_mtime(&half, NOW - 120);
        let s = scan(&dir, NOW, 60);
        assert!((s.zips.len(), s.deferred, s.broken) == (0, 1, 0), "{s:?}");
        set_mtime(&half, NOW - 3_600);
        let s = scan(&dir, NOW, 60);
        assert!((s.zips.len(), s.deferred, s.broken) == (0, 0, 1), "{s:?}");
    }

    #[test]
    fn empty_files_are_ignored() {
        let dir = temp_dir("empty");
        let p = dir.join("Global Backups/empty.zip");
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, b"").unwrap();
        old(&p);
        assert_eq!(scan(&dir, NOW, 60), Scan::default());
    }

    #[cfg(unix)]
    #[test]
    fn links_are_never_followed() {
        let dir = temp_dir("links");
        let real = dir.join("elsewhere.zip");
        write_zip(&real, "a", b"data");
        old(&real);
        fs::create_dir_all(dir.join("Global Backups")).unwrap();
        std::os::unix::fs::symlink(&real, dir.join("Global Backups/link.zip")).unwrap();
        let other = dir.join("elsewhere_dir");
        write_zip(&other.join("x.zip"), "a", b"data");
        old(&other.join("x.zip"));
        fs::create_dir_all(dir.join("Update Backups")).unwrap();
        std::os::unix::fs::symlink(&other, dir.join("Update Backups/linked")).unwrap();
        assert!(scan(&dir, NOW, 60).zips.is_empty());
    }

    #[test]
    fn a_missing_folder_is_an_empty_scan() {
        let dir = temp_dir("none");
        assert_eq!(scan(&dir.join("nope"), NOW, 60), Scan::default());
    }
}
