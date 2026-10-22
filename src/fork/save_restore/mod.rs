//! Restore saves: list the save backups the app has made for a port and put one back.
//!
//! Three kinds of backup exist under the app's `Saves Backup` folder, all zips:
//! - `Update Backups/<folder>/<stamp>.zip`, made before every update (`save_backup::backup_before_update`),
//!   holding `save_folder/...` and `save_folder2/...`;
//! - `Global Backups/<date>.zip`, made by Settings > Backup Saves (`save_backup::run_global_backup`),
//!   holding `<folder>/save_folder/...` for every port;
//! - `Restore Backups/<folder>/<stamp>.zip`, made here, right before a restore, same layout as an
//!   update backup, so a restore can itself be undone from the same list.
//!
//! A restore never starts unless the port's current saves were zipped first and that zip reads back
//! with the right number of files. Then the backup is unpacked into a scratch folder, every save
//! folder it covers is rebuilt beside the real one and swapped in by renaming, and only once every
//! swap succeeded are the old folders removed. Names in the zip are checked before 7-Zip sees the
//! file, so an entry like `../x` or `/x` refuses the whole backup.
//!
//! A save folder is only used when it is known for this system. A plain relative path inside the game
//! folder counts on every system, even when the catalog lists it only under `windows`. A value that only
//! names a real location on another system (a Windows drive or `%VARIABLE%` path, an Android path), or
//! still holds a `%VARIABLE%` after expansion, means "no save folder known", and nothing is offered
//! rather than a guess.

pub mod running;
pub mod view;
pub mod zip_index;

use crate::core::installer::{run_7z, sevenzip_exe_path};
use crate::core::models::Port;
use crate::core::path_safety::{is_within, safe_join};
use crate::core::platform_resolve::{expand_env_path, get_platform_key, resolve_per_platform, resolve_save_folder};
use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;
use zip_index::{is_safe_name, read_index, ZipEntry};

/// Must match `src/core/save_backup.rs` (pinned by the tests that list real backups).
pub const UPDATE_BACKUPS_DIR: &str = "Update Backups";
pub const GLOBAL_BACKUPS_DIR: &str = "Global Backups";
pub const RESTORE_BACKUPS_DIR: &str = "Restore Backups";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SaveField {
    Save,
    Save2,
}

impl SaveField {
    pub const ALL: [SaveField; 2] = [SaveField::Save, SaveField::Save2];

    /// The folder name inside a backup zip (upstream's names).
    pub fn zip_dir(self) -> &'static str {
        match self {
            SaveField::Save => "save_folder",
            SaveField::Save2 => "save_folder2",
        }
    }

    /// The name the Info dialog uses for the button that opens this folder.
    pub fn label(self) -> &'static str {
        match self {
            SaveField::Save => "Save folder",
            SaveField::Save2 => "Save folder 2",
        }
    }

    fn value(self, port: &Port) -> Option<&Value> {
        match self {
            SaveField::Save => port.save.as_ref(),
            SaveField::Save2 => port.save2.as_ref(),
        }
    }
}

/// Where one save field of a port points on this system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A folder it is safe to restore into (it may not exist yet).
    Folder(PathBuf),
    /// The catalog names no location for this system (only Windows or another system, or a
    /// `%VARIABLE%` that is not set here).
    NotForThisSystem,
    /// A location the app will not replace wholesale; the text says why.
    Refused(String),
    /// The catalog names the field but leaves it empty: it does not say where the saves are.
    NotListed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveTarget {
    pub field: SaveField,
    pub target: Target,
}

pub fn system_name() -> &'static str {
    if cfg!(windows) {
        "Windows"
    } else {
        "Linux"
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `%NAME%` left in a path after expansion: the variable is not set on this system.
fn has_unexpanded_variable(text: &str) -> bool {
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else { return false };
        let name = &after[..end];
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return true;
        }
        rest = &after[end..];
    }
    false
}

/// A Windows-style path (`C:...` or backslashes) cannot be a save folder on Linux.
fn looks_like_windows_path(text: &str) -> bool {
    let b = text.as_bytes();
    cfg!(not(windows)) && (text.contains('\\') || (b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic()))
}

/// A plain path inside the game folder: no `%VAR%`, no `~`, no drive letter, no leading `/` or `\`.
fn is_plain_relative(s: &str) -> bool {
    let b = s.as_bytes();
    !s.is_empty() && !s.contains('%') && !s.starts_with(['~', '/', '\\']) && !(b.len() >= 2 && b[1] == b':') && !Path::new(s).is_absolute()
}

/// A save value that is a plain relative path counts on every system (coordinator ruling 2026-10-22), also
/// when the catalog only lists it under `windows`. Returned with `/` separators off Windows.
fn os_neutral_relative(value: &Value) -> Option<String> {
    let s = match value {
        Value::String(s) => s.as_str(),
        Value::Object(obj) => match obj.get(get_platform_key()).and_then(Value::as_str).filter(|s| !s.is_empty()) {
            Some(own) => own,
            None => obj.get("windows").and_then(Value::as_str)?,
        },
        _ => return None,
    };
    is_plain_relative(s).then(|| if cfg!(windows) { s.to_string() } else { s.replace('\\', "/") })
}

/// Null, "" or a per-system object whose entry for this system (or, without one, every entry) is empty.
fn is_empty_value(value: &Value) -> bool {
    let blank = |v: &Value| v.is_null() || v.as_str().is_some_and(|s| s.trim().is_empty());
    match value {
        Value::Object(obj) => match obj.get(get_platform_key()) {
            Some(own) => blank(own),
            None => obj.values().all(blank),
        },
        other => blank(other),
    }
}

fn resolve_target(value: &Value, game_dir: &Path, saves_backup_dir: &Path, home: Option<&Path>) -> Target {
    if is_empty_value(value) {
        return Target::NotListed;
    }
    let (path, from_relative) = if let Some(rel) = os_neutral_relative(value) {
        (normalize(&game_dir.join(rel)), true)
    } else {
        // A per-system object with no entry for this system: do not borrow another system's real path
        // (a Windows drive or %VAR% path, an Android path).
        if let Some(obj) = value.as_object() {
            if !obj.get(get_platform_key()).and_then(Value::as_str).is_some_and(|s| !s.is_empty()) {
                return Target::NotForThisSystem;
            }
        }
        let Some(path) = resolve_save_folder(value, game_dir) else { return Target::NotForThisSystem };
        let text = path.to_string_lossy();
        if has_unexpanded_variable(&text) || looks_like_windows_path(&text) {
            return Target::NotForThisSystem;
        }
        let raw = resolve_per_platform(value).and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
        (normalize(&path), !expand_env_path(&raw).is_absolute())
    };
    if from_relative && !is_within(&path, game_dir) {
        return Target::Refused("points outside the game folder".into());
    }
    if is_within(game_dir, &path) {
        return Target::Refused("is the whole game folder".into());
    }
    if is_within(saves_backup_dir, &path) || is_within(&path, saves_backup_dir) {
        return Target::Refused("overlaps the app's Saves Backup folder".into());
    }
    if home.is_some_and(|h| is_within(h, &path)) {
        return Target::Refused("is your whole home folder".into());
    }
    if path.components().filter(|c| matches!(c, Component::Normal(_))).count() < 2 {
        return Target::Refused("is too close to the top of the drive".into());
    }
    Target::Folder(path)
}

/// Every save field the port declares, with where it points on this system.
pub fn save_targets(port: &Port, game_dir: &Path, saves_backup_dir: &Path, home: Option<&Path>) -> Vec<SaveTarget> {
    SaveField::ALL
        .iter()
        .filter_map(|&field| {
            let value = field.value(port)?;
            Some(SaveTarget { field, target: resolve_target(value, game_dir, saves_backup_dir, home) })
        })
        .collect()
}

// -- Listing -----------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupKind {
    BeforeUpdate,
    AllPorts,
    BeforeRestore,
}

impl BackupKind {
    pub fn label(self) -> &'static str {
        match self {
            BackupKind::BeforeUpdate => "Before update",
            BackupKind::AllPorts => "Backup Saves (all ports)",
            BackupKind::BeforeRestore => "Before restore",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Backup {
    pub kind: BackupKind,
    pub zip: PathBuf,
    /// Where this port's folders sit inside the zip: "" or "<folder>/".
    pub prefix: String,
    /// The file name without `.zip` (the date it was made, as written by the app).
    pub name: String,
    pub modified: SystemTime,
    /// Bytes and files of this port's saves in the zip (unpacked size).
    pub size: u64,
    pub files: usize,
    pub fields: Vec<SaveField>,
    /// Set when the zip cannot be used: unreadable, or a name that would land outside the folder.
    pub problem: Option<String>,
}

fn zip_problem(entries: &[ZipEntry]) -> Option<String> {
    if let Some(bad) = entries.iter().find(|e| !is_safe_name(&e.name)) {
        return Some(format!("it holds a file name that points outside its folder ({})", bad.name));
    }
    if let Some(link) = entries.iter().find(|e| e.is_symlink) {
        return Some(format!("it holds a link instead of a file ({})", link.name));
    }
    None
}

fn summarize(kind: BackupKind, zip: &Path, prefix: String, entries: &[ZipEntry], problem: Option<String>) -> Backup {
    let mut fields = Vec::new();
    let (mut size, mut files) = (0u64, 0usize);
    for field in SaveField::ALL {
        let dir = format!("{prefix}{}/", field.zip_dir());
        let mut any = false;
        for e in entries.iter().filter(|e| !e.is_dir && e.name.starts_with(&dir)) {
            any = true;
            size += e.size;
            files += 1;
        }
        if any {
            fields.push(field);
        }
    }
    Backup {
        kind,
        zip: zip.to_path_buf(),
        prefix,
        name: zip.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        modified: fs::metadata(zip).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH),
        size,
        files,
        fields,
        problem,
    }
}

fn zips_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = fs::read_dir(dir) else { return Vec::new() };
    read.filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")))
        .collect()
}

fn port_zip_backup(kind: BackupKind, zip: &Path) -> Backup {
    match read_index(zip) {
        Ok(entries) => {
            let problem = zip_problem(&entries);
            summarize(kind, zip, String::new(), &entries, problem)
        }
        Err(e) => summarize(kind, zip, String::new(), &[], Some(format!("the file cannot be read ({e})"))),
    }
}

fn newest_first(list: &mut [Backup]) {
    list.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| b.name.cmp(&a.name)));
}

/// Every backup that holds saves of the port in `folder`, newest first. A per-port zip that cannot be
/// read is listed with its problem so it does not silently vanish; a whole-library zip that cannot be
/// read is left out (it may not hold this port at all).
pub fn list_backups(saves_backup_dir: &Path, folder: &str) -> Vec<Backup> {
    sweep_stale_staging(saves_backup_dir, SystemTime::now());
    let mut out = Vec::new();
    for (kind, dir) in [(BackupKind::BeforeUpdate, UPDATE_BACKUPS_DIR), (BackupKind::BeforeRestore, RESTORE_BACKUPS_DIR)] {
        let Ok(port_dir) = safe_join(&saves_backup_dir.join(dir), folder) else { continue };
        out.extend(zips_in(&port_dir).iter().map(|zip| port_zip_backup(kind, zip)).filter(|b| b.problem.is_some() || !b.fields.is_empty()));
    }
    for zip in zips_in(&saves_backup_dir.join(GLOBAL_BACKUPS_DIR)) {
        let Ok(entries) = read_index(&zip) else { continue };
        let backup = summarize(BackupKind::AllPorts, &zip, format!("{folder}/"), &entries, zip_problem(&entries));
        if !backup.fields.is_empty() {
            out.push(backup);
        }
    }
    newest_first(&mut out);
    out
}

/// Every port folder that has at least one backup, with its backups newest first.
pub fn list_all_backups(saves_backup_dir: &Path) -> BTreeMap<String, Vec<Backup>> {
    let mut folders: Vec<String> = Vec::new();
    for dir in [UPDATE_BACKUPS_DIR, RESTORE_BACKUPS_DIR] {
        if let Ok(read) = fs::read_dir(saves_backup_dir.join(dir)) {
            folders.extend(read.filter_map(|e| e.ok()).filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).filter_map(|e| e.file_name().into_string().ok()));
        }
    }
    for zip in zips_in(&saves_backup_dir.join(GLOBAL_BACKUPS_DIR)) {
        let Ok(entries) = read_index(&zip) else { continue };
        for e in entries.iter().filter(|e| !e.is_dir) {
            let mut parts = e.name.splitn(3, '/');
            if let (Some(folder), Some(field), Some(_)) = (parts.next(), parts.next(), parts.next()) {
                if SaveField::ALL.iter().any(|f| f.zip_dir() == field) {
                    folders.push(folder.to_string());
                }
            }
        }
    }
    folders.sort();
    folders.dedup();
    folders
        .into_iter()
        .filter_map(|folder| {
            let list = list_backups(saves_backup_dir, &folder);
            (!list.is_empty()).then_some((folder, list))
        })
        .collect()
}

// -- Leftovers ---------------------------------------------------------------------------------------

/// Scratch folders a restore makes inside `Saves Backup` (`_restore_*`, `_restore_backup_*`) are removed
/// when the restore ends; one left by a crash is removed here once it is a day old. They only ever hold
/// copies (of current saves already zipped, or of an unpacked backup). Nothing else is touched.
pub fn sweep_stale_staging(saves_backup_dir: &Path, now: SystemTime) {
    const DAY: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
    let Ok(read) = fs::read_dir(saves_backup_dir) else { return };
    for entry in read.filter_map(|e| e.ok()) {
        let ours = entry.file_name().to_str().is_some_and(|n| n.starts_with("_restore_"));
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| now.duration_since(t).ok()).is_some_and(|age| age > DAY);
        if ours && is_dir && old {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// A folder a restore left beside a save folder because it stopped part way (the app was killed, the
/// Deck lost power) or could not remove the old copy. Never removed by the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interrupted {
    pub field: SaveField,
    /// The save folder it belongs to.
    pub folder: PathBuf,
    pub path: PathBuf,
    /// True: the saves that were in the folder before the restore. False: an unfinished copy of a backup.
    pub holds_previous: bool,
}

const TAG_BEFORE: &str = "before-restore";
const TAG_RESTORING: &str = "restoring";

fn real_folder(path: &Path) -> PathBuf {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
        _ => path.to_path_buf(),
    }
}

pub fn interrupted_restores(targets: &[SaveTarget]) -> Vec<Interrupted> {
    let mut out = Vec::new();
    for t in targets {
        let Target::Folder(path) = &t.target else { continue };
        let real = real_folder(path);
        let (Some(parent), Some(name)) = (real.parent(), real.file_name().map(|n| n.to_string_lossy().into_owned())) else { continue };
        let Ok(read) = fs::read_dir(parent) else { continue };
        for entry in read.filter_map(|e| e.ok()) {
            let file = entry.file_name().to_string_lossy().into_owned();
            let holds_previous = if file.starts_with(&format!(".{name}.{TAG_BEFORE}-")) {
                true
            } else if file.starts_with(&format!(".{name}.{TAG_RESTORING}-")) {
                false
            } else {
                continue;
            };
            out.push(Interrupted { field: t.field, folder: path.clone(), path: entry.path(), holds_previous });
        }
    }
    out.sort_by(|a, b| (a.field, !a.holds_previous, &a.path).cmp(&(b.field, !b.holds_previous, &b.path)));
    out
}

// -- Restoring ---------------------------------------------------------------------------------------

/// What a restore of one backup would do, worked out before anything is touched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestorePlan {
    /// Save folders that will be replaced by the backup's copy.
    pub replace: Vec<(SaveField, PathBuf)>,
    /// Folders the backup holds but that cannot be restored here, and why.
    pub skipped: Vec<(SaveField, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreError {
    /// What is running: "The game (started from this app)" or "<program> (process <pid>)".
    GameRunning(String),
    NothingToRestore(String),
    UnusableBackup(String),
    SafetyBackupFailed(String),
    /// Nothing was changed.
    Failed(String),
    /// The swap failed and putting the old folders back failed too; the text says where they are.
    RollbackFailed(String),
}

impl std::fmt::Display for RestoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RestoreError::GameRunning(what) => {
                write!(f, "{what} is running from this port's folders. Quit it first, then restore; nothing was changed.")
            }
            RestoreError::NothingToRestore(why) => write!(f, "Nothing to restore. {why}."),
            RestoreError::UnusableBackup(why) => write!(f, "This backup cannot be used: {why}. Nothing was changed."),
            RestoreError::SafetyBackupFailed(why) => {
                write!(f, "Your current saves could not be backed up ({why}), so nothing was restored and nothing was changed.")
            }
            RestoreError::Failed(why) => write!(f, "The restore did not happen ({why}). Nothing was changed."),
            RestoreError::RollbackFailed(why) => write!(f, "The restore failed part way and could not be fully undone: {why}"),
        }
    }
}

pub fn plan_restore(targets: &[SaveTarget], backup: &Backup) -> RestorePlan {
    let mut plan = RestorePlan { replace: Vec::new(), skipped: Vec::new() };
    for &field in &backup.fields {
        match targets.iter().find(|t| t.field == field).map(|t| &t.target) {
            Some(Target::Folder(path)) => plan.replace.push((field, path.clone())),
            Some(Target::NotForThisSystem) => plan.skipped.push((field, format!("no save folder known for {}", system_name()))),
            Some(Target::Refused(why)) => plan.skipped.push((field, why.clone())),
            Some(Target::NotListed) => plan.skipped.push((field, "the catalog does not say where this port keeps these saves".into())),
            None => plan.skipped.push((field, "this port no longer names that save folder".into())),
        }
    }
    plan
}

pub struct RestoreInput<'a> {
    pub port: &'a Port,
    pub game_dir: &'a Path,
    pub saves_backup_dir: &'a Path,
    pub home: Option<&'a Path>,
    pub backup: &'a Backup,
    /// The app's own record of a game it started; programs started some other way are found by `running`.
    pub game_running: bool,
    /// Names the "Before restore" zip, e.g. `2026-10-21_14-05-00`.
    pub stamp: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoredFolder {
    pub field: SaveField,
    pub path: PathBuf,
    pub files: usize,
    /// The replaced folder, kept when it could not be removed or held something the safety backup
    /// could not copy (a link to a folder).
    pub kept_old: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreDone {
    /// `None` when there were no current saves to back up.
    pub safety_backup: Option<PathBuf>,
    pub folders: Vec<RestoredFolder>,
    pub skipped: Vec<(SaveField, String)>,
}

/// Copies a folder tree (files and folders, empty ones too). Links to files are copied as the file;
/// links to folders and other special entries are counted in the returned number and left out.
fn copy_tree(src: &Path, dst: &Path) -> io::Result<(usize, usize)> {
    fs::create_dir_all(dst)?;
    let (mut files, mut skipped) = (0, 0);
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if kind.is_dir() {
            let (f, s) = copy_tree(&from, &to)?;
            files += f;
            skipped += s;
        } else if kind.is_file() || (kind.is_symlink() && fs::metadata(&from).is_ok_and(|m| m.is_file())) {
            fs::copy(&from, &to)?;
            files += 1;
        } else {
            skipped += 1;
        }
    }
    Ok((files, skipped))
}

fn unused_path(dir: &Path, base: &str, ext: &str) -> Result<PathBuf, String> {
    (1..1000)
        .map(|n| if n == 1 { dir.join(format!("{base}{ext}")) } else { dir.join(format!("{base}-{n}{ext}")) })
        .find(|p| fs::symlink_metadata(p).is_err())
        .ok_or_else(|| format!("too many files named {base}"))
}

fn count_zip_files(zip: &Path) -> Result<usize, String> {
    Ok(read_index(zip)?.iter().filter(|e| !e.is_dir).count())
}

/// Zips every current save folder of the port into `Restore Backups/<folder>/<stamp>.zip`, checks the
/// zip reads back with every file, and returns the folders whose copy skipped something (a link to a
/// folder), which must then be kept rather than removed. `Ok((None, _))`: no saves to back up.
fn safety_backup(port: &Port, targets: &[SaveTarget], saves_backup_dir: &Path, stamp: &str) -> Result<(Option<PathBuf>, Vec<SaveField>), String> {
    let tool = sevenzip_exe_path().ok_or("7-Zip is missing next to the application")?;
    fs::create_dir_all(saves_backup_dir).map_err(|e| e.to_string())?;
    let staging = tempfile::Builder::new().prefix("_restore_backup_").tempdir_in(saves_backup_dir).map_err(|e| e.to_string())?;
    let (mut total, mut incomplete) = (0usize, Vec::new());
    for t in targets {
        let Target::Folder(path) = &t.target else { continue };
        if !path.is_dir() {
            continue;
        }
        let (files, skipped) = copy_tree(path, &staging.path().join(t.field.zip_dir())).map_err(|e| format!("{e} -- {}", path.display()))?;
        total += files;
        if skipped > 0 {
            incomplete.push(t.field);
        }
    }
    if total == 0 {
        return Ok((None, incomplete));
    }
    let dir = safe_join(&saves_backup_dir.join(RESTORE_BACKUPS_DIR), &port.folder)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let zip = unused_path(&dir, stamp, ".zip")?;
    let made = run_7z(&tool, &[OsStr::new("a"), OsStr::new("-tzip"), OsStr::new("--"), zip.as_os_str(), OsStr::new(".")], "create", Some(staging.path()))
        .and_then(|_| count_zip_files(&zip))
        .and_then(|n| if n == total { Ok(()) } else { Err(format!("the backup holds {n} of {total} files")) });
    if let Err(e) = made {
        let _ = fs::remove_file(&zip);
        return Err(e);
    }
    Ok((Some(zip), incomplete))
}

/// Files under `root` by relative path ('/'-separated) with their sizes. Fails on anything that is
/// neither a file nor a folder (a link), so an unpacked backup cannot point somewhere else.
fn tree_files(root: &Path) -> Result<BTreeMap<String, u64>, String> {
    fn walk(dir: &Path, rel: &str, out: &mut BTreeMap<String, u64>) -> Result<(), String> {
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            if kind.is_dir() {
                walk(&entry.path(), &rel, out)?;
            } else if kind.is_file() {
                out.insert(rel, entry.metadata().map_err(|e| e.to_string())?.len());
            } else {
                return Err(format!("the backup unpacked a link ({rel})"));
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    if root.is_dir() {
        walk(root, "", &mut out)?;
    }
    Ok(out)
}

fn expected_files(entries: &[ZipEntry], prefix: &str, field: SaveField) -> BTreeMap<String, u64> {
    let dir = format!("{prefix}{}/", field.zip_dir());
    entries
        .iter()
        .filter(|e| !e.is_dir)
        .filter_map(|e| e.name.strip_prefix(&dir).filter(|r| !r.is_empty()).map(|r| (r.trim_end_matches('/').to_string(), e.size)))
        .collect()
}

fn extract(zip: &Path, staging: &Path, prefix: &str) -> Result<(), String> {
    let tool = sevenzip_exe_path().ok_or("7-Zip is missing next to the application")?;
    let out_arg = format!("-o{}", staging.display());
    // `--` ends the switches, so neither the zip path nor a folder name starting with '-' is read as one.
    let mut args = vec![OsStr::new("x"), OsStr::new(&out_arg), OsStr::new("-y"), OsStr::new("-bd"), OsStr::new("--"), zip.as_os_str()];
    // A whole-library backup: unpack only this port's folder (7-Zip takes a folder name as "that folder and
    // everything in it"). A name holding 7-Zip's wildcards `*` or `?` would match other folders, so it is
    // not passed: the whole zip is unpacked instead (every name in it was checked first) and only this
    // port's folder is read from it.
    let folder = prefix.trim_end_matches('/');
    if !folder.is_empty() && !folder.contains(['*', '?']) {
        args.push(OsStr::new(folder));
    }
    run_7z(&tool, &args, "extract", None).map(|_| ())
}

fn hidden_sibling(target: &Path, tag: &str) -> Result<PathBuf, String> {
    let parent = target.parent().ok_or("the save folder has no parent folder")?;
    let name = target.file_name().ok_or("the save folder has no name")?.to_string_lossy();
    unused_path(parent, &format!(".{name}.{tag}-{}", std::process::id()), "")
}

struct Prepared {
    field: SaveField,
    /// The path the user knows (shown in messages).
    shown: PathBuf,
    /// The folder actually replaced: the link's destination when the save folder is a link.
    real: PathBuf,
    staged: PathBuf,
    files: usize,
}

struct Swapped {
    real: PathBuf,
    staged: PathBuf,
    old: Option<PathBuf>,
}

/// Puts every swapped folder back as it was. Returns the problems it could not fix.
fn roll_back(done: &[Swapped]) -> Vec<String> {
    let mut problems = Vec::new();
    for s in done.iter().rev() {
        if fs::rename(&s.real, &s.staged).is_err() {
            problems.push(format!("{} now holds the backup's copy", s.real.display()));
            if let Some(old) = &s.old {
                problems.push(format!("your previous saves are in {}", old.display()));
            }
            continue;
        }
        if let Some(old) = &s.old {
            if fs::rename(old, &s.real).is_err() {
                problems.push(format!("your previous saves are in {} (move them back to {})", old.display(), s.real.display()));
            }
        }
        let _ = fs::remove_dir_all(&s.staged);
    }
    problems
}

/// Bytes in files under `path` (links not followed); 0 when it does not exist.
fn tree_size(path: &Path) -> u64 {
    let Ok(read) = fs::read_dir(path) else { return 0 };
    read.filter_map(|e| e.ok())
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => tree_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}

/// Spare room kept on top of what a restore needs.
const SPACE_MARGIN: u64 = 16 * 1024 * 1024;

/// `Some(reason)` when the drive is known to be too full. Unknown free space is not a reason to refuse.
pub fn space_problem(need: u64, free: Option<u64>, place: &Path) -> Option<String> {
    let free = free?;
    (free < need.saturating_add(SPACE_MARGIN)).then(|| {
        format!("not enough free space: it needs about {} and the drive holding {} has {} free", view::human_size(need), place.display(), view::human_size(free))
    })
}

/// Free bytes on the drive holding `path` (Linux; `None` elsewhere or when unknown).
fn free_bytes(path: &Path) -> Option<u64> {
    crate::fork::rom_screens::util::free_bytes(path)
}

/// Programs running from the game folder or a save folder (see `running`), as text for the refusal.
pub fn running_from(game_dir: &Path, targets: &[SaveTarget]) -> Option<String> {
    let mut watch = vec![game_dir.to_path_buf()];
    watch.extend(targets.iter().filter_map(|t| if let Target::Folder(p) = &t.target { Some(p.clone()) } else { None }));
    running::processes_under(&watch).first().map(running::Found::describe)
}

#[cfg(test)]
thread_local! {
    /// Tests: make the swap of the n-th folder (0-based) fail, to exercise the roll back.
    static FAIL_SWAP_AT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

fn injected_swap_failure(index: usize) -> bool {
    #[cfg(test)]
    {
        FAIL_SWAP_AT.with(|f| f.get() == Some(index))
    }
    #[cfg(not(test))]
    {
        let _ = index;
        false
    }
}

/// Restores one backup. See the module notes for the order of steps; each refusal leaves every file as it was.
pub fn restore(input: &RestoreInput, on_progress: &mut dyn FnMut(&str)) -> Result<RestoreDone, RestoreError> {
    if input.game_running {
        return Err(RestoreError::GameRunning("The game (started from this app)".into()));
    }
    if let Some(problem) = &input.backup.problem {
        return Err(RestoreError::UnusableBackup(problem.clone()));
    }
    let targets = save_targets(input.port, input.game_dir, input.saves_backup_dir, input.home);
    if let Some(what) = running_from(input.game_dir, &targets) {
        return Err(RestoreError::GameRunning(what));
    }
    let plan = plan_restore(&targets, input.backup);
    if plan.replace.is_empty() {
        let why = plan.skipped.first().map(|(f, why)| format!("{}: {why}", f.label())).unwrap_or_else(|| "the backup holds no save folders".into());
        return Err(RestoreError::NothingToRestore(why));
    }
    for (i, (_, a)) in plan.replace.iter().enumerate() {
        if plan.replace.iter().skip(i + 1).any(|(_, b)| is_within(a, b) || is_within(b, a)) {
            return Err(RestoreError::Failed("two save folders of this port overlap".into()));
        }
    }

    // Read the names again from the file itself: it may have changed since the list was shown.
    let entries = read_index(&input.backup.zip).map_err(|e| RestoreError::UnusableBackup(format!("the file cannot be read ({e})")))?;
    if let Some(problem) = zip_problem(&entries) {
        return Err(RestoreError::UnusableBackup(problem));
    }
    let expected: Vec<BTreeMap<String, u64>> = plan.replace.iter().map(|(f, _)| expected_files(&entries, &input.backup.prefix, *f)).collect();
    if expected.iter().any(BTreeMap::is_empty) {
        return Err(RestoreError::UnusableBackup("it no longer holds the save folders it listed".into()));
    }

    // Saves Backup holds a copy of the current saves, their zip and the unpacked backup at once; each save
    // folder's drive holds the rebuilt folder beside the old one.
    let current: u64 = targets.iter().filter_map(|t| if let Target::Folder(p) = &t.target { Some(tree_size(p)) } else { None }).sum();
    let unpacked: u64 = expected.iter().flat_map(|m| m.values()).sum();
    let mut checks = vec![(current.saturating_mul(2).saturating_add(unpacked), input.saves_backup_dir.to_path_buf())];
    checks.extend(plan.replace.iter().zip(&expected).map(|((_, path), want)| (want.values().sum::<u64>(), path.clone())));
    if let Some(problem) = checks.iter().find_map(|(need, place)| space_problem(*need, free_bytes(place), place)) {
        return Err(RestoreError::Failed(problem));
    }

    on_progress("Backing up your current saves...");
    let (safety, incomplete) = safety_backup(input.port, &targets, input.saves_backup_dir, input.stamp).map_err(RestoreError::SafetyBackupFailed)?;

    on_progress("Unpacking the backup...");
    let staging = tempfile::Builder::new().prefix("_restore_").tempdir_in(input.saves_backup_dir).map_err(|e| RestoreError::Failed(e.to_string()))?;
    extract(&input.backup.zip, staging.path(), &input.backup.prefix).map_err(RestoreError::Failed)?;

    on_progress("Checking the unpacked files...");
    let mut prepared: Vec<Prepared> = Vec::new();
    let clean_up = |prepared: &[Prepared]| {
        for p in prepared {
            let _ = fs::remove_dir_all(&p.staged);
        }
    };
    for ((field, shown), want) in plan.replace.iter().zip(&expected) {
        let src = staging.path().join(&input.backup.prefix).join(field.zip_dir());
        let got = tree_files(&src).map_err(|e| {
            clean_up(&prepared);
            RestoreError::UnusableBackup(e)
        })?;
        if &got != want {
            clean_up(&prepared);
            return Err(RestoreError::Failed(format!("the backup did not unpack as listed ({} of {} files)", got.len(), want.len())));
        }
        let meta = fs::symlink_metadata(shown);
        let real = match &meta {
            Ok(m) if m.file_type().is_symlink() => match fs::canonicalize(shown) {
                Ok(p) if p.is_dir() => p,
                _ => {
                    clean_up(&prepared);
                    return Err(RestoreError::Failed(format!("{} is a link that does not lead to a folder", shown.display())));
                }
            },
            Ok(m) if !m.is_dir() => {
                clean_up(&prepared);
                return Err(RestoreError::Failed(format!("{} is a file, not a folder", shown.display())));
            }
            _ => shown.clone(),
        };
        let parent_ok = real.parent().is_some_and(|p| fs::create_dir_all(p).is_ok());
        let staged = match hidden_sibling(&real, TAG_RESTORING) {
            Ok(p) if parent_ok => p,
            Ok(_) | Err(_) => {
                clean_up(&prepared);
                return Err(RestoreError::Failed(format!("cannot write next to {}", real.display())));
            }
        };
        on_progress(&format!("Preparing {}...", field.label()));
        match copy_tree(&src, &staged) {
            Ok((files, _)) => prepared.push(Prepared { field: *field, shown: shown.clone(), real, staged, files }),
            Err(e) => {
                let _ = fs::remove_dir_all(&staged);
                clean_up(&prepared);
                return Err(RestoreError::Failed(format!("{e} -- {}", staged.display())));
            }
        }
    }

    on_progress("Putting the saves in place...");
    let mut swapped: Vec<Swapped> = Vec::new();
    for (index, p) in prepared.iter().enumerate() {
        let old = if fs::symlink_metadata(&p.real).is_ok() {
            let old = match hidden_sibling(&p.real, TAG_BEFORE) {
                Ok(old) => old,
                Err(e) => return Err(fail_swap(&swapped, &prepared, e)),
            };
            if let Err(e) = fs::rename(&p.real, &old) {
                return Err(fail_swap(&swapped, &prepared, format!("{e} -- {}", p.real.display())));
            }
            Some(old)
        } else {
            None
        };
        let swapped_in = if injected_swap_failure(index) { Err(io::Error::other("test: swap fails")) } else { fs::rename(&p.staged, &p.real) };
        if let Err(e) = swapped_in {
            let mut problems = Vec::new();
            if let Some(old) = &old {
                if fs::rename(old, &p.real).is_err() {
                    problems.push(format!("your previous saves are in {} (move them back to {})", old.display(), p.real.display()));
                }
            }
            problems.extend(roll_back(&swapped));
            clean_up(&prepared);
            let why = format!("{e} -- {}", p.real.display());
            return Err(if problems.is_empty() { RestoreError::Failed(why) } else { RestoreError::RollbackFailed(problems.join("; ")) });
        }
        swapped.push(Swapped { real: p.real.clone(), staged: p.staged.clone(), old });
    }

    // Every folder is in place; the old ones are in the safety backup, so they can go (unless the backup
    // could not copy all of a folder's contents).
    let folders = prepared
        .iter()
        .zip(&swapped)
        .map(|(p, s)| {
            let keep = incomplete.contains(&p.field);
            let kept_old = s.old.clone().filter(|old| keep || fs::remove_dir_all(old).is_err());
            RestoredFolder { field: p.field, path: p.shown.clone(), files: p.files, kept_old }
        })
        .collect();
    on_progress("Done.");
    Ok(RestoreDone { safety_backup: safety, folders, skipped: plan.skipped })
}

fn fail_swap(swapped: &[Swapped], prepared: &[Prepared], why: String) -> RestoreError {
    let problems = roll_back(swapped);
    for p in prepared {
        let _ = fs::remove_dir_all(&p.staged);
    }
    if problems.is_empty() {
        RestoreError::Failed(why)
    } else {
        RestoreError::RollbackFailed(problems.join("; "))
    }
}

#[cfg(test)]
mod tests;
