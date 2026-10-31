//! Uninstall never deletes game files: before a port's folder is removed, the files the hard rule protects are
//! moved out to `<Saves Backup>/Kept files/<port folder>/<time>/` (relative paths kept) with a `kept.json` manifest.
//!
//! What is kept (saves are handled by `save_backup`, which runs first):
//! - `.o2r` / `.otr` archives (the HarbourMasters ports make `oot.o2r`, `mm.o2r` from the ROM; mods use them too);
//! - game files connected into the folder (Settings > Game files, `rom_library.json`) and files the port's entry
//!   names as required;
//! - files with a console ROM or disc image extension (`rom_library::system::is_rom_extension`), except the generic
//!   ones a port ships as program data (`bin`, `zip`, `7z`, `dat`, `img`, `md`, `swf`);
//! - keys (`*.keys`, `prod.keys`, `title.keys`) and BIOS or boot dumps (a ROM-like file whose name says `bios`,
//!   `scph`, `boot.bin`, `boot.rom`);
//! - the `.previous/` folder of older versions (`fork::versioned`).
//!
//! A link is never followed and never kept: removing the folder removes the link only, its target stays where it is.
//! A move that fails puts back what was already moved and cancels the uninstall: nothing is deleted.
//! Follow-up (not built): reinstalling could offer the kept files back.

use crate::core::models::Port;
use crate::core::path_safety::is_within;
use crate::fork::rom_library::{system::is_rom_extension, RomLibraryStore, RomSpec};
use crate::fork::versioned::PREVIOUS;
use serde_json::json;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The folder under Saves Backup that holds kept files.
pub const KEPT_DIR: &str = "Kept files";
pub const MANIFEST: &str = "kept.json";
/// How deep the walk looks inside the port folder.
const MAX_DEPTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum KeepKind {
    /// An `.o2r` / `.otr` archive.
    Archive,
    /// A game file connected into the folder or named as required by the port's entry.
    GameFile,
    /// A ROM or disc image, by extension.
    Rom,
    Key,
    /// A BIOS or boot dump.
    Dump,
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
    pub older_versions: usize,
    pub names: Vec<String>,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

impl KeepSummary {
    pub fn of(items: &[KeepItem], dir: &Path) -> KeepSummary {
        let mut s = KeepSummary::default();
        for i in items {
            match i.kind {
                KeepKind::Archive | KeepKind::GameFile | KeepKind::Rom => s.game_files += 1,
                KeepKind::Key | KeepKind::Dump => s.keys_and_dumps += 1,
                KeepKind::Previous => s.older_versions += versions_in(&dir.join(&i.rel)).max(1),
            }
            if i.kind != KeepKind::Previous {
                s.names.push(i.rel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            }
        }
        s
    }

    pub fn is_empty(&self) -> bool {
        self.game_files == 0 && self.keys_and_dumps == 0 && self.older_versions == 0
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

/// Where a port's kept files go: `<Saves Backup>/Kept files/<port folder>`.
pub fn kept_dir_for(saves_backup_dir: &Path, folder: &str) -> PathBuf {
    saves_backup_dir.join(KEPT_DIR).join(folder)
}

fn lower_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default()
}

fn ext_of(name: &str) -> &str {
    name.rsplit_once('.').map_or("", |(_, e)| e)
}

/// Generic extensions a port ships as its own data: kept only when named as a dump or a required file.
const GENERIC: [&str; 7] = ["bin", "zip", "7z", "dat", "img", "md", "swf"];

/// The kind of a file by its name alone, or `None` for an ordinary program file.
pub fn kind_by_name(name: &str) -> Option<KeepKind> {
    let name = name.to_lowercase();
    let ext = ext_of(&name);
    if ext == "o2r" || ext == "otr" {
        return Some(KeepKind::Archive);
    }
    if ext == "keys" || matches!(name.as_str(), "prod.keys" | "title.keys" | "dev.keys") {
        return Some(KeepKind::Key);
    }
    let rom_like = is_rom_extension(ext);
    if rom_like && (name.contains("bios") || name.contains("scph") || name == "boot.bin" || name == "boot.rom") {
        return Some(KeepKind::Dump);
    }
    if rom_like && !GENERIC.contains(&ext) {
        return Some(KeepKind::Rom);
    }
    None
}

/// Lowercase names of the files the port's entry says it needs, and the connected copies inside `dir`.
fn known_game_files(port: &Port, dir: &Path, config_dir: Option<&Path>) -> (Vec<String>, Vec<PathBuf>) {
    let locals = config_dir.map(|c| crate::fork::rom_library::load_local_entries(&c.join("ports.local.json"))).unwrap_or_default();
    let spec = RomSpec::for_port(port, locals.get(&port.folder));
    let mut names: Vec<String> = spec.requirement.slots.iter().flat_map(|s| s.alternatives.iter().flat_map(|a| a.names.iter().map(|n| n.to_lowercase()))).collect();
    names.extend(crate::fork::rom_library::requirements::generated_archives(&port.folder).iter().map(|n| n.to_string()));
    let connected = config_dir
        .and_then(|c| RomLibraryStore::load(&c.join("rom_library.json")).ok())
        .map(|s| s.mappings_for(port.key()).into_iter().filter_map(|m| m.dest.clone()).filter(|d| is_within(d, dir)).collect())
        .unwrap_or_default();
    (names, connected)
}

/// The files to keep in the port folder `dir`. Links are skipped (never followed); a folder that is itself a link
/// (an adopted install) keeps nothing, since removing it removes only the link.
pub fn plan(port: &Port, dir: &Path, config_dir: Option<&Path>) -> Vec<KeepItem> {
    if fs::symlink_metadata(dir).map_or(true, |m| !m.is_dir()) {
        return Vec::new();
    }
    let (names, connected) = known_game_files(port, dir, config_dir);
    let connected: Vec<PathBuf> = connected.iter().filter_map(|d| d.strip_prefix(dir).ok().map(Path::to_path_buf)).collect();
    let mut out = Vec::new();
    walk(dir, Path::new(""), MAX_DEPTH, &names, &connected, &mut out);
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

fn walk(root: &Path, rel: &Path, depth: usize, names: &[String], connected: &[PathBuf], out: &mut Vec<KeepItem>) {
    let Ok(entries) = fs::read_dir(root.join(rel)) else { return };
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
            continue;
        }
        if !t.is_file() {
            continue;
        }
        let name = lower_name(&r);
        let kind = if connected.iter().any(|c| c == &r) || names.contains(&name) {
            // An archive keeps its own kind so the manifest says what it is.
            Some(kind_by_name(&name).filter(|k| *k == KeepKind::Archive).unwrap_or(KeepKind::GameFile))
        } else {
            kind_by_name(&name)
        };
        if let Some(kind) = kind {
            let size = e.metadata().map_or(0, |m| m.len());
            out.push(KeepItem { rel: r, kind, size });
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
    /// "3 game files and 1 older version were moved to Kept files: <dir>."
    pub fn line(&self) -> String {
        let s = &self.summary;
        let one = s.game_files + s.keys_and_dumps + s.older_versions == 1;
        format!("{} {} moved to Kept files: {}. Nothing in it is deleted.", s.what().unwrap_or_default(), if one { "was" } else { "were" }, self.dir.display())
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
    copy_tree(from, to)?;
    if tree_bytes(from)? != tree_bytes(to)? {
        let _ = remove_copy(to);
        return Err(io::Error::other("the copy does not match the original"));
    }
    if fs::symlink_metadata(from)?.is_dir() { fs::remove_dir_all(from) } else { fs::remove_file(from) }
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
    let items = plan(port, dir, config_dir);
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

    #[test]
    fn kinds_by_name() {
        assert_eq!(kind_by_name("oot.o2r"), Some(KeepKind::Archive));
        assert_eq!(kind_by_name("soh.OTR"), Some(KeepKind::Archive));
        assert_eq!(kind_by_name("prod.keys"), Some(KeepKind::Key));
        assert_eq!(kind_by_name("Zelda (USA).z64"), Some(KeepKind::Rom));
        assert_eq!(kind_by_name("game.iso"), Some(KeepKind::Rom));
        assert_eq!(kind_by_name("scph1001.bin"), Some(KeepKind::Dump));
        assert_eq!(kind_by_name("gba_bios.bin"), Some(KeepKind::Dump));
        for plain in ["game.exe", "README.md", "data.bin", "assets.zip", "level.dat", "lib.so", "soh.appimage", "save.sav"] {
            assert_eq!(kind_by_name(plain), None, "{plain}");
        }
    }

    #[test]
    fn summary_words() {
        let s = KeepSummary { game_files: 3, keys_and_dumps: 0, older_versions: 1, names: vec![] };
        assert_eq!(s.what().as_deref(), Some("3 game files and 1 older version"));
        let s = KeepSummary { game_files: 1, keys_and_dumps: 2, older_versions: 2, names: vec![] };
        assert_eq!(s.what().as_deref(), Some("1 game file, 2 keys and dumps and 2 older versions"));
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
        let items = plan(&port("shipwright"), &dir, Some(&config));
        let got: Vec<(String, KeepKind)> = items.iter().map(|i| (i.rel.to_string_lossy().replace('\\', "/"), i.kind)).collect();
        assert_eq!(got, [(".previous".to_string(), KeepKind::Previous), ("oot.o2r".into(), KeepKind::Archive), ("roms/my copy.n64x".into(), KeepKind::GameFile)]);
        let s = KeepSummary::of(&items, &dir);
        assert_eq!(s.what().as_deref(), Some("2 game files and 1 older version"));
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
        assert!(plan(&port("game"), &dir, None).is_empty());
    }

    #[test]
    fn a_failed_move_puts_back_what_moved_and_reports() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("game");
        write(&dir.join("a.o2r"), b"a");
        write(&dir.join("b.o2r"), b"b");
        let items = plan(&port("game"), &dir, None);
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

    #[test]
    fn keep_writes_a_manifest_and_a_second_run_gets_its_own_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("Kept files").join("game");
        for round in 0..2 {
            let dir = tmp.path().join("game");
            write(&dir.join("disc").join("game.iso"), format!("iso{round}").as_bytes());
            let items = plan(&port("game"), &dir, None);
            let kept = keep_items(items, &dir, &base, "2026-10-31_10-00-00", &mut move_path).unwrap().unwrap();
            assert!(!dir.join("disc").join("game.iso").exists());
            assert_eq!(fs::read(kept.dir.join("disc").join("game.iso")).unwrap(), format!("iso{round}").as_bytes());
            let manifest: serde_json::Value = serde_json::from_str(&fs::read_to_string(kept.dir.join(MANIFEST)).unwrap()).unwrap();
            assert_eq!(manifest["files"][0]["kind"], "rom");
            assert_eq!(manifest["files"][0]["size"], 4);
            assert!(manifest["files"][0]["source"].as_str().unwrap().ends_with("game.iso"));
            assert!(kept.line().starts_with("1 game file was moved to Kept files"), "{}", kept.line());
        }
        assert!(base.join("2026-10-31_10-00-00-2").join(MANIFEST).exists());
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
        let manifest: serde_json::Value = serde_json::from_str(&fs::read_to_string(stamp.join(MANIFEST)).unwrap()).unwrap();
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
}
