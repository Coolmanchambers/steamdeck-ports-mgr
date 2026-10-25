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
