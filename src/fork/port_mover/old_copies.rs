//! Old copies kept after a move, and the explicit "Remove old copy" action.
//!
//! A move never deletes the original. After the switch, the old folder is recorded in
//! `port-moves.json` (next to the app) so the app can ask right away, and list it later. Removing one
//! is only possible from an explicit confirm, and only when:
//! - the record exists and says the switch finished (records are written before the switch with
//!   `switched: false` and marked true once Steam and the records follow);
//! - the new copy exists, has the same folder name (an import may differ: the new copy then carries the
//!   catalog folder name recorded with it), is where the app now looks for that port, and the
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
//!
//! On top of those checks, [`remove`] only removes an old copy that is [`Readiness::Ready`]: Steam
//! starts the new copy (no repoint queued, no shortcut into the old copy), and the port has been started
//! from its new place (from this app for at least [`MIN_RUN_SECONDS`], or from Steam after its shortcut
//! was switched) or the user confirmed "Yes, it starts and my saves are there".

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
    /// Set only by Import ports, to the catalog folder the port was imported as: the old folder may be
    /// named differently (`dusk-old` imported as `dusklight`), which is the one rule it relaxes (the name
    /// equality in `removal_check` and `finish_check`); the new copy must then be named exactly that.
    pub import_folder: Option<String>,
    /// The port was shown to work from its new place (see [`Readiness`]); recorded once, never cleared.
    pub started: Option<Started>,
    /// When the queued Steam shortcut change for this port was applied (seconds since 1970, UTC), or when the
    /// app first saw no shortcut pointing into the old copy any more. A change written at move time (Steam was
    /// closed) leaves it empty: the move time counts then.
    pub steam_at: Option<i64>,
    /// The move queued its Steam change (`Some(true)`), wrote it at once (`Some(false)`), or the record is from a
    /// build that did not say (`None`). Only `Some(false)` lets the move time stand in for `steam_at`.
    pub repoint_queued: Option<bool>,
}

/// How a port was shown to start from its new place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartedBy {
    /// Played from this app for at least [`MIN_RUN_SECONDS`].
    App,
    /// Started from its Steam shortcut after the shortcut was switched to the new copy.
    Steam,
    /// The user said "Yes, it starts and my saves are there".
    User,
}

impl StartedBy {
    fn as_str(self) -> &'static str {
        match self {
            StartedBy::App => "app",
            StartedBy::Steam => "steam",
            StartedBy::User => "user",
        }
    }

    fn parse(text: &str) -> Option<StartedBy> {
        match text {
            "app" => Some(StartedBy::App),
            "steam" => Some(StartedBy::Steam),
            "user" => Some(StartedBy::User),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Started {
    pub by: StartedBy,
    /// Seconds since 1970, UTC.
    pub at: i64,
}

/// A run from this app shorter than this does not count (it may have crashed at start).
pub const MIN_RUN_SECONDS: u64 = 30;

impl OldCopy {
    /// Whether the old and new folder names agree, or this is an import whose new copy carries the
    /// catalog folder name it was imported as.
    pub fn names_agree(&self) -> bool {
        if self.old_path.file_name() == self.new_path.file_name() {
            return true;
        }
        match (&self.import_folder, self.new_path.file_name()) {
            (Some(folder), Some(new)) => !folder.is_empty() && new == std::ffi::OsStr::new(folder),
            _ => false,
        }
    }
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
            "import_folder": self.import_folder,
            "started_by": self.started.map(|s| s.by.as_str()),
            "started_at": self.started.map(|s| s.at),
            "steam_at": self.steam_at,
            "repoint_queued": self.repoint_queued,
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
            import_folder: text("import_folder").filter(|f| !f.is_empty()),
            started: match (text("started_by").as_deref().and_then(StartedBy::parse), v.get("started_at").and_then(Value::as_i64)) {
                (Some(by), Some(at)) => Some(Started { by, at }),
                _ => None,
            },
            steam_at: v.get("steam_at").and_then(Value::as_i64),
            repoint_queued: v.get("repoint_queued").and_then(Value::as_bool),
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

/// Changes the records `pick` chooses with `change`; saves only when something changed. Returns how many changed.
fn update(config_dir: &Path, pick: impl Fn(&OldCopy) -> bool, change: impl Fn(&mut OldCopy) -> bool) -> io::Result<usize> {
    let mut list = load(config_dir).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut changed = 0;
    for entry in list.iter_mut().filter(|e| pick(e)) {
        if change(entry) {
            changed += 1;
        }
    }
    if changed > 0 {
        save(config_dir, &list)?;
    }
    Ok(changed)
}

/// Records that the port of the old copy at `old_path` was shown to start from its new place. A record that
/// already says so keeps its first reason.
pub fn mark_started(config_dir: &Path, old_path: &Path, by: StartedBy, at: i64) -> io::Result<usize> {
    update(config_dir, |e| e.old_path == old_path, |e| e.started.is_none() && { e.started = Some(Started { by, at }); true })
}

/// A queued Steam repoint for `port_key` from `from` was just written: the records it concerns note when. (It is
/// also called for a queued repoint that was dropped as no longer applicable; readiness still needs no shortcut
/// pointing into the old copy, so that is harmless.)
pub fn mark_steam_switched(config_dir: &Path, port_key: &str, from: &Path, at: i64) -> io::Result<usize> {
    update(config_dir, |e| e.port_key == port_key || is_within(&e.old_path, from) || e.old_path == from, |e| {
        let was = e.steam_at;
        e.steam_at = Some(at);
        was != e.steam_at
    })
}

/// The old copy at `old_path` had its Steam shortcuts repointed some time after the move (the checkup's "Point Steam at the new
/// copy"): written now (`queued` false) the switch time is `at`, so only a Steam start after it counts; queued, the record says so
/// (`repoint_queued`, no time yet) and the queue's apply stamps the time (`mark_steam_switched`), as after a move with Steam open.
pub fn note_repoint(config_dir: &Path, old_path: &Path, queued: bool, at: i64) -> io::Result<usize> {
    update(config_dir, |e| e.old_path == old_path, |e| {
        let (steam_at, repoint_queued) = if queued { (None, Some(true)) } else { (Some(at), e.repoint_queued) };
        let changed = (e.steam_at, e.repoint_queued) != (steam_at, repoint_queued);
        e.steam_at = steam_at;
        e.repoint_queued = repoint_queued;
        changed
    })
}

/// The app's own launch of `port_key` from `running_from` ended after `ran_seconds`: a run long enough counts
/// as "started from its new place" for the old copies of that port whose new copy is where it ran.
pub fn note_launch(config_dir: &Path, port_key: &str, running_from: &Path, ran_seconds: u64, at: i64) -> io::Result<usize> {
    if ran_seconds < MIN_RUN_SECONDS {
        return Ok(0);
    }
    update(config_dir, |e| e.port_key == port_key && e.switched && same(&e.new_path, running_from), |e| e.started.is_none() && { e.started = Some(Started { by: StartedBy::App, at }); true })
}

/// What a moved port's old copy waits for before it can be removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    /// A Steam shortcut still starts the old copy, or its change is still queued.
    WaitingForSteam,
    /// Steam is switched; the port has not been started from its new place yet.
    StartOnce,
    Ready(StartedBy),
}

impl Readiness {
    /// The one status line the old copy pages show.
    pub fn line(self) -> &'static str {
        match self {
            Readiness::WaitingForSteam => "Waiting for Steam to update",
            Readiness::StartOnce => "Start it once from its new place",
            Readiness::Ready(_) => "Ready to remove",
        }
    }

    pub fn is_ready(self) -> bool {
        matches!(self, Readiness::Ready(_))
    }
}

/// What Steam says about one old copy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SteamView {
    /// A repoint for the port is queued, a shortcut points into the old copy, or Steam's files could not be read.
    pub waiting: bool,
    /// The newest time Steam started a shortcut that points into the new copy (seconds since 1970, UTC).
    pub played_new_at: Option<i64>,
}

/// Reads the queue and every account's shortcuts for one old copy.
pub fn steam_view(entry: &OldCopy, home: &Path, config_dir: &Path) -> SteamView {
    // Read-only: a queue file that cannot be read keeps the copy waiting, and looking never moves it aside.
    let queue = super::super::steam_queue::peek(&super::super::steam_queue::queue_path(config_dir));
    let queued = queue.is_err()
        || queue.iter().flat_map(|q| q.entries.iter()).any(|e| {
            e.kind == super::super::steam_queue::QueuedKind::Repoint
                && (e.port_key == entry.port_key || e.repoint.as_ref().is_some_and(|(from, _)| is_within(Path::new(from), &entry.old_path) || Path::new(from) == entry.old_path))
        });
    let pointing = super::steam::shortcuts_pointing_into(home, &entry.old_path).map_or(true, |names| !names.is_empty());
    let mut played: Option<i64> = None;
    for account in super::steam::device_accounts(home) {
        let Ok(file) = crate::fork::store::ShortcutsFile::load(&account.shortcuts) else { continue };
        for s in crate::fork::shortcuts::list(&file.doc) {
            if s.last_play_time > 0 && super::steam::moved_path(s.exe_path(), &entry.new_path, &entry.new_path).is_some() {
                played = Some(played.map_or(i64::from(s.last_play_time), |p| p.max(i64::from(s.last_play_time))));
            }
        }
    }
    SteamView { waiting: queued || pointing, played_new_at: played }
}

/// Since when a Steam start counts as a start of the new copy: the time its shortcut was switched, else the
/// move when the move itself switched it (`repoint_queued == Some(false)`). `None` when neither is known (a queued
/// change fixed by hand, a record from an older build): only the app or the user can tell then, until
/// [`readiness_now`] notes the switch.
pub fn steam_since(entry: &OldCopy) -> Option<i64> {
    entry.steam_at.or_else(|| {
        (entry.repoint_queued == Some(false))
            .then(|| chrono::NaiveDateTime::parse_from_str(&entry.moved_at, "%Y%m%d-%H%M%S").ok().and_then(|t| t.and_local_timezone(chrono::Local).single()).map(|t| t.timestamp()))
            .flatten()
    })
}

/// The readiness of the old copy as it is now: the record read again, Steam read, and, the first time Steam is
/// seen switched without a recorded time, that time noted (`steam_at`), so only a Steam start after it counts.
pub fn readiness_now(entry: &OldCopy, home: &Path, config_dir: &Path) -> Result<Readiness, String> {
    let mut entry = current(entry, config_dir)?;
    let view = steam_view(&entry, home, config_dir);
    if !view.waiting && entry.steam_at.is_none() && entry.repoint_queued != Some(false) {
        let now = chrono::Utc::now().timestamp();
        if mark_steam_seen(config_dir, &entry.old_path, now).unwrap_or(0) > 0 {
            entry.steam_at = Some(now);
        }
    }
    Ok(readiness(&entry, &view))
}

//// The readiness of the old copy as it is now, read only (the checkup): [`readiness`] over what Steam says now, with nothing
/// noted in the record (unlike [`readiness_now`], which notes the first time Steam is seen switched). A Steam start counts only
/// when a switch time is recorded (`steam_at`, or the move time when the move itself switched the shortcut); with none the
/// copy is "Start it once": removal must never look closer than it is.
pub fn readiness_read_only(entry: &OldCopy, home: &Path, config_dir: &Path) -> Readiness {
    readiness(entry, &steam_view(entry, home, config_dir))
}


fn mark_steam_seen(config_dir: &Path, old_path: &Path, at: i64) -> io::Result<usize> {
    update(config_dir, |e| e.old_path == old_path, |e| e.steam_at.is_none() && { e.steam_at = Some(at); true })
}

pub fn readiness(entry: &OldCopy, steam: &SteamView) -> Readiness {
    if steam.waiting {
        return Readiness::WaitingForSteam;
    }
    if let Some(started) = entry.started {
        return Readiness::Ready(started.by);
    }
    match (steam.played_new_at, steam_since(entry)) {
        (Some(played), Some(since)) if played > since => Readiness::Ready(StartedBy::Steam),
        _ => Readiness::StartOnce,
    }
}

/// Why an old copy that is not ready is kept, in plain words.
pub fn not_ready_reason(entry: &OldCopy, readiness: Readiness) -> Option<String> {
    match readiness {
        Readiness::Ready(_) => None,
        Readiness::WaitingForSteam => Some(format!("Steam still starts the old copy of {} (or its change is waiting): close Steam and update the shortcuts first, so the old copy is kept.", entry.name)),
        Readiness::StartOnce => Some(format!("Start {} once from its new place (or say it starts and your saves are there), then the old copy can go. Until then it is kept.", entry.name)),
    }
}

/// The latest record for the same old copy (a confirm or a launch may have been recorded since `entry` was read).
/// A list that cannot be read is an error (never the copy held in memory); a record no longer listed is `entry`.
pub fn current(entry: &OldCopy, config_dir: &Path) -> Result<OldCopy, String> {
    Ok(load(config_dir)?.into_iter().find(|e| e.old_path == entry.old_path).unwrap_or_else(|| entry.clone()))
}

/// [`removal_check`] for an old copy that must also be [`Readiness::Ready`]: what every removal goes through.
pub fn ready_removal_check(entry: &OldCopy, ctx: &RemovalCtx) -> Result<u64, String> {
    let entry = current(entry, ctx.config_dir).map_err(|e| format!("{e} The old copy is kept."))?;
    let readiness = readiness_now(&entry, ctx.home, ctx.config_dir)?;
    if let Some(why) = not_ready_reason(&entry, readiness) {
        return Err(why);
    }
    removal_check(&entry, ctx)
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

pub(crate) fn same(a: &Path, b: &Path) -> bool {
    a == b || matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

pub(crate) fn holds(outer: &Path, inner: &Path) -> bool {
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
    if !entry.names_agree() {
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

/// Removes the old copy after the checks pass again, readiness included. `backup_saves` zips the saves found
/// in the old copy first (a failure stops the removal). The record is dropped afterwards.
pub fn remove(entry: &OldCopy, ctx: &RemovalCtx, backup_saves: &dyn Fn(&OldCopy) -> Result<Option<PathBuf>, String>) -> Result<Removed, String> {
    let bytes = ready_removal_check(entry, ctx)?;
    let saves_zip = if entry.link_only { None } else { backup_saves(entry).map_err(|e| format!("The saves in the old copy could not be backed up, so nothing was removed: {e}"))? };
    remove_copy(&entry.old_path).map_err(|e| format!("Could not remove {} completely: {e}. What is left stays; nothing else was touched.", entry.old_path.display()))?;
    let _ = forget(ctx.config_dir, &entry.old_path);
    Ok(Removed { bytes, saves_zip })
}
