//! Loading and saving `shortcuts.vdf` safely.
//!
//! Steam rewrites this file from memory when it exits, so a write while Steam runs is lost or, worse,
//! races Steam's own write. A save therefore:
//! 1. refuses while a Steam process exists (or when the check itself fails),
//! 2. refuses if the file changed on disk since it was loaded,
//! 3. writes a timestamped backup of the current file and checks the copy,
//! 4. writes a temp file in the same folder, syncs it and renames it over the original,
//! 5. reads the result back and compares it.
//! A failure at any step leaves the original file as it was.

use super::vdf::{Document, ParseError};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const TEMP_SUFFIX: &str = ".dpm-tmp";
const MAX_BACKUP_COLLISIONS: u32 = 1000;
/// Process names (`/proc/<pid>/comm`) that mean Steam is running.
const STEAM_PROCESS_NAMES: [&str; 2] = ["steam", "steamwebhelper"];

#[derive(Debug)]
pub enum StoreError {
    SteamRunning(Vec<String>),
    SteamCheckFailed(io::Error),
    /// The file on disk is not the one that was loaded (Steam or another tool wrote to it).
    ChangedOnDisk,
    Parse { path: PathBuf, source: ParseError },
    /// The edited document did not encode to something that parses back to itself.
    EncodeCheckFailed,
    Io { action: &'static str, path: PathBuf, source: io::Error },
    /// The file on disk after the write is not what was written.
    VerifyFailed(PathBuf),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::SteamRunning(procs) => {
                write!(f, "Steam is running ({}); close it before changing shortcuts", procs.join(", "))
            }
            StoreError::SteamCheckFailed(e) => write!(f, "could not tell whether Steam is running: {e}"),
            StoreError::ChangedOnDisk => write!(f, "shortcuts.vdf changed since it was loaded; load it again"),
            StoreError::Parse { path, source } => write!(f, "{} is not a readable shortcuts file: {source}", path.display()),
            StoreError::EncodeCheckFailed => write!(f, "the edited shortcuts did not encode cleanly; nothing was written"),
            StoreError::Io { action, path, source } => write!(f, "could not {action} {}: {source}", path.display()),
            StoreError::VerifyFailed(path) => write!(f, "{} does not match what was written", path.display()),
        }
    }
}

impl std::error::Error for StoreError {}

fn io_err<'a>(action: &'static str, path: &'a Path) -> impl FnOnce(io::Error) -> StoreError + 'a {
    move |source| StoreError::Io { action, path: path.to_path_buf(), source }
}

/// Says which Steam processes are running. Injectable so tests never look at the real machine; any
/// `Fn() -> io::Result<Vec<String>>` works.
pub trait SteamProcessCheck {
    fn running_steam_processes(&self) -> io::Result<Vec<String>>;
}

impl<F: Fn() -> io::Result<Vec<String>>> SteamProcessCheck for F {
    fn running_steam_processes(&self) -> io::Result<Vec<String>> {
        self()
    }
}

/// The real check: scans `/proc` on Linux. Elsewhere it fails, which refuses the write.
pub struct SystemSteamCheck;

impl SteamProcessCheck for SystemSteamCheck {
    #[cfg(target_os = "linux")]
    fn running_steam_processes(&self) -> io::Result<Vec<String>> {
        steam_processes_in(Path::new("/proc"))
    }

    #[cfg(not(target_os = "linux"))]
    fn running_steam_processes(&self) -> io::Result<Vec<String>> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "Steam process check is only implemented for Linux"))
    }
}

/// Lists `pid name` for every Steam process under a `/proc`-shaped folder. A process that vanishes
/// while the scan runs is skipped; an unreadable folder is an error.
pub fn steam_processes_in(proc_root: &Path) -> io::Result<Vec<String>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(proc_root)? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name();
        let Some(pid) = name.to_str().filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) else {
            continue;
        };
        let Ok(comm) = fs::read_to_string(entry.path().join("comm")) else { continue };
        let comm = comm.trim_end_matches('\n');
        if STEAM_PROCESS_NAMES.contains(&comm) {
            found.push(format!("{pid} {comm}"));
        }
    }
    found.sort();
    Ok(found)
}

/// UTC time as it appears in backup names.
pub fn backup_stamp() -> String {
    chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveReport {
    /// False when the document encodes to exactly what is on disk: no backup, no write.
    pub wrote: bool,
    pub backup: Option<PathBuf>,
}

/// A loaded `shortcuts.vdf` plus the bytes it was loaded from.
pub struct ShortcutsFile {
    path: PathBuf,
    original: Option<Vec<u8>>,
    pub doc: Document,
}

impl ShortcutsFile {
    /// A missing file loads as an empty document (Steam has not created one yet).
    pub fn load(path: &Path) -> Result<ShortcutsFile, StoreError> {
        match fs::read(path) {
            Ok(bytes) => {
                let doc = Document::parse(&bytes).map_err(|source| StoreError::Parse { path: path.to_path_buf(), source })?;
                Ok(ShortcutsFile { path: path.to_path_buf(), original: Some(bytes), doc })
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                Ok(ShortcutsFile { path: path.to_path_buf(), original: None, doc: Document::empty() })
            }
            Err(e) => Err(io_err("read", path)(e)),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `stamp` names the backup; use [`backup_stamp`] outside tests.
    pub fn save(&self, steam: &dyn SteamProcessCheck, stamp: &str) -> Result<SaveReport, StoreError> {
        self.save_with(steam, stamp, &|_| Ok(()))
    }

    /// `before_rename` runs after the temp file is complete and before it replaces the original;
    /// tests use it to simulate a failure at that point.
    fn save_with(
        &self,
        steam: &dyn SteamProcessCheck,
        stamp: &str,
        before_rename: &dyn Fn(&Path) -> io::Result<()>,
    ) -> Result<SaveReport, StoreError> {
        match steam.running_steam_processes() {
            Ok(procs) if procs.is_empty() => {}
            Ok(procs) => return Err(StoreError::SteamRunning(procs)),
            Err(e) => return Err(StoreError::SteamCheckFailed(e)),
        }

        let current = match fs::read(&self.path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(io_err("read", &self.path)(e)),
        };
        if current != self.original {
            return Err(StoreError::ChangedOnDisk);
        }

        let encoded = self.doc.encode();
        if Document::parse(&encoded).as_ref() != Ok(&self.doc) {
            return Err(StoreError::EncodeCheckFailed);
        }
        if current.as_deref() == Some(encoded.as_slice()) {
            return Ok(SaveReport { wrote: false, backup: None });
        }

        let backup = match &current {
            Some(bytes) => Some(write_backup(&self.path, bytes, stamp)?),
            None => None,
        };

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(io_err("create folder", parent))?;
        }
        write_atomic(&self.path, &encoded, before_rename)?;

        let written = fs::read(&self.path).map_err(io_err("read back", &self.path))?;
        if written != encoded {
            return Err(StoreError::VerifyFailed(self.path.clone()));
        }
        Ok(SaveReport { wrote: true, backup })
    }
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

/// Copies the current bytes to `<file>.bak-<stamp>` (never over an existing backup) and checks the copy.
fn write_backup(path: &Path, bytes: &[u8], stamp: &str) -> Result<PathBuf, StoreError> {
    for attempt in 0..MAX_BACKUP_COLLISIONS {
        let suffix = if attempt == 0 { format!(".bak-{stamp}") } else { format!(".bak-{stamp}-{attempt}") };
        let backup = sibling(path, &suffix);
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&backup) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io_err("create backup", &backup)(e)),
        };
        file.write_all(bytes).map_err(io_err("write backup", &backup))?;
        file.sync_all().map_err(io_err("sync backup", &backup))?;
        drop(file);
        if fs::read(&backup).map_err(io_err("read backup", &backup))? != bytes {
            return Err(StoreError::VerifyFailed(backup));
        }
        return Ok(backup);
    }
    Err(io_err("create backup", path)(io::Error::new(io::ErrorKind::AlreadyExists, "too many backups with this timestamp")))
}

/// Temp file in the same folder, synced, then renamed over `path`. The temp file is removed if
/// anything fails, and `path` is untouched until the rename.
fn write_atomic(path: &Path, bytes: &[u8], before_rename: &dyn Fn(&Path) -> io::Result<()>) -> Result<(), StoreError> {
    let temp = sibling(path, TEMP_SUFFIX);
    let result = (|| {
        let mut file = File::create(&temp).map_err(io_err("create", &temp))?;
        file.write_all(bytes).map_err(io_err("write", &temp))?;
        file.sync_all().map_err(io_err("sync", &temp))?;
        drop(file);
        if let Ok(meta) = fs::metadata(path) {
            // Keep the original's permissions; a failure here is not worth losing the write over.
            let _ = fs::set_permissions(&temp, meta.permissions());
        }
        before_rename(&temp).map_err(io_err("finish", &temp))?;
        fs::rename(&temp, path).map_err(io_err("replace", path))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    } else if let Some(parent) = path.parent() {
        // Persist the rename itself; not available on every platform, so best effort.
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::super::shortcuts::{self, NewShortcut};
    use super::super::vdf::fixtures::*;
    use super::*;
    use std::cell::Cell;

    fn temp_dir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("dpm_steam_store_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn no_steam() -> io::Result<Vec<String>> {
        Ok(Vec::new())
    }

    fn port() -> NewShortcut {
        NewShortcut {
            app_name: "Ship".into(),
            exe: "/home/deck/Ports/ship".into(),
            start_dir: "/home/deck/Ports".into(),
            ..Default::default()
        }
    }

    fn setup(name: &str, bytes: &[u8]) -> (PathBuf, PathBuf) {
        let dir = temp_dir(name);
        let path = dir.join("config").join("shortcuts.vdf");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        (dir, path)
    }

    fn files_in(path: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn loading_and_saving_an_unchanged_file_writes_nothing() {
        for (i, bytes) in [plain_file(), odd_file(), EMPTY_FILE.to_vec()].into_iter().enumerate() {
            let (dir, path) = setup(&format!("unchanged{i}"), &bytes);
            let file = ShortcutsFile::load(&path).unwrap();
            assert_eq!(file.doc.encode(), bytes);
            let report = file.save(&no_steam, "T").unwrap();
            assert_eq!(report, SaveReport { wrote: false, backup: None });
            assert_eq!(fs::read(&path).unwrap(), bytes);
            assert_eq!(files_in(&path), ["shortcuts.vdf"]);
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn save_writes_a_backup_of_the_old_bytes_then_the_new_file() {
        let before = plain_file();
        let (dir, path) = setup("backup", &before);
        let mut file = ShortcutsFile::load(&path).unwrap();
        shortcuts::add(&mut file.doc, &port()).unwrap();
        let report = file.save(&no_steam, "20261020-120000").unwrap();

        assert!(report.wrote);
        let backup = report.backup.unwrap();
        assert_eq!(backup.file_name().unwrap(), "shortcuts.vdf.bak-20261020-120000");
        assert_eq!(fs::read(&backup).unwrap(), before);
        assert_eq!(fs::read(&path).unwrap(), file.doc.encode());
        assert_eq!(shortcuts::list(&ShortcutsFile::load(&path).unwrap().doc).len(), 3);
        assert_eq!(files_in(&path), ["shortcuts.vdf", "shortcuts.vdf.bak-20261020-120000"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_backup_exists_before_the_replacement_happens() {
        let before = plain_file();
        let (dir, path) = setup("order", &before);
        let mut file = ShortcutsFile::load(&path).unwrap();
        shortcuts::add(&mut file.doc, &port()).unwrap();
        let seen = Cell::new(false);
        let report = file
            .save_with(&no_steam, "T1", &|_| {
                let backup = path.with_file_name("shortcuts.vdf.bak-T1");
                assert_eq!(fs::read(&backup).unwrap(), before);
                assert_eq!(fs::read(&path).unwrap(), before, "original is still in place at this point");
                seen.set(true);
                Ok(())
            })
            .unwrap();
        assert!(seen.get() && report.wrote);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn backups_with_the_same_stamp_never_overwrite_each_other() {
        let (dir, path) = setup("collide", &plain_file());
        let mut file = ShortcutsFile::load(&path).unwrap();
        shortcuts::add(&mut file.doc, &port()).unwrap();
        let first = file.save(&no_steam, "S").unwrap().backup.unwrap();
        let mut file = ShortcutsFile::load(&path).unwrap();
        shortcuts::update(
            &mut file.doc,
            shortcuts::Target::Key("0"),
            &shortcuts::ShortcutUpdate { exe: Some("/moved".into()), ..Default::default() },
        )
        .unwrap();
        let second = file.save(&no_steam, "S").unwrap().backup.unwrap();
        assert_ne!(first, second);
        assert_eq!(second.file_name().unwrap(), "shortcuts.vdf.bak-S-1");
        assert_eq!(fs::read(&first).unwrap(), plain_file());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_is_refused_while_steam_runs_and_touches_nothing() {
        let before = plain_file();
        let (dir, path) = setup("running", &before);
        let mut file = ShortcutsFile::load(&path).unwrap();
        shortcuts::add(&mut file.doc, &port()).unwrap();
        let fake = || -> io::Result<Vec<String>> { Ok(vec!["4242 steam".to_string()]) };
        let err = file.save(&fake, "T").unwrap_err();
        assert!(matches!(&err, StoreError::SteamRunning(p) if p == &["4242 steam"]));
        assert!(err.to_string().contains("close it"));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(files_in(&path), ["shortcuts.vdf"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_is_refused_when_the_steam_check_itself_fails() {
        let before = plain_file();
        let (dir, path) = setup("checkfail", &before);
        let mut file = ShortcutsFile::load(&path).unwrap();
        shortcuts::add(&mut file.doc, &port()).unwrap();
        let broken = || -> io::Result<Vec<String>> { Err(io::Error::new(io::ErrorKind::PermissionDenied, "no /proc")) };
        assert!(matches!(file.save(&broken, "T"), Err(StoreError::SteamCheckFailed(_))));
        assert_eq!(fs::read(&path).unwrap(), before);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_is_refused_when_the_file_changed_after_loading() {
        let (dir, path) = setup("stale", &plain_file());
        let mut file = ShortcutsFile::load(&path).unwrap();
        shortcuts::add(&mut file.doc, &port()).unwrap();
        fs::write(&path, EMPTY_FILE).unwrap();
        assert!(matches!(file.save(&no_steam, "T"), Err(StoreError::ChangedOnDisk)));
        assert_eq!(fs::read(&path).unwrap(), EMPTY_FILE);
        assert_eq!(files_in(&path), ["shortcuts.vdf"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failure_before_the_rename_leaves_the_old_file_intact_and_no_temp_file() {
        let before = plain_file();
        let (dir, path) = setup("atomic", &before);
        let mut file = ShortcutsFile::load(&path).unwrap();
        shortcuts::add(&mut file.doc, &port()).unwrap();
        let err = file
            .save_with(&no_steam, "T", &|temp| {
                assert!(temp.exists() && fs::metadata(temp).unwrap().len() > before.len() as u64);
                Err(io::Error::new(io::ErrorKind::Other, "disk full"))
            })
            .unwrap_err();
        assert!(matches!(err, StoreError::Io { action: "finish", .. }));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(files_in(&path), ["shortcuts.vdf", "shortcuts.vdf.bak-T"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_temp_file_that_cannot_be_created_leaves_the_old_file_intact() {
        let before = plain_file();
        let (dir, path) = setup("tempblocked", &before);
        fs::create_dir(path.with_file_name("shortcuts.vdf.dpm-tmp")).unwrap();
        let mut file = ShortcutsFile::load(&path).unwrap();
        shortcuts::add(&mut file.doc, &port()).unwrap();
        assert!(matches!(file.save(&no_steam, "T"), Err(StoreError::Io { action: "create", .. })));
        assert_eq!(fs::read(&path).unwrap(), before);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_file_loads_empty_and_is_created_without_a_backup() {
        let dir = temp_dir("missing");
        let path = dir.join("userdata").join("1").join("config").join("shortcuts.vdf");
        let mut file = ShortcutsFile::load(&path).unwrap();
        assert!(shortcuts::list(&file.doc).is_empty());
        shortcuts::add(&mut file.doc, &port()).unwrap();
        let report = file.save(&no_steam, "T").unwrap();
        assert_eq!(report, SaveReport { wrote: true, backup: None });
        assert_eq!(shortcuts::list(&ShortcutsFile::load(&path).unwrap().doc).len(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unreadable_file_is_refused_not_overwritten() {
        let dir = temp_dir("garbage");
        let path = dir.join("shortcuts.vdf");
        fs::write(&path, b"\x00shortcuts\x00\x09bad\x00").unwrap();
        assert!(matches!(ShortcutsFile::load(&path), Err(StoreError::Parse { .. })));
        assert_eq!(fs::read(&path).unwrap(), b"\x00shortcuts\x00\x09bad\x00");
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_new_file_keeps_the_old_files_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, path) = setup("perms", &plain_file());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let mut file = ShortcutsFile::load(&path).unwrap();
        shortcuts::add(&mut file.doc, &port()).unwrap();
        file.save(&no_steam, "T").unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o640);
        let _ = fs::remove_dir_all(dir);
    }

    fn fake_proc(dir: &Path, entries: &[(&str, &str)]) {
        for (pid, comm) in entries {
            let p = dir.join(pid);
            fs::create_dir_all(&p).unwrap();
            fs::write(p.join("comm"), format!("{comm}\n")).unwrap();
        }
    }

    #[test]
    fn steam_processes_are_found_by_name_in_a_proc_style_folder() {
        let dir = temp_dir("proc");
        fake_proc(&dir, &[("1", "systemd"), ("200", "steam"), ("201", "steamwebhelper"), ("300", "steamos-manager"), ("400", "bash")]);
        fs::create_dir_all(dir.join("self")).unwrap();
        fs::write(dir.join("meminfo"), "x").unwrap();
        fs::create_dir_all(dir.join("500")).unwrap();
        assert_eq!(steam_processes_in(&dir).unwrap(), ["200 steam", "201 steamwebhelper"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn no_steam_processes_is_an_empty_list_and_a_missing_proc_is_an_error() {
        let dir = temp_dir("proc_none");
        fake_proc(&dir, &[("1", "systemd")]);
        assert!(steam_processes_in(&dir).unwrap().is_empty());
        assert!(steam_processes_in(&dir.join("nope")).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_system_check_can_read_this_machines_proc() {
        assert!(SystemSteamCheck.running_steam_processes().is_ok());
    }

    #[test]
    fn backup_stamp_is_filename_safe() {
        let stamp = backup_stamp();
        assert_eq!(stamp.len(), 15);
        assert!(stamp.chars().all(|c| c.is_ascii_digit() || c == '-'));
    }
}
