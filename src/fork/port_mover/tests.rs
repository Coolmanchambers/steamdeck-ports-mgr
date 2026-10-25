//! Fixture tests for the move engine: temp folders, a fake Steam folder, fake process checks.
//! Nothing here touches the real Steam folder, saves or ROMs.

use super::copy::{self, Policy};
use super::game_files::{self, GameFiles, RequiredFile};
use super::old_copies::{self, RemovalCtx};
use super::*;
use crate::fork::shortcuts;
use crate::fork::steam_queue::{self, ApplyInputs, QueuedKind};
use crate::fork::store::ShortcutsFile;
use crate::fork::vdf::fixtures::*;
use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

fn temp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("dpm_port_mover_{}_{name}", std::process::id()));
    if p.exists() {
        let _ = copy::remove_copy(&p);
    }
    fs::create_dir_all(&p).unwrap();
    p
}

fn no_steam() -> io::Result<Vec<String>> {
    Ok(Vec::new())
}

fn steam_up() -> io::Result<Vec<String>> {
    Ok(vec!["77 steam".into()])
}

fn nothing_running(_: &Path) -> Vec<String> {
    Vec::new()
}

fn plenty(_: &Path) -> Option<u64> {
    Some(u64::MAX / 4)
}

/// The fixture port keeps its saves in `saves/`.
fn saves_folder(_: &old_copies::OldCopy, dir: &Path) -> Option<Vec<PathBuf>> {
    Some(vec![dir.join("saves")])
}

fn put(path: &Path, data: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}

#[cfg(unix)]
fn chmod(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(not(unix))]
fn chmod(_path: &Path, _mode: u32) {}

/// Every entry under `dir` with its kind, size, bits, link target and content hash.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
        let mut entries: Vec<_> = fs::read_dir(dir).unwrap().flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let path = e.path();
            let rel = path.strip_prefix(root).unwrap().to_path_buf();
            let meta = fs::symlink_metadata(&path).unwrap();
            let desc = if meta.file_type().is_symlink() {
                format!("link {}", fs::read_link(&path).unwrap().display())
            } else if meta.is_dir() {
                walk(root, &path, out);
                format!("dir {:o}", copy::mode_of(&meta))
            } else if meta.is_file() {
                format!("file {} {:o} {:?} {:?}", meta.len(), copy::mode_of(&meta), copy::sha1_of(&path).unwrap(), meta.modified().unwrap())
            } else {
                "special".to_string()
            };
            out.insert(rel, desc);
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// A fake world: home with a Steam account, an app folder, an old ports folder with one port, a new
/// ports folder, and a ROM folder outside both.
struct World {
    root: PathBuf,
    home: PathBuf,
    app: PathBuf,
    old_lib: PathBuf,
    new_lib: PathBuf,
    roms: PathBuf,
    vdf: PathBuf,
    settings: ForkSettings,
}

impl World {
    fn new(name: &str) -> World {
        let root = temp(name);
        let home = root.join("home");
        let app = home.join("Applications").join("dpm");
        let old_lib = app.join("Library");
        let new_lib = root.join("sd").join("Ports");
        let roms = root.join("sd").join("roms");
        fs::create_dir_all(&new_lib).unwrap();
        let user = home.join(".local/share/Steam/userdata/1234/config");
        fs::create_dir_all(&user).unwrap();
        let vdf = user.join("shortcuts.vdf");
        let port = old_lib.join("SoH");
        put(&port.join("soh.appimage"), b"#!/bin/sh\necho soh\n");
        chmod(&port.join("soh.appimage"), 0o755);
        put(&port.join("readme.txt"), b"read me");
        chmod(&port.join("readme.txt"), 0o444);
        put(&port.join("saves/slot1.sav"), &vec![7u8; 70_000]);
        fs::create_dir_all(port.join("empty")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("soh.appimage", port.join("soh")).unwrap();
        put(&roms.join("n64/Zelda.z64"), &vec![3u8; 50_000]);
        let soh_exe = port.join("soh.appimage");
        fs::write(
            &vdf,
            file_of(&[
                steam_entry(0, 0xAAAA_0001, "Ship of Harkinian", &format!("\"{}\"", soh_exe.display()), &format!("\"{}\"", port.display()), &[]),
                steam_entry(1, 0xAAAA_0002, "Chiaki", "\"/usr/bin/chiaki\"", "\"/usr/bin/\"", &[]),
            ]),
        )
        .unwrap();
        let settings = ForkSettings::load(&app.join("fork-settings.json"));
        World { root, home, app, old_lib, new_lib, roms, vdf, settings }
    }

    fn port(&self) -> PortRef {
        PortRef { key: "harbour/soh".into(), name: "Ship of Harkinian".into() }
    }

    fn env<'a>(&'a self, steam: &'a dyn SteamProcessCheck, running: &'a dyn Fn(&Path) -> Vec<String>, free: &'a dyn Fn(&Path) -> Option<u64>) -> MoveEnv<'a> {
        MoveEnv { home: &self.home, config_dir: &self.app, settings: &self.settings, steam, game_mode: false, running, free_space: free }
    }

    fn prepare_with(&self, options: &MoveOptions) -> Result<Prepared, MoveError> {
        let env = self.env(&no_steam, &nothing_running, &plenty);
        prepare(&self.port(), &self.old_lib.join("SoH"), &self.new_lib.join("SoH"), &env, options, &mut |_, _, _| {}, &AtomicBool::new(false))
    }

    fn shortcuts(&self) -> Vec<shortcuts::Shortcut> {
        shortcuts::list(&ShortcutsFile::load(&self.vdf).unwrap().doc)
    }

    fn staging_left(&self) -> bool {
        fs::read_dir(&self.new_lib).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains("dpm-moving"))
    }

    fn removal_ctx<'a>(&'a self, active: &'a [PathBuf]) -> RemovalCtx<'a> {
        RemovalCtx { home: &self.home, config_dir: &self.app, library_dir: &self.new_lib, active_paths: active, running: &nothing_running, save_folders: &saves_folder }
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = copy::remove_copy(&self.root);
    }
}

// -- copy and verify ---------------------------------------------------------------------------

#[test]
fn a_move_copies_bits_links_and_times_verifies_and_keeps_the_original() {
    let w = World::new("happy");
    let old = w.old_lib.join("SoH");
    let before = snapshot(&old);
    let mut seen = 0u64;
    let env = w.env(&no_steam, &nothing_running, &plenty);
    let p = prepare(&w.port(), &old, &w.new_lib.join("SoH"), &env, &MoveOptions::default(), &mut |done, total, _| { assert!(done <= total); seen = done; }, &AtomicBool::new(false)).unwrap();
    assert_eq!(snapshot(&old), before, "the original is untouched");
    assert_eq!(snapshot(&p.to), before, "the copy matches entry for entry (bits, sizes, hashes, times, links)");
    assert_eq!(p.files, 3);
    assert_eq!(p.hashed, 3, "small ports are hashed whole");
    assert_eq!(seen, p.bytes);
    assert!(!w.staging_left());
    assert!(old.is_dir());
}

#[test]
fn above_the_limit_only_programs_and_game_files_are_read_back() {
    let w = World::new("limit");
    let options = MoveOptions { policy: Some(Policy { full_hash_limit: 10 }), ..Default::default() };
    let p = w.prepare_with(&options).unwrap();
    assert_eq!(p.hashed, if cfg!(unix) { 1 } else { 0 }, "only the program (execute bit)");
}

#[test]
fn a_copy_that_differs_is_caught_and_removed() {
    let w = World::new("corrupt");
    let corrupt = |staging: &Path| {
        let f = staging.join("saves/slot1.sav");
        let mut data = fs::read(&f).unwrap();
        data[100] ^= 0xFF;
        let t = fs::metadata(&f).unwrap().modified().unwrap();
        fs::write(&f, data).unwrap();
        fs::File::options().write(true).open(&f).unwrap().set_modified(t).unwrap();
    };
    let options = MoveOptions { after_copy: Some(&corrupt), ..Default::default() };
    let err = w.prepare_with(&options).unwrap_err();
    assert!(matches!(&err, MoveError::Verify(e) if e.contains("slot1.sav")), "{err}");
    assert!(!w.new_lib.join("SoH").exists() && !w.staging_left());
}

#[cfg(unix)]
#[test]
fn a_failed_copy_leaves_everything_as_it_was_and_removes_only_the_partial_copy() {
    let w = World::new("partial");
    let old = w.old_lib.join("SoH");
    // A named pipe cannot be copied: the copy stops part way (after the regular files, sorted before it).
    assert!(std::process::Command::new("mkfifo").arg(old.join("zz.sock")).status().unwrap().success());
    let before = snapshot(&old);
    let other = w.new_lib.join("Other");
    put(&other.join("keep.txt"), b"keep");
    let err = w.prepare_with(&MoveOptions::default()).unwrap_err();
    assert!(matches!(&err, MoveError::Copy(e) if e.contains("zz.sock")), "{err}");
    assert_eq!(snapshot(&old), before);
    assert!(!w.new_lib.join("SoH").exists() && !w.staging_left());
    assert_eq!(fs::read(other.join("keep.txt")).unwrap(), b"keep", "nothing else in the new folder was touched");
    assert!(w.shortcuts()[0].exe_path().starts_with(old.to_str().unwrap()), "Steam is untouched");
}

#[test]
fn cancel_stops_and_removes_the_partial_copy() {
    let w = World::new("cancel");
    let cancel = AtomicBool::new(false);
    let env = w.env(&no_steam, &nothing_running, &plenty);
    let err = prepare(&w.port(), &w.old_lib.join("SoH"), &w.new_lib.join("SoH"), &env, &MoveOptions::default(), &mut |_, _, _| cancel.store(true, Ordering::Relaxed), &cancel).unwrap_err();
    assert!(matches!(err, MoveError::Cancelled));
    assert!(!w.new_lib.join("SoH").exists() && !w.staging_left());
}

#[test]
fn a_running_game_target_in_use_or_lack_of_space_refuse_before_copying() {
    let w = World::new("refusals");
    let running = |_: &Path| vec!["soh.appimage (process 42)".to_string()];
    let env = w.env(&no_steam, &running, &plenty);
    let err = prepare(&w.port(), &w.old_lib.join("SoH"), &w.new_lib.join("SoH"), &env, &MoveOptions::default(), &mut |_, _, _| {}, &AtomicBool::new(false)).unwrap_err();
    assert!(matches!(&err, MoveError::Running(n) if n[0].contains("process 42")), "{err}");

    let small = |_: &Path| Some(1000u64);
    let env = w.env(&no_steam, &nothing_running, &small);
    let err = prepare(&w.port(), &w.old_lib.join("SoH"), &w.new_lib.join("SoH"), &env, &MoveOptions::default(), &mut |_, _, _| {}, &AtomicBool::new(false)).unwrap_err();
    assert!(matches!(err, MoveError::NotEnoughSpace { free: 1000, .. }));

    fs::create_dir_all(w.new_lib.join("SoH")).unwrap();
    assert!(matches!(w.prepare_with(&MoveOptions::default()).unwrap_err(), MoveError::TargetExists(_)));
    fs::remove_dir(w.new_lib.join("SoH")).unwrap();

    let env = w.env(&no_steam, &nothing_running, &plenty);
    let inside = w.old_lib.join("SoH").join("saves").join("SoH");
    let err = prepare(&w.port(), &w.old_lib.join("SoH"), &inside, &env, &MoveOptions::default(), &mut |_, _, _| {}, &AtomicBool::new(false)).unwrap_err();
    assert!(matches!(err, MoveError::Nested));
    assert!(!w.new_lib.join("SoH").exists() && !w.staging_left());
}

#[test]
fn discard_removes_only_the_new_copy() {
    let w = World::new("discard");
    let p = w.prepare_with(&MoveOptions::default()).unwrap();
    discard(&p).unwrap();
    assert!(!p.to.exists() && p.from.is_dir());
    assert!(old_copies::load(&w.app).unwrap().is_empty(), "nothing was recorded before a switch");
}

// -- switch over: Steam, records, old copy -------------------------------------------------------

#[test]
fn switching_repoints_steam_keeping_app_ids_and_records_the_old_copy() {
    let w = World::new("switch");
    let p = w.prepare_with(&MoveOptions::default()).unwrap();
    let soh_exe = p.from.join("soh.appimage");
    fs::write(
        &w.vdf,
        file_of(&[
            steam_entry(0, 0xAAAA_0001, "Ship of Harkinian", &format!("\"{}\"", soh_exe.display()), &format!("\"{}\"", p.from.display()), &[]),
            steam_entry(1, 0xAAAA_0002, "Chiaki", "\"/usr/bin/chiaki\"", "\"/usr/bin/\"", &[]),
            // A hand-made one Steam has not given an ID yet.
            object("2", &[str_field("AppName", b"No ID"), str_field("Exe", format!("\"{}\"", soh_exe.display()).as_bytes()), str_field("StartDir", b"\"/x\"")]),
        ]),
    )
    .unwrap();
    let env = w.env(&no_steam, &nothing_running, &plenty);
    let report = switch_over(&p, &env);
    let s = w.shortcuts();
    assert_eq!(s[0].app_id, Some(0xAAAA_0001), "the app ID is kept");
    assert_eq!(s[0].exe_path(), p.to.join("soh.appimage").to_str().unwrap());
    assert_eq!(s[0].start_dir_path(), p.to.to_str().unwrap());
    assert_eq!(s[1].exe_path(), "/usr/bin/chiaki", "other shortcuts are untouched");
    assert!(s[2].exe_path().starts_with(p.from.to_str().unwrap()), "one without an app ID is left alone");
    assert_eq!(report.steam.changed_count(), 1);
    assert!(report.steam.skipped.iter().any(|(n, why)| n == "No ID" && why == steam::NO_APP_ID));
    assert!(crate::fork::backups::list_backups(&w.vdf).unwrap().len() == 1, "a backup was made first");
    let old = old_copies::load(&w.app).unwrap();
    assert_eq!(old.len(), 1);
    assert_eq!((old[0].old_path.as_path(), old[0].new_path.as_path()), (p.from.as_path(), p.to.as_path()));
    assert!(p.from.is_dir(), "the old copy is kept");
    assert!(report.text().contains("1 shortcut now points"), "{}", report.text());
}

#[test]
fn while_steam_runs_the_repoint_is_queued_and_applied_later() {
    let w = World::new("queue");
    let p = w.prepare_with(&MoveOptions::default()).unwrap();
    let before = fs::read(&w.vdf).unwrap();
    let env = w.env(&steam_up, &nothing_running, &plenty);
    let report = switch_over(&p, &env);
    assert_eq!(fs::read(&w.vdf).unwrap(), before, "nothing written while Steam runs");
    assert_eq!(report.steam.queued.len(), 1);
    let queue_file = steam_queue::queue_path(&w.app);
    let queued = steam_queue::load(&queue_file).queue;
    assert_eq!(queued.entries.len(), 1);
    assert_eq!(queued.entries[0].kind, QueuedKind::Repoint);
    assert_eq!(queued.entries[0].repoint.as_ref().map(|(f, t)| (PathBuf::from(f), PathBuf::from(t))), Some((p.from.clone(), p.to.clone())));

    // The old copy cannot go while the shortcut still points into it.
    let active = vec![p.to.clone()];
    let old = &old_copies::load(&w.app).unwrap()[0];
    let refused = old_copies::removal_check(old, &w.removal_ctx(&active)).unwrap_err();
    assert!(refused.contains("still queued"), "{refused}");

    // Steam closed (Desktop Mode): applying the queue writes the repoint.
    let settings = ForkSettings::load(&w.app.join("fork-settings.json"));
    let inputs = ApplyInputs { home: &w.home, library_dir: &w.new_lib, catalog: &[], favorite_exe: &|_| None, cache_dir: None, settings: &settings, game_mode: false };
    let out = steam_queue::apply_queue_file(&queue_file, &inputs, &no_steam, SystemTime::now());
    assert!(out.wrote && !out.refused, "{}", out.text);
    assert_eq!(w.shortcuts()[0].exe_path(), p.to.join("soh.appimage").to_str().unwrap());
    assert_eq!(w.shortcuts()[0].app_id, Some(0xAAAA_0001));
    assert!(!queue_file.exists(), "the queue is empty now");
    assert!(old_copies::removal_check(old, &w.removal_ctx(&active)).is_ok());
}

#[test]
fn in_game_mode_the_repoint_is_always_queued() {
    let w = World::new("gamemode");
    let p = w.prepare_with(&MoveOptions::default()).unwrap();
    let before = fs::read(&w.vdf).unwrap();
    let mut env = w.env(&no_steam, &nothing_running, &plenty);
    env.game_mode = true;
    let report = switch_over(&p, &env);
    assert_eq!(fs::read(&w.vdf).unwrap(), before);
    assert_eq!(report.steam.queued.len(), 1);
}

#[test]
fn a_second_move_chains_onto_a_queued_one_and_a_move_back_cancels_it() {
    let mut queue = steam_queue::SteamQueue::default();
    let change = |from: &str, to: &str| steam_queue::QueuedChange {
        account_id: 1,
        account_label: String::new(),
        port_key: "k".into(),
        name: "P".into(),
        kind: QueuedKind::Repoint,
        repoint_by_name: false,
        art: steam_queue::ArtChoice::None,
        from_game_mode: true,
        queued_at: String::new(),
        repoint: Some((from.into(), to.into())),
        icon: None,
    };
    queue.upsert(change("/a", "/b"));
    queue.upsert(change("/b", "/c"));
    assert_eq!(queue.entries.len(), 1);
    assert_eq!(queue.entries[0].repoint, Some(("/a".into(), "/c".into())));
    queue.upsert(change("/c", "/a"));
    assert!(queue.is_empty());
    assert!(!queue.contains(1, "k"));
}

#[test]
fn rom_connection_records_follow_the_move() {
    let w = World::new("records");
    let old_port = w.old_lib.join("SoH");
    let mapping = connect_link(&w, &old_port);
    let p = w.prepare_with(&MoveOptions { game_files: GameFiles { mappings: vec![mapping], required: Vec::new() }, ..Default::default() }).unwrap();
    let report = switch_over(&p, &w.env(&no_steam, &nothing_running, &plenty));
    assert_eq!(report.rom_records, 1);
    let store = crate::fork::rom_library::RomLibraryStore::load(&w.app.join("rom_library.json")).unwrap();
    assert_eq!(store.mappings()[0].dest, Some(p.to.join("Zelda.z64")));
    assert_eq!(crate::fork::rom_library::check_mapping(&store.mappings()[0]), crate::fork::rom_library::Health::Ok);
}

// -- removing the old copy -----------------------------------------------------------------------

#[test]
fn the_old_copy_is_removed_only_on_request_after_the_switch_with_saves_zipped_first() {
    let w = World::new("remove");
    let p = w.prepare_with(&MoveOptions::default()).unwrap();
    switch_over(&p, &w.env(&no_steam, &nothing_running, &plenty));
    let entry = old_copies::load(&w.app).unwrap().remove(0);

    // Not while the app does not use the new copy.
    let refused = old_copies::removal_check(&entry, &w.removal_ctx(&[p.from.clone()])).unwrap_err();
    assert!(refused.contains("does not use the new copy"), "{refused}");
    // Not when the saves cannot be backed up.
    let active = vec![p.to.clone()];
    // Not before the port was started from its new place (or the user said it starts): nothing is touched.
    let not_yet = old_copies::remove(&entry, &w.removal_ctx(&active), &|_| panic!("no zip before it is ready")).unwrap_err();
    assert!(not_yet.contains("once from its new place") && p.from.is_dir(), "{not_yet}");
    old_copies::mark_started(&w.app, &entry.old_path, old_copies::StartedBy::User, 1).unwrap();
    let err = old_copies::remove(&entry, &w.removal_ctx(&active), &|_| Err("disk full".into())).unwrap_err();
    assert!(err.contains("nothing was removed") && p.from.is_dir(), "{err}");
    // A running program blocks it too.
    let running = |_: &Path| vec!["soh".to_string()];
    let ctx = RemovalCtx { running: &running, ..w.removal_ctx(&active) };
    assert!(old_copies::removal_check(&entry, &ctx).unwrap_err().contains("running"));

    let size = old_copies::removal_check(&entry, &w.removal_ctx(&active)).unwrap();
    assert_eq!(size, p.bytes);
    let zipped = std::cell::Cell::new(false);
    let done = old_copies::remove(&entry, &w.removal_ctx(&active), &|e| {
        assert!(e.old_path.join("saves/slot1.sav").is_file(), "the zip hook runs before anything is removed");
        zipped.set(true);
        Ok(Some(w.app.join("zip")))
    })
    .unwrap();
    assert!(zipped.get() && !p.from.exists() && p.to.is_dir());
    assert_eq!(done.bytes, p.bytes);
    assert!(old_copies::load(&w.app).unwrap().is_empty(), "the record is dropped");
    assert!(old_copies::removal_check(&entry, &w.removal_ctx(&active)).unwrap_err().contains("already gone"));
}

#[test]
fn protected_folders_are_never_removed_as_old_copies() {
    let w = World::new("protected");
    for old in [w.app.clone(), w.home.clone(), w.root.join("sd")] {
        // A new copy of the same name, so the name check does not answer first.
        let new_copy = w.root.join("elsewhere").join(old.file_name().unwrap());
        fs::create_dir_all(&new_copy).unwrap();
        let active = vec![new_copy.clone()];
        let entry = old_copies::OldCopy { port_key: "k".into(), name: "n".into(), old_path: old.clone(), new_path: new_copy.clone(), bytes: 0, moved_at: String::new(), link_only: false, switched: true, import_folder: None, started: None, steam_at: None, repoint_queued: None };
        assert!(old_copies::removal_check(&entry, &w.removal_ctx(&active)).unwrap_err().contains("never removed"), "{}", old.display());
    }
}

// -- adopted ports (a link to another folder) -----------------------------------------------------

#[cfg(unix)]
#[test]
fn an_adopted_port_moves_as_a_link_to_the_same_folder() {
    let w = World::new("adopted");
    let install = w.home.join("Games").join("2s2h");
    put(&install.join("2s2h.appimage"), b"x");
    let link = w.old_lib.join("2s2h");
    std::os::unix::fs::symlink(&install, &link).unwrap();
    let env = w.env(&no_steam, &nothing_running, &plenty);
    let to = w.new_lib.join("2s2h");
    let p = prepare(&w.port(), &link, &to, &env, &MoveOptions::default(), &mut |_, _, _| {}, &AtomicBool::new(false)).unwrap();
    assert!(p.link_only);
    assert_eq!(fs::read_link(&to).unwrap(), install);
    assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the old link is kept");
    switch_over(&p, &env);
    let entry = old_copies::load(&w.app).unwrap().remove(0);
    let active = vec![to.clone()];
    old_copies::mark_started(&w.app, &entry.old_path, old_copies::StartedBy::App, 1).unwrap();
    old_copies::remove(&entry, &w.removal_ctx(&active), &|_| panic!("a link has no saves of its own")).unwrap();
    assert!(fs::symlink_metadata(&link).is_err() && install.join("2s2h.appimage").is_file(), "only the link went");
}

// -- the game file check ---------------------------------------------------------------------------

/// Connects the ROM to the port folder as a link, the way Settings > Game files does.
fn connect_link(w: &World, port_dir: &Path) -> crate::fork::rom_library::Mapping {
    use crate::fork::rom_library::{apply, plan_connect, Access, Approval, ConnectRequest, Method};
    let req = ConnectRequest {
        port_key: w.port().key,
        slot: "zelda".into(),
        source: w.roms.join("n64/Zelda.z64"),
        port_dir: port_dir.to_path_buf(),
        dest_subdir: String::new(),
        dest_name: None,
        method: if cfg!(unix) { Method::Symlink } else { Method::Copy },
        access: Access::Link,
        byte_order: None,
        known_crc32: None,
    };
    let mapping = apply(&plan_connect(&req).unwrap(), Approval::CopyAllowed).unwrap();
    let mut store = crate::fork::rom_library::RomLibraryStore::load(&w.app.join("rom_library.json")).unwrap();
    store.record(mapping.clone()).unwrap();
    mapping
}

#[test]
fn the_game_file_check_passes_when_everything_is_there() {
    let w = World::new("rom_ok");
    let port_dir = w.old_lib.join("SoH");
    let mapping = connect_link(&w, &port_dir);
    let required = vec![RequiredFile { label: "Zelda".into(), names: vec!["zelda.z64".into()], slot_id: "other".into(), crc32: Vec::new() }];
    let files = GameFiles { mappings: vec![mapping], required };
    assert!(game_files::check_before(&port_dir, &files).is_empty());
    let p = w.prepare_with(&MoveOptions { game_files: files, ..Default::default() }).unwrap();
    assert!(fs::metadata(p.to.join("Zelda.z64")).is_ok(), "the connected file is reachable at the new place");
}

#[test]
fn a_missing_game_file_stops_the_move_unless_the_user_moves_anyway() {
    let w = World::new("rom_missing");
    let port_dir = w.old_lib.join("SoH");
    let mapping = connect_link(&w, &port_dir);
    // A link needs its original; a copy (Windows has no links here) only needs itself.
    if cfg!(unix) {
        fs::remove_file(w.roms.join("n64/Zelda.z64")).unwrap();
    } else {
        fs::remove_file(port_dir.join("Zelda.z64")).unwrap();
    }
    let files = GameFiles { mappings: vec![mapping], required: Vec::new() };
    let err = w.prepare_with(&MoveOptions { game_files: files.clone(), ..Default::default() }).unwrap_err();
    let MoveError::GameFiles { stage: Stage::BeforeCopy, issues } = &err else { panic!("{err}") };
    assert!(issues.iter().any(|i| i.file == "Zelda.z64" && i.problem.contains("missing")), "{issues:?}");
    assert!(!w.new_lib.join("SoH").exists() && !w.staging_left(), "nothing was copied");
    assert!(err.to_string().contains("Nothing was changed"));

    // "Move anyway" before the copy: allowed, and the accepted problems are reported.
    #[cfg(unix)]
    {
        let p = w.prepare_with(&MoveOptions { game_files: files, accept_issues_before: true, ..Default::default() }).unwrap();
        assert!(!p.accepted_issues.is_empty());
    }
}

#[test]
fn a_required_file_that_is_not_connected_or_present_never_stops_the_move() {
    // Like Space Cadet Pinball: the catalog names a zip needed once at install time.
    let w = World::new("rom_required");
    let port_dir = w.old_lib.join("SoH");
    let required = vec![RequiredFile { label: "Space Cadet data".into(), names: vec!["3d-pinball-space-cadet_multi.zip".into()], slot_id: "zip".into(), crc32: Vec::new() }];
    let files = GameFiles { mappings: Vec::new(), required };
    assert!(game_files::check_before(&port_dir, &files).is_empty(), "no prompt for it");
    assert_eq!(game_files::required_not_found(&port_dir, &files), ["3d-pinball-space-cadet_multi.zip"], "only an info line");
    let p = w.prepare_with(&MoveOptions { game_files: files.clone(), ..Default::default() }).unwrap();
    assert!(p.accepted_issues.is_empty());
    // Found in the folder (any case), it is not listed at all.
    put(&p.to.join("data/3D-PINBALL-SPACE-CADET_MULTI.ZIP"), b"zip");
    assert!(game_files::required_not_found(&p.to, &files).is_empty());
}

#[cfg(unix)]
#[test]
fn a_relative_link_to_a_game_file_outside_is_rewritten_to_an_absolute_one() {
    let w = World::new("rom_relative");
    let port_dir = w.old_lib.join("SoH");
    // <root>/sd/roms, seen from <root>/home/Applications/dpm/Library/SoH (five levels up)
    let relative = PathBuf::from("../../../../../sd/roms/n64/Zelda.z64");
    std::os::unix::fs::symlink(&relative, port_dir.join("rom.z64")).unwrap();
    assert!(fs::metadata(port_dir.join("rom.z64")).is_ok(), "the fixture link works where it is");
    let p = w.prepare_with(&MoveOptions::default()).unwrap();
    let new_link = p.to.join("rom.z64");
    let target = fs::read_link(&new_link).unwrap();
    assert!(target.is_absolute(), "{}", target.display());
    assert_eq!(fs::canonicalize(&new_link).unwrap(), fs::canonicalize(w.roms.join("n64/Zelda.z64")).unwrap());
    assert_eq!(p.rewritten_links, vec![new_link]);
    assert_eq!(fs::read_link(p.to.join("soh")).unwrap(), PathBuf::from("soh.appimage"), "links inside stay relative");
    assert_eq!(fs::read_link(port_dir.join("rom.z64")).unwrap(), relative, "the original link is untouched");
}

#[cfg(unix)]
#[test]
fn a_game_file_link_that_breaks_in_the_copy_stops_the_move() {
    let w = World::new("rom_link_breaks");
    let port_dir = w.old_lib.join("SoH");
    let mapping = connect_link(&w, &port_dir);
    put(&w.roms.join("n64/Other.z64"), &vec![3u8; 50_000]);
    let other = w.roms.join("n64/Other.z64");
    // The copy's link is swapped for one to another file before the checks run (the verify step would
    // see a different link target, so swap it in the manifest's terms: same text, other file).
    let swap = |staging: &Path| {
        // The copy's link now leads somewhere else, through a detour link.
        let link = staging.join("Zelda.z64");
        fs::remove_file(&link).unwrap();
        let detour = staging.join(".detour");
        std::os::unix::fs::symlink(&other, &detour).unwrap();
        std::os::unix::fs::symlink(&detour, &link).unwrap();
    };
    let files = GameFiles { mappings: vec![mapping], required: Vec::new() };
    let err = w.prepare_with(&MoveOptions { game_files: files.clone(), after_copy: Some(&swap), ..Default::default() }).unwrap_err();
    // The verify step catches the changed link first; either way the move stops and nothing stays.
    assert!(matches!(err, MoveError::Verify(_) | MoveError::GameFiles { stage: Stage::AfterCopy, .. }), "{err}");
    assert!(!w.new_lib.join("SoH").exists() && !w.staging_left());

    // The game file check on its own: a copy whose link reaches another file is reported.
    let copy_dir = w.root.join("copy");
    let mut flag = |_: u64, _: &Path| {};
    let cancel = AtomicBool::new(false);
    let manifest = copy::Copier::new(&cancel, &mut flag).copy_tree(&port_dir, &copy_dir).unwrap();
    fs::remove_file(copy_dir.join("Zelda.z64")).unwrap();
    std::os::unix::fs::symlink(&other, copy_dir.join("Zelda.z64")).unwrap();
    let issues = game_files::check_after(&port_dir, &copy_dir, &manifest, &files);
    assert!(issues.iter().any(|i| i.problem.contains("different file")), "{issues:?}");
    assert!(issues.iter().any(|i| i.problem.contains("does not reach the original")), "{issues:?}");
}

#[test]
fn a_copied_game_file_with_another_checksum_after_the_copy_stops_the_move() {
    use crate::fork::rom_library::{apply, plan_connect, Access, Approval, ConnectRequest, Method};
    let w = World::new("rom_hash");
    let port_dir = w.old_lib.join("SoH");
    let req = ConnectRequest {
        port_key: w.port().key,
        slot: "zelda".into(),
        source: w.roms.join("n64/Zelda.z64"),
        port_dir: port_dir.clone(),
        dest_subdir: "rom".into(),
        dest_name: None,
        method: Method::Copy,
        access: Access::Copy,
        byte_order: None,
        known_crc32: None,
    };
    let mapping = apply(&plan_connect(&req).unwrap(), Approval::CopyAllowed).unwrap();
    assert!(mapping.crc32.is_some(), "a copy records its CRC");
    let files = GameFiles { mappings: vec![mapping], required: Vec::new() };
    let corrupt = |staging: &Path| {
        let f = staging.join("rom/Zelda.z64");
        let t = fs::metadata(&f).unwrap().modified().unwrap();
        let mut data = fs::read(&f).unwrap();
        data[0] ^= 1;
        fs::write(&f, data).unwrap();
        fs::File::options().write(true).open(&f).unwrap().set_modified(t).unwrap();
    };
    // Even above the whole-port hash limit, the game file is read back.
    let options = MoveOptions { game_files: files.clone(), after_copy: Some(&corrupt), policy: Some(Policy { full_hash_limit: 0 }), ..Default::default() };
    let err = w.prepare_with(&options).unwrap_err();
    assert!(matches!(&err, MoveError::Verify(e) if e.contains("Zelda.z64")), "{err}");
    assert!(!w.new_lib.join("SoH").exists() && !w.staging_left());

    // The CRC check on its own.
    let copy_dir = w.root.join("copy");
    let mut flag = |_: u64, _: &Path| {};
    let cancel = AtomicBool::new(false);
    let manifest = copy::Copier::new(&cancel, &mut flag).copy_tree(&port_dir, &copy_dir).unwrap();
    corrupt(&copy_dir);
    let issues = game_files::check_after(&port_dir, &copy_dir, &manifest, &files);
    assert!(issues.iter().any(|i| i.problem.contains("not the same file as the one connected")), "{issues:?}");
}

// -- review fixes ------------------------------------------------------------------------------------

/// Moves the fixture port and switches over with Steam closed; returns the old-copy record.
fn moved_and_switched(w: &World) -> (Prepared, old_copies::OldCopy) {
    let p = w.prepare_with(&MoveOptions::default()).unwrap();
    switch_over(&p, &w.env(&no_steam, &nothing_running, &plenty));
    let entry = old_copies::load(&w.app).unwrap().into_iter().find(|o| o.old_path == p.from).unwrap();
    (p, entry)
}

#[test]
fn newer_saves_in_the_old_copy_keep_it() {
    let w = World::new("newer_saves");
    let (p, entry) = moved_and_switched(&w);
    let active = vec![p.to.clone()];
    assert!(old_copies::removal_check(&entry, &w.removal_ctx(&active)).is_ok());
    // A game started from Steam before the repoint applied saved into the old copy.
    let save = p.from.join("saves/slot1.sav");
    fs::write(&save, b"played later").unwrap();
    let later = SystemTime::now() + std::time::Duration::from_secs(60);
    fs::File::options().write(true).open(&save).unwrap().set_modified(later).unwrap();
    let refused = old_copies::removal_check(&entry, &w.removal_ctx(&active)).unwrap_err();
    assert!(refused.contains("The old copy has newer saves (last played"), "{refused}");
    assert!(refused.contains("Copy the save files from"));
}

#[test]
fn a_port_that_left_the_catalog_or_an_unfinished_switch_keeps_the_old_copy() {
    let w = World::new("unknown_port");
    let (p, mut entry) = moved_and_switched(&w);
    let active = vec![p.to.clone()];
    let unknown = |_: &old_copies::OldCopy, _: &Path| None;
    let ctx = RemovalCtx { save_folders: &unknown, ..w.removal_ctx(&active) };
    assert!(old_copies::removal_check(&entry, &ctx).unwrap_err().contains("not in the catalog any more"));
    entry.switched = false;
    assert!(old_copies::removal_check(&entry, &w.removal_ctx(&active)).unwrap_err().contains("did not finish"));
    // Records written before the setting changes are marked unfinished until switch_over.
    record_pending(&w.app, std::slice::from_ref(&p)).unwrap();
    assert!(!old_copies::load(&w.app).unwrap()[0].switched);
    switch_over(&p, &w.env(&no_steam, &nothing_running, &plenty));
    assert!(old_copies::load(&w.app).unwrap()[0].switched);
}

#[test]
fn an_old_copy_that_no_longer_looks_like_the_port_or_has_another_name_is_kept() {
    let w = World::new("lookalike");
    let (p, entry) = moved_and_switched(&w);
    let active = vec![p.to.clone()];
    fs::write(p.to.join("new-file-after-move"), b"x").unwrap();
    assert!(old_copies::removal_check(&entry, &w.removal_ctx(&active)).unwrap_err().contains("does not look like the same port"));
    fs::remove_file(p.to.join("new-file-after-move")).unwrap();
    let renamed = old_copies::OldCopy { new_path: w.new_lib.join("Other"), ..entry.clone() };
    fs::create_dir_all(&renamed.new_path).unwrap();
    let active2 = vec![renamed.new_path.clone()];
    assert!(old_copies::removal_check(&renamed, &w.removal_ctx(&active2)).unwrap_err().contains("different folder names"));
}

#[test]
fn a_launch_option_naming_the_old_folder_keeps_it() {
    let w = World::new("launch_options");
    let (p, entry) = moved_and_switched(&w);
    let options = format!("--data {}/saves %command%", p.from.display());
    fs::write(
        &w.vdf,
        file_of(&[object(
            "0",
            &[int_field("appid", 0xAAAA_0009), str_field("AppName", b"Wrapper"), str_field("Exe", b"\"/usr/bin/env\""), str_field("StartDir", b"\"/usr/bin/\""), str_field("LaunchOptions", options.as_bytes())],
        )]),
    )
    .unwrap();
    let refused = old_copies::removal_check(&entry, &w.removal_ctx(&[p.to.clone()])).unwrap_err();
    assert!(refused.contains("Wrapper"), "{refused}");
}

#[test]
fn a_copied_game_file_whose_original_is_gone_does_not_stop_a_move() {
    use crate::fork::rom_library::{apply, plan_connect, Access, Approval, ConnectRequest, Method};
    let w = World::new("copy_original_gone");
    let port_dir = w.old_lib.join("SoH");
    let req = ConnectRequest {
        port_key: w.port().key,
        slot: "zelda".into(),
        source: w.roms.join("n64/Zelda.z64"),
        port_dir: port_dir.clone(),
        dest_subdir: String::new(),
        dest_name: None,
        method: Method::Copy,
        access: Access::Copy,
        byte_order: None,
        known_crc32: None,
    };
    let mapping = apply(&plan_connect(&req).unwrap(), Approval::CopyAllowed).unwrap();
    fs::remove_file(w.roms.join("n64/Zelda.z64")).unwrap();
    let files = GameFiles { mappings: vec![mapping.clone()], required: Vec::new() };
    assert!(game_files::check_before(&port_dir, &files).is_empty(), "only the copy matters");
    // A missing copy does stop it.
    fs::remove_file(port_dir.join("Zelda.z64")).unwrap();
    let issues = game_files::check_before(&port_dir, &files);
    assert!(issues.iter().any(|i| i.problem.contains("link or copy in the port folder is missing")), "{issues:?}");
}

#[cfg(unix)]
#[test]
fn an_absolute_link_inside_the_port_becomes_relative_and_a_stale_one_is_caught() {
    let w = World::new("absolute_inside");
    let port_dir = w.old_lib.join("SoH");
    std::os::unix::fs::symlink(port_dir.join("soh.appimage"), port_dir.join("abs-link")).unwrap();
    std::os::unix::fs::symlink(port_dir.join("saves"), port_dir.join("saves-link")).unwrap();
    let p = w.prepare_with(&MoveOptions::default()).unwrap();
    assert_eq!(fs::read_link(p.to.join("abs-link")).unwrap(), PathBuf::from("soh.appimage"));
    assert_eq!(fs::read_link(p.to.join("saves-link")).unwrap(), PathBuf::from("saves"));
    assert!(p.rewritten_links.contains(&p.to.join("abs-link")));
    assert!(old_copies::links_into(&p.to, &p.from).is_empty());

    // The check after the copy flags an absolute link into the old folder that was not rewritten.
    let copy_dir = w.root.join("copy");
    let cancel = AtomicBool::new(false);
    let mut flag = |_: u64, _: &Path| {};
    let mut manifest = copy::Copier::new(&cancel, &mut flag).copy_tree(&port_dir, &copy_dir).unwrap();
    for e in &mut manifest.entries {
        if let copy::Kind::Link { target, original } = &mut e.kind {
            if original.is_absolute() {
                *target = original.clone();
            }
        }
    }
    let issues = game_files::check_after(&port_dir, &copy_dir, &manifest, &GameFiles::default());
    assert!(issues.iter().any(|i| i.problem == "this link still points into the old folder"), "{issues:?}");

    // And an old copy is kept while a link in the new copy reaches into it.
    switch_over(&p, &w.env(&no_steam, &nothing_running, &plenty));
    let entry = old_copies::load(&w.app).unwrap().into_iter().find(|o| o.old_path == p.from).unwrap();
    fs::remove_file(p.to.join("abs-link")).unwrap();
    std::os::unix::fs::symlink(p.from.join("soh.appimage"), p.to.join("abs-link")).unwrap();
    let refused = old_copies::removal_check(&entry, &w.removal_ctx(&[p.to.clone()])).unwrap_err();
    assert!(refused.contains("still points into the old one"), "{refused}");
}

#[test]
fn leftover_staging_folders_of_dead_runs_are_swept() {
    let w = World::new("sweep");
    let dead = w.new_lib.join(".SoH.dpm-moving-4000000000");
    let own = w.new_lib.join(format!(".Dusk.dpm-moving-{}", std::process::id()));
    let other = w.new_lib.join(".notes.txt");
    for d in [&dead, &own] {
        fs::create_dir_all(d.join("x")).unwrap();
    }
    fs::write(&other, b"keep").unwrap();
    sweep_staging(&w.new_lib);
    assert!(!own.exists(), "this process's own leftover goes");
    // Whether another process is gone can only be told on Linux (/proc); elsewhere its folder stays.
    assert_eq!(dead.exists(), !cfg!(target_os = "linux"));
    assert!(other.exists(), "only staging folders are touched");
    #[cfg(target_os = "linux")]
    {
        let alive = w.new_lib.join(".X.dpm-moving-1");
        fs::create_dir_all(&alive).unwrap();
        sweep_staging(&w.new_lib);
        assert!(alive.exists(), "a running process's staging folder is left alone");
    }
}

// Windows cannot rename a folder while files in it are open, which is how the test pulls the card.
#[cfg(unix)]
#[test]
fn a_drive_pulled_out_during_a_batch_stops_it_with_one_message() {
    let w = World::new("yanked");
    put(&w.old_lib.join("Dusk/dusk"), b"x");
    let items = vec![
        BatchItem { port: w.port(), from: w.old_lib.join("SoH"), to: w.new_lib.join("SoH") },
        BatchItem { port: PortRef { key: "d".into(), name: "Dusk".into() }, from: w.old_lib.join("Dusk"), to: w.new_lib.join("Dusk") },
    ];
    let env = w.env(&no_steam, &nothing_running, &plenty);
    let yanked = std::cell::Cell::new(false);
    let mut progress = |_: usize, _: u64, _: u64| {
        if !yanked.get() {
            yanked.set(true);
            // The card goes away: its folder disappears under the copy.
            fs::rename(&w.new_lib, w.root.join("pulled-out")).unwrap();
        }
    };
    let result = run_batch(&items, &env, &|_| MoveOptions::default(), &mut progress, &AtomicBool::new(false));
    assert_eq!(result.outcomes.len(), 1, "the second port is not tried");
    assert!(matches!(result.outcomes[0], BatchOutcome::Failed { .. }));
    assert!(result.drive_gone.as_deref().is_some_and(|m| m.contains("went away during the move")), "{:?}", result.drive_gone);
    assert!(w.old_lib.join("SoH/soh.appimage").is_file() && w.old_lib.join("Dusk/dusk").is_file(), "originals untouched");
}

#[test]
fn a_cancelled_batch_reports_it() {
    let w = World::new("batch_cancel");
    let items = vec![BatchItem { port: w.port(), from: w.old_lib.join("SoH"), to: w.new_lib.join("SoH") }];
    let env = w.env(&no_steam, &nothing_running, &plenty);
    let cancel = AtomicBool::new(true);
    let result = run_batch(&items, &env, &|_| MoveOptions::default(), &mut |_, _, _| {}, &cancel);
    assert!(result.cancelled && result.outcomes.is_empty());
}

#[test]
fn the_queued_repoint_summary_warns_that_steam_still_runs_the_old_copy() {
    let w = World::new("queued_text");
    let p = w.prepare_with(&MoveOptions::default()).unwrap();
    let report = switch_over(&p, &w.env(&steam_up, &nothing_running, &plenty));
    let text = report.text();
    assert!(text.contains("starting Ship of Harkinian from Steam still runs the OLD copy"), "{text}");
}

// -- review round 2 ------------------------------------------------------------------------------------

/// A kept old copy of the fixture port at `stale`, recorded, with the port in use at `current`.
fn kept_copy(w: &World, stale: &Path, current: &Path) -> old_copies::OldCopy {
    put(&stale.join("soh.appimage"), b"#!/bin/sh\necho soh\n");
    put(&stale.join("saves/slot1.sav"), b"old save");
    put(&current.join("saves/slot1.sav"), b"current save");
    let entry = old_copies::OldCopy {
        port_key: w.port().key,
        name: w.port().name,
        old_path: stale.to_path_buf(),
        new_path: current.to_path_buf(),
        bytes: 10,
        moved_at: String::new(),
        link_only: false,
        switched: true,
        import_folder: None,
        started: None,
        steam_at: None,
        repoint_queued: None,
    };
    old_copies::record(&w.app, entry.clone()).unwrap();
    entry
}

fn set_time(path: &Path, t: SystemTime) {
    fs::File::options().write(true).open(path).unwrap().set_modified(t).unwrap();
}

#[test]
fn replacing_a_kept_old_copy_is_checked_first() {
    let w = World::new("replace_check");
    let current = w.old_lib.join("SoH");
    let stale = w.new_lib.join("SoH");
    let entry = kept_copy(&w, &stale, &current);
    let now = SystemTime::now();
    set_time(&current.join("saves/slot1.sav"), now);
    set_time(&stale.join("saves/slot1.sav"), now - std::time::Duration::from_secs(3600));
    let ctx = w.removal_ctx(&[]);
    assert!(old_copies::replace_check(&entry, &current, &ctx).is_ok(), "older saves: fine to replace");

    // Newer saves in the kept copy: refused, naming both save folders.
    set_time(&stale.join("saves/slot1.sav"), now + std::time::Duration::from_secs(3600));
    let refused = old_copies::replace_check(&entry, &current, &ctx).unwrap_err();
    assert!(refused.contains("has newer saves (last played") && refused.contains("than the copy in use"), "{refused}");
    assert!(refused.contains(&stale.join("saves").display().to_string()) && refused.contains(&current.join("saves").display().to_string()), "{refused}");
    set_time(&stale.join("saves/slot1.sav"), now - std::time::Duration::from_secs(3600));

    // A game running from it.
    let running = |_: &Path| vec!["soh (process 9)".to_string()];
    let ctx_running = RemovalCtx { running: &running, ..w.removal_ctx(&[]) };
    assert!(old_copies::replace_check(&entry, &current, &ctx_running).unwrap_err().contains("is running from the old copy"));

    // Not what was recorded (a link expected).
    let as_link = old_copies::OldCopy { link_only: true, ..entry.clone() };
    assert!(old_copies::replace_check(&as_link, &current, &ctx).unwrap_err().contains("not what was recorded"));

    // A protected folder: the app folder, or one holding the copy in use.
    let app_entry = old_copies::OldCopy { old_path: w.app.clone(), ..entry.clone() };
    assert!(old_copies::replace_check(&app_entry, &current, &ctx).unwrap_err().contains("never removed"));
    let holder = old_copies::OldCopy { old_path: w.old_lib.clone(), ..entry.clone() };
    assert!(old_copies::replace_check(&holder, &current, &ctx).unwrap_err().contains("never removed"));

    // The port left the catalog.
    let unknown = |_: &old_copies::OldCopy, _: &Path| None;
    let ctx_unknown = RemovalCtx { save_folders: &unknown, ..w.removal_ctx(&[]) };
    assert!(old_copies::replace_check(&entry, &current, &ctx_unknown).unwrap_err().contains("not in the catalog"));

    // Already gone: nothing to replace.
    copy::remove_copy(&stale).unwrap();
    assert!(old_copies::replace_check(&entry, &current, &ctx).is_ok());
}

#[test]
fn an_unfinished_switch_is_finished_from_its_record() {
    let w = World::new("finish_switch");
    let p = w.prepare_with(&MoveOptions::default()).unwrap();
    // The app closed after the records and the setting, before Steam followed.
    record_pending(&w.app, std::slice::from_ref(&p)).unwrap();
    let record = old_copies::load(&w.app).unwrap().remove(0);
    assert!(!record.switched);
    let active = vec![p.to.clone()];
    let refused = old_copies::removal_check(&record, &w.removal_ctx(&active)).unwrap_err();
    assert!(refused.contains("Finish switching Ship of Harkinian") && refused.contains("Settings > Ports folder"), "{refused}");
    assert!(w.shortcuts()[0].exe_path().starts_with(p.from.to_str().unwrap()), "Steam still points at the old copy");

    let report = finish_switch(&record, &w.env(&no_steam, &nothing_running, &plenty), &w.new_lib, &active).unwrap();
    assert_eq!(report.steam.changed_count(), 1);
    assert_eq!(w.shortcuts()[0].exe_path(), p.to.join("soh.appimage").to_str().unwrap());
    let record = old_copies::load(&w.app).unwrap().remove(0);
    assert!(record.switched);
    assert!(old_copies::removal_check(&record, &w.removal_ctx(&active)).is_ok());
}

#[test]
fn a_shortcut_steam_has_not_set_up_names_itself_in_the_refusal() {
    let w = World::new("no_app_id_refusal");
    let (p, entry) = moved_and_switched(&w);
    let exe = p.from.join("soh.appimage");
    fs::write(&w.vdf, file_of(&[object("0", &[str_field("AppName", b"Hand Made"), str_field("Exe", format!("\"{}\"", exe.display()).as_bytes()), str_field("StartDir", b"\"/x\"")])])).unwrap();
    let refused = old_copies::removal_check(&entry, &w.removal_ctx(&[p.to.clone()])).unwrap_err();
    assert!(refused.contains("\"Hand Made\"") && refused.contains("open Steam, quit it, then try again"), "{refused}");
}

#[test]
fn the_newer_saves_refusal_says_where_to_copy_them() {
    let w = World::new("newer_saves_text");
    let (p, entry) = moved_and_switched(&w);
    let save = p.from.join("saves/slot1.sav");
    set_time(&save, SystemTime::now() + std::time::Duration::from_secs(60));
    let refused = old_copies::removal_check(&entry, &w.removal_ctx(&[p.to.clone()])).unwrap_err();
    let expected = format!("Copy the save files from {} into {} (or play once from the new copy to check), then try again; or keep the old copy.", p.from.join("saves").display(), p.to.join("saves").display());
    assert!(refused.contains(&expected), "{refused}");
}

#[cfg(unix)]
#[test]
fn a_relative_link_that_leaves_the_port_on_its_way_is_made_safe() {
    let w = World::new("steps_outside");
    let port_dir = w.old_lib.join("SoH");
    fs::create_dir_all(port_dir.join("sub")).unwrap();
    std::os::unix::fs::symlink("../../SoH/saves", port_dir.join("sub/saves-link")).unwrap();
    assert!(copy::steps_outside(&port_dir, &port_dir.join("sub/saves-link"), Path::new("../../SoH/saves")));
    assert!(!copy::steps_outside(&port_dir, &port_dir.join("sub/x"), Path::new("../saves")));
    let p = w.prepare_with(&MoveOptions::default()).unwrap();
    assert_eq!(fs::read_link(p.to.join("sub/saves-link")).unwrap(), PathBuf::from("../saves"));
    assert!(p.rewritten_links.contains(&p.to.join("sub/saves-link")));
}

#[test]
fn a_corrupted_unfinished_record_repoints_nothing() {
    let w = World::new("finish_corrupt");
    let p = w.prepare_with(&MoveOptions::default()).unwrap();
    record_pending(&w.app, std::slice::from_ref(&p)).unwrap();
    let record = old_copies::load(&w.app).unwrap().remove(0);
    let before = fs::read(&w.vdf).unwrap();
    let active = vec![p.to.clone()];
    let env = w.env(&no_steam, &nothing_running, &plenty);
    let root = if cfg!(unix) { PathBuf::from("/") } else { PathBuf::from("C:\\") };
    let named_like_home = w.home.clone();
    let holder = w.root.join("holder").join("SoH");
    for bad in [
        old_copies::OldCopy { old_path: root.clone(), ..record.clone() },
        old_copies::OldCopy { old_path: named_like_home.clone(), ..record.clone() },
        old_copies::OldCopy { new_path: w.new_lib.join("Missing").join("SoH"), ..record.clone() },
        old_copies::OldCopy { old_path: p.to.clone(), ..record.clone() },
        old_copies::OldCopy { old_path: p.to.join("saves").join("SoH"), ..record.clone() },
    ] {
        let refused = finish_switch(&bad, &env, &w.new_lib, &active).unwrap_err();
        assert!(refused.contains("can't be finished") && refused.contains("Nothing was changed"), "{refused}");
    }
    // An old place that holds the app folder, even with the right name.
    let holding_app = old_copies::OldCopy { old_path: holder.clone(), ..record.clone() };
    let _ = fs::create_dir_all(&holder);
    let app_like = MoveEnv { config_dir: &holder, ..w.env(&no_steam, &nothing_running, &plenty) };
    assert!(finish_switch(&holding_app, &app_like, &w.new_lib, &active).unwrap_err().contains("holds"));
    assert_eq!(fs::read(&w.vdf).unwrap(), before, "no shortcut was touched");
    assert!(!old_copies::load(&w.app).unwrap()[0].switched, "the record is still unfinished");
}
