//! Putting a release into a port folder (an update, a reinstall over the old folder, or the extras) without losing the
//! user's files (2026-11-04 review W2 / D1, D7, F-D2, F-D3).
//!
//! Before each file of the release goes in, what already sits at the same place is looked at:
//! - **The user's own data** (a file under a save folder, a connected or required game file such as a generated
//!   `oot.o2r`, or what `uninstall_keep::classify` calls a ROM, key, dump or save file): it stays where it is, and the
//!   release's copy goes to `.previous/update-copies-<time>/<path>` in the port folder instead. An update is never
//!   refused for a clash; an unreadable record of connected game files stops it before anything changes. A save
//!   extension, or `save` on generic data, counts as a save here (`save_icon.png` is a program file). With a save
//!   folder that is the whole port folder (`"save": "."`), every file a release replaces is kept as below.
//! - **Other protected files** (an `.o2r` / `.otr` archive the release ships itself, like `soh.o2r`): the old file is
//!   moved to `<Saves Backup>/Replaced by update/<port folder>/<time>/<path>` and the release's copy goes in. A folder
//!   that a release file replaces goes there too.
//! - **Everything else** (program files) is replaced as before.
//!
//! A file with the same bytes as the release's is left as it is. Nothing is deleted while the merge runs: replaced
//! program files wait in a `_displaced_` folder beside the port folders, and a failure part way moves every file back
//! where it was (D7). Only after the whole release is in are the replaced program files removed. A folder the user
//! replaced with a link stays a link; the release's files go through it (F-D3). An entry of the release that cannot be
//! read stops the merge (F-D2) instead of being skipped.
//!
//! The installer records what it set aside ([`record`]); the app reads it once ([`take`]) for the Done page.

use crate::core::models::Port;
use crate::core::path_safety::is_within;
use crate::core::platform_resolve::resolve_save_folder;
use crate::fork::uninstall_keep::{self, KeepKind};
use crate::fork::versioned::PREVIOUS;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The folder under Saves Backup that holds files an update replaced.
pub const REPLACED_BY_UPDATE: &str = "Replaced by update";

/// The start of the `.previous/` folder that holds a release's copies of the user's files (`update-copies-<time>`);
/// it is not an older version, so the Port page leaves it out of that list.
pub const UPDATE_COPIES: &str = "update-copies-";

/// What counts as the user's own data in one port folder.
#[derive(Debug, Clone, Default)]
pub struct Protect {
    /// Save folders (or save files) of the port, as paths.
    saves: Vec<PathBuf>,
    /// Lowercase names the port's entry needs, and the archives the port makes from them (`oot.o2r`).
    names: Vec<String>,
    /// Connected game files inside the port folder.
    connected: Vec<PathBuf>,
}

fn real(p: &Path) -> PathBuf {
    match (p.parent().and_then(|d| fs::canonicalize(d).ok()), p.file_name()) {
        (Some(dir), Some(name)) => dir.join(name),
        _ => p.to_path_buf(),
    }
}

/// `path` is `base` or below it, as written or through links.
fn within(path: &Path, base: &Path) -> bool {
    is_within(path, base) || is_within(&real(path), &fs::canonicalize(base).unwrap_or_else(|_| base.to_path_buf()))
}

impl Protect {
    /// The port's saves, required and generated game files, and the files connected into `port_dir` (read from
    /// `rom_library.json` in `config_dir`). A record that cannot be read is an error (`UNREADABLE_RECORD`): without
    /// it nobody can say which files are the user's, so the install stops before anything changes, as an uninstall does.
    pub fn for_port(port: &Port, port_dir: &Path, config_dir: Option<&Path>) -> Result<Protect, String> {
        let saves = [port.save.as_ref(), port.save2.as_ref()].into_iter().flatten().filter_map(|v| resolve_save_folder(v, port_dir)).collect();
        let (names, connected) = uninstall_keep::known_game_files(port, port_dir, config_dir)?;
        Ok(Protect { saves, names, connected })
    }

    /// A save folder that is the whole port folder (or holds it): `"save": "."`. Then any file may be a save, and the
    /// pre-update zip skips it (it would copy the whole game), so every replaced file is kept instead (review C4).
    fn saves_hold_the_folder(&self, port_dir: &Path) -> bool {
        self.saves.iter().any(|s| is_within(port_dir, s))
    }

    /// The file at `there` is the user's own and must stay.
    fn user_data(&self, there: &Path, port_dir: &Path) -> bool {
        // A save that is the whole game folder would make every file a save; those files are kept in Replaced by update
        // when replaced instead (`saves_hold_the_folder`).
        if self.saves.iter().any(|s| !is_within(port_dir, s) && within(there, s)) {
            return true;
        }
        if self.connected.iter().any(|c| c == there || real(c) == real(there)) {
            return true;
        }
        let name = there.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        if self.names.contains(&name) {
            return true;
        }
        let size = fs::metadata(there).map_or(0, |m| m.len());
        let named_by_sheet = there.parent().is_some_and(|dir| uninstall_keep::sheets_in(dir).names(&name));
        match uninstall_keep::classify(&name, size, named_by_sheet) {
            Some(KeepKind::Rom | KeepKind::Key | KeepKind::Dump) => true,
            // Review S1: `save*` alone is too broad for an update (`save_icon.png`, `savemanager.lua` are program files
            // a release must be able to update). A save file here has a save extension, or a `save` name on generic data
            // (`savegame.dat`, `SAVE1.bin`) or no extension; the uninstall keeps the broad rule.
            Some(KeepKind::Save) => uninstall_keep::save_like_for_update(&name),
            _ => false,
        }
    }
}

fn protected_archive(there: &Path) -> bool {
    let name = there.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    uninstall_keep::classify(&name, 0, false) == Some(KeepKind::Archive)
}

/// How a merge treats what is already there.
pub struct Guard {
    /// `None`: everything is replaced (upstream's merge; tests of the unpacking only).
    protect: Option<Protect>,
    /// `<Saves Backup>/Replaced by update/<port folder>`.
    replaced_base: PathBuf,
    /// Where replaced program files wait until the merge is done (the ports folder).
    displaced_in: PathBuf,
    stamp: String,
}

impl Guard {
    /// Replaces whatever is there (upstream's behavior), still with the roll back on failure.
    pub fn plain(displaced_in: &Path) -> Guard {
        Guard { protect: None, replaced_base: PathBuf::new(), displaced_in: displaced_in.to_path_buf(), stamp: stamp() }
    }

    pub fn protecting(protect: Protect, saves_backup_dir: &Path, folder: &str, displaced_in: &Path, stamp: &str) -> Guard {
        let replaced_base = crate::core::path_safety::safe_join(&saves_backup_dir.join(REPLACED_BY_UPDATE), folder).unwrap_or_else(|_| saves_backup_dir.join(REPLACED_BY_UPDATE).join("_"));
        Guard { protect: Some(protect), replaced_base, displaced_in: displaced_in.to_path_buf(), stamp: stamp.to_string() }
    }
}

/// "2026-11-04_10-00-00", the shape the installer and Kept files use.
pub fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string()
}

/// What a merge set aside.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeReport {
    /// Files of the user's that stayed; the release's copies went to [`MergeReport::previous_dir`].
    pub kept_yours: Vec<PathBuf>,
    pub previous_dir: Option<PathBuf>,
    /// Old files (and folders) moved to [`MergeReport::replaced_dir`] before the release's copy went in.
    pub replaced: Vec<PathBuf>,
    pub replaced_dir: Option<PathBuf>,
    /// Review S3: saves kept from an earlier uninstall went back, and what was at their place (newer saves) was moved
    /// to this folder (`<Saves Backup>/Replaced by restore/<port folder>/<time>`) first.
    pub saves_set_aside: Option<PathBuf>,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

impl MergeReport {
    pub fn is_empty(&self) -> bool {
        self.kept_yours.is_empty() && self.replaced.is_empty() && self.saves_set_aside.is_none()
    }

    /// One plain fact for the Done page; `what` is "update" or "extras". `None` when nothing was set aside. Places
    /// stop at the port's folder in Saves Backup (the time-named folder inside is not spelled out).
    pub fn line(&self, what: &str, place: &dyn Fn(&Path) -> String) -> Option<String> {
        let port_place = |dir: &Path| place(dir.parent().unwrap_or(dir));
        let mut parts = Vec::new();
        if !self.kept_yours.is_empty() {
            let n = self.kept_yours.len();
            let yours = if n == 1 { "yours is" } else { "yours are" };
            parts.push(format!("Kept your files: {} in the {what} had the same name as yours; {yours} still in place.", plural(n, "file", "files")));
        }
        if let (false, Some(dir)) = (self.replaced.is_empty(), &self.replaced_dir) {
            let n = self.replaced.len();
            parts.push(format!("{} replaced by the {what} {} kept in {}.", plural(n, "older file", "older files"), if n == 1 { "is" } else { "are" }, port_place(dir)));
        }
        if let Some(dir) = &self.saves_set_aside {
            parts.push(format!("Saves kept from an earlier uninstall were put back; the saves that were there were moved to {} first.", port_place(dir)));
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    }
}

/// One move the merge made, to undo it on failure.
enum Done {
    Created(PathBuf),
    Moved { from: PathBuf, to: PathBuf },
}

struct Run<'a> {
    guard: &'a Guard,
    port_dir: PathBuf,
    mover: &'a mut dyn FnMut(&Path, &Path) -> io::Result<()>,
    journal: Vec<Done>,
    displaced: Option<tempfile::TempDir>,
    previous: Option<PathBuf>,
    replaced: Option<PathBuf>,
    report: MergeReport,
}

/// `name`, or `name-2`, `name-3`... whichever is free in `parent`.
fn unique_dir(parent: &Path, name: &str) -> PathBuf {
    let first = parent.join(name);
    if fs::symlink_metadata(&first).is_err() {
        return first;
    }
    (2..1000).map(|n| parent.join(format!("{name}-{n}"))).find(|p| fs::symlink_metadata(p).is_err()).unwrap_or_else(|| parent.join(format!("{name}-x")))
}

impl Run<'_> {
    fn mv(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Err(e) = (self.mover)(from, to) {
            // Review C6: a move that failed after something landed at `to` is undone (or named as stuck) too.
            if fs::symlink_metadata(to).is_ok() {
                self.journal.push(Done::Moved { from: from.to_path_buf(), to: to.to_path_buf() });
            }
            return Err(e);
        }
        self.journal.push(Done::Moved { from: from.to_path_buf(), to: to.to_path_buf() });
        Ok(())
    }

    fn displaced_dir(&mut self) -> io::Result<PathBuf> {
        if self.displaced.is_none() {
            self.displaced = Some(tempfile::Builder::new().prefix("_displaced_").tempdir_in(&self.guard.displaced_in)?);
        }
        Ok(self.displaced.as_ref().map(|d| d.path().to_path_buf()).unwrap_or_default())
    }

    fn previous_dir(&mut self) -> PathBuf {
        let port_dir = self.port_dir.clone();
        let stamp = self.guard.stamp.clone();
        self.previous.get_or_insert_with(|| unique_dir(&port_dir.join(PREVIOUS), &format!("{UPDATE_COPIES}{stamp}"))).clone()
    }

    fn replaced_dir(&mut self) -> PathBuf {
        let base = self.guard.replaced_base.clone();
        let stamp = self.guard.stamp.clone();
        self.replaced.get_or_insert_with(|| unique_dir(&base, &stamp)).clone()
    }

    /// Clears `there` (an entry of the port folder at `rel`) for the release: a protected file or a folder is kept in
    /// Replaced by update; anything else waits in the displaced folder.
    fn clear(&mut self, there: &Path, rel: &Path) -> io::Result<()> {
        let meta = fs::symlink_metadata(there)?;
        let keep = self.guard.protect.as_ref().is_some_and(|p| p.saves_hold_the_folder(&self.port_dir))
            || (self.guard.protect.is_some() && ((meta.is_dir() && !meta.file_type().is_symlink()) || protected_archive(there) || self.user_data(there)));
        if keep {
            let to = self.replaced_dir().join(rel);
            self.mv(there, &to)?;
            self.report.replaced.push(rel.to_path_buf());
        } else {
            let to = self.displaced_dir()?.join(rel);
            self.mv(there, &to)?;
        }
        Ok(())
    }

    fn user_data(&self, there: &Path) -> bool {
        self.guard.protect.as_ref().is_some_and(|p| p.user_data(there, &self.port_dir))
    }

    fn merge_dir(&mut self, src_root: &Path, dest_root: &Path, rel: &Path) -> io::Result<()> {
        // Fork (F-D2): an entry that cannot be read stops the merge; it is never skipped.
        let entries = fs::read_dir(src_root.join(rel))?.collect::<io::Result<Vec<_>>>()?;
        for e in entries {
            let r = rel.join(e.file_name());
            let (from, there) = (e.path(), dest_root.join(&r));
            let found = fs::symlink_metadata(&there).ok();
            if e.file_type()?.is_dir() {
                match found {
                    None => {
                        fs::create_dir(&there)?;
                        self.journal.push(Done::Created(there.clone()));
                    }
                    Some(m) if m.is_dir() => {}
                    // A folder the user swapped for a link stays a link (F-D3).
                    Some(m) if m.file_type().is_symlink() && there.is_dir() => {}
                    Some(_) => {
                        self.clear(&there, &r)?;
                        fs::create_dir(&there)?;
                        self.journal.push(Done::Created(there.clone()));
                    }
                }
                self.merge_dir(src_root, dest_root, &r)?;
                continue;
            }
            match found {
                None => {}
                Some(m) if m.is_dir() && !m.file_type().is_symlink() => self.clear(&there, &r)?,
                Some(m) => {
                    let protecting = self.guard.protect.is_some();
                    if protecting && m.is_file() && uninstall_keep::same_bytes(&there, &from) {
                        continue;
                    }
                    if protecting && self.user_data(&there) {
                        let to = self.previous_dir().join(&r);
                        self.mv(&from, &to)?;
                        self.report.kept_yours.push(r.clone());
                        continue;
                    }
                    self.clear(&there, &r)?;
                }
            }
            self.mv(&from, &there)?;
        }
        Ok(())
    }

    /// Puts every moved file back, newest first. Returns the places of the ones that could not go back.
    fn roll_back(&mut self) -> Vec<PathBuf> {
        let mut stuck = Vec::new();
        for step in std::mem::take(&mut self.journal).into_iter().rev() {
            match step {
                Done::Created(dir) => {
                    let _ = fs::remove_dir(&dir);
                }
                Done::Moved { from, to } => {
                    if uninstall_keep::move_path(&to, &from).is_err() {
                        stuck.push(to);
                    }
                }
            }
        }
        for dir in [self.previous.take(), self.replaced.take()].into_iter().flatten() {
            let _ = remove_empty_dirs(&dir);
        }
        stuck
    }
}

/// Removes empty folders under `dir`, and `dir` itself when it ends up empty (never a file).
fn remove_empty_dirs(dir: &Path) -> io::Result<()> {
    for e in fs::read_dir(dir)?.flatten() {
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            let _ = remove_empty_dirs(&e.path());
        }
    }
    fs::remove_dir(dir)
}

/// Merges the release in `src` into the port folder `dest` (see the module docs). `mover` moves one entry (tests make it
/// fail part way). On an error everything is put back as it was and the error returned; files that could not be put
/// back are named in it, and the folder holding them is kept.
pub fn merge(src: &Path, dest: &Path, guard: &Guard, mover: &mut dyn FnMut(&Path, &Path) -> io::Result<()>) -> io::Result<MergeReport> {
    fs::create_dir_all(dest)?;
    let mut run = Run { guard, port_dir: dest.to_path_buf(), mover, journal: Vec::new(), displaced: None, previous: None, replaced: None, report: MergeReport::default() };
    if let Err(e) = run.merge_dir(src, dest, Path::new("")) {
        let stuck = run.roll_back();
        if stuck.is_empty() {
            return Err(io::Error::new(e.kind(), format!("{e}; everything was put back as it was")));
        }
        // What could not go back must not be deleted with the displaced folder.
        if let Some(d) = run.displaced.take() {
            let _ = d.keep();
        }
        let names: Vec<String> = stuck.iter().map(|p| crate::fork::port_page::paths::human_path_here(p)).collect();
        return Err(io::Error::new(e.kind(), format!("{e}; {} could not be put back and {} kept at: {}", plural(stuck.len(), "file", "files"), if stuck.len() == 1 { "is" } else { "are" }, names.join(", "))));
    }
    let mut report = std::mem::take(&mut run.report);
    report.previous_dir = run.previous.clone().filter(|_| !report.kept_yours.is_empty());
    report.replaced_dir = run.replaced.clone().filter(|_| !report.replaced.is_empty());
    // The release is in: the replaced program files go (the displaced folder is removed when dropped here).
    drop(run);
    Ok(report)
}

static REPORTS: Mutex<Vec<(PathBuf, MergeReport)>> = Mutex::new(Vec::new());

/// Called by the installer (worker thread) when a merge into `port_dir` set something aside.
pub fn record(port_dir: &Path, report: MergeReport) {
    let mut all = REPORTS.lock().unwrap_or_else(|e| e.into_inner());
    // Review S2: an earlier merge's report never outlives a newer merge into the same folder.
    all.retain(|(d, _)| d != port_dir);
    if report.is_empty() {
        return;
    }
    for r in &report.kept_yours {
        eprintln!("update merge: kept the user's {} in {}; the release's copy is in .previous", r.display(), port_dir.display());
    }
    for r in &report.replaced {
        eprintln!("update merge: moved the old {} of {} to {}", r.display(), port_dir.display(), report.replaced_dir.as_deref().unwrap_or(Path::new("?")).display());
    }
    all.push((port_dir.to_path_buf(), report));
}

/// The app (UI thread) takes what the last merge into `port_dir` set aside, once.
pub fn take(port_dir: &Path) -> Option<MergeReport> {
    let mut all = REPORTS.lock().unwrap_or_else(|e| e.into_inner());
    let i = all.iter().position(|(d, _)| d == port_dir)?;
    Some(all.remove(i).1)
}

#[cfg(test)]
mod tests;
