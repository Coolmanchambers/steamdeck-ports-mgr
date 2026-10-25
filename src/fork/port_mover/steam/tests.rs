//! The paths are `/`-joined fixtures, and `moved_path` joins with the platform's separator: Linux (the Deck) only,
//! like the existing repoint tests.
#![cfg(unix)]
use super::*;
use crate::fork::vdf::fixtures::*;
use std::fs;
use std::io;

fn no_steam() -> io::Result<Vec<String>> {
    Ok(Vec::new())
}

fn doc_with(entries: &[Vec<u8>]) -> Document {
    Document::parse(&file_of(entries)).unwrap()
}

fn only(doc: &Document) -> shortcuts::Shortcut {
    shortcuts::list(doc).remove(0)
}

#[test]
fn a_quoted_icon_inside_the_old_folder_is_repointed_and_written_plain() {
    let entry = steam_entry_with(0, 0xAAAA_0001, "Zelda", "\"/old/Port/run\"", "\"/old/Port\"", "\"/old/Port/art/icon.png\"", "--fullscreen");
    let mut doc = doc_with(&[entry]);
    let result = repoint_doc(&mut doc, Path::new("/old/Port"), Path::new("/new/Port"));
    assert_eq!(result.changed.len(), 1, "{result:?}");
    assert_eq!(result.changed[0].fields, ["program", "start folder", "icon"]);
    let s = only(&doc);
    assert_eq!(s.icon, "/new/Port/art/icon.png", "written without quote marks, the way Steam stores an icon");
    assert_eq!(s.exe, "\"/new/Port/run\"", "the program keeps its quotes");
    assert_eq!(s.launch_options, "--fullscreen", "nothing else changes");
    assert_eq!(s.app_id, Some(0xAAAA_0001));
}

#[test]
fn a_quoted_icon_alone_is_enough_to_repoint_and_a_plain_one_still_works() {
    for icon in ["\"/old/Port/icon.png\"", "/old/Port/icon.png"] {
        let entry = steam_entry_with(0, 7, "Zelda", "\"/usr/bin/other\"", "\"/usr/bin/\"", icon, "");
        let mut doc = doc_with(&[entry]);
        let result = repoint_doc(&mut doc, Path::new("/old/Port"), Path::new("/new/Port"));
        assert_eq!(result.changed[0].fields, ["icon"], "{icon}");
        assert_eq!(only(&doc).icon, "/new/Port/icon.png");
    }
}

#[test]
fn an_icon_outside_the_old_folder_is_left_alone() {
    let entry = steam_entry_with(0, 7, "Zelda", "\"/old/Port/run\"", "\"/old/Port\"", "\"/elsewhere/icon.png\"", "");
    let mut doc = doc_with(&[entry]);
    let result = repoint_doc(&mut doc, Path::new("/old/Port"), Path::new("/new/Port"));
    assert_eq!(result.changed[0].fields, ["program", "start folder"]);
    assert_eq!(only(&doc).icon, "\"/elsewhere/icon.png\"", "an icon this change does not touch keeps its stored text");
}

fn home_with(entries: &[Vec<u8>], name: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("dpm_port_mover_steam_{}_{name}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    let config = home.join(".local/share/Steam/userdata/1234/config");
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join("shortcuts.vdf"), file_of(entries)).unwrap();
    home
}

#[test]
fn the_pointing_into_check_sees_a_quoted_icon() {
    let entry = steam_entry_with(0, 7, "Zelda", "\"/usr/bin/other\"", "\"/usr/bin/\"", "\"/old/Port/icon.png\"", "");
    let home = home_with(&[entry], "quoted_icon");
    let found = shortcuts_pointing_into_detailed(&home, Path::new("/old/Port")).unwrap();
    assert_eq!(found, [("Zelda".to_string(), true)], "the old copy cannot be removed while the icon still lives in it");
    assert!(shortcuts_pointing_into_detailed(&home, Path::new("/other/Port")).unwrap().is_empty());
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn write_repoint_changes_the_file_and_only_the_named_fields() {
    let entry = steam_entry_with(0, 7, "Zelda", "\"/old/Port/run\"", "\"/old/Port\"", "\"/old/Port/icon.png\"", "-x");
    let other = steam_entry_with(1, 8, "Chiaki", "\"/usr/bin/chiaki\"", "\"/usr/bin/\"", "/usr/share/c.png", "");
    let home = home_with(&[entry, other], "write");
    let path = home.join(".local/share/Steam/userdata/1234/config/shortcuts.vdf");
    let settings = ForkSettings::load(&home.join("fork-settings.json"));
    let (result, backup) = write_repoint(&path, Path::new("/old/Port"), Path::new("/new/Port"), &no_steam, &settings, SystemTime::now()).unwrap();
    assert_eq!(result.changed.len(), 1);
    assert!(backup.is_some());
    let list = shortcuts::list(&ShortcutsFile::load(&path).unwrap().doc);
    assert_eq!(list[0].icon, "/new/Port/icon.png");
    assert_eq!(list[0].launch_options, "-x");
    assert_eq!(list[1].icon, "/usr/share/c.png");
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn the_checkups_repoint_changes_the_old_copy_shortcuts_of_every_account_whatever_they_are_called_and_adds_nothing() {
    // Two accounts: one shortcut still has the port's catalog name, the other was renamed by the user. Both start the old copy.
    let same_name = steam_entry_with(0, 0xAAAA_0001, "Ship of Harkinian", "\"/old/SoH/soh\"", "\"/old/SoH\"", "\"/old/SoH/art/i.png\"", "--fullscreen");
    let renamed = steam_entry_with(0, 0xBBBB_0002, "Zelda (my copy)", "\"/old/SoH/soh\"", "\"/old/SoH\"", "/old/SoH/art/i.png", "-x");
    let unrelated = steam_entry_with(1, 0xBBBB_0003, "Chiaki", "\"/usr/bin/chiaki\"", "\"/usr/bin/\"", "/usr/share/c.png", "");
    let home = home_with(&[same_name], "repoint_all");
    let second = home.join(".local/share/Steam/userdata/5678/config");
    fs::create_dir_all(&second).unwrap();
    fs::write(second.join("shortcuts.vdf"), file_of(&[renamed, unrelated])).unwrap();
    let config = home.join("app");
    fs::create_dir_all(&config).unwrap();
    let settings = ForkSettings::load(&home.join("fork-settings.json"));
    let ctx = SteamCtx { home: &home, config_dir: &config, settings: &settings, steam: &no_steam, game_mode: false, now: SystemTime::now() };
    let report = repoint_everywhere("a/soh", "Ship of Harkinian", Path::new("/old/SoH"), Path::new("/new/SoH"), &ctx);
    assert_eq!((report.changed_count(), report.queued.len(), report.failed.len()), (2, 0, 0), "{report:?}");
    let list = |account: u32| shortcuts::list(&ShortcutsFile::load(&home.join(format!(".local/share/Steam/userdata/{account}/config/shortcuts.vdf"))).unwrap().doc);
    let (one, two) = (list(1234), list(5678));
    assert_eq!((one.len(), two.len()), (1, 2), "no shortcut was added");
    assert_eq!((one[0].app_name.as_str(), one[0].app_id, one[0].launch_options.as_str()), ("Ship of Harkinian", Some(0xAAAA_0001), "--fullscreen"));
    assert_eq!((one[0].exe.as_str(), one[0].start_dir.as_str(), one[0].icon.as_str()), ("\"/new/SoH/soh\"", "\"/new/SoH\"", "/new/SoH/art/i.png"));
    assert_eq!((two[0].app_name.as_str(), two[0].app_id, two[0].launch_options.as_str()), ("Zelda (my copy)", Some(0xBBBB_0002), "-x"), "a renamed shortcut keeps its name, id and launch options");
    assert_eq!((two[0].exe.as_str(), two[0].icon.as_str()), ("\"/new/SoH/soh\"", "/new/SoH/art/i.png"));
    assert_eq!(two[1].exe, "\"/usr/bin/chiaki\"", "another program is untouched");
    // With Steam running nothing is written: one Repoint is queued for each account that has something to change.
    let steam_up = || -> io::Result<Vec<String>> { Ok(vec!["77 steam".into()]) };
    let again = home_with(&[steam_entry_with(0, 7, "Zelda", "\"/old/SoH/soh\"", "\"/old/SoH\"", "", "")], "repoint_queue");
    let ctx = SteamCtx { home: &again, config_dir: &config, settings: &settings, steam: &steam_up, game_mode: false, now: SystemTime::now() };
    let queued = repoint_everywhere("a/soh", "Ship of Harkinian", Path::new("/old/SoH"), Path::new("/new/SoH"), &ctx);
    assert_eq!((queued.changed_count(), queued.queued.len()), (0, 1), "{queued:?}");
    let _ = fs::remove_dir_all(&home);
    let _ = fs::remove_dir_all(&again);
}
