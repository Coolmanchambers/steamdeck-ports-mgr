//! Uninstall never deletes game files: before a port's folder is removed, the files the hard rule protects are
//! moved out to `<Saves Backup>/Kept files/<port folder>/<time>/` (relative paths kept) with a `kept.json` manifest.
//!
//! What is kept (saves are handled by `save_backup`, which runs first):
//! - `.o2r` / `.otr` archives (the HarbourMasters ports make `oot.o2r`, `mm.o2r` from the ROM; mods use them too);
//! - game files connected into the folder (Settings > Game files, `rom_library.json`) and files the port's entry
//!   names as required;
//! - files with a console ROM or disc image extension (`rom_library::system::is_rom_extension`), and cue sheets
//!   (`.cue`, `.gdi`, `.toc`) with the tracks they name;
//! - keys (`*.keys`) and BIOS, boot and flash dumps (a ROM-like name with `bios`, `boot`, `flash`, or starting `scph`:
//!   `dc_boot.bin`, `dc_flash.bin`, `scph1001.bin`);
//! - a generic data file (`bin`, `img`, `zip`, `7z`, `dat`, `swf`, `md`) that none of the above names is kept when it is
//!   over 1 MB, as "maybe game data" (a hand-placed disc image looks like this; the cost is disk space only);
//! - the `.previous/` folder of older versions (`fork::versioned`).
//!
//! A link is never followed and never kept: removing the folder removes the link only, its target stays where it is.
//! A record of connected game files that cannot be read stops the uninstall before anything moves. A move that fails
//! puts back what was already moved (a half-made copy is removed first) and cancels the uninstall: nothing is deleted.
//! The app does not put kept files back on reinstall (follow-up); Settings > Ports > Kept files lists them.

use crate::core::models::Port;
use crate::core::path_safety::is_within;
use crate::fork::page::{Focus, Page, PageButton, PageOption, LEFT};
use crate::fork::rom_library::{system::is_rom_extension, RomLibraryStore, RomSpec};
use crate::fork::versioned::PREVIOUS;
use serde_json::{json, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The folder under Saves Backup that holds kept files.
pub const KEPT_DIR: &str = "Kept files";
pub const MANIFEST: &str = "kept.json";
/// How deep the walk looks inside the port folder.
const MAX_DEPTH: usize = 8;
/// A generic data file over this size is kept anyway.
pub const MAYBE_GAME_DATA_BYTES: u64 = 1024 * 1024;
/// Cue sheets read for the tracks they name; bigger "sheets" are not sheets.
const SHEET_MAX_BYTES: u64 = 1024 * 1024;

pub const UNREADABLE_RECORD: &str = "The game files record could not be read; nothing was deleted.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum KeepKind {
    /// An `.o2r` / `.otr` archive.
    Archive,
    /// A game file connected into the folder or named as required by the port's entry.
    GameFile,
    /// A ROM, disc image or cue sheet track.
    Rom,
    Key,
    /// A BIOS, boot or flash dump.
    Dump,
    /// A generic data file over 1 MB that nothing names.
    MaybeGameData,
    /// The `.previous/` folder of older versions.
    Previous,
}

impl KeepKind {
    pub fn id(self) -> &'static str {
        match self {
            KeepKind::Archive => "archive",
            KeepKind::GameFile => "game_file",
            KeepKind::Rom => "rom",
            KeepKind::Key => "key",
            KeepKind::Dump => "dump",
            KeepKind::MaybeGameData => "maybe game data",
            KeepKind::Previous => "previous",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepItem {
    /// Path relative to the port folder.
    pub rel: PathBuf,
    pub kind: KeepKind,
    pub size: u64,
}

/// What an uninstall keeps, in words. Counts only; `names` for the confirm page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeepSummary {
    pub game_files: usize,
    pub keys_and_dumps: usize,
    pub data_files: usize,
    pub older_versions: usize,
    pub names: Vec<String>,
    /// Bytes of the kept files (the rest of the folder is what is deleted).
    pub bytes: u64,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

impl KeepSummary {
    pub fn of(items: &[KeepItem], dir: &Path) -> KeepSummary {
        let mut s = KeepSummary::default();
        for i in items {
            s.bytes += i.size;
            match i.kind {
                KeepKind::Archive | KeepKind::GameFile | KeepKind::Rom => s.game_files += 1,
                KeepKind::Key | KeepKind::Dump => s.keys_and_dumps += 1,
                KeepKind::MaybeGameData => s.data_files += 1,
                KeepKind::Previous => s.older_versions += versions_in(&dir.join(&i.rel)).max(1),
            }
            if i.kind != KeepKind::Previous {
                s.names.push(i.rel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            }
        }
        s
    }

    pub fn count(&self) -> usize {
        self.game_files + self.keys_and_dumps + self.data_files + self.older_versions
    }

    /// "3 game files and 1 older version"; `None` when nothing is kept.
    pub fn what(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.game_files > 0 {
            parts.push(plural(self.game_files, "game file", "game files"));
        }
        if self.keys_and_dumps > 0 {
            parts.push(plural(self.keys_and_dumps, "key or dump", "keys and dumps"));
        }
        if self.data_files > 0 {
            parts.push(plural(self.data_files, "large data file", "large data files"));
        }
        if self.older_versions > 0 {
            parts.push(plural(self.older_versions, "older version", "older versions"));
        }
        match parts.len() {
            0 => None,
            1 => parts.pop(),
            _ => {
                let last = parts.pop().unwrap_or_default();
                Some(format!("{} and {last}", parts.join(", ")))
            }
        }
    }
}

fn versions_in(previous: &Path) -> usize {
    fs::read_dir(previous).into_iter().flatten().flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).count()
}

/// The Kept files folder: `<Saves Backup>/Kept files`.
pub fn kept_root(saves_backup_dir: &Path) -> PathBuf {
    saves_backup_dir.join(KEPT_DIR)
}

/// Where a port's kept files go: `<Saves Backup>/Kept files/<port folder>`.
pub fn kept_dir_for(saves_backup_dir: &Path, folder: &str) -> PathBuf {
    kept_root(saves_backup_dir).join(folder)
}

fn ext_of(name: &str) -> &str {
    name.rsplit_once('.').map_or("", |(_, e)| e)
}

fn stem_of(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

/// Generic extensions a port also ships as its own data.
const GENERIC: [&str; 7] = ["bin", "zip", "7z", "dat", "img", "md", "swf"];
/// Cue sheets: they name the track files beside them.
const SHEETS: [&str; 3] = ["cue", "gdi", "toc"];

fn is_dump_name(name: &str) -> bool {
    name.contains("bios") || name.contains("boot") || name.contains("flash") || name.starts_with("scph")
}

/// The kind of a file, or `None` for an ordinary program file. `name` is the file name, `named_by_sheet` says a cue
/// sheet beside it names it.
pub fn classify(name: &str, size: u64, named_by_sheet: bool) -> Option<KeepKind> {
    let name = name.to_lowercase();
    let ext = ext_of(&name);
    if ext == "o2r" || ext == "otr" {
        return Some(KeepKind::Archive);
    }
    if ext == "keys" {
        return Some(KeepKind::Key);
    }
    let rom_like = is_rom_extension(ext) || SHEETS.contains(&ext);
    if rom_like && is_dump_name(&name) {
        return Some(KeepKind::Dump);
    }
    if !GENERIC.contains(&ext) {
        return rom_like.then_some(KeepKind::Rom);
    }
    if named_by_sheet {
        return Some(KeepKind::Rom);
    }
    (size > MAYBE_GAME_DATA_BYTES).then_some(KeepKind::MaybeGameData)
}

/// Lowercase names of the files the port's entry says it needs, and the connected copies inside `dir`. An unreadable
/// record of connections is an error: without it nobody can say which files are the user's.
fn known_game_files(port: &Port, dir: &Path, config_dir: Option<&Path>) -> Result<(Vec<String>, Vec<PathBuf>), String> {
    let locals = config_dir.map(|c| crate::fork::rom_library::load_local_entries(&c.join("ports.local.json"))).unwrap_or_default();
    let spec = RomSpec::for_port(port, locals.get(&port.folder));
    let mut names: Vec<String> = spec.requirement.slots.iter().flat_map(|s| s.alternatives.iter().flat_map(|a| a.names.iter().map(|n| n.to_lowercase()))).collect();
    names.extend(crate::fork::rom_library::requirements::generated_archives(&port.folder).iter().map(|n| n.to_string()));
    let connected = match config_dir {
        None => Vec::new(),
        Some(c) => {
            let store = RomLibraryStore::load(&c.join("rom_library.json")).map_err(|_| UNREADABLE_RECORD.to_string())?;
            store.mappings_for(port.key()).into_iter().filter_map(|m| m.dest.clone()).filter(|d| is_within(d, dir)).collect()
        }
    };
    Ok((names, connected))
}

/// The files to keep in the port folder `dir`. Links are skipped (never followed); a folder that is itself a link
/// (an adopted install) keeps nothing, since removing it removes only the link.
pub fn plan(port: &Port, dir: &Path, config_dir: Option<&Path>) -> Result<Vec<KeepItem>, String> {
    if fs::symlink_metadata(dir).map_or(true, |m| !m.is_dir()) {
        return Ok(Vec::new());
    }
    let (names, connected) = known_game_files(port, dir, config_dir)?;
    let connected: Vec<PathBuf> = connected.iter().filter_map(|d| d.strip_prefix(dir).ok().map(Path::to_path_buf)).collect();
    let mut out = Vec::new();
    walk(dir, Path::new(""), MAX_DEPTH, &names, &connected, &mut out);
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

/// What the cue sheets in one folder name: their lowercase text and stems.
struct Sheets {
    texts: Vec<String>,
    stems: Vec<String>,
}

impl Sheets {
    fn read(files: &[(String, PathBuf, u64)]) -> Sheets {
        let mut s = Sheets { texts: Vec::new(), stems: Vec::new() };
        for (name, path, size) in files {
            if SHEETS.contains(&ext_of(name)) && *size <= SHEET_MAX_BYTES {
                if let Ok(bytes) = fs::read(path) {
                    s.texts.push(String::from_utf8_lossy(&bytes).to_lowercase());
                }
                s.stems.push(stem_of(name).to_string());
            }
        }
        s
    }

    /// The sheet's text has the file's name (`FILE "Game (Track 1).bin" BINARY`), or the file shares the sheet's stem.
    fn names(&self, name: &str) -> bool {
        self.texts.iter().any(|t| t.contains(name)) || self.stems.iter().any(|s| s == stem_of(name))
    }
}

fn walk(root: &Path, rel: &Path, depth: usize, names: &[String], connected: &[PathBuf], out: &mut Vec<KeepItem>) {
    let Ok(entries) = fs::read_dir(root.join(rel)) else { return };
    let mut files: Vec<(String, PathBuf, u64)> = Vec::new();
    for e in entries.flatten() {
        let Ok(t) = e.file_type() else { continue };
        let r = rel.join(e.file_name());
        if t.is_symlink() {
            continue;
        }
        if t.is_dir() {
            if rel.as_os_str().is_empty() && e.file_name() == PREVIOUS {
                let size = crate::fork::save_restore::tree_size_capped(&e.path(), u64::MAX / 2, 200_000).0;
                out.push(KeepItem { rel: r, kind: KeepKind::Previous, size });
            } else if depth > 0 {
                walk(root, &r, depth - 1, names, connected, out);
            }
        } else if t.is_file() {
            files.push((e.file_name().to_string_lossy().to_lowercase(), e.path(), e.metadata().map_or(0, |m| m.len())));
        }
    }
    let sheets = Sheets::read(&files);
    for (name, path, size) in &files {
        let r = rel.join(path.file_name().unwrap_or_default());
        let kind = if connected.iter().any(|c| c == &r) || names.contains(name) {
            // An archive keeps its own kind so the manifest says what it is.
            Some(classify(name, *size, false).filter(|k| *k == KeepKind::Archive).unwrap_or(KeepKind::GameFile))
        } else {
            classify(name, *size, sheets.names(name))
        };
        if let Some(kind) = kind {
            out.push(KeepItem { rel: r, kind, size: *size });
        }
    }
}

/// What was kept, for the result message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    /// The folder the files went to (holds `kept.json`).
    pub dir: PathBuf,
    pub items: Vec<KeepItem>,
    pub summary: KeepSummary,
}

impl Kept {
    /// "3 game files and 1 older version were moved to Kept files: Internal storage > ...".
    pub fn line(&self) -> String {
        self.line_at(&crate::fork::port_page::paths::human_path_here(&self.dir))
    }

    pub fn line_at(&self, place: &str) -> String {
        let s = &self.summary;
        let verb = if s.count() == 1 { "was" } else { "were" };
        format!("{} {verb} moved to Kept files: {place}. The app will not put them back when you reinstall; Settings > Ports > Kept files lists them.", s.what().unwrap_or_default())
    }
}

/// Moves one file or folder: a rename, or (another drive) a copy that is checked and then the original removed.
pub fn move_path(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::symlink_metadata(to).is_ok() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", to.display())));
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_then_remove(from, to)
}

/// The move across drives: copy, compare the sizes, then remove the original. A copy that fails or does not match is
/// removed again (it was not there before), and the original is left as it was.
pub fn copy_then_remove(from: &Path, to: &Path) -> io::Result<()> {
    if fs::symlink_metadata(to).is_ok() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", to.display())));
    }
    let checked = copy_tree(from, to).and_then(|()| {
        if tree_bytes(from)? == tree_bytes(to)? { Ok(()) } else { Err(io::Error::other("the copy does not match the original")) }
    });
    if let Err(e) = checked {
        if fs::symlink_metadata(to).is_ok() {
            let _ = remove_copy(to);
        }
        return Err(e);
    }
    remove_copy(from)
}

fn remove_copy(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path)?.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) }
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        #[cfg(unix)]
        return std::os::unix::fs::symlink(fs::read_link(from)?, to);
        #[cfg(not(unix))]
        return Err(io::Error::other("cannot copy a link here"));
    }
    if meta.is_dir() {
        fs::create_dir_all(to)?;
        for e in fs::read_dir(from)? {
            let e = e?;
            copy_tree(&e.path(), &to.join(e.file_name()))?;
        }
        return Ok(());
    }
    fs::copy(from, to).map(|_| ())
}

fn tree_bytes(path: &Path) -> io::Result<u64> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        let mut total = 0;
        for e in fs::read_dir(path)? {
            total += tree_bytes(&e?.path())?;
        }
        Ok(total)
    } else if meta.file_type().is_symlink() {
        Ok(0)
    } else {
        Ok(meta.len())
    }
}

fn unused_stamp_dir(base: &Path, stamp: &str) -> PathBuf {
    let first = base.join(stamp);
    if fs::symlink_metadata(&first).is_err() {
        return first;
    }
    (2..1000).map(|n| base.join(format!("{stamp}-{n}"))).find(|p| fs::symlink_metadata(p).is_err()).unwrap_or_else(|| base.join(format!("{stamp}-x")))
}

/// Moves the files `plan` finds out of `dir` before it is deleted. `Ok(None)`: nothing to keep. An error means the
/// caller must not delete the folder (what had moved is put back).
pub fn keep_before_uninstall(port: &Port, dir: &Path, config_dir: Option<&Path>, saves_backup_dir: &Path) -> Result<Option<Kept>, String> {
    let items = plan(port, dir, config_dir)?;
    if items.is_empty() {
        return Ok(None);
    }
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    keep_items(items, dir, &kept_dir_for(saves_backup_dir, &port.folder), &stamp, &mut move_path)
}

/// The move itself, with the mover passed in (tests make it fail part way).
pub fn keep_items(items: Vec<KeepItem>, dir: &Path, base: &Path, stamp: &str, mover: &mut dyn FnMut(&Path, &Path) -> io::Result<()>) -> Result<Option<Kept>, String> {
    if items.is_empty() {
        return Ok(None);
    }
    let summary = KeepSummary::of(&items, dir);
    let target = unused_stamp_dir(base, stamp);
    let cancelled = |what: String| format!("Couldn't keep the game files before uninstalling ({what}) -- uninstall cancelled, nothing was deleted.");
    fs::create_dir_all(&target).map_err(|e| cancelled(format!("{}: {e}", target.display())))?;
    let mut moved: Vec<&KeepItem> = Vec::new();
    for item in &items {
        if let Err(e) = mover(&dir.join(&item.rel), &target.join(&item.rel)) {
            let mut stuck = Vec::new();
            for done in moved.iter().rev() {
                if move_path(&target.join(&done.rel), &dir.join(&done.rel)).is_err() {
                    stuck.push(done.rel.display().to_string());
                }
            }
            let mut message = cancelled(format!("{}: {e}", item.rel.display()));
            if stuck.is_empty() {
                let _ = remove_empty_dirs(&target);
            } else {
                message.push_str(&format!(" These are safe in {} and were not put back: {}.", target.display(), stuck.join(", ")));
            }
            return Err(message);
        }
        moved.push(item);
    }
    let now = chrono::Local::now().to_rfc3339();
    let manifest = json!({
        "port_folder": dir.to_string_lossy(),
        "kept_at": now,
        "files": items.iter().map(|i| json!({
            "source": dir.join(&i.rel).to_string_lossy(),
            "kept_as": i.rel.to_string_lossy(),
            "kind": i.kind.id(),
            "size": i.size,
            "time": now,
        })).collect::<Vec<_>>(),
    });
    // The files are already safe; a manifest that cannot be written is logged, not a reason to put them back.
    if let Err(e) = fs::write(target.join(MANIFEST), serde_json::to_string_pretty(&manifest).unwrap_or_default()) {
        eprintln!("uninstall keep: couldn't write {} ({e})", target.join(MANIFEST).display());
    }
    Ok(Some(Kept { dir: target, items, summary }))
}

/// Removes the empty folders a cancelled keep left behind (never a file).
fn remove_empty_dirs(dir: &Path) -> io::Result<()> {
    for e in fs::read_dir(dir)?.flatten() {
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            let _ = remove_empty_dirs(&e.path());
        }
    }
    fs::remove_dir(dir)
}

// -- Settings > Ports > Kept files ------------------------------------------------------------------------------

/// One uninstall's kept files: the port's folder name, when, how many files and bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptRecord {
    pub folder: String,
    /// "2026-10-31 10:00".
    pub when: String,
    pub files: usize,
    pub bytes: u64,
}

/// Every uninstall's kept files under `<Saves Backup>/Kept files`, newest first within a port, ports by name. Reads
/// each `kept.json`; a folder without one is counted by its files.
pub fn list_kept(saves_backup_dir: &Path) -> Vec<KeptRecord> {
    let mut out = Vec::new();
    let mut ports: Vec<_> = fs::read_dir(kept_root(saves_backup_dir)).into_iter().flatten().flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).collect();
    ports.sort_by_key(|e| e.file_name().to_string_lossy().to_lowercase());
    for port in ports {
        let folder = port.file_name().to_string_lossy().into_owned();
        let mut stamps: Vec<_> = fs::read_dir(port.path()).into_iter().flatten().flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).collect();
        stamps.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
        for stamp in stamps {
            let name = stamp.file_name().to_string_lossy().into_owned();
            let manifest: Option<Value> = fs::read_to_string(stamp.path().join(MANIFEST)).ok().and_then(|t| serde_json::from_str(&t).ok());
            let (files, bytes) = match manifest.as_ref().and_then(|m| m["files"].as_array()) {
                Some(list) => (list.len(), list.iter().filter_map(|f| f["size"].as_u64()).sum()),
                None => {
                    let (bytes, _) = crate::fork::save_restore::tree_size_capped(&stamp.path(), u64::MAX / 2, 200_000);
                    (fs::read_dir(stamp.path()).into_iter().flatten().count(), bytes)
                }
            };
            out.push(KeptRecord { folder: folder.clone(), when: stamp_words(&name), files, bytes });
        }
    }
    out
}

/// "2026-10-31_10-00-00" (and "-2" copies) as "2026-10-31 10:00".
fn stamp_words(stamp: &str) -> String {
    match stamp.split_once('_') {
        Some((date, time)) if time.len() >= 5 => format!("{date} {}", time[..5].replace('-', ":")),
        _ => stamp.to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeptAct {
    Back,
    OpenFolder,
}

/// The Kept files page: where they are, one line per uninstall. Desktop Mode adds "Open the Kept files folder"; Game
/// Mode only lists (it has no file manager).
pub fn kept_files_page(records: &[KeptRecord], place: &str, game_mode: bool) -> Page<KeptAct> {
    let mut page = Page::<KeptAct> {
        window_title: "Settings: Ports: Kept files".to_string(),
        large: true,
        header: "Settings".to_string(),
        title: "Kept files".to_string(),
        back_hint: "Ports".to_string(),
        ..Page::default()
    };
    page.paragraphs.push(format!("When a port is uninstalled, its game files, keys and older versions are moved here, never deleted: {place}."));
    page.paragraphs.push("The app does not put them back when you reinstall. Copy what you need into the port's folder yourself.".to_string());
    if records.is_empty() {
        page.paragraphs.push("Nothing is kept yet.".to_string());
    }
    for r in records {
        page.facts.push(format!("{}: {}, {} ({})", r.folder, r.when, plural(r.files, "file", "files"), crate::fork::port_page::paths::size_text(r.bytes)));
    }
    page.buttons = [Some(PageButton { label: "Back".to_string(), action: KeptAct::Back }), None, None, None];
    if game_mode {
        page.paragraphs.push("Game Mode has no file manager: open the folder from here in Desktop Mode.".to_string());
        page.initial = Focus::Button(LEFT);
    } else {
        page.options.push(PageOption { title: "Open the Kept files folder".to_string(), detail: place.to_string(), choice: None, enabled: true, recommended: true, action: KeptAct::OpenFolder });
        page.initial = Focus::Option(0);
    }
    page
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::port_from_value;

    fn port(folder: &str) -> Port {
        port_from_value(&json!({"name": "n", "folder": folder, "source": "https://example.com/x.zip"})).unwrap()
    }

    fn write(path: &Path, data: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    }

    fn kinds(items: &[KeepItem]) -> Vec<(String, KeepKind)> {
        items.iter().map(|i| (i.rel.to_string_lossy().replace('\\', "/"), i.kind)).collect()
    }

    #[test]
    fn kinds_by_name() {
        let k = |n: &str| classify(n, 10, false);
        assert_eq!(k("oot.o2r"), Some(KeepKind::Archive));
        assert_eq!(k("soh.OTR"), Some(KeepKind::Archive));
        assert_eq!(k("prod.keys"), Some(KeepKind::Key));
        assert_eq!(k("Zelda (USA).z64"), Some(KeepKind::Rom));
        assert_eq!(k("game.iso"), Some(KeepKind::Rom));
        assert_eq!(k("game.cue"), Some(KeepKind::Rom));
        assert_eq!(k("scph1001.bin"), Some(KeepKind::Dump));
        assert_eq!(k("gba_bios.bin"), Some(KeepKind::Dump));
        assert_eq!(k("dc_boot.bin"), Some(KeepKind::Dump));
        assert_eq!(k("dc_flash.bin"), Some(KeepKind::Dump));
        assert_eq!(classify("data.bin", 2 * 1024 * 1024, false), Some(KeepKind::MaybeGameData));
        assert_eq!(classify("track 02.bin", 10, true), Some(KeepKind::Rom));
        for plain in ["game.exe", "README.md", "config.bin", "assets.zip", "level.dat", "lib.so", "soh.appimage", "save.sav"] {
            assert_eq!(k(plain), None, "{plain}");
        }
    }

    #[test]
    fn summary_words() {
        let s = KeepSummary { game_files: 3, older_versions: 1, ..KeepSummary::default() };
        assert_eq!(s.what().as_deref(), Some("3 game files and 1 older version"));
        let s = KeepSummary { game_files: 1, keys_and_dumps: 2, data_files: 1, older_versions: 2, ..KeepSummary::default() };
        assert_eq!(s.what().as_deref(), Some("1 game file, 2 keys and dumps, 1 large data file and 2 older versions"));
        assert_eq!(KeepSummary::default().what(), None);
    }

    #[test]
    fn plan_finds_archives_connected_copies_previous_and_skips_program_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("Library").join("shipwright");
        let config = tmp.path().join("config");
        write(&dir.join("oot.o2r"), b"arch");
        write(&dir.join("soh.appimage"), b"prog");
        write(&dir.join("roms").join("my copy.n64x"), b"connected");
        write(&dir.join(".previous").join("8.0.5").join("soh.appimage"), b"old");
        let store = json!({"version": 1, "mappings": [{"port_key": port("shipwright").key(), "slot": "s", "method": "copy", "source": "/elsewhere/zelda.z64", "dest": dir.join("roms").join("my copy.n64x").to_string_lossy(), "size": 9}]});
        write(&config.join("rom_library.json"), store.to_string().as_bytes());
        let items = plan(&port("shipwright"), &dir, Some(&config)).unwrap();
        assert_eq!(kinds(&items), [(".previous".to_string(), KeepKind::Previous), ("oot.o2r".into(), KeepKind::Archive), ("roms/my copy.n64x".into(), KeepKind::GameFile)]);
        let s = KeepSummary::of(&items, &dir);
        assert_eq!(s.what().as_deref(), Some("2 game files and 1 older version"));
    }

    #[test]
    fn hand_placed_discs_dumps_and_big_data_are_kept_small_program_data_is_not() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("severedchains");
        let disc = "Legend of Dragoon (USA) (Disc 1)";
        write(&dir.join("isos").join(format!("{disc}.cue")), format!("FILE \"{disc}.bin\" BINARY\n  TRACK 01 MODE2/2352\n").as_bytes());
        write(&dir.join("isos").join(format!("{disc}.bin")), b"small test disc");
        write(&dir.join("sega").join("Game.gdi"), b"3\n1 0 4 2352 track01.bin 0\n2 600 0 2352 track02.raw 0\n");
        write(&dir.join("sega").join("track01.bin"), b"t1");
        write(&dir.join("bios").join("dc_boot.bin"), b"boot");
        write(&dir.join("bios").join("dc_flash.bin"), b"flash");
        write(&dir.join("data.bin"), &vec![0u8; 2 * 1024 * 1024]);
        write(&dir.join("config.bin"), &vec![0u8; 10 * 1024]);
        write(&dir.join("README.md"), b"read me");
        let items = plan(&port("severedchains"), &dir, None).unwrap();
        let got = kinds(&items);
        assert_eq!(
            got,
            [
                ("bios/dc_boot.bin".to_string(), KeepKind::Dump),
                ("bios/dc_flash.bin".into(), KeepKind::Dump),
                ("data.bin".into(), KeepKind::MaybeGameData),
                (format!("isos/{disc}.bin"), KeepKind::Rom),
                (format!("isos/{disc}.cue"), KeepKind::Rom),
                ("sega/Game.gdi".into(), KeepKind::Rom),
                ("sega/track01.bin".into(), KeepKind::Rom),
            ]
        );
    }

    #[test]
    fn an_unreadable_record_of_connected_files_stops_the_plan() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("game");
        let config = tmp.path().join("config");
        write(&dir.join("game.x86_64"), b"prog");
        write(&config.join("rom_library.json"), b"{ not json");
        assert_eq!(plan(&port("game"), &dir, Some(&config)).unwrap_err(), UNREADABLE_RECORD);
        // A missing record is an empty one.
        assert!(plan(&port("game"), &dir, Some(&tmp.path().join("nowhere"))).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_game_file_is_not_kept_and_its_target_is_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("game");
        let rom = tmp.path().join("ROMs").join("zelda.z64");
        write(&rom, b"rom");
        fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(&rom, dir.join("zelda.z64")).unwrap();
        std::os::unix::fs::symlink(rom.parent().unwrap(), dir.join("romdir")).unwrap();
        assert!(plan(&port("game"), &dir, None).unwrap().is_empty());
    }

    #[test]
    fn a_failed_move_puts_back_what_moved_and_reports() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("game");
        write(&dir.join("a.o2r"), b"a");
        write(&dir.join("b.o2r"), b"b");
        let items = plan(&port("game"), &dir, None).unwrap();
        assert_eq!(items.len(), 2);
        let base = tmp.path().join("Kept files").join("game");
        let mut calls = 0;
        let mut mover = |from: &Path, to: &Path| {
            calls += 1;
            if calls == 2 { Err(io::Error::other("disk full")) } else { move_path(from, to) }
        };
        let err = keep_items(items, &dir, &base, "2026-10-31_10-00-00", &mut mover).unwrap_err();
        assert!(err.contains("nothing was deleted"), "{err}");
        assert_eq!(fs::read(dir.join("a.o2r")).unwrap(), b"a");
        assert_eq!(fs::read(dir.join("b.o2r")).unwrap(), b"b");
        assert!(!base.join("2026-10-31_10-00-00").exists(), "the empty kept folder is tidied away");
    }

    #[cfg(unix)]
    #[test]
    fn a_copy_across_drives_that_fails_half_way_is_removed_and_the_original_stays() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("game").join(".previous");
        write(&from.join("1.0").join("a.bin"), b"first");
        write(&from.join("1.0").join("z.bin"), b"unreadable");
        fs::set_permissions(from.join("1.0").join("z.bin"), fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(from.join("1.0").join("z.bin")).is_ok() {
            return; // running as root: permissions do not stop a read
        }
        let to = tmp.path().join("Kept").join(".previous");
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        assert!(copy_then_remove(&from, &to).is_err());
        assert!(!to.exists(), "the half-made copy is removed");
        assert_eq!(fs::read(from.join("1.0").join("a.bin")).unwrap(), b"first");
        fs::set_permissions(from.join("1.0").join("z.bin"), fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(fs::read(from.join("1.0").join("z.bin")).unwrap(), b"unreadable");
    }

    #[test]
    fn a_copy_across_drives_that_works_removes_the_original() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("a").join("disc.iso");
        write(&from, b"iso");
        let to = tmp.path().join("b").join("disc.iso");
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        copy_then_remove(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(fs::read(&to).unwrap(), b"iso");
    }

    #[test]
    fn keep_writes_a_manifest_and_a_second_run_gets_its_own_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("Kept files").join("game");
        for round in 0..2 {
            let dir = tmp.path().join("game");
            write(&dir.join("disc").join("game.iso"), format!("iso{round}").as_bytes());
            let items = plan(&port("game"), &dir, None).unwrap();
            let kept = keep_items(items, &dir, &base, "2026-10-31_10-00-00", &mut move_path).unwrap().unwrap();
            assert!(!dir.join("disc").join("game.iso").exists());
            assert_eq!(fs::read(kept.dir.join("disc").join("game.iso")).unwrap(), format!("iso{round}").as_bytes());
            let manifest: Value = serde_json::from_str(&fs::read_to_string(kept.dir.join(MANIFEST)).unwrap()).unwrap();
            assert_eq!(manifest["files"][0]["kind"], "rom");
            assert_eq!(manifest["files"][0]["size"], 4);
            assert!(manifest["files"][0]["source"].as_str().unwrap().ends_with("game.iso"));
            assert!(kept.line_at("Internal storage > Saves Backup > Kept files > game").starts_with("1 game file was moved to Kept files: Internal storage > "), "{}", kept.line());
            assert!(kept.line().contains("will not put them back"));
        }
        assert!(base.join("2026-10-31_10-00-00-2").join(MANIFEST).exists());
        let records = list_kept(tmp.path());
        assert_eq!(records.len(), 2);
        assert_eq!((records[0].folder.as_str(), records[0].when.as_str(), records[0].files, records[0].bytes), ("game", "2026-10-31 10:00", 1, 4));
    }

    #[test]
    fn move_path_refuses_to_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("a"), b"new");
        write(&tmp.path().join("b"), b"old");
        assert!(move_path(&tmp.path().join("a"), &tmp.path().join("b")).is_err());
        assert_eq!(fs::read(tmp.path().join("b")).unwrap(), b"old");
        assert_eq!(fs::read(tmp.path().join("a")).unwrap(), b"new");
    }

    #[test]
    fn the_kept_files_page_lists_ports_and_dates_and_opens_the_folder_only_in_desktop_mode() {
        let records = vec![KeptRecord { folder: "shipwright".into(), when: "2026-10-31 10:00".into(), files: 3, bytes: 120_000_000 }];
        let desk = kept_files_page(&records, "Internal storage > Saves Backup > Kept files", false);
        assert_eq!(desk.facts, ["shipwright: 2026-10-31 10:00, 3 files (120 MB)"]);
        assert_eq!(desk.options[0].action, KeptAct::OpenFolder);
        assert!(desk.paragraphs.iter().any(|p| p.contains("does not put them back")));
        let game = kept_files_page(&records, "x", true);
        assert!(game.options.is_empty());
        assert_eq!(game.facts.len(), 1);
        assert!(kept_files_page(&[], "x", false).paragraphs.iter().any(|p| p == "Nothing is kept yet."));
    }

    #[test]
    fn no_code_shows_upstreams_all_of_its_files_will_be_deleted_text() {
        // Every uninstall entry goes to the Port page's Uninstall page; upstream's confirm text is no longer true.
        let needle = ["invoke_message_", "uninstall_confirm("].concat();
        fn walk(dir: &Path, needle: &str, hits: &mut Vec<String>) {
            for e in fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, needle, hits);
                } else if p.extension().is_some_and(|x| x == "rs") && fs::read_to_string(&p).unwrap_or_default().contains(needle) {
                    hits.push(p.display().to_string());
                }
            }
        }
        let mut hits = Vec::new();
        walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &needle, &mut hits);
        assert!(hits.is_empty(), "the old uninstall text is still shown from {hits:?}");
        let dialogs = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app/dialogs.rs")).unwrap();
        let body = dialogs.split("fn open_uninstall_confirm_dialog").nth(1).unwrap().split("\n}\n").next().unwrap();
        assert!(body.contains("open_card") && body.contains("Card::Uninstall"), "the old entry opens the Port page's Uninstall page");
    }

    // -- Through the uninstall itself (`installer::uninstall_port_keeping`) --

    fn only_child(dir: &Path) -> PathBuf {
        let mut all: Vec<PathBuf> = fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
        assert_eq!(all.len(), 1, "{all:?}");
        all.pop().unwrap()
    }

    #[test]
    fn uninstall_keeps_archives_connected_copies_and_previous_and_deletes_program_files() {
        use crate::core::installer::uninstall_port_keeping;
        let tmp = tempfile::tempdir().unwrap();
        let library = tmp.path().join("Library");
        let saves = tmp.path().join("Saves Backup");
        let config = tmp.path().join("config");
        let dir = library.join("shipwright");
        write(&dir.join("oot.o2r"), b"arch");
        write(&dir.join("soh.appimage"), b"prog");
        write(&dir.join("lib").join("libfoo.so"), b"lib");
        write(&dir.join("copied").join("my zelda.n64x"), b"copied rom");
        write(&dir.join(".previous").join("8.0.5").join("soh.appimage"), b"old");
        let p = port("shipwright");
        let store = json!({"version": 1, "mappings": [{"port_key": p.key(), "slot": "s", "method": "copy", "source": "/elsewhere/zelda.z64", "dest": dir.join("copied").join("my zelda.n64x").to_string_lossy(), "size": 10}]});
        write(&config.join("rom_library.json"), store.to_string().as_bytes());

        let kept = uninstall_port_keeping(&p, &library, &saves, Some(&config)).unwrap().unwrap();

        assert!(!dir.exists(), "the port folder is gone");
        let stamp = only_child(&kept_dir_for(&saves, "shipwright"));
        assert_eq!(stamp, kept.dir);
        assert_eq!(fs::read(stamp.join("oot.o2r")).unwrap(), b"arch");
        assert_eq!(fs::read(stamp.join("copied").join("my zelda.n64x")).unwrap(), b"copied rom");
        assert_eq!(fs::read(stamp.join(".previous").join("8.0.5").join("soh.appimage")).unwrap(), b"old");
        assert!(!stamp.join("soh.appimage").exists() && !stamp.join("lib").exists(), "program files are not kept");
        let manifest: Value = serde_json::from_str(&fs::read_to_string(stamp.join(MANIFEST)).unwrap()).unwrap();
        let kinds: Vec<&str> = manifest["files"].as_array().unwrap().iter().map(|f| f["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["previous", "game_file", "archive"]);
        assert_eq!(kept.summary.what().as_deref(), Some("2 game files and 1 older version"));
        assert!(kept.line().contains("were moved to Kept files"), "{}", kept.line());
    }

    #[cfg(unix)]
    #[test]
    fn uninstall_removes_a_linked_game_file_as_a_link_and_leaves_its_target() {
        use crate::core::installer::uninstall_port_keeping;
        let tmp = tempfile::tempdir().unwrap();
        let library = tmp.path().join("Library");
        let saves = tmp.path().join("Saves Backup");
        let roms = tmp.path().join("ROMs");
        write(&roms.join("zelda.z64"), b"the rom");
        let dir = library.join("game");
        write(&dir.join("game.x86_64"), b"prog");
        std::os::unix::fs::symlink(roms.join("zelda.z64"), dir.join("zelda.z64")).unwrap();
        std::os::unix::fs::symlink(&roms, dir.join("roms")).unwrap();

        let kept = uninstall_port_keeping(&port("game"), &library, &saves, None).unwrap();

        assert!(kept.is_none(), "a link is not a file to keep");
        assert!(!dir.exists());
        assert_eq!(fs::read(roms.join("zelda.z64")).unwrap(), b"the rom");
        assert_eq!(fs::read_dir(&roms).unwrap().count(), 1);
    }

    #[test]
    fn uninstall_with_nowhere_to_keep_is_cancelled_and_deletes_nothing() {
        use crate::core::installer::uninstall_port_keeping;
        let tmp = tempfile::tempdir().unwrap();
        let library = tmp.path().join("Library");
        let saves = tmp.path().join("Saves Backup");
        fs::create_dir_all(&saves).unwrap();
        fs::write(saves.join(KEPT_DIR), b"a file where the Kept files folder would go").unwrap();
        let dir = library.join("2ship2harkinian");
        write(&dir.join("mm.o2r"), b"arch");
        write(&dir.join("prod.keys"), b"keys");
        write(&dir.join("2s2h.appimage"), b"prog");

        let err = uninstall_port_keeping(&port("2ship2harkinian"), &library, &saves, None).unwrap_err();

        assert!(err.contains("nothing was deleted"), "{err}");
        assert_eq!(fs::read(dir.join("mm.o2r")).unwrap(), b"arch");
        assert_eq!(fs::read(dir.join("prod.keys")).unwrap(), b"keys");
        assert_eq!(fs::read(dir.join("2s2h.appimage")).unwrap(), b"prog");
    }

    #[test]
    fn uninstall_with_an_unreadable_record_is_cancelled_and_deletes_nothing() {
        use crate::core::installer::uninstall_port_keeping;
        let tmp = tempfile::tempdir().unwrap();
        let library = tmp.path().join("Library");
        let config = tmp.path().join("config");
        write(&config.join("rom_library.json"), b"{ damaged");
        let dir = library.join("game");
        write(&dir.join("game.x86_64"), b"prog");
        write(&dir.join("my game.v64x"), b"copied by hand, connected in the record we cannot read");

        let err = uninstall_port_keeping(&port("game"), &library, &tmp.path().join("Saves Backup"), Some(&config)).unwrap_err();

        assert_eq!(err, UNREADABLE_RECORD);
        assert!(dir.join("game.x86_64").exists() && dir.join("my game.v64x").exists());
    }
}
