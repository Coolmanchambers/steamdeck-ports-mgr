//! Versioned program files: releases that put the version in the file name
//! (`Dusklight-v2.0.3-linux-x86_64.AppImage`). An update lands the new file beside the old one, so
//! after a verified update the old release's program files are moved to `.previous/<old version>/`
//! inside the port folder (never deleted), and the app's launch target and the Steam shortcut are
//! pointed at the new file (same app ID; queued while Steam runs).
//!
//! Works for any port, by name alone:
//! - "Same family" = the same words once version and platform words are dropped
//!   (`adopt::release_tokens`), so `Game-v1.0-linux-x86_64.AppImage` and `Game-v1.1-linux-x86_64.AppImage`
//!   are one family and `Game-Editor-v1.1` is another.
//! - Only names with a real version (`v1`, `1.0`, `v2.0.3`; not a bare `2`, not a `(1)` copy) count, so a
//!   user's own `game.AppImage` or `Sonic-2.AppImage` is never touched.
//! - Files move only when they are programs (`.AppImage`, `.x86_64`, `.x86`, `.sh`, `.run`, or an ELF or
//!   `#!` script by content). Folders move only when they are a versioned release folder holding no
//!   save folder, link, `.o2r`/`.otr`, mods folder, settings file or game file.
//! - Nothing moves until the update is installed and the new launch target exists; a Steam shortcut
//!   that still points at an old file (change queued, or not writable) keeps that file in place until
//!   the shortcut is updated (`after_queued_repoint`).
//! - Nothing is ever deleted: every superseded version stays in `.previous/`; the summary says how much
//!   room the older ones take.
//!
//! The installer (worker thread) calls `before_update` and `record_installed`; the app (UI thread) calls
//! `finish_update` when the install is done.

use crate::core::executable_detect::resolve_executable;
use crate::core::models::Port;
use crate::core::path_safety::safe_join;
use crate::core::platform_resolve::{save_location, SaveLocation};
use crate::fork::adopt::release_tokens;
use crate::fork::port_mover::steam::{self, SteamCtx, SteamReport};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[cfg(test)]
mod tests;

/// The folder inside a port that holds superseded releases.
pub const PREVIOUS: &str = ".previous";

/// Endings that make a file a program on their own (`.bin` is left out: disc images use it).
const PROGRAM_ENDINGS: [&str; 5] = [".appimage", ".x86_64", ".x86", ".sh", ".run"];
/// Files that mark a folder as holding more than a release: settings, saves, keys, packed game data,
/// archives, disc images and game files.
const KEEP_EXTENSIONS: &[&str] = &[
    "o2r", "otr", "ini", "cfg", "conf", "json", "toml", "yaml", "yml", "xml", "sav", "srm", "mcr", "mcd", "sol", "db", "keys", "dat", "bin", "img",
    "iso", "cue", "chd", "gcm", "rvz", "wbfs", "pbp", "pkg", "wad", "cia", "3ds", "zip", "7z", "rar", "z64", "n64", "v64", "nds", "gba", "gbc",
    "gb", "sfc", "smc", "nes", "xci", "nsp",
];
/// How many entries a folder check looks at before giving up (and keeping the folder).
const MAX_FOLDER_ENTRIES: usize = 20_000;
/// Launcher scripts larger than this are never rewritten.
const MAX_SCRIPT_BYTES: u64 = 256 * 1024;

// -- Names ----------------------------------------------------------------------------------------

/// What a top-level file or folder name says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name {
    /// Words once version and platform words are dropped (`["dusklight"]`).
    pub family: Vec<String>,
    /// The version as written (`v2.0.3`), when the name has a real one.
    pub version: Option<String>,
    /// The name cut at `-`, `_` and spaces, lowercase (for telling close relatives apart).
    segments: Vec<String>,
}

fn strip_program_ending(name: &str) -> &str {
    let lower = name.to_lowercase();
    match PROGRAM_ENDINGS.iter().find(|e| lower.ends_with(*e)) {
        Some(end) => &name[..name.len() - end.len()],
        None => name,
    }
}

/// `v1`, `1.0`, `v2.0.3`, `2026.10`: a version. A bare `2` is not (it may be part of the title).
fn strong_version(segment: &str) -> bool {
    let digits = segment.strip_prefix(['v', 'V']).unwrap_or(segment);
    let has_prefix = digits.len() < segment.len();
    let parts: Vec<&str> = digits.split('.').collect();
    !digits.is_empty() && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())) && (has_prefix || parts.len() > 1)
}

pub fn parse(name: &str) -> Name {
    let stem = strip_program_ending(name);
    let lowered = stem.to_lowercase().replace("x86_64", "x8664").replace("x86-64", "x8664");
    let segments: Vec<String> = lowered.split(['-', '_', ' ']).filter(|s| !s.is_empty()).map(str::to_string).collect();
    // A browser or file-manager copy (`Game-v1.0 (1).AppImage`) is the user's, never a release file.
    let copy = name.contains(['(', ')']);
    let version = if copy { None } else { segments.iter().rev().find(|s| strong_version(s)).cloned() };
    Name { family: release_tokens(stem), version, segments }
}

/// How many cut-points differ between two names of one family (fewer = closer relatives).
fn distance(a: &Name, b: &Name) -> usize {
    if a.segments.len() != b.segments.len() {
        return 1000 + a.segments.len().abs_diff(b.segments.len());
    }
    a.segments.iter().zip(&b.segments).filter(|(x, y)| x != y).count()
}

// -- What counts as a program, what must stay --------------------------------------------------

/// A plain file (not a link) that is a program: a program ending, or an ELF / `#!` script by content.
pub fn is_program_file(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else { return false };
    if !meta.is_file() {
        return false;
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
    if PROGRAM_ENDINGS.iter().any(|e| name.ends_with(e)) {
        return true;
    }
    let mut head = [0u8; 4];
    let read = fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    (read >= 4 && head == *b"\x7fELF") || (read >= 2 && head[..2] == *b"#!")
}

/// Why a versioned folder must stay where it is, if it must. A folder with no program in it is not a
/// release folder and stays too.
fn folder_must_stay(dir: &Path) -> Option<&'static str> {
    let mut stack = vec![dir.to_path_buf()];
    let mut seen = 0usize;
    let mut has_program = false;
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else { return Some("it could not be read") };
        for e in entries.flatten() {
            seen += 1;
            if seen > MAX_FOLDER_ENTRIES {
                return Some("it holds too many files to check");
            }
            let Ok(meta) = fs::symlink_metadata(e.path()) else { return Some("it could not be read") };
            let name = e.file_name().to_string_lossy().to_lowercase();
            if meta.file_type().is_symlink() {
                return Some("it holds a link (game files are often connected that way)");
            }
            if meta.is_dir() {
                if name == "mods" || name.starts_with("save") {
                    return Some("it holds mods or saves");
                }
                stack.push(e.path());
            } else {
                if let Some(ext) = Path::new(&name).extension().and_then(|x| x.to_str()) {
                    if KEEP_EXTENSIONS.contains(&ext) {
                        return Some("it holds settings, saves or game files");
                    }
                }
                has_program = has_program || is_program_file(&e.path());
            }
        }
    }
    (!has_program).then_some("it holds no program")
}

/// Where the port keeps its saves on this system.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SaveInfo {
    /// `save` and `save2` as resolved here.
    pub folders: Vec<PathBuf>,
    /// Every save field the port has resolves here (and it has one). Only then can a release folder move:
    /// otherwise the saves may sit inside it.
    pub folders_may_move: bool,
}

pub fn save_info(port: &Port, game_dir: &Path) -> SaveInfo {
    let mut info = SaveInfo { folders: Vec::new(), folders_may_move: true };
    for v in [port.save.as_ref(), port.save2.as_ref()].into_iter().flatten() {
        match save_location(v, game_dir) {
            SaveLocation::Known(p) => info.folders.push(p),
            SaveLocation::UnknownOnLinux => info.folders_may_move = false,
            SaveLocation::NotListed => {}
        }
    }
    info.folders_may_move &= !info.folders.is_empty();
    info
}

fn real(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// `entry` is a save folder or holds one, or (folders only) sits inside one.
fn touches_saves(entry: &Path, is_dir: bool, saves: &[PathBuf]) -> bool {
    let e = real(entry);
    saves.iter().any(|s| {
        let s = real(s);
        s == e || s.starts_with(&e) || (is_dir && e.starts_with(&s))
    })
}

// -- The installer's part (worker thread) ------------------------------------------------------

/// The port folder's top-level names before an update, and the program the app would have started.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Before {
    pub entries: Vec<String>,
    /// Relative to the port folder, `/`-separated.
    pub target: Option<String>,
}

fn top_level_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<String> = entries.flatten().filter_map(|e| e.file_name().into_string().ok()).collect();
    names.sort();
    names
}

fn relative(path: &Path, dir: &Path) -> Option<String> {
    let rel = path.strip_prefix(dir).ok()?;
    let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

pub fn before_update(port: &Port, game_dir: &Path) -> Before {
    let target = resolve_executable(port.executable.as_ref(), game_dir).ok().filter(|p| p.is_file()).and_then(|p| relative(&p, game_dir));
    Before { entries: top_level_names(game_dir), target }
}

/// An installed update waiting for the app to finish it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub before: Before,
    /// The release's own top-level names.
    pub release: Vec<String>,
}

static PENDING: Mutex<Vec<(PathBuf, Pending)>> = Mutex::new(Vec::new());

/// Called once an update is unpacked into `game_dir` and the saves are back. Nothing moves here.
pub fn record_installed(game_dir: &Path, before: Before, release: Vec<String>) {
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    pending.retain(|(d, _)| d != game_dir);
    pending.push((game_dir.to_path_buf(), Pending { before, release }));
}

pub fn take_installed(game_dir: &Path) -> Option<Pending> {
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    let i = pending.iter().position(|(d, _)| d == game_dir)?;
    Some(pending.remove(i).1)
}

// -- Planning ----------------------------------------------------------------------------------

/// One old top-level entry replaced by one new one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Supersede {
    pub old: String,
    pub new: String,
    pub dir: bool,
    /// The old entry's version, the name of its folder in `.previous/`.
    pub version: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub moves: Vec<Supersede>,
    /// (name, why it stays) for entries of the new release's family that are left where they are.
    pub kept: Vec<(String, String)>,
}

/// Which entries of the old release the new one supersedes. Reads the folder, changes nothing.
pub fn plan(game_dir: &Path, pending: &Pending, save_info: &SaveInfo) -> Plan {
    let mut out = Plan::default();
    let saves = save_info.folders.as_slice();
    let saves_hold_the_folder = saves.iter().any(|s| real(game_dir).starts_with(real(s)));
    let new_entries: Vec<(String, Name, bool)> = pending
        .release
        .iter()
        .filter(|n| !n.starts_with('.'))
        .filter_map(|n| {
            let meta = fs::symlink_metadata(game_dir.join(n)).ok()?;
            let parsed = parse(n);
            (parsed.version.is_some() && (meta.is_file() || meta.is_dir())).then(|| (n.clone(), parsed, meta.is_dir()))
        })
        .collect();
    for old in &pending.before.entries {
        if old.starts_with('.') || pending.release.contains(old) {
            continue;
        }
        let old_path = game_dir.join(old);
        let Ok(meta) = fs::symlink_metadata(&old_path) else { continue };
        let is_dir = meta.is_dir();
        if !(is_dir || meta.is_file()) {
            continue;
        }
        let parsed = parse(old);
        let Some(version) = parsed.version.clone() else { continue };
        if parsed.family.is_empty() {
            continue;
        }
        let same_family: Vec<&(String, Name, bool)> = new_entries.iter().filter(|(_, n, d)| *d == is_dir && n.family == parsed.family).collect();
        if same_family.is_empty() {
            continue;
        }
        if !is_dir && !is_program_file(&old_path) {
            continue; // data named like the release (`Game-v1.0.o2r`, a ROM): not ours to move
        }
        if touches_saves(&old_path, is_dir, saves) || (is_dir && saves_hold_the_folder) {
            out.kept.push((old.clone(), "it is or holds a save folder".into()));
            continue;
        }
        if is_dir && !save_info.folders_may_move {
            out.kept.push((old.clone(), "this game's save folder is not known on this system, so its folders stay where they are".into()));
            continue;
        }
        if is_dir {
            if let Some(why) = folder_must_stay(&old_path) {
                out.kept.push((old.clone(), why.into()));
                continue;
            }
        }
        let best = same_family.iter().map(|(_, n, _)| distance(&parsed, n)).min().unwrap_or(usize::MAX);
        let closest: Vec<&&(String, Name, bool)> = same_family.iter().filter(|(_, n, _)| distance(&parsed, n) == best).collect();
        match closest.as_slice() {
            [(new, _, _)] => out.moves.push(Supersede { old: old.clone(), new: new.clone(), dir: is_dir, version }),
            _ => out.kept.push((old.clone(), "several new files could be its replacement".into())),
        }
    }
    out
}

// -- Finishing (UI thread) ---------------------------------------------------------------------

pub struct FinishInput<'a> {
    pub port: &'a Port,
    pub game_dir: &'a Path,
    /// The program picked in the port's info (relative), if any.
    pub favorite: Option<&'a str>,
    /// `None` when there is no home folder: then nothing is moved (shortcuts cannot be checked).
    pub steam: Option<&'a SteamCtx<'a>>,
}

#[derive(Debug, Default)]
pub struct Outcome {
    /// What was moved, and the `.previous/` folder each went to.
    pub moved: Vec<(Supersede, String)>,
    /// The new launch target (relative), when the old one was superseded and the new one exists.
    pub target: Option<String>,
    /// The version the port had before (the old launch target's, else the first one moved or waiting).
    pub had: Option<String>,
    /// Set the port's favorite program to this.
    pub new_favorite: Option<String>,
    pub steam: SteamReport,
    /// Old entries that stay until their queued Steam change is applied (`after_queued_repoint`).
    pub waiting: Vec<String>,
    pub kept: Vec<(String, String)>,
    /// Launcher scripts now naming the new file.
    pub scripts: Vec<String>,
    /// Bytes held by the versions in `.previous/` other than the ones this update put there.
    pub older_bytes: u64,
    pub notes: Vec<String>,
}

/// `3 MB` (rounded up, so a small file never reads as `0 MB`), or `1.2 GB` above 1000 MB.
pub fn size_text(bytes: u64) -> String {
    let mb = bytes.div_ceil(1_000_000);
    if mb > 1000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else {
        format!("{mb} MB")
    }
}

impl Outcome {
    pub fn is_empty(&self) -> bool {
        self.moved.is_empty() && self.target.is_none() && self.waiting.is_empty() && self.kept.is_empty() && self.notes.is_empty() && self.scripts.is_empty()
    }

    /// Something the user should know even when the game started right after the update.
    pub fn needs_attention(&self) -> bool {
        !self.waiting.is_empty() || !self.kept.is_empty() || !self.notes.is_empty() || !self.steam.skipped.is_empty() || !self.steam.failed.is_empty()
    }

    /// The text shown after the update, `None` when there is nothing to say. `folder` is the port's
    /// folder name, as the file manager shows it.
    pub fn summary(&self, port_name: &str, folder: &str, tag: Option<&str>) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut lines = Vec::new();
        let now = tag
            .map(str::to_string)
            .or_else(|| self.target.as_deref().and_then(|t| parse(t.split('/').next().unwrap_or(t)).version))
            .or_else(|| self.moved.first().and_then(|(m, _)| parse(&m.new).version));
        let now_line = match &now {
            Some(v) => format!("{port_name} is now {v}."),
            None => format!("{port_name} was updated."),
        };
        if !self.moved.is_empty() {
            let had = self.had.as_ref().map(|v| format!("The version you had ({v})")).unwrap_or_else(|| "The version you had".to_string());
            lines.push(format!("{now_line} {had} is kept in a hidden folder (.previous) inside the {folder} folder; to go back, use Info > Select version."));
        } else if self.target.is_some() || !self.waiting.is_empty() {
            lines.push(now_line);
        }
        if self.steam.changed_count() > 0 {
            lines.push("The Steam shortcut now starts the new version (artwork and play time kept).".into());
        }
        if !self.waiting.is_empty() {
            lines.push(
                "Steam is running, so the shortcut change waits. Close Steam (or come back to Desktop Mode) and this app will ask to apply it; until then Steam starts the old version, which stays in place."
                    .into(),
            );
        }
        for (name, why) in &self.steam.skipped {
            lines.push(format!("Steam: {name} was left alone: {why}."));
        }
        for f in &self.steam.failed {
            lines.push(format!("Steam: {f}"));
        }
        for s in &self.scripts {
            lines.push(format!("{s} now names the new version (the original is in .previous)."));
        }
        for (name, why) in &self.kept {
            let label = parse(name).version.map(|v| format!("The old version ({v})")).unwrap_or_else(|| name.clone());
            lines.push(format!("{label} was left where it is: {why}."));
        }
        if self.older_bytes > 0 {
            lines.push(format!(
                "Older versions in .previous use {} (in Desktop Mode's file manager, Ctrl+H shows hidden folders); remove them from the port's folder if you need the room.",
                size_text(self.older_bytes)
            ));
        }
        lines.extend(self.notes.iter().cloned());
        Some(lines.join("\n"))
    }
}

fn merge(into: &mut SteamReport, from: SteamReport) {
    into.written.extend(from.written);
    into.queued.extend(from.queued.into_iter().filter(|q| !into.queued.contains(q)).collect::<Vec<_>>());
    into.skipped.extend(from.skipped);
    into.failed.extend(from.failed);
}

/// `name`, or `name-2`, `name-3`... whichever does not exist yet in `parent`.
fn unique_dir(parent: &Path, name: &str) -> PathBuf {
    let safe: String = name.chars().map(|c| if c == '/' || c == '\\' { '_' } else { c }).collect();
    let mut candidate = parent.join(&safe);
    let mut n = 2;
    while fs::symlink_metadata(&candidate).is_ok() {
        candidate = parent.join(format!("{safe}-{n}"));
        n += 1;
    }
    candidate
}

/// Size of the files under `path` (links not followed).
fn tree_bytes(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else { return 0 };
    if !meta.is_dir() {
        return if meta.is_file() { meta.len() } else { 0 };
    }
    fs::read_dir(path).into_iter().flatten().flatten().map(|e| tree_bytes(&e.path())).sum()
}

/// Bytes held by the versions in `.previous/` other than `skip`.
fn older_versions_bytes(game_dir: &Path, skip: &[PathBuf]) -> u64 {
    fs::read_dir(game_dir.join(PREVIOUS)).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| !skip.contains(p)).map(|p| tree_bytes(&p)).sum()
}

/// Moves `entry` (a top-level name of `game_dir`) into `.previous/<version>/`, reusing `folder` when
/// given. Returns the folder used.
fn move_to_previous(game_dir: &Path, entry: &str, version: &str, folder: Option<&Path>) -> std::io::Result<PathBuf> {
    let dest_dir = match folder {
        Some(f) => f.to_path_buf(),
        None => unique_dir(&game_dir.join(PREVIOUS), version),
    };
    fs::create_dir_all(&dest_dir)?;
    let dest = dest_dir.join(entry);
    if fs::symlink_metadata(&dest).is_ok() {
        return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, format!("{} already exists", dest.display())));
    }
    fs::rename(game_dir.join(entry), &dest)?;
    Ok(dest_dir)
}

fn path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

/// Whether the text before a match lets it name an entry of the port folder: nothing, a separator or
/// quote, `./`, or a variable folder (`$HERE/`, `${DIR}/`, `"$(dirname "$0")"/`). Not a deeper path
/// (`~/.local/share/Game-v1.0`) and not the middle of a longer name.
fn starts_a_port_path(before: &str) -> bool {
    let Some(last) = before.chars().last() else { return true };
    if last != '/' {
        return !path_char(last);
    }
    let head = &before[..before.len() - 1];
    let Some(prev) = head.chars().last() else { return false };
    if matches!(prev, '"' | '\'' | ')' | '}') {
        return true;
    }
    let component = head.rsplit(|c: char| c == '/' || c.is_whitespace() || matches!(c, '"' | '\'' | '=' | '(')).next().unwrap_or("");
    component == "." || component.starts_with('$')
}

/// `text` with every `old` that stands alone as a port-folder name (see `starts_a_port_path`, and no
/// name character right after it) replaced by `new`; `None` when there is none.
fn replace_names(text: &str, old: &str, new: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut any = false;
    for (i, _) in text.match_indices(old) {
        let after = &text[i + old.len()..];
        if starts_a_port_path(&text[..i]) && !after.chars().next().is_some_and(path_char) {
            out.push_str(&text[last..i]);
            out.push_str(new);
            last = i + old.len();
            any = true;
        }
    }
    any.then(|| {
        out.push_str(&text[last..]);
        out
    })
}

/// Top-level `.sh` launchers (not part of the release) that name a moved entry get the new name; the
/// original goes next to the old version as `<script>.before-update`.
fn rewrite_scripts(game_dir: &Path, release: &[String], moved: &[(Supersede, String)], notes: &mut Vec<String>) -> Vec<String> {
    let mut done = Vec::new();
    for name in top_level_names(game_dir) {
        if !name.to_lowercase().ends_with(".sh") || release.contains(&name) {
            continue;
        }
        let path = game_dir.join(&name);
        let Ok(meta) = fs::symlink_metadata(&path) else { continue };
        if !meta.is_file() || meta.len() > MAX_SCRIPT_BYTES {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let mut new_text = text.clone();
        let mut folder: Option<&str> = None;
        for (m, f) in moved {
            if let Some(changed) = replace_names(&new_text, &m.old, &m.new) {
                new_text = changed;
                folder.get_or_insert(f.as_str());
            }
        }
        let Some(folder) = folder else { continue };
        let keep_copy = game_dir.join(PREVIOUS).join(folder).join(format!("{name}.before-update"));
        let tmp = game_dir.join(format!(".{name}.dpm-new"));
        let result = fs::copy(&path, &keep_copy)
            .and_then(|_| fs::write(&tmp, new_text.as_bytes()))
            .and_then(|_| fs::set_permissions(&tmp, meta.permissions()))
            .and_then(|_| fs::rename(&tmp, &path));
        match result {
            Ok(()) => done.push(name),
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                notes.push(format!("{name} still names the old version: it could not be updated ({e}). Change it by hand or pick the new program in Info > Favorite executable."));
            }
        }
    }
    done
}

/// A repoint to `to` for this port is in the Steam queue now (a queued change can cancel out an
/// earlier one, which leaves nothing waiting).
fn still_queued(ctx: &SteamCtx, port_key: &str, to: &Path) -> bool {
    let to = to.to_string_lossy();
    crate::fork::steam_queue::load(&crate::fork::steam_queue::queue_path(ctx.config_dir))
        .queue
        .entries
        .iter()
        .any(|e| e.kind == crate::fork::steam_queue::QueuedKind::Repoint && e.port_key == port_key && e.repoint.as_ref().is_some_and(|(_, t)| *t == to))
}

/// The program the app starts today for this port (relative), if it can tell without asking.
fn effective_target(port: &Port, game_dir: &Path, favorite: Option<&str>) -> Option<String> {
    if let Some(f) = favorite.filter(|f| !f.is_empty()) {
        if safe_join(game_dir, f).is_ok_and(|p| p.exists()) {
            return Some(f.to_string());
        }
    }
    resolve_executable(port.executable.as_ref(), game_dir).ok().filter(|p| p.is_file()).and_then(|p| relative(&p, game_dir))
}

/// Finishes an installed update: works out what the new release supersedes, points the launch target
/// and Steam at the new files, then moves the old ones to `.previous/`.
pub fn finish_update(input: &FinishInput) -> Outcome {
    let Some(pending) = take_installed(input.game_dir) else { return Outcome::default() };
    finish_with(input, &pending)
}

pub fn finish_with(input: &FinishInput, pending: &Pending) -> Outcome {
    let game_dir = input.game_dir;
    let plan = plan(game_dir, pending, &save_info(input.port, game_dir));
    let mut out = Outcome { kept: plan.kept.clone(), ..Default::default() };
    if plan.moves.is_empty() {
        return out;
    }
    let Some(ctx) = input.steam else {
        out.notes.push("The old version was left in place: Steam shortcuts could not be checked (no home folder).".into());
        return out;
    };

    // The launch target before the update, and its counterpart in the new release.
    let old_target = input.favorite.filter(|f| !f.is_empty()).map(str::to_string).or_else(|| pending.before.target.clone());
    let target_move = old_target.as_deref().and_then(|t| {
        let (head, rest) = t.split_once('/').map_or((t, None), |(h, r)| (h, Some(r)));
        plan.moves.iter().find(|m| m.old == head).map(|m| (m, rest.map_or(m.new.clone(), |r| format!("{}/{r}", m.new))))
    });
    if let Some((_, new_target)) = &target_move {
        if !safe_join(game_dir, new_target).is_ok_and(|p| p.is_file()) {
            out.kept.extend(plan.moves.iter().map(|m| (m.old.clone(), "the new version's program was not found where expected, so nothing was moved".to_string())));
            return out;
        }
        out.target = Some(new_target.clone());
    }

    // Steam first (the old paths still exist), then move only what nothing points at any more.
    let mut moves: Vec<&Supersede> = Vec::new();
    for m in &plan.moves {
        let from = game_dir.join(&m.old);
        let to = game_dir.join(&m.new);
        if fs::symlink_metadata(&to).is_err() {
            out.kept.push((m.old.clone(), "its replacement is missing".into()));
            continue;
        }
        // A change queued by an earlier update may still end at this old entry: it ends at the new one now.
        let queue = crate::fork::steam_queue::queue_path(ctx.config_dir);
        if let Err(e) = crate::fork::steam_queue::retarget_repoints(&queue, input.port.key(), &from.to_string_lossy(), &to.to_string_lossy()) {
            out.kept.push((m.old.clone(), format!("the queue of Steam changes could not be updated ({e})")));
            continue;
        }
        let holds_target = target_move.as_ref().is_some_and(|(t, _)| t.old == m.old);
        let mut queued_here = false;
        if !m.dir || holds_target {
            let report = steam::repoint_everywhere(input.port.key(), input.port.display_name(), &from, &to, ctx);
            queued_here = !report.queued.is_empty() && still_queued(ctx, input.port.key(), &to);
            merge(&mut out.steam, report);
        }
        match steam::shortcuts_pointing_into(ctx.home, &from) {
            Ok(names) if names.is_empty() => moves.push(m),
            Ok(_) if queued_here => out.waiting.push(m.old.clone()),
            Ok(names) => out.kept.push((m.old.clone(), format!("a Steam shortcut ({}) still points at it", names.join(", ")))),
            Err(e) => out.kept.push((m.old.clone(), format!("Steam shortcuts could not be read ({e})"))),
        }
    }

    let mut folders: Vec<(String, PathBuf)> = Vec::new();
    for m in moves {
        let existing = folders.iter().find(|(v, _)| *v == m.version).map(|(_, f)| f.clone());
        match move_to_previous(game_dir, &m.old, &m.version, existing.as_deref()) {
            Ok(folder) => {
                if existing.is_none() {
                    folders.push((m.version.clone(), folder.clone()));
                }
                let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                out.moved.push((m.clone(), name));
            }
            Err(e) => out.kept.push((m.old.clone(), format!("it could not be moved ({e})"))),
        }
    }
    out.had = target_move
        .as_ref()
        .map(|(m, _)| m.version.clone())
        .or_else(|| out.moved.first().map(|(m, _)| m.version.clone()))
        .or_else(|| plan.moves.iter().find(|m| out.waiting.contains(&m.old)).map(|m| m.version.clone()));
    if !out.moved.is_empty() {
        out.scripts = rewrite_scripts(game_dir, &pending.release, &out.moved, &mut out.notes);
        let this_update: Vec<PathBuf> = folders.iter().map(|(_, f)| f.clone()).collect();
        out.older_bytes = older_versions_bytes(game_dir, &this_update);
    }

    // The app starts the new program: a favorite follows it, and one is set when the app would not
    // find the new program on its own (the old one still beside it, or a catalog name that is gone).
    if let Some(new_target) = &out.target {
        let had_favorite = input.favorite.is_some_and(|f| !f.is_empty());
        if had_favorite || effective_target(input.port, game_dir, input.favorite).as_deref() != Some(new_target.as_str()) {
            out.new_favorite = Some(new_target.clone());
        }
    }
    out
}

/// After a queued Steam repoint from one versioned file (or release folder) to its successor in the
/// same port folder was written: the old one is moved to `.previous/` now that nothing points at it.
/// Returns a line for the summary. Anything that is not such a pair is left alone (`None`).
pub fn after_queued_repoint(from: &Path, to: &Path, library_dir: &Path, catalog: &[Port], home: &Path) -> Option<String> {
    let game_dir = from.parent()?;
    if to.parent()? != game_dir || from == to || fs::symlink_metadata(to).is_err() {
        return None;
    }
    let port = catalog.iter().find(|p| safe_join(library_dir, &p.folder).is_ok_and(|d| d == game_dir || real(&d) == real(game_dir)))?;
    let old = from.file_name()?.to_str()?.to_string();
    let new = to.file_name()?.to_str()?.to_string();
    if old.starts_with('.') {
        return None;
    }
    let before = Before { entries: vec![old.clone(), new.clone()], target: None };
    let pending = Pending { before, release: vec![new.clone()] };
    let plan = plan(game_dir, &pending, &save_info(port, game_dir));
    let m = plan.moves.first()?;
    if !steam::shortcuts_pointing_into(home, from).ok()?.is_empty() {
        return None;
    }
    let folder = move_to_previous(game_dir, &m.old, &m.version, None).ok()?;
    let name = folder.file_name()?.to_string_lossy().into_owned();
    let mut lines = vec![format!("{}: the old version ({}) is now kept in .previous/{name}.", port.display_name(), m.version)];
    let mut notes = Vec::new();
    for script in rewrite_scripts(game_dir, &[new], &[(m.clone(), name.clone())], &mut notes) {
        lines.push(format!("{script} now names the new version (the original is in .previous)."));
    }
    lines.extend(notes);
    Some(lines.join("
"))
}
