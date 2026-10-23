//! Old copies kept after a move, and the explicit "Remove old copy" action.
//!
//! A move never deletes the original. After the switch, the old folder is recorded in
//! `port-moves.json` (next to the app) so the app can ask right away, and list it later. Removing one
//! is only possible from an explicit confirm, and only when:
//! - the record exists and says the switch finished (records are written before the switch with
//!   `switched: false` and marked true once Steam and the records follow);
//! - the new copy exists, has the same folder name, is where the app now looks for that port, and the
//!   old copy still holds everything at the top of the new one;
//! - the old copy is not the app's folder, the home folder, a ports folder in use, or a folder that
//!   holds any of them, and no port lives there now;
//! - no link in the new copy reaches into it;
//! - no Steam shortcut (any account) points into it by program, start folder, icon or launch options,
//!   no repoint for the port is still queued, and the shortcuts and queue could be read;
//! - no program is running from it;
//! - the port is still in the catalog (so its saves can be found) and the old copy has no saves newer
//!   than the new copy's (a game started from Steam before a queued repoint applied saves there).
//! The saves in the old copy are zipped first by the caller's hook (the app uses the same zip as
//! before an update), so even an old save is never lost without a backup.

use super::copy::{absolute_target, remove_copy, tree_size};
use crate::core::files::write_atomic;
use crate::core::path_safety::is_within;
use serde_json::{json, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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
    /// False while the switch to the new copy is under way (written before the ports folder setting
    /// changes); a record left false by a crash keeps the old copy.
    pub switched: bool,
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
            "switched": self.switched,
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
            switched: v.get("switched").and_then(Value::as_bool).unwrap_or(true),
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
    /// The port's save folders inside a given port folder (only those that resolve inside it), or
    /// `None` when the port is not in the catalog any more (its saves cannot be found, so it is kept).
    pub save_folders: &'a dyn Fn(&OldCopy, &Path) -> Option<Vec<PathBuf>>,
}

fn same(a: &Path, b: &Path) -> bool {
    a == b || matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

fn holds(outer: &Path, inner: &Path) -> bool {
    is_within(inner, outer) || matches!((fs::canonicalize(outer), fs::canonicalize(inner)), (Ok(o), Ok(i)) if i.starts_with(&o))
}

/// The newest modification time of any file under `dir` (links not followed).
pub fn newest_file_time(dir: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    let Ok(entries) = fs::read_dir(dir) else { return None };
    for e in entries.flatten() {
        let Ok(t) = e.file_type() else { continue };
        let time = if t.is_dir() {
            newest_file_time(&e.path())
        } else if t.is_file() {
            e.metadata().ok().and_then(|m| m.modified().ok())
        } else {
            None
        };
        if let Some(time) = time {
            newest = Some(newest.map_or(time, |n| n.max(time)));
        }
    }
    newest
}

/// Links in `dir` (walked without following links) that reach into `old`.
pub fn links_into(dir: &Path, old: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, old: &Path, real_old: Option<&Path>, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let path = e.path();
            match e.file_type() {
                Ok(t) if t.is_symlink() => {
                    let lexical = fs::read_link(&path).is_ok_and(|target| is_within(&absolute_target(&path, &target), old));
                    let real = real_old.is_some_and(|ro| fs::canonicalize(&path).is_ok_and(|p| p.starts_with(ro)));
                    if lexical || real {
                        out.push(path);
                    }
                }
                Ok(t) if t.is_dir() => walk(&path, old, real_old, out),
                _ => {}
            }
        }
    }
    let real_old = fs::canonicalize(old).ok();
    let mut out = Vec::new();
    walk(dir, old, real_old.as_deref(), &mut out);
    out
}

fn local_time(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M").to_string()
}

/// Whether the old copy can be removed now; Ok gives its size in bytes.
pub fn removal_check(entry: &OldCopy, ctx: &RemovalCtx) -> Result<u64, String> {
    let old = &entry.old_path;
    let meta = fs::symlink_metadata(old).map_err(|_| format!("{} is already gone. Forget this record?", old.display()))?;
    if entry.link_only != meta.file_type().is_symlink() {
        return Err(format!("{} is not what was recorded (a {} was expected), so it is left alone.", old.display(), if entry.link_only { "link" } else { "folder" }));
    }
    if !entry.switched {
        return Err(format!(
            "The switch to the new copy did not finish (the app closed during it), so the old copy is kept. If {} now runs from {}, choose \"Finish switching {}\" in Settings > Ports folder first.",
            entry.name,
            entry.new_path.display(),
            entry.name
        ));
    }
    if fs::symlink_metadata(&entry.new_path).is_err() {
        return Err(format!("The new copy {} is missing, so the old one is kept.", entry.new_path.display()));
    }
    if old.file_name() != entry.new_path.file_name() {
        return Err("The old and new copies have different folder names, so the old one is kept.".into());
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
    if !entry.link_only {
        // The old copy must still look like the port: everything at the top of the new copy is in it.
        if let Ok(entries) = fs::read_dir(&entry.new_path) {
            for e in entries.flatten() {
                if fs::symlink_metadata(old.join(e.file_name())).is_err() {
                    return Err(format!("{} does not look like the same port any more ({} is not in it), so it is kept.", old.display(), e.file_name().to_string_lossy()));
                }
            }
        }
        if let Some(link) = links_into(&entry.new_path, old).first() {
            return Err(format!("A link in the new copy ({}) still points into the old one, so it is kept.", link.display()));
        }
    }
    let queue = super::super::steam_queue::load(&super::super::steam_queue::queue_path(ctx.config_dir));
    if queue.unreadable {
        return Err("The queue of Steam changes cannot be read, so the old copy is kept until it is fixed.".into());
    }
    let queued = queue.queue.entries.iter().any(|e| {
        e.kind == super::super::steam_queue::QueuedKind::Repoint
            && (e.port_key == entry.port_key || e.repoint.as_ref().is_some_and(|(from, _)| is_within(Path::new(from), old)))
    });
    if queued {
        return Err("A Steam shortcut change for this port is still queued (it applies once Steam is closed in Desktop Mode). Until then Steam still starts the old copy, so it is kept.".into());
    }
    let pointing = super::steam::shortcuts_pointing_into_detailed(ctx.home, old)?;
    if let Some((name, _)) = pointing.iter().find(|(_, has_id)| !has_id) {
        return Err(format!(
            "The Steam shortcut \"{name}\" still points into it, and Steam has not finished setting that shortcut up, so it could not be moved: open Steam, quit it, then try again."
        ));
    }
    let pointing: Vec<String> = pointing.into_iter().map(|(name, _)| name).collect();
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
    if !entry.link_only {
        let Some(old_saves) = (ctx.save_folders)(entry, old) else {
            return Err(format!("{} is not in the catalog any more, so the app can't tell where its saves are; the old copy is kept (remove it by hand if you are sure).", entry.name));
        };
        let new_saves = (ctx.save_folders)(entry, &entry.new_path).unwrap_or_default();
        for (i, old_dir) in old_saves.iter().enumerate() {
            let Some(old_time) = newest_file_time(old_dir) else { continue };
            let newer = match new_saves.get(i).and_then(|d| newest_file_time(d)) {
                None => true,
                Some(new_time) => old_time.duration_since(new_time).is_ok_and(|d| d > std::time::Duration::from_secs(2)),
            };
            if newer {
                let new_dir = new_saves.get(i).map(|d| d.display().to_string()).unwrap_or_else(|| "the new copy".into());
                return Err(format!(
                    "The old copy has newer saves (last played {}). Copy the save files from {} into {new_dir} (or play once from the new copy to check), then try again; or keep the old copy.",
                    local_time(old_time),
                    old_dir.display()
                ));
            }
        }
    }
    Ok(if entry.link_only { 0 } else { tree_size(old) })
}

/// Before an old copy the user kept is replaced (a port moving back onto it): it is not a protected
/// folder, nothing runs from it, it is what was recorded, and it has no saves newer than the copy in use
/// (`current`). An old copy that is already gone needs no replacing (Ok).
pub fn replace_check(entry: &OldCopy, current: &Path, ctx: &RemovalCtx) -> Result<(), String> {
    let stale = &entry.old_path;
    let Ok(meta) = fs::symlink_metadata(stale) else { return Ok(()) };
    if entry.link_only != meta.file_type().is_symlink() {
        return Err(format!(
            "The old copy at {} is not what was recorded (a {} was expected), so it is left alone and this port is not moved.",
            stale.display(),
            if entry.link_only { "link" } else { "folder" }
        ));
    }
    for (what, protected) in [("the ports folder in use", ctx.library_dir), ("the app's folder", ctx.config_dir), ("the home folder", ctx.home), ("the copy in use", current)] {
        if holds(stale, protected) {
            return Err(format!("{} holds {what}, so it is never removed and this port is not moved.", stale.display()));
        }
    }
    let running = (ctx.running)(stale);
    if !running.is_empty() {
        return Err(format!("{} is running from the old copy at {}. Close it first, then move again.", running.join(", "), stale.display()));
    }
    if entry.link_only {
        return Ok(());
    }
    let Some(old_saves) = (ctx.save_folders)(entry, stale) else {
        return Err(format!("{} is not in the catalog any more, so the app can't tell where its saves are; the old copy at {} is kept and this port is not moved.", entry.name, stale.display()));
    };
    let new_saves = (ctx.save_folders)(entry, current).unwrap_or_default();
    for (i, old_dir) in old_saves.iter().enumerate() {
        let Some(old_time) = newest_file_time(old_dir) else { continue };
        let newer = match new_saves.get(i).and_then(|d| newest_file_time(d)) {
            None => true,
            Some(new_time) => old_time.duration_since(new_time).is_ok_and(|d| d > std::time::Duration::from_secs(2)),
        };
        if newer {
            let new_dir = new_saves.get(i).map(|d| d.display().to_string()).unwrap_or_else(|| "the copy in use".into());
            return Err(format!(
                "The old copy at {} has newer saves (last played {}) than the copy in use. Copy the save files from {} into {new_dir}, or keep the old copy and cancel.",
                stale.display(),
                local_time(old_time),
                old_dir.display()
            ));
        }
    }
    Ok(())
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
