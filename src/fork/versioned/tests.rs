//! Versioned program files: name rules, the plan, and whole updates through the real installer with a
//! fake Steam folder. Nothing here touches the real Steam folder, saves or ROMs.

use super::*;
use crate::core::installer::{apply_archive, InstallPaths};
use crate::core::models::port_from_value;
use crate::fork::settings::ForkSettings;
use crate::fork::shortcuts;
use crate::fork::steam_queue::{self, ApplyInputs, QueuedKind};
use crate::fork::store::ShortcutsFile;
use crate::fork::vdf::fixtures::*;
use std::io;
use std::time::SystemTime;

const OLD: &str = "Game-v1.0-linux-x86_64.AppImage";
const NEW: &str = "Game-v1.1-linux-x86_64.AppImage";
const APP_ID: u32 = 0xBEEF_0001;

fn temp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("dpm_versioned_{}_{name}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn put(path: &Path, data: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}

/// A minimal 64-bit little-endian ELF executable header, then the tag (so two releases differ).
fn elf(tag: &str) -> Vec<u8> {
    let mut head = vec![0u8; 64];
    head[..4].copy_from_slice(b"\x7fELF");
    head[4] = 2;
    head[5] = 1;
    head[16] = 2;
    [head.as_slice(), tag.as_bytes()].concat()
}

fn no_steam() -> io::Result<Vec<String>> {
    Ok(Vec::new())
}

fn steam_up() -> io::Result<Vec<String>> {
    Ok(vec!["77 steam".into()])
}

fn port(save: Option<&str>) -> Port {
    let mut value = serde_json::json!({"name": "Game", "folder": "game", "source": "https://github.com/owner/game"});
    if let Some(s) = save {
        value["save"] = serde_json::json!(s);
    }
    port_from_value(&value).unwrap()
}

/// A home with one Steam account, an app folder, a ports folder and one port folder.
struct World {
    root: PathBuf,
    home: PathBuf,
    app: PathBuf,
    library: PathBuf,
    game: PathBuf,
    vdf: PathBuf,
    settings: ForkSettings,
}

impl World {
    fn new(name: &str) -> World {
        let root = temp(name);
        let home = root.join("home");
        let app = home.join("Applications").join("dpm");
        let library = app.join("Library");
        let game = library.join("game");
        fs::create_dir_all(&game).unwrap();
        let user = home.join(".local/share/Steam/userdata/1234/config");
        fs::create_dir_all(&user).unwrap();
        let settings = ForkSettings::load(&app.join("fork-settings.json"));
        World { vdf: user.join("shortcuts.vdf"), root, home, app, library, game, settings }
    }

    /// A Steam shortcut for the port pointing at `exe` inside the port folder.
    fn shortcut_to(&self, exe: &str) {
        let exe = self.game.join(exe);
        let entries = [
            steam_entry(0, APP_ID, "Game", &format!("\"{}\"", exe.display()), &format!("\"{}\"", self.game.display()), &[]),
            steam_entry(1, 0xAAAA_0002, "Chiaki", "\"/usr/bin/chiaki\"", "\"/usr/bin/\"", &[]),
        ];
        fs::write(&self.vdf, file_of(&entries)).unwrap();
    }

    fn shortcuts(&self) -> Vec<shortcuts::Shortcut> {
        shortcuts::list(&ShortcutsFile::load(&self.vdf).unwrap().doc)
    }

    /// Installs `release` (a file whose name is the download's) over the port with the real installer.
    fn install(&self, port: &Port, release_name: &str, content: &[u8]) -> Result<(), crate::core::installer::InstallError> {
        let download = self.root.join("downloads").join(release_name);
        put(&download, content);
        let saves = self.root.join("Saves Backup");
        apply_archive(port, InstallPaths { library_dir: &self.library, saves_backup_dir: &saves }, &download, &self.game, "2026-10-24_12-00-00", &mut |_| {})
    }

    fn finish(&self, port: &Port, favorite: Option<&str>, steam: &dyn crate::fork::store::SteamProcessCheck, game_mode: bool) -> Outcome {
        let ctx = SteamCtx { home: &self.home, config_dir: &self.app, settings: &self.settings, steam, game_mode, now: SystemTime::now() };
        finish_update(&FinishInput { port, game_dir: &self.game, favorite, steam: Some(&ctx) })
    }

    fn previous(&self, version: &str, name: &str) -> PathBuf {
        self.game.join(PREVIOUS).join(version).join(name)
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Saves known on this system, outside the port folder: release folders may move.
fn known() -> SaveInfo {
    SaveInfo { folders: vec![PathBuf::from("/nowhere/saves")], folders_may_move: true }
}

fn pending(before: &[&str], release: &[&str]) -> Pending {
    Pending { before: Before { entries: before.iter().map(|s| s.to_string()).collect(), target: None }, release: release.iter().map(|s| s.to_string()).collect() }
}

fn moved_names(plan: &Plan) -> Vec<(&str, &str)> {
    plan.moves.iter().map(|m| (m.old.as_str(), m.new.as_str())).collect()
}

fn kept_names(plan: &Plan) -> Vec<&str> {
    plan.kept.iter().map(|(n, _)| n.as_str()).collect()
}

// -- names -------------------------------------------------------------------------------------

#[test]
fn a_name_gives_its_family_and_its_version() {
    let dusk = parse("Dusklight-v2.0.3-linux-x86_64.AppImage");
    assert_eq!((dusk.family.as_slice(), dusk.version.as_deref()), (["dusklight".to_string()].as_slice(), Some("v2.0.3")));
    assert_eq!(parse(OLD).family, parse(NEW).family);
    assert_eq!(parse(OLD).version.as_deref(), Some("v1.0"));
    assert_eq!(parse("Game_1.2.3.x86_64").version.as_deref(), Some("1.2.3"));
    assert_eq!(parse("Game-v1.1").family, ["game"], "a release folder");
    assert_eq!(parse("Game-Editor-v1.1-linux-x86_64.AppImage").family, ["game", "editor"], "another program, another family");
}

#[test]
fn names_without_a_real_version_are_never_versioned() {
    for name in ["game.AppImage", "Game.AppImage", "Sonic-2.AppImage", "Game-v1.0-linux-x86_64 (1).AppImage", "launch.sh", "Save", "x86_64"] {
        assert_eq!(parse(name).version, None, "{name}");
    }
}

#[test]
fn programs_are_known_endings_or_elf_and_scripts_by_content() {
    let dir = temp("programs");
    for (name, data) in [("a.AppImage", b"x".to_vec()), ("b.x86_64", b"x".to_vec()), ("c.sh", b"x".to_vec()), ("Game-v1.0-linux-x86_64", elf("")), ("run-v1", b"#!/bin/sh\n".to_vec())] {
        put(&dir.join(name), &data);
        assert!(is_program_file(&dir.join(name)), "{name}");
    }
    for (name, data) in [("Game-v1.0.o2r", b"PK".to_vec()), ("Game-v1.0.z64", vec![0x80, 0x37]), ("Game-v1.0.ini", b"[x]".to_vec()), ("Game-v1.0.bin", b"disc".to_vec())] {
        put(&dir.join(name), &data);
        assert!(!is_program_file(&dir.join(name)), "{name}");
    }
    assert!(!is_program_file(&dir.join("missing")));
    let _ = fs::remove_dir_all(&dir);
}

// -- the plan ----------------------------------------------------------------------------------

#[test]
fn only_old_programs_of_the_same_family_are_superseded() {
    let dir = temp("plan_family");
    for name in [OLD, NEW, "Game-Editor-v1.0-linux-x86_64.AppImage", "game.AppImage", "Game-v1.0-linux-x86_64 (1).AppImage"] {
        put(&dir.join(name), &elf(name));
    }
    put(&dir.join("Game-v1.0.o2r"), b"PK packed assets");
    put(&dir.join("Game-v1.0.z64"), b"rom");
    put(&dir.join("Game-v1.0.ini"), b"[settings]");
    put(&dir.join("Save").join("slot1.sav"), b"save");
    put(&dir.join("mods").join("m.zip"), b"mod");
    let before = [OLD, "Game-Editor-v1.0-linux-x86_64.AppImage", "game.AppImage", "Game-v1.0-linux-x86_64 (1).AppImage", "Game-v1.0.o2r", "Game-v1.0.z64", "Game-v1.0.ini", "Save", "mods"];
    let plan = plan(&dir, &pending(&before, &[NEW]), &known());
    assert_eq!(moved_names(&plan), [(OLD, NEW)]);
    assert!(plan.kept.is_empty(), "{:?}", plan.kept);
    assert_eq!(plan.moves[0].version, "v1.0");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_entry_that_is_in_the_new_release_or_hidden_is_not_superseded() {
    let dir = temp("plan_same_name");
    put(&dir.join(NEW), &elf("new"));
    put(&dir.join(".Game-v1.0.AppImage"), &elf("hidden"));
    let plan = plan(&dir, &pending(&[NEW, ".Game-v1.0.AppImage"], &[NEW]), &known());
    assert!(plan.moves.is_empty() && plan.kept.is_empty(), "{plan:?}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn several_appimages_each_go_to_their_closest_successor() {
    let dir = temp("plan_several");
    let names = ["Game-v1.0-linux-x86_64.AppImage", "Game-v1.0-linux-aarch64.AppImage", "Game-v1.1-linux-x86_64.AppImage", "Game-v1.1-linux-aarch64.AppImage"];
    for n in names {
        put(&dir.join(n), &elf(n));
    }
    let mut plan = plan(&dir, &pending(&names[..2], &names[2..]), &known());
    plan.moves.sort_by(|a, b| a.old.cmp(&b.old));
    assert_eq!(moved_names(&plan), [(names[1], names[3]), (names[0], names[2])]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_old_file_two_new_files_fit_equally_is_left_with_a_reason() {
    let dir = temp("plan_ambiguous");
    for n in ["Game-v1.0.AppImage", "Game-v1.1.AppImage", "Game-v1.2.AppImage"] {
        put(&dir.join(n), &elf(n));
    }
    let plan = plan(&dir, &pending(&["Game-v1.0.AppImage"], &["Game-v1.1.AppImage", "Game-v1.2.AppImage"]), &known());
    assert!(plan.moves.is_empty());
    assert_eq!(kept_names(&plan), ["Game-v1.0.AppImage"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_versioned_release_folder_moves_only_when_it_holds_the_release_alone() {
    let dir = temp("plan_folders");
    put(&dir.join("Game-v1.0").join("game"), &elf("old"));
    put(&dir.join("Game-v1.0").join("lib").join("libgame.so"), b"lib");
    put(&dir.join("Game-v1.1").join("game"), &elf("new"));
    let plan_ok = plan(&dir, &pending(&["Game-v1.0"], &["Game-v1.1"]), &known());
    assert_eq!(moved_names(&plan_ok), [("Game-v1.0", "Game-v1.1")]);
    assert!(plan_ok.moves[0].dir);

    let keep_files = [
        ("saves/slot.sav", b"s".as_slice()),
        ("config.json", b"{}".as_slice()),
        ("assets/game.o2r", b"PK".as_slice()),
        ("mods/a.zip", b"m".as_slice()),
        ("prod.keys", b"k".as_slice()),
        ("rom.bin", b"r".as_slice()),
        ("data/game.dat", b"d".as_slice()),
        ("dump.zip", b"PK".as_slice()),
        ("dump.7z", b"7z".as_slice()),
        ("disc.img", b"i".as_slice()),
        ("disc.iso", b"i".as_slice()),
        ("disc.chd", b"c".as_slice()),
        ("game.wad", b"w".as_slice()),
        ("card.mcr", b"m".as_slice()),
        ("state.srm", b"s".as_slice()),
        ("user.db", b"d".as_slice()),
    ];
    for (extra, data) in keep_files {
        let keep = dir.join("keep");
        let _ = fs::remove_dir_all(&keep);
        put(&keep.join("Game-v1.0").join("game"), &elf("old"));
        put(&keep.join("Game-v1.0").join(extra), data);
        put(&keep.join("Game-v1.1").join("game"), &elf("new"));
        let plan = plan(&keep, &pending(&["Game-v1.0"], &["Game-v1.1"]), &known());
        assert!(plan.moves.is_empty(), "{extra}");
        assert_eq!(kept_names(&plan), ["Game-v1.0"], "{extra}");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_versioned_folder_without_a_program_is_not_a_release_folder() {
    let dir = temp("plan_no_program");
    put(&dir.join("Game-v1.0").join("lib").join("libgame.so"), b"lib");
    put(&dir.join("Game-v1.0").join("readme.txt"), b"r");
    put(&dir.join("Game-v1.1").join("game"), &elf("new"));
    let plan = plan(&dir, &pending(&["Game-v1.0"], &["Game-v1.1"]), &known());
    assert!(plan.moves.is_empty());
    assert_eq!(plan.kept, [("Game-v1.0".to_string(), "it holds no program".to_string())]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn without_a_known_save_folder_folders_stay_and_program_files_still_move() {
    let dir = temp("plan_no_saves");
    put(&dir.join(OLD), &elf("old"));
    put(&dir.join(NEW), &elf("new"));
    put(&dir.join("Game-v1.0").join("game"), &elf("old"));
    put(&dir.join("Game-v1.1").join("game"), &elf("new"));
    let no_save_field = save_info(&port(None), &dir);
    assert!(!no_save_field.folders_may_move);
    let plan = plan(&dir, &pending(&[OLD, "Game-v1.0"], &[NEW, "Game-v1.1"]), &no_save_field);
    assert_eq!(moved_names(&plan), [(OLD, NEW)]);
    assert_eq!(kept_names(&plan), ["Game-v1.0"]);
    assert!(plan.kept[0].1.contains("save folder is not known"), "{:?}", plan.kept);
    let _ = fs::remove_dir_all(&dir);
}

// Linux only: `%APPDATA%` is set on Windows, so there the same value is a known folder.
#[cfg(target_os = "linux")]
#[test]
fn a_save_folder_unknown_on_linux_keeps_folders_in_place() {
    let dir = temp("plan_unknown_on_linux");
    put(&dir.join(OLD), &elf("old"));
    put(&dir.join(NEW), &elf("new"));
    put(&dir.join("Game-v1.0").join("game"), &elf("old"));
    put(&dir.join("Game-v1.1").join("game"), &elf("new"));
    let mut p = port(Some("saves"));
    p.save2 = Some(serde_json::json!("%APPDATA%\\TwilitRealm"));
    let info = save_info(&p, &dir);
    assert_eq!(info.folders, [dir.join("saves")], "the known one still protects its folder");
    assert!(!info.folders_may_move, "one save field unknown here is enough to keep folders");
    let plan = plan(&dir, &pending(&[OLD, "Game-v1.0"], &[NEW, "Game-v1.1"]), &info);
    assert_eq!(moved_names(&plan), [(OLD, NEW)]);
    assert_eq!(kept_names(&plan), ["Game-v1.0"]);
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_release_folder_holding_a_link_stays() {
    let dir = temp("plan_link");
    put(&dir.join("Game-v1.0").join("game"), &elf("old"));
    std::os::unix::fs::symlink("/somewhere/rom.z64", dir.join("Game-v1.0").join("rom.z64")).unwrap();
    put(&dir.join("Game-v1.1").join("game"), &elf("new"));
    let plan = plan(&dir, &pending(&["Game-v1.0"], &["Game-v1.1"]), &known());
    assert!(plan.moves.is_empty());
    assert_eq!(kept_names(&plan), ["Game-v1.0"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_entry_named_by_the_save_field_or_holding_it_never_moves() {
    let dir = temp("plan_saves");
    put(&dir.join("Game-v1.0").join("slot.sav"), b"save");
    put(&dir.join("Game-v1.1").join("game"), &elf("new"));
    let p = port(Some("Game-v1.0"));
    let saves = save_info(&p, &dir);
    let plan_a = plan(&dir, &pending(&["Game-v1.0"], &["Game-v1.1"]), &saves);
    assert!(plan_a.moves.is_empty());
    assert_eq!(kept_names(&plan_a), ["Game-v1.0"]);

    let p = port(Some("Game-v1.0/data/slots"));
    let saves = save_info(&p, &dir);
    fs::create_dir_all(dir.join("Game-v1.0/data/slots")).unwrap();
    let plan_b = plan(&dir, &pending(&["Game-v1.0"], &["Game-v1.1"]), &saves);
    assert!(plan_b.moves.is_empty(), "a folder holding the save folder stays");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn when_the_saves_are_the_game_folder_program_files_still_move_but_folders_stay() {
    let dir = temp("plan_saves_dot");
    put(&dir.join(OLD), &elf("old"));
    put(&dir.join(NEW), &elf("new"));
    put(&dir.join("Game-v1.0").join("game"), &elf("old"));
    put(&dir.join("Game-v1.1").join("game"), &elf("new"));
    let saves = save_info(&port(Some(".")), &dir);
    let plan = plan(&dir, &pending(&[OLD, "Game-v1.0"], &[NEW, "Game-v1.1"]), &saves);
    assert_eq!(moved_names(&plan), [(OLD, NEW)]);
    assert_eq!(kept_names(&plan), ["Game-v1.0"]);
    let _ = fs::remove_dir_all(&dir);
}

// -- whole updates -----------------------------------------------------------------------------

#[test]
fn a_versioned_update_moves_the_old_file_and_points_favorite_and_steam_at_the_new_one() {
    let w = World::new("e2e");
    let p = port(None);
    put(&w.game.join(OLD), &elf("old"));
    put(&w.game.join("Save").join("slot1.sav"), b"precious");
    put(&w.game.join("game.AppImage"), &elf("my own copy"));
    w.shortcut_to(OLD);

    w.install(&p, NEW, &elf("new")).unwrap();
    let out = w.finish(&p, Some(OLD), &no_steam, false);

    assert_eq!(out.older_bytes, 0, "a first update has no older versions to mention");
    assert_eq!(fs::read(w.previous("v1.0", OLD)).unwrap(), elf("old"), "moved, never deleted");
    assert!(!w.game.join(OLD).exists());
    assert_eq!(fs::read(w.game.join(NEW)).unwrap(), elf("new"));
    assert_eq!(fs::read(w.game.join("Save").join("slot1.sav")).unwrap(), b"precious");
    assert_eq!(fs::read(w.game.join("game.AppImage")).unwrap(), elf("my own copy"), "a user's copy stays");
    assert_eq!(out.new_favorite.as_deref(), Some(NEW));
    assert_eq!(out.target.as_deref(), Some(NEW));
    let s = w.shortcuts();
    assert_eq!(s[0].exe_path(), w.game.join(NEW).to_str().unwrap());
    assert_eq!(s[0].app_id, Some(APP_ID), "same app ID, so art and play time stay");
    assert_eq!(s[0].start_dir_path(), w.game.to_str().unwrap());
    assert_eq!(s[1].exe_path(), "/usr/bin/chiaki", "other shortcuts untouched");
    assert_eq!(crate::fork::backups::list_backups(&w.vdf).unwrap().len(), 1, "shortcuts.vdf backed up first");
    let text = out.summary("Game", "game", Some("v1.1")).unwrap();
    assert!(
        text.starts_with("Game is now v1.1. The version you had (v1.0) is kept in a hidden folder (.previous) inside the game folder; to go back, use Info > Select version."),
        "{text}"
    );
    assert!(text.contains("Steam shortcut now starts the new version"), "{text}");
}

#[cfg(unix)]
fn chmod_x(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

// Linux only: without an extension a program is found by its executable bit, which Windows lacks.
#[cfg(unix)]
#[test]
fn a_program_without_extension_and_no_favorite_is_found_again_by_the_app() {
    let w = World::new("e2e_elf");
    let p = port(None);
    put(&w.game.join("Game-v1.0-linux-x86_64"), &elf("old"));
    chmod_x(&w.game.join("Game-v1.0-linux-x86_64"));
    let download = w.root.join("update.zip");
    // The release is an archive holding the program and a readme.
    put(&w.root.join("pack").join("Game-v1.1-linux-x86_64"), &elf("new"));
    chmod_x(&w.root.join("pack").join("Game-v1.1-linux-x86_64"));
    put(&w.root.join("pack").join("README.txt"), b"r");
    let tool = crate::core::installer::sevenzip_exe_path().expect("7-Zip next to the test binary");
    let status = std::process::Command::new(tool).arg("a").arg("-tzip").arg(&download).arg(".").current_dir(w.root.join("pack")).output().unwrap();
    assert!(status.status.success());
    let saves = w.root.join("Saves Backup");
    apply_archive(&p, InstallPaths { library_dir: &w.library, saves_backup_dir: &saves }, &download, &w.game, "2026-10-24_12-00-00", &mut |_| {}).unwrap();

    let out = w.finish(&p, None, &no_steam, false);

    assert!(w.previous("v1.0", "Game-v1.0-linux-x86_64").is_file());
    assert!(w.game.join("Game-v1.1-linux-x86_64").is_file());
    assert_eq!(out.target.as_deref(), Some("Game-v1.1-linux-x86_64"));
    assert_eq!(out.new_favorite, None, "the app finds the only program itself; no favorite is set");
    assert_eq!(crate::core::executable_detect::autodetect_executable(&w.game).unwrap(), w.game.join("Game-v1.1-linux-x86_64"));
}

#[test]
fn while_steam_runs_the_repoint_is_queued_and_the_old_file_moves_once_it_is_applied() {
    let w = World::new("e2e_queue");
    let p = port(None);
    put(&w.game.join(OLD), &elf("old"));
    w.shortcut_to(OLD);
    let vdf_before = fs::read(&w.vdf).unwrap();

    w.install(&p, NEW, &elf("new")).unwrap();
    let out = w.finish(&p, Some(OLD), &steam_up, false);

    assert_eq!(fs::read(&w.vdf).unwrap(), vdf_before, "shortcuts.vdf is never written while Steam runs");
    assert_eq!(out.steam.queued.len(), 1);
    assert!(w.game.join(OLD).is_file(), "Steam still starts the old file until the change is applied");
    assert_eq!(out.new_favorite.as_deref(), Some(NEW), "the app starts the new one meanwhile");
    let queue_file = steam_queue::queue_path(&w.app);
    let queued = steam_queue::load(&queue_file).queue;
    assert_eq!(queued.entries.len(), 1);
    assert_eq!(queued.entries[0].kind, QueuedKind::Repoint);
    assert_eq!(out.waiting, [OLD]);
    let text = out.summary("Game", "game", Some("v1.1")).unwrap();
    assert!(text.starts_with("Game is now v1.1.\nSteam is running, so the shortcut change waits. Close Steam (or come back to Desktop Mode) and this app will ask to apply it; until then Steam starts the old version, which stays in place."), "{text}");
    assert!(!text.contains("left where it is") && !text.contains(OLD), "no duplicate line, no file names: {text}");

    // Desktop Mode, Steam closed: the queue is applied, the old file moves.
    let catalog = [p.clone()];
    let inputs = ApplyInputs { home: &w.home, library_dir: &w.library, catalog: &catalog, favorite_exe: &|_| None, cache_dir: None, settings: &w.settings, game_mode: false };
    let applied = steam_queue::apply_queue_file(&queue_file, &inputs, &no_steam, SystemTime::now());
    assert!(applied.wrote && !applied.refused, "{}", applied.text);
    assert_eq!(w.shortcuts()[0].exe_path(), w.game.join(NEW).to_str().unwrap());
    assert_eq!(w.shortcuts()[0].app_id, Some(APP_ID));
    assert!(w.previous("v1.0", OLD).is_file(), "{}", applied.text);
    assert!(!w.game.join(OLD).exists());
    assert!(applied.text.contains("now kept in .previous"), "{}", applied.text);
}

#[test]
fn in_game_mode_nothing_is_written_and_the_change_is_queued() {
    let w = World::new("e2e_gamemode");
    let p = port(None);
    put(&w.game.join(OLD), &elf("old"));
    w.shortcut_to(OLD);
    let vdf_before = fs::read(&w.vdf).unwrap();
    w.install(&p, NEW, &elf("new")).unwrap();
    let out = w.finish(&p, Some(OLD), &no_steam, true);
    assert_eq!(fs::read(&w.vdf).unwrap(), vdf_before);
    assert_eq!(out.steam.queued.len(), 1);
    assert!(w.game.join(OLD).is_file());
}

#[test]
fn an_update_that_fails_halfway_leaves_everything_as_it_was() {
    let w = World::new("e2e_fail");
    let p = port(None);
    put(&w.game.join(OLD), &elf("old"));
    w.shortcut_to(OLD);
    let vdf_before = fs::read(&w.vdf).unwrap();

    assert!(w.install(&p, "Game-v1.1-linux-x86_64.zip", b"not a real zip").is_err());
    let out = w.finish(&p, Some(OLD), &no_steam, false);

    assert!(out.is_empty() && out.new_favorite.is_none(), "{out:?}");
    assert_eq!(fs::read(w.game.join(OLD)).unwrap(), elf("old"));
    assert!(!w.game.join(PREVIOUS).exists());
    assert_eq!(fs::read(&w.vdf).unwrap(), vdf_before);
}

#[test]
fn when_the_new_program_is_not_where_the_old_one_was_nothing_moves() {
    let w = World::new("e2e_unverified");
    let p = port(Some("saves"));
    put(&w.game.join("Game-v1.0").join("bin").join("game"), &elf("old"));
    put(&w.game.join("Game-v1.1").join("game"), &elf("new"));
    w.shortcut_to("Game-v1.0/bin/game");
    let vdf_before = fs::read(&w.vdf).unwrap();
    let pend = Pending { before: Before { entries: vec!["Game-v1.0".into()], target: None }, release: vec!["Game-v1.1".into()] };
    let ctx = SteamCtx { home: &w.home, config_dir: &w.app, settings: &w.settings, steam: &no_steam, game_mode: false, now: SystemTime::now() };

    let out = finish_with(&FinishInput { port: &p, game_dir: &w.game, favorite: Some("Game-v1.0/bin/game"), steam: Some(&ctx) }, &pend);

    assert!(out.moved.is_empty());
    assert!(w.game.join("Game-v1.0").join("bin").join("game").is_file());
    assert_eq!(out.new_favorite, None);
    assert_eq!(fs::read(&w.vdf).unwrap(), vdf_before);
    let text = out.summary("Game", "game", None).unwrap();
    assert!(text.contains("The old version (v1.0) was left where it is: the new version's program was not found where expected"), "{text}");
}

#[test]
fn a_release_folder_update_repoints_program_and_start_folder() {
    let w = World::new("e2e_folder");
    let p = port(Some("saves"));
    put(&w.game.join("Game-v1.0").join("game"), &elf("old"));
    put(&w.game.join("Game-v1.1").join("game"), &elf("new"));
    let exe = w.game.join("Game-v1.0").join("game");
    let entries = [steam_entry(0, APP_ID, "Game", &format!("\"{}\"", exe.display()), &format!("\"{}\"", w.game.join("Game-v1.0").display()), &[])];
    fs::write(&w.vdf, file_of(&entries)).unwrap();
    let pend = Pending { before: Before { entries: vec!["Game-v1.0".into()], target: Some("Game-v1.0/game".into()) }, release: vec!["Game-v1.1".into()] };
    let ctx = SteamCtx { home: &w.home, config_dir: &w.app, settings: &w.settings, steam: &no_steam, game_mode: false, now: SystemTime::now() };

    let out = finish_with(&FinishInput { port: &p, game_dir: &w.game, favorite: None, steam: Some(&ctx) }, &pend);

    assert!(w.previous("v1.0", "Game-v1.0").join("game").is_file());
    assert_eq!(out.target.as_deref(), Some("Game-v1.1/game"));
    let s = w.shortcuts();
    assert_eq!(s[0].exe_path(), w.game.join("Game-v1.1").join("game").to_str().unwrap());
    assert_eq!(s[0].start_dir_path(), w.game.join("Game-v1.1").to_str().unwrap());
    assert_eq!(s[0].app_id, Some(APP_ID));
}

#[test]
fn a_shortcut_without_an_app_id_keeps_the_old_file_in_place() {
    let w = World::new("e2e_no_id");
    let p = port(None);
    put(&w.game.join(OLD), &elf("old"));
    let exe = format!("\"{}\"", w.game.join(OLD).display());
    fs::write(&w.vdf, file_of(&[object("0", &[str_field("AppName", b"Game"), str_field("Exe", exe.as_bytes()), str_field("StartDir", b"\"/x\"")])])).unwrap();
    w.install(&p, NEW, &elf("new")).unwrap();
    let out = w.finish(&p, Some(OLD), &no_steam, false);
    assert!(w.game.join(OLD).is_file(), "the shortcut would break otherwise");
    assert_eq!(out.new_favorite.as_deref(), Some(NEW), "the app starts the new one");
    assert!(out.kept.iter().any(|(n, why)| n == OLD && why.contains("still points")), "{:?}", out.kept);
}

#[test]
fn a_launcher_script_naming_the_old_file_is_updated_and_its_original_kept() {
    let w = World::new("e2e_script");
    let p = port(None);
    put(&w.game.join(OLD), &elf("old"));
    let script = format!("#!/bin/sh\ncd \"$(dirname \"$0\")\"\nexec ./{OLD} \"$@\"\n");
    put(&w.game.join("launch.sh"), script.as_bytes());
    w.install(&p, NEW, &elf("new")).unwrap();

    let out = w.finish(&p, Some("launch.sh"), &no_steam, false);

    assert_eq!(fs::read_to_string(w.game.join("launch.sh")).unwrap(), script.replace(OLD, NEW));
    assert_eq!(fs::read_to_string(w.previous("v1.0", "launch.sh.before-update")).unwrap(), script);
    assert_eq!(out.scripts, ["launch.sh"]);
    assert_eq!(out.new_favorite, None, "the favorite is the script, which still exists");
    assert!(w.previous("v1.0", OLD).is_file());
}

#[test]
fn every_previous_version_is_kept_and_the_summary_says_how_much_room_the_older_ones_use() {
    let w = World::new("e2e_keep_all");
    let p = port(None);
    put(&w.game.join(OLD), &elf("v1.0"));
    put(&w.previous("v0.9", "Game-v0.9-linux-x86_64.AppImage"), &elf("v0.9"));
    put(&w.previous("v0.8", "Game-v0.8-linux-x86_64.AppImage"), &elf("v0.8"));
    put(&w.previous("v0.8", "settings.ini"), b"[mine]");
    w.install(&p, NEW, &elf("v1.1")).unwrap();

    let out = w.finish(&p, None, &no_steam, false);

    assert!(w.previous("v1.0", OLD).is_file());
    assert_eq!(fs::read(w.previous("v0.9", "Game-v0.9-linux-x86_64.AppImage")).unwrap(), elf("v0.9"), "nothing is ever deleted");
    assert_eq!(fs::read(w.previous("v0.8", "Game-v0.8-linux-x86_64.AppImage")).unwrap(), elf("v0.8"));
    assert_eq!(fs::read(w.previous("v0.8", "settings.ini")).unwrap(), b"[mine]");
    let older = (elf("v0.9").len() + elf("v0.8").len() + b"[mine]".len()) as u64;
    assert_eq!(out.older_bytes, older, "v0.9 and v0.8, not the v1.0 this update kept");
    let text = out.summary("Game", "game", Some("v1.1")).unwrap();
    assert!(
        text.contains("Older versions in .previous use 1 MB (in Desktop Mode's file manager, Ctrl+H shows hidden folders); remove them from the port's folder if you need the room."),
        "{text}"
    );
}

#[test]
fn going_back_with_select_version_keeps_the_newer_one_in_previous() {
    let w = World::new("e2e_rollback");
    let p = port(None);
    put(&w.game.join(OLD), &elf("v1.0"));
    w.shortcut_to(OLD);
    w.install(&p, NEW, &elf("v1.1")).unwrap();
    let first = w.finish(&p, Some(OLD), &no_steam, false);
    assert_eq!(first.new_favorite.as_deref(), Some(NEW));

    // Info > Select version installs the older release again.
    w.install(&p, OLD, &elf("v1.0 again")).unwrap();
    let back = w.finish(&p, Some(NEW), &no_steam, false);

    assert_eq!(fs::read(w.game.join(OLD)).unwrap(), elf("v1.0 again"));
    assert!(w.previous("v1.1", NEW).is_file());
    assert_eq!(back.new_favorite.as_deref(), Some(OLD));
    assert_eq!(w.shortcuts()[0].exe_path(), w.game.join(OLD).to_str().unwrap());
    assert_eq!(w.shortcuts()[0].app_id, Some(APP_ID));
    assert_eq!(fs::read(w.previous("v1.0", OLD)).unwrap(), elf("v1.0"), "the earlier copy of v1.0 stays too");
    assert!(back.older_bytes > 0);
    let text = back.summary("Game", "game", Some("v1.0")).unwrap();
    assert!(text.starts_with("Game is now v1.0. The version you had (v1.1) is kept in a hidden folder (.previous)"), "{text}");
}

#[test]
fn the_same_version_twice_gets_its_own_previous_folder() {
    let w = World::new("e2e_same_version");
    let p = port(None);
    put(&w.game.join(OLD), &elf("old"));
    put(&w.previous("v1.0", "notes.txt"), b"kept by the user");
    w.install(&p, NEW, &elf("new")).unwrap();
    let out = w.finish(&p, None, &no_steam, false);
    assert_eq!(out.moved[0].1, "v1.0-2");
    assert!(w.previous("v1.0-2", OLD).is_file());
    assert_eq!(fs::read(w.previous("v1.0", "notes.txt")).unwrap(), b"kept by the user");
}

#[test]
fn a_plain_update_with_unchanged_names_says_nothing() {
    let w = World::new("e2e_plain");
    let p = port(None);
    put(&w.game.join("game.AppImage"), &elf("old"));
    w.install(&p, "game.AppImage", &elf("new")).unwrap();
    let out = w.finish(&p, None, &no_steam, false);
    assert!(out.is_empty());
    assert_eq!(out.summary("Game", "game", Some("v2")), None);
}

#[test]
fn a_queued_port_move_is_not_mistaken_for_a_version_change() {
    let w = World::new("hook_move");
    let other = w.root.join("sd").join("Ports").join("game");
    put(&other.join(OLD), &elf("x"));
    put(&w.game.join(OLD), &elf("x"));
    assert_eq!(after_queued_repoint(&w.game, &other, &w.library, &[port(None)], &w.home), None);
    assert_eq!(after_queued_repoint(&w.game.join(OLD), &other.join(OLD), &w.library, &[port(None)], &w.home), None);
    assert!(w.game.join(OLD).is_file());
}

#[test]
fn sizes_read_in_megabytes_and_above_1000_mb_in_gigabytes() {
    assert_eq!(size_text(1), "1 MB");
    assert_eq!(size_text(999_000_000), "999 MB");
    assert_eq!(size_text(1_000_000_000), "1000 MB");
    assert_eq!(size_text(1_260_000_000), "1.3 GB");
}

#[test]
fn script_names_are_replaced_only_where_they_name_an_entry_of_the_port_folder() {
    let old = "Game-v1.0";
    let new = "Game-v1.1";
    for (line, want) in [
        ("exec ./Game-v1.0/game \"$@\"", Some("exec ./Game-v1.1/game \"$@\"")),
        ("cd Game-v1.0 && ./game", Some("cd Game-v1.1 && ./game")),
        ("\"$HERE/Game-v1.0/game\"", Some("\"$HERE/Game-v1.1/game\"")),
        ("\"${DIR}/Game-v1.0/game\"", Some("\"${DIR}/Game-v1.1/game\"")),
        ("\"$(dirname \"$0\")\"/Game-v1.0/game", Some("\"$(dirname \"$0\")\"/Game-v1.1/game")),
        ("BIN=Game-v1.0", Some("BIN=Game-v1.1")),
        ("SAVES=./Game-v1.0-saves", None),
        ("CONF=~/.local/share/Game-v1.0", None),
        ("cp Game-v1.0.cfg /tmp", None),
        ("MyGame-v1.0", None),
        ("/Game-v1.0/game", None),
        ("exec /home/deck/Applications/dpm/Library/game/Game-v1.0/game", Some("exec /home/deck/Applications/dpm/Library/game/Game-v1.1/game")),
        ("\"$HOME/Ports/game/Game-v1.0/game\"", Some("\"$HOME/Ports/game/Game-v1.1/game\"")),
        ("/home/deck/other/Game-v1.0/game", None),
    ] {
        assert_eq!(replace_names(line, old, new, "game").as_deref(), want, "{line}");
    }
}

#[test]
fn a_launcher_script_keeps_its_other_lines_when_its_program_moves() {
    let w = World::new("e2e_script_bounds");
    let p = port(None);
    put(&w.game.join(OLD), &elf("old"));
    let script = format!("#!/bin/sh\nCONF=~/.local/share/{OLD}\ncp ./{OLD}.cfg /tmp\nexec ./{OLD}\n");
    put(&w.game.join("launch.sh"), script.as_bytes());
    w.install(&p, NEW, &elf("new")).unwrap();
    // No favorite: the script is not the known launch target, so the launcher guard does not apply.
    w.finish(&p, None, &no_steam, false);
    assert_eq!(fs::read_to_string(w.game.join("launch.sh")).unwrap(), format!("#!/bin/sh\nCONF=~/.local/share/{OLD}\ncp ./{OLD}.cfg /tmp\nexec ./{NEW}\n"));
}

#[test]
fn a_launcher_script_is_updated_when_the_queued_steam_change_moves_the_old_file() {
    let w = World::new("e2e_script_queue");
    let p = port(None);
    put(&w.game.join(OLD), &elf("old"));
    put(&w.game.join("launch.sh"), format!("#!/bin/sh\nexec ./{OLD}\n").as_bytes());
    w.shortcut_to(OLD);
    w.install(&p, NEW, &elf("new")).unwrap();

    let out = w.finish(&p, Some("launch.sh"), &steam_up, false);
    assert_eq!(out.waiting, [OLD]);
    assert_eq!(fs::read_to_string(w.game.join("launch.sh")).unwrap(), format!("#!/bin/sh\nexec ./{OLD}\n"), "the old file is still there, so the script still works");

    let catalog = [p.clone()];
    let inputs = ApplyInputs { home: &w.home, library_dir: &w.library, catalog: &catalog, favorite_exe: &|_| None, cache_dir: None, settings: &w.settings, game_mode: false };
    let applied = steam_queue::apply_queue_file(&steam_queue::queue_path(&w.app), &inputs, &no_steam, SystemTime::now());
    assert!(w.previous("v1.0", OLD).is_file(), "{}", applied.text);
    assert_eq!(fs::read_to_string(w.game.join("launch.sh")).unwrap(), format!("#!/bin/sh\nexec ./{NEW}\n"));
    assert!(w.previous("v1.0", "launch.sh.before-update").is_file());
    assert!(applied.text.contains("launch.sh now names the new version"), "{}", applied.text);
}

#[test]
fn the_queued_repoint_hook_leaves_everything_that_is_not_a_versioned_pair_alone() {
    let w = World::new("hook_guards");
    let p = port(None);
    put(&w.game.join(OLD), &elf("old"));
    put(&w.game.join(NEW), &elf("new"));
    put(&w.game.join(".Game-v0.9.AppImage"), &elf("hidden"));
    put(&w.game.join("Other-v1.1.AppImage"), &elf("other"));
    let cat = [p.clone()];
    let hook = |from: &str, to: &str, catalog: &[Port]| after_queued_repoint(&w.game.join(from), &w.game.join(to), &w.library, catalog, &w.home);
    assert_eq!(hook(OLD, OLD, &cat), None, "same file");
    assert_eq!(hook(OLD, "Game-v1.2.AppImage", &cat), None, "the new file is missing");
    assert_eq!(hook(".Game-v0.9.AppImage", NEW, &cat), None, "hidden");
    assert_eq!(hook(OLD, "Other-v1.1.AppImage", &cat), None, "another family");
    assert_eq!(hook(OLD, NEW, &[]), None, "a port the catalog does not know");
    w.shortcut_to(OLD);
    assert_eq!(hook(OLD, NEW, &cat), None, "a shortcut still points at it");
    assert!(w.game.join(OLD).is_file());
    fs::remove_file(&w.vdf).unwrap();
    assert!(hook(OLD, NEW, &cat).unwrap().contains("the old version (v1.0) is now kept in .previous/v1.0"));
    assert!(w.previous("v1.0", OLD).is_file());
}

#[test]
fn two_updates_while_steam_runs_chain_into_one_change() {
    let w = World::new("e2e_chain");
    let p = port(None);
    let v12 = "Game-v1.2-linux-x86_64.AppImage";
    put(&w.game.join(OLD), &elf("v1.0"));
    w.shortcut_to(OLD);
    w.install(&p, NEW, &elf("v1.1")).unwrap();
    let first = w.finish(&p, Some(OLD), &steam_up, false);
    assert_eq!(first.waiting, [OLD]);

    w.install(&p, v12, &elf("v1.2")).unwrap();
    let second = w.finish(&p, Some(NEW), &steam_up, false);
    assert!(w.previous("v1.1", NEW).is_file(), "nothing points at v1.1, so it moves at once");
    assert_eq!(second.waiting, [OLD], "v1.0 waits for Steam");
    assert_eq!(second.new_favorite.as_deref(), Some(v12));

    let catalog = [p.clone()];
    let inputs = ApplyInputs { home: &w.home, library_dir: &w.library, catalog: &catalog, favorite_exe: &|_| None, cache_dir: None, settings: &w.settings, game_mode: false };
    let applied = steam_queue::apply_queue_file(&steam_queue::queue_path(&w.app), &inputs, &no_steam, SystemTime::now());
    assert!(applied.wrote && !applied.refused, "{}", applied.text);
    assert_eq!(w.shortcuts()[0].exe_path(), w.game.join(v12).to_str().unwrap(), "A to B, then B to C, ends at C");
    assert_eq!(w.shortcuts()[0].app_id, Some(APP_ID));
    assert!(w.previous("v1.0", OLD).is_file(), "{}", applied.text);
    assert!(!steam_queue::queue_path(&w.app).exists(), "the queue is empty now");
}

#[test]
fn a_change_that_cancels_out_in_the_queue_leaves_nothing_waiting() {
    let w = World::new("e2e_cancel");
    let p = port(None);
    put(&w.game.join(OLD), &elf("v1.0"));
    w.shortcut_to(OLD);
    w.install(&p, NEW, &elf("v1.1")).unwrap();
    let first = w.finish(&p, Some(OLD), &steam_up, false);
    assert_eq!(first.waiting, [OLD]);
    // Back to v1.0 before Steam closes: the queued v1.0 -> v1.1 change cancels out.
    w.install(&p, OLD, &elf("v1.0 again")).unwrap();
    let back = w.finish(&p, Some(NEW), &steam_up, false);
    assert!(back.waiting.is_empty(), "{back:?}");
    assert!(!back.summary("Game", "game", Some("v1.0")).unwrap().contains("shortcut change waits"));
    assert!(w.previous("v1.1", NEW).is_file());
    assert_eq!(w.shortcuts()[0].exe_path(), w.game.join(OLD).to_str().unwrap(), "the shortcut never left v1.0");
    assert!(!steam_queue::queue_path(&w.app).exists(), "the queued v1.0 -> v1.1 change is gone: applied later it would point Steam into .previous");
}

#[test]
fn a_launcher_script_with_a_full_path_into_the_port_folder_is_updated() {
    let w = World::new("e2e_script_full_path");
    let p = port(None);
    put(&w.game.join(OLD), &elf("old"));
    let script = format!("#!/bin/sh\nexec \"{}\"\n", w.game.join(OLD).display());
    put(&w.game.join("launch.sh"), script.as_bytes());
    w.install(&p, NEW, &elf("new")).unwrap();
    let out = w.finish(&p, Some("launch.sh"), &no_steam, false);
    assert!(w.previous("v1.0", OLD).is_file(), "{:?}", out.kept);
    assert_eq!(fs::read_to_string(w.game.join("launch.sh")).unwrap(), script.replace(OLD, NEW));
}

#[test]
fn a_launcher_in_a_subfolder_that_names_the_old_files_keeps_them_in_place() {
    let w = World::new("e2e_launcher_guard");
    let p = port(None);
    let arm_old = "Game-v1.0-linux-aarch64.AppImage";
    let arm_new = "Game-v1.1-linux-aarch64.AppImage";
    put(&w.game.join(OLD), &elf("old"));
    put(&w.game.join(arm_old), &elf("old arm"));
    let script = format!("#!/bin/sh\ncd \"$(dirname \"$0\")/..\"\n[ \"$(uname -m)\" = aarch64 ] && exec ./{arm_old}\nexec ./{OLD}\n");
    put(&w.game.join("bin").join("run.sh"), script.as_bytes());
    let download = w.root.join("pack");
    put(&download.join(NEW), &elf("new"));
    put(&download.join(arm_new), &elf("new arm"));
    let archive = w.root.join("Game-v1.1.zip");
    let tool = crate::core::installer::sevenzip_exe_path().expect("7-Zip next to the test binary");
    assert!(std::process::Command::new(tool).arg("a").arg("-tzip").arg(&archive).arg(".").current_dir(&download).output().unwrap().status.success());
    let saves = w.root.join("Saves Backup");
    apply_archive(&p, InstallPaths { library_dir: &w.library, saves_backup_dir: &saves }, &archive, &w.game, "2026-10-24_12-00-00", &mut |_| {}).unwrap();

    let out = w.finish(&p, Some("bin/run.sh"), &no_steam, false);

    assert!(out.moved.is_empty(), "{:?}", out.moved);
    assert!(w.game.join(OLD).is_file() && w.game.join(arm_old).is_file(), "the script would no longer start the game otherwise");
    assert_eq!(fs::read_to_string(w.game.join("bin").join("run.sh")).unwrap(), script, "a subfolder script is never rewritten");
    assert_eq!(out.new_favorite, None);
    let text = out.summary("Game", "game", Some("v1.1")).unwrap();
    let line = "The old version (v1.0) was left where it is: the launcher bin/run.sh still names it.";
    assert_eq!(text.matches(line).count(), 1, "two files, one line: {text}");
}

#[test]
fn queued_repoints_are_matched_by_path_whichever_port_queued_them() {
    let w = World::new("retarget_paths");
    let queue = steam_queue::queue_path(&w.app);
    let entry = |port_key: &str, from: &str, to: &str| steam_queue::QueuedChange {
        account_id: 1234,
        account_label: "Steam user".into(),
        port_key: port_key.into(),
        name: "Game".into(),
        kind: QueuedKind::Repoint,
        repoint_by_name: false,
        art: steam_queue::ArtChoice::None,
        from_game_mode: false,
        queued_at: "2026-10-24_12-00-00".into(),
        repoint: Some((from.into(), to.into())),
    };
    let a = w.game.join("Game-v1.0").to_string_lossy().into_owned();
    let b = w.game.join("Game-v1.1");
    steam_queue::save(&queue, &steam_queue::SteamQueue { entries: vec![entry("someone/else", &a, &format!("{}/", b.display()))] }).unwrap();
    let c = w.game.join("Game-v1.2");
    assert_eq!(steam_queue::retarget_repoints(&queue, &b, &c), Ok(1), "a trailing slash is the same path, and the port key does not matter");
    let entries = steam_queue::load(&queue).queue.entries;
    assert_eq!(entries[0].repoint.as_ref().map(|(_, t)| PathBuf::from(t)), Some(c));
}
