//! Generated fixtures only: every home, game folder and save here is made by the test in a temp folder.

use super::search::{self, Input, Place};
use super::*;
use crate::core::models::port_from_value;
use crate::core::save_backup::{backup_before_update, preserve_all_before_uninstall, save_folder_unknown_on_linux, unknown_on_linux_fields};
use serde_json::json;
use std::cell::Cell;
use std::time::{Duration, SystemTime};

struct Fx {
    _dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    library: PathBuf,
    saves_backup: PathBuf,
}

impl Fx {
    fn new() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let (home, library) = (root.join("home"), root.join("home/Ports"));
        fs::create_dir_all(&library).unwrap();
        Fx { saves_backup: root.join("Saves Backup"), home, library, root, _dir: dir }
    }

    fn game_dir(&self, port: &Port) -> PathBuf {
        let dir = self.library.join(&port.folder);
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}

fn port(name: &str, folder: &str, save: Option<Value>) -> Port {
    let mut v = json!({"name": name, "folder": folder, "source": "https://example.com/x.zip"});
    if let Some(s) = save {
        v["save"] = s;
    }
    port_from_value(&v).unwrap()
}

fn put(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn run(fx: &Fx, p: &Port, catalog: &[Port], steam_roots: &[PathBuf]) -> search::Found {
    let game_dir = fx.library.join(&p.folder);
    let input = Input { port: p, catalog, game_dir: &game_dir, library_dir: &fx.library, saves_backup_dir: &fx.saves_backup, home: &fx.home, steam_roots };
    search::find(&input, &|| false, &mut |_| {})
}

// -- State ---------------------------------------------------------------------------------------------

#[test]
fn state_found_empty_missing_and_unknown() {
    let fx = Fx::new();
    let found = port("Found", "found", Some(json!("saves")));
    let gd = fx.game_dir(&found);
    put(&gd.join("saves/slot1.sav"), b"x");
    assert_eq!(state_of(&found, &gd, None), SaveState::Found);

    let empty = port("Empty", "empty", Some(json!("saves")));
    let gd = fx.game_dir(&empty);
    fs::create_dir_all(gd.join("saves/deeper")).unwrap();
    assert_eq!(state_of(&empty, &gd, None), SaveState::Empty);

    let missing = port("Missing", "missing", Some(json!("saves")));
    assert_eq!(state_of(&missing, &fx.game_dir(&missing), None), SaveState::Missing);

    let none = port("None", "none", None);
    assert_eq!(state_of(&none, &fx.game_dir(&none), None), SaveState::Unknown);

    // Dusklight's case: the catalog names only a Windows path.
    let windows_only = port("Dusklight", "dusklight", Some(json!({"windows": "%APPDATA%\\Dusklight"})));
    let state = state_of(&windows_only, &fx.game_dir(&windows_only), None);
    assert_eq!(state, if cfg!(target_os = "linux") { SaveState::Unknown } else { state });
}

#[test]
fn the_game_folder_itself_is_not_a_save_folder() {
    let fx = Fx::new();
    let p = port("Dot", "dot", Some(json!(".")));
    let gd = fx.game_dir(&p);
    put(&gd.join("game.bin"), b"x");
    assert_eq!(state_of(&p, &gd, None), SaveState::Unknown);
}

#[test]
fn no_saves_override_wins_and_never_warns() {
    let fx = Fx::new();
    let p = port("Sev", "sev", None);
    let gd = fx.game_dir(&p);
    assert!(state_of(&p, &gd, None).warns(false));
    let state = state_of(&p, &gd, Some(&Override::NoSaves));
    assert_eq!(state, SaveState::NoSaves);
    assert!(!state.warns(true));
}

#[test]
fn badge_rule_empty_and_missing_need_a_played_game() {
    assert!(SaveState::Unknown.warns(false));
    assert!(!SaveState::Empty.warns(false) && SaveState::Empty.warns(true));
    assert!(!SaveState::Missing.warns(false) && SaveState::Missing.warns(true));
    assert!(!SaveState::Found.warns(true));
}

#[test]
fn has_files_looks_a_few_levels_down_and_ignores_empty_trees() {
    let fx = Fx::new();
    fs::create_dir_all(fx.root.join("a/b/c")).unwrap();
    assert!(!has_files(&fx.root.join("a")));
    put(&fx.root.join("a/b/c/f.sav"), b"x");
    assert!(has_files(&fx.root.join("a")));
    assert!(!has_files(&fx.root.join("nope")));
}

#[test]
fn info_line_for_each_state() {
    assert_eq!(info_line(SaveState::Found, None), None);
    assert_eq!(info_line(SaveState::Found, Some(&Override::Folder("x".into()))).as_deref(), Some("Saves: using the folder you chose"));
    assert!(info_line(SaveState::Unknown, None).unwrap().starts_with("Saves not found"));
    assert!(info_line(SaveState::NoSaves, Some(&Override::NoSaves)).unwrap().contains("none"));
}

// -- Overrides: stored, applied, cleared -----------------------------------------------------------------

#[test]
fn overrides_round_trip_keep_other_keys_and_clear() {
    let fx = Fx::new();
    let path = fx.root.join("fork-settings.json");
    fs::write(&path, r#"{"setup_done": true, "adopt_folders": []}"#).unwrap();
    let mut settings = ForkSettings::load(&path);
    set_override(&mut settings, "sev", &Override::NoSaves).unwrap();
    set_override(&mut settings, "dusk", &Override::Folder("%HOME%/.local/share/Dusk".into())).unwrap();

    let again = ForkSettings::load(&path);
    let map = overrides(&again);
    assert_eq!(map.get("sev"), Some(&Override::NoSaves));
    assert_eq!(map.get("dusk"), Some(&Override::Folder("%HOME%/.local/share/Dusk".into())));
    assert_eq!(again.get("setup_done"), Some(&json!(true)), "other keys are kept");

    let mut settings = ForkSettings::load(&path);
    clear_override(&mut settings, "sev").unwrap();
    clear_override(&mut settings, "never-set").unwrap();
    assert_eq!(overrides(&ForkSettings::load(&path)).len(), 1);
    clear_override(&mut settings, "dusk").unwrap();
    let last = ForkSettings::load(&path);
    assert!(last.get(KEY).is_none(), "the key goes with its last entry");
    assert_eq!(last.get("setup_done"), Some(&json!(true)));
}

#[test]
fn bad_entries_are_ignored() {
    let fx = Fx::new();
    let path = fx.root.join("fork-settings.json");
    fs::write(&path, r#"{"save_overrides": {"a": 5, "b": {"path": ""}, "c": {"no_saves": false}, "d": {"path": "x/y"}}}"#).unwrap();
    let map = overrides(&ForkSettings::load(&path));
    assert_eq!(map.len(), 1);
    assert_eq!(map.get("d"), Some(&Override::Folder("x/y".into())));
}

#[test]
fn apply_replaces_both_save_fields_or_empties_them() {
    let mut catalog = vec![port("A", "a", Some(json!("old"))), port("B", "b", Some(json!("old"))), port("C", "c", Some(json!("keep")))];
    catalog[0].save2 = Some(json!("old2"));
    let map = BTreeMap::from([("a".to_string(), Override::Folder("new".into())), ("b".to_string(), Override::NoSaves)]);
    apply_overrides(&mut catalog, &map);
    assert_eq!(catalog[0].save, Some(json!("new")));
    assert_eq!(catalog[0].save2, None);
    assert_eq!((catalog[1].save.clone(), catalog[1].save2.clone()), (None, None));
    assert_eq!(catalog[2].save, Some(json!("keep")));
}

#[test]
fn fresh_entry_applies_then_a_cleared_override_brings_the_catalog_back() {
    let fx = Fx::new();
    let cfg = fx.root.join("cfg");
    fs::create_dir_all(&cfg).unwrap();
    fs::write(cfg.join("ports.json"), r#"{"ports":[{"name":"Dusk","folder":"dusk","source":"https://example.com/d.zip","save":{"windows":"%APPDATA%\\Dusk"}}]}"#).unwrap();
    let settings_path = cfg.join(crate::fork::settings::FILE_NAME);
    let mut settings = ForkSettings::load(&settings_path);
    assert_eq!(fresh_entry(&cfg, "dusk").unwrap().save, Some(json!({"windows": "%APPDATA%\\Dusk"})));
    set_override(&mut settings, "dusk", &Override::Folder("saves".into())).unwrap();
    assert_eq!(fresh_entry(&cfg, "dusk").unwrap().save, Some(json!("saves")));
    clear_override(&mut settings, "dusk").unwrap();
    assert_eq!(fresh_entry(&cfg, "dusk").unwrap().save, Some(json!({"windows": "%APPDATA%\\Dusk"})));
    assert!(fresh_entry(&cfg, "other").is_none());
}

// -- Portable form and the checks on a chosen folder ------------------------------------------------------

#[test]
fn portable_form_is_relative_in_the_game_percent_home_in_home_else_absolute() {
    let (game, home) = (Path::new("/home/deck/Ports/Dusk"), Path::new("/home/deck"));
    assert_eq!(portable_form(&game.join("Save/slots"), game, Some(home)).unwrap(), "Save/slots");
    assert_eq!(portable_form(Path::new("/home/deck/.local/share/Dusk"), game, Some(home)).unwrap(), "%HOME%/.local/share/Dusk");
    assert_eq!(portable_form(Path::new("/run/media/sd/Dusk/save"), game, Some(home)).unwrap(), "/run/media/sd/Dusk/save");
    assert!(portable_form(Path::new("/home/deck/100%/x"), game, Some(home)).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn check_choice_refuses_what_restore_would_refuse() {
    let fx = Fx::new();
    let p = port("Dusk", "dusk", None);
    let gd = fx.game_dir(&p);
    fs::create_dir_all(&fx.saves_backup).unwrap();
    let refused = |dir: &Path| check_choice(&p, dir, &gd, &fx.saves_backup, Some(&fx.home)).unwrap_err();
    assert!(refused(&gd).contains("whole game folder"), "{}", refused(&gd));
    assert!(refused(&fx.home).starts_with("That folder cannot be used for saves"));
    assert!(refused(&fx.saves_backup).contains("Saves Backup"));
    assert!(refused(&fx.root.join("missing")).contains("not a folder"));
    let ok = fx.home.join(".local/share/Dusk");
    fs::create_dir_all(&ok).unwrap();
    assert_eq!(check_choice(&p, &ok, &gd, &fx.saves_backup, Some(&fx.home)).unwrap(), Override::Folder("%HOME%/.local/share/Dusk".into()));
    let inside = gd.join("Save");
    fs::create_dir_all(&inside).unwrap();
    assert_eq!(check_choice(&p, &inside, &gd, &fx.saves_backup, Some(&fx.home)).unwrap(), Override::Folder("Save".into()));
}

// -- Every save feature sees the override -----------------------------------------------------------------

#[cfg(target_os = "linux")]
#[test]
fn an_override_is_seen_by_backup_restore_uninstall_and_the_unknown_check() {
    let fx = Fx::new();
    let mut p = port("Severed Chains", "severed", None);
    let gd = fx.game_dir(&p);
    assert!(save_folder_unknown_on_linux(&p, &gd) || p.save.is_none(), "no field: nothing to back up yet");
    let outside = fx.root.join("elsewhere/SeveredSaves");
    put(&outside.join("slot1.sav"), b"one");
    put(&outside.join("slot2.sav"), b"two");

    let chosen = check_choice(&p, &outside, &gd, &fx.saves_backup, Some(&fx.home)).unwrap();
    let mut catalog = vec![p.clone()];
    apply_overrides(&mut catalog, &BTreeMap::from([("severed".to_string(), chosen)]));
    p = catalog.remove(0);

    assert_eq!(unknown_on_linux_fields(&p, &gd), (false, false));
    assert_eq!(state_of(&p, &gd, None), SaveState::Found);

    // Update backup: the zip exists and holds both files.
    let zip = backup_before_update(&p, &fx.saves_backup, &gd, "2026-10-24_10-00-00", &mut |_| {}).unwrap().expect("a backup");
    let backups = crate::fork::save_restore::list_backups(&fx.saves_backup, "severed");
    assert_eq!(backups.len(), 1);
    assert_eq!(backups[0].files, 2, "{}", zip.display());

    // Restore saves: the target is the chosen folder.
    let targets = crate::fork::save_restore::save_targets(&p, &gd, &fx.saves_backup, Some(&fx.home));
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].target, crate::fork::save_restore::Target::Folder(outside.clone()));

    // Uninstall: a folder outside the game folder is not deleted with it, and is left alone.
    assert!(preserve_all_before_uninstall(&p, &fx.saves_backup, &gd));
    assert_eq!(fs::read(outside.join("slot1.sav")).unwrap(), b"one");

    // Uninstall preserve: a folder inside the game folder is copied aside first.
    let inside = port("Inside", "inside", None);
    let igd = fx.game_dir(&inside);
    put(&igd.join("Save/slot.sav"), b"in");
    let chosen = check_choice(&inside, &igd.join("Save"), &igd, &fx.saves_backup, Some(&fx.home)).unwrap();
    let mut catalog = vec![inside];
    apply_overrides(&mut catalog, &BTreeMap::from([("inside".to_string(), chosen)]));
    assert!(preserve_all_before_uninstall(&catalog[0], &fx.saves_backup, &igd));
    assert_eq!(fs::read(crate::core::save_backup::pending_restore_dir(&fx.saves_backup, "inside", "save_folder").join("slot.sav")).unwrap(), b"in");
}

#[cfg(target_os = "linux")]
#[test]
fn a_no_saves_override_removes_the_unknown_warning_from_every_feature() {
    let fx = Fx::new();
    let p = port("Sev", "sev", None);
    let gd = fx.game_dir(&p);
    assert!(unknown_on_linux_fields(&p, &gd) == (false, false), "a missing field is NotListed, not unknown on Linux");
    let win = port("Dusk", "dusk", Some(json!({"windows": "%APPDATA%\\Dusk"})));
    assert!(save_folder_unknown_on_linux(&win, &gd));
    let mut catalog = vec![win];
    apply_overrides(&mut catalog, &BTreeMap::from([("dusk".to_string(), Override::NoSaves)]));
    assert!(!save_folder_unknown_on_linux(&catalog[0], &gd));
}

// -- Search -----------------------------------------------------------------------------------------------

fn dusk_catalog() -> (Port, Vec<Port>) {
    let dusk = port("Dusklight", "dusklight", None);
    let soh = port("Ship of Harkinian", "soh", None);
    let s2 = port("2 Ship 2 Harkinian", "2s2h", None);
    (dusk.clone(), vec![dusk, soh, s2])
}

#[test]
fn finds_a_proton_prefix_folder_and_reports_files_and_age() {
    let fx = Fx::new();
    let (dusk, catalog) = dusk_catalog();
    let steam = fx.home.join(".steam/steam");
    let saves = steam.join("steamapps/compatdata/4012345/pfx/drive_c/users/steamuser/AppData/Roaming/Dusklight");
    put(&saves.join("slot1.sav"), b"a");
    put(&saves.join("slot2.sav"), b"b");
    put(&saves.join("settings.ini"), b"c");
    let found = run(&fx, &dusk, &catalog, &[]);
    assert_eq!(found.candidates.len(), 1, "{:?}", found.candidates);
    let c = &found.candidates[0];
    assert_eq!((c.name.as_str(), c.files, c.place), ("Dusklight", 3, Place::Proton));
    assert_eq!(search::ago(SystemTime::now(), c.newest), "just now");
    assert!(!found.stopped_early);
}

#[test]
fn finds_local_share_hidden_home_and_company_game_folders_newest_first() {
    let fx = Fx::new();
    let (dusk, catalog) = dusk_catalog();
    put(&fx.home.join(".local/share/Dusklight/save1.sav"), b"new");
    put(&fx.home.join(".dusklight/old.sav"), b"old");
    put(&fx.home.join(".config/SomeCompany/Dusklight/data.dat"), b"x");
    let old = SystemTime::now() - Duration::from_secs(40 * 86_400);
    let f = fs::File::options().write(true).open(fx.home.join(".dusklight/old.sav")).unwrap();
    f.set_modified(old).unwrap();
    let found = run(&fx, &dusk, &catalog, &[]);
    let places: Vec<Place> = found.candidates.iter().map(|c| c.place).collect();
    assert_eq!(found.candidates.len(), 3, "{places:?}");
    assert_eq!(found.candidates.last().unwrap().place, Place::HomeHidden, "the 40 day old one is last");
    assert!(places.contains(&Place::LocalShare) && places.contains(&Place::Config));
}

#[test]
fn finds_a_save_folder_inside_the_port_folder_and_offers_just_that_folder() {
    let fx = Fx::new();
    let (dusk, catalog) = dusk_catalog();
    let gd = fx.game_dir(&dusk);
    put(&gd.join("Save/file1.bin"), b"a");
    put(&gd.join("Save/file2.bin"), b"b");
    put(&gd.join("assets/big.pak"), b"not saves");
    put(&gd.join("logs/run.log"), b"log");
    let found = run(&fx, &dusk, &catalog, &[]);
    assert_eq!(found.candidates.len(), 1, "{:?}", found.candidates);
    assert_eq!(found.candidates[0].path, gd.join("Save"));
    assert_eq!(found.candidates[0].place, Place::PortFolder);
}

#[test]
fn narrows_to_the_one_save_subfolder_when_every_save_file_is_in_it() {
    let fx = Fx::new();
    let (dusk, catalog) = dusk_catalog();
    let dir = fx.home.join(".local/share/Dusklight");
    put(&dir.join("Saves/slot1.sav"), b"a");
    put(&dir.join("config.json"), b"c");
    put(&dir.join("log.txt"), b"l");
    let found = run(&fx, &dusk, &catalog, &[]);
    assert_eq!(found.candidates[0].path, dir.join("Saves"));
    assert_eq!(found.candidates[0].files, 1);
}

#[test]
fn decoys_other_ports_saves_config_only_folders_and_the_backup_folder_are_not_offered() {
    let fx = Fx::new();
    let (_, catalog) = dusk_catalog();
    let soh = catalog[1].clone();
    // Ship of Harkinian is looking; 2 Ship 2 Harkinian's saves and an unrelated game's are all around.
    put(&fx.home.join(".local/share/2ship2harkinian/slot.sav"), b"other port");
    put(&fx.home.join(".local/share/Some Other Game/slot.sav"), b"unrelated");
    put(&fx.home.join(".config/soh/only-settings.cfg"), b"settings only, nothing save-like");
    put(&fx.saves_backup.join("Update Backups/soh/Ship of Harkinian/x.sav"), b"backup");
    // Another port's folder in the library, with a save folder in it.
    put(&fx.library.join("2s2h/Save/slot.sav"), b"other port in the library");
    let found = run(&fx, &soh, &catalog, &[]);
    assert!(found.candidates.is_empty(), "{:?}", found.candidates);
    // The real one is found alongside the decoys.
    put(&fx.home.join(".local/share/soh/Save/file1.sav"), b"mine");
    let found = run(&fx, &soh, &catalog, &[]);
    assert_eq!(found.candidates.len(), 1, "{:?}", found.candidates);
    assert!(found.candidates[0].path.ends_with("soh/Save"));
}

#[test]
fn a_huge_tree_is_bounded_and_cancel_stops_the_search() {
    let fx = Fx::new();
    let (dusk, catalog) = dusk_catalog();
    for i in 0..300 {
        put(&fx.home.join(format!(".local/share/folder{i}/a/b/c/file.txt")), b"x");
    }
    put(&fx.home.join(".local/share/Dusklight/slot.sav"), b"x");
    let gd = fx.game_dir(&dusk);
    let input = Input { port: &dusk, catalog: &catalog, game_dir: &gd, library_dir: &fx.library, saves_backup_dir: &fx.saves_backup, home: &fx.home, steam_roots: &[] };

    let started = std::time::Instant::now();
    let found = search::find(&input, &|| false, &mut |_| {});
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(found.candidates.len(), 1);

    let asked = Cell::new(0);
    let found = search::find(&input, &|| { asked.set(asked.get() + 1); true }, &mut |_| {});
    assert!(found.stopped_early, "a cancel is reported as an early stop");
    assert!(found.candidates.is_empty(), "cancelled before looking anywhere: {:?}", found.candidates);
    assert!(asked.get() >= 1);

    let found = search::find_with_budget(&input, &|| false, &mut |_| {}, Duration::ZERO);
    assert!(found.stopped_early && found.candidates.is_empty());
}

#[test]
fn the_search_never_writes() {
    let fx = Fx::new();
    let (dusk, catalog) = dusk_catalog();
    put(&fx.home.join(".local/share/Dusklight/slot.sav"), b"x");
    let before = snapshot(&fx.root);
    let _ = run(&fx, &dusk, &catalog, &[]);
    assert_eq!(snapshot(&fx.root), before);
}

fn snapshot(root: &Path) -> Vec<(PathBuf, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap().flatten() {
            let m = e.metadata().unwrap();
            out.push((e.path(), m.len()));
            if m.is_dir() {
                stack.push(e.path());
            }
        }
    }
    out.sort();
    out
}

#[test]
fn name_fits_uses_the_import_words_and_other_ports_names() {
    let (dusk, catalog) = dusk_catalog();
    assert!(search::name_fits("Dusklight", &dusk, &catalog));
    assert!(search::name_fits("dusklight", &dusk, &catalog));
    assert!(!search::name_fits("2ship2harkinian", &dusk, &catalog));
    assert!(!search::name_fits("zz", &dusk, &catalog));
    let soh = &catalog[1];
    assert!(search::name_fits("Ship of Harkinian", soh, &catalog));
    assert!(!search::name_fits("2s2h", soh, &catalog), "another port's own folder name");
}

#[test]
fn ago_reads_naturally() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100_000_000);
    let at = |secs: u64| search::ago(now, now - Duration::from_secs(secs));
    assert_eq!(at(10), "just now");
    assert_eq!(at(600), "10 minutes ago");
    assert_eq!(at(7_200), "2 hours ago");
    assert_eq!(at(100_000), "yesterday");
    assert_eq!(at(2 * 86_400 + 5), "2 days ago");
    assert_eq!(at(70 * 86_400), "2 months ago");
    assert_eq!(at(800 * 86_400), "2 years ago");
}
