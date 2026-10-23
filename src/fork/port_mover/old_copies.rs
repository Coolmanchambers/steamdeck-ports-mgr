//! Old copies kept after a move, and the explicit "Remove old copy" action.
//!
//! A move never deletes the original. After the switch, the old folder is recorded in
//! `port-moves.json` (next to the app) so the app can ask right away, and list it later. Removing one
//! is only possible from an explicit confirm, and only when:
//! - the record exists (it is written only after the copy was verified and switched to);
//! - the new copy exists and is where the app now looks for that port;
//! - the old copy is not the app's folder, the home folder, a ports folder in use, or a folder that
//!   holds any of them, and no port lives there now;
//! - no Steam shortcut (any account) still points into it, and the shortcuts could be read;
//! - no program is running from it.
//! The saves in the old copy are zipped first by the caller's hook (the app uses the same zip as
//! before an update), so even an old save is never lost without a backup.

use super::copy::{remove_copy, tree_size};
use crate::core::files::write_atomic;
use crate::core::path_safety::is_within;
use serde_json::{json, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "port-moves.json";
const VERSION: u64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OldCopy {
    pub port_key: String,
    pub name: String,
    pub old_path: PathBuf,
    pub new_path: PathBuf,
    pub bytes: u64,
    pub moved_at: String,
    /// The old "copy" is only a link (an adopted port): removing it removes the link, not its target.
    pub link_only: bool,
}

impl OldCopy {
    fn to_json(&self) -> Value {
        json!({
            "port_key": self.port_key,
            "name": self.name,
            "old_path": self.old_path.to_string_lossy(),
            "new_path": self.new_path.to_string_lossy(),
            "bytes": self.bytes,
            "moved_at": self.moved_at,
            "link_only": self.link_only,
        })
    }

    fn from_json(v: &Value) -> Option<OldCopy> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        Some(OldCopy {
            port_key: text("port_key")?,
            name: text("name").unwrap_or_default(),
            old_path: PathBuf::from(text("old_path").filter(|p| !p.is_empty())?),
            new_path: PathBuf::from(text("new_path").filter(|p| !p.is_empty())?),
            bytes: v.get("bytes").and_then(Value::as_u64).unwrap_or(0),
            moved_at: text("moved_at").unwrap_or_default(),
            link_only: v.get("link_only").and_then(Value::as_bool).unwrap_or(false),
        })
    }
}

pub fn path(config_dir: &Path) -> PathBuf {
    config_dir.join(FILE_NAME)
}

/// The recorded old copies. A missing file is none; a file that cannot be read is an error and is
/// never overwritten.
pub fn load(config_dir: &Path) -> Result<Vec<OldCopy>, String> {
    let file = path(config_dir);
    let text = match fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("Could not read {}: {e}", file.display())),
    };
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{FEFF}')).map_err(|_| format!("{} is not readable; it was left as it is", file.display()))?;
    let list = value.get("old_copies").and_then(Value::as_array).ok_or_else(|| format!("{} is not readable; it was left as it is", file.display()))?;
    Ok(list.iter().filter_map(OldCopy::from_json).collect())
}

fn save(config_dir: &Path, list: &[OldCopy]) -> io::Result<()> {
    let file = path(config_dir);
    if list.is_empty() {
        return match fs::remove_file(&file) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let text = serde_json::to_string_pretty(&json!({ "version": VERSION, "old_copies": list.iter().map(OldCopy::to_json).collect::<Vec<_>>() })).map_err(io::Error::other)?;
    write_atomic(&file, text.as_bytes())
}

/// Adds (or replaces, by old path) a record.
pub fn record(config_dir: &Path, entry: OldCopy) -> io::Result<()> {
    let mut list = load(config_dir).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    list.retain(|e| e.old_path != entry.old_path);
    list.push(entry);
    save(config_dir, &list)
}

/// Drops a record (the files are not touched).
pub fn forget(config_dir: &Path, old_path: &Path) -> io::Result<()> {
    let mut list = load(config_dir).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    list.retain(|e| e.old_path != old_path);
    save(config_dir, &list)
}

/// What removal needs from the app.
pub struct RemovalCtx<'a> {
    pub home: &'a Path,
    pub config_dir: &'a Path,
    pub library_dir: &'a Path,
    /// Where the app looks for each port now (`<ports folder>/<folder>` for every catalog port).
    pub active_paths: &'a [PathBuf],
    /// Programs running from a folder (names); empty when none.
    pub running: &'a dyn Fn(&Path) -> Vec<String>,
}

fn same(a: &Path, b: &Path) -> bool {
    a == b || matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

fn holds(outer: &Path, inner: &Path) -> bool {
    is_within(inner, outer) || matches!((fs::canonicalize(outer), fs::canonicalize(inner)), (Ok(o), Ok(i)) if i.starts_with(&o))
}

/// Whether the old copy can be removed now; Ok gives its size in bytes.
pub fn removal_check(entry: &OldCopy, ctx: &RemovalCtx) -> Result<u64, String> {
    let old = &entry.old_path;
    let meta = fs::symlink_metadata(old).map_err(|_| format!("{} is already gone. Forget this record?", old.display()))?;
    if entry.link_only != meta.file_type().is_symlink() {
        return Err(format!("{} is not what was recorded (a {} was expected), so it is left alone.", old.display(), if entry.link_only { "link" } else { "folder" }));
    }
    if fs::symlink_metadata(&entry.new_path).is_err() {
        return Err(format!("The new copy {} is missing, so the old one is kept.", entry.new_path.display()));
    }
    if !ctx.active_paths.iter().any(|p| same(p, &entry.new_path)) {
        return Err(format!("The app does not use the new copy {} now (was the ports folder changed back?), so the old one is kept.", entry.new_path.display()));
    }
    // Two links to one folder are still two copies; two paths to one folder are not.
    let same_place = if entry.link_only { *old == entry.new_path } else { same(old, &entry.new_path) };
    if same_place {
        return Err("The old and new copies are the same folder.".into());
    }
    for (what, protected) in [("the ports folder", ctx.library_dir), ("the app's folder", ctx.config_dir), ("the home folder", ctx.home)] {
        if holds(old, protected) {
            return Err(format!("{} holds {what}, so it is never removed.", old.display()));
        }
    }
    if !entry.link_only && ctx.active_paths.iter().any(|p| same(p, old) || holds(old, p)) {
        return Err(format!("A port is installed at {} now, so it is kept.", old.display()));
    }
    let pointing = super::steam::shortcuts_pointing_into(ctx.home, old)?;
    if !pointing.is_empty() {
        return Err(format!(
            "Steam shortcuts still point into it ({}). If the change is queued, it applies once Steam is closed in Desktop Mode; remove the old copy after that.",
            pointing.join(", ")
        ));
    }
    let running = (ctx.running)(old);
    if !running.is_empty() {
        return Err(format!("{} is running from it. Close it first.", running.join(", ")));
    }
    Ok(if entry.link_only { 0 } else { tree_size(old) })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    pub bytes: u64,
    /// Where the saves of the old copy were zipped, if it had any.
    pub saves_zip: Option<PathBuf>,
}

/// Removes the old copy after the checks pass again. `backup_saves` zips the saves found in the old
/// copy first (a failure stops the removal). The record is dropped afterwards.
pub fn remove(entry: &OldCopy, ctx: &RemovalCtx, backup_saves: &dyn Fn(&OldCopy) -> Result<Option<PathBuf>, String>) -> Result<Removed, String> {
    let bytes = removal_check(entry, ctx)?;
    let saves_zip = if entry.link_only { None } else { backup_saves(entry).map_err(|e| format!("The saves in the old copy could not be backed up, so nothing was removed: {e}"))? };
    remove_copy(&entry.old_path).map_err(|e| format!("Could not remove {} completely: {e}. What is left stays; nothing else was touched.", entry.old_path.display()))?;
    let _ = forget(ctx.config_dir, &entry.old_path);
    Ok(Removed { bytes, saves_zip })
}
