//! Generated fixtures only: every save, game folder and backup here is made by the test in a temp folder.

use super::zip_index::write_stored_zip;
use super::*;
use crate::core::models::port_from_value;
use crate::core::save_backup::{backup_before_update, run_global_backup};
use serde_json::json;
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    library: PathBuf,
    saves_backup: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let library = root.join("Library");
        fs::create_dir_all(&library).unwrap();
        Fixture { saves_backup: root.join("Saves Backup"), library, root, _dir: dir }
    }

    fn game_dir(&self, port: &Port) -> PathBuf {
        let dir = self.library.join(&port.folder);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn update_backup(&self, port: &Port, stamp: &str) -> PathBuf {
        backup_before_update(port, &self.saves_backup, &self.game_dir(port), stamp, &mut |_| {}).unwrap().expect("a backup zip")
    }

    fn input<'a>(&'a self, port: &'a Port, game_dir: &'a Path, backup: &'a Backup, stamp: &'a str) -> RestoreInput<'a> {
        RestoreInput { port, game_dir, saves_backup_dir: &self.saves_backup, home: None, backup, game_running: false, stamp }
    }
}

fn port(folder: &str, save: Option<Value>, save2: Option<Value>) -> Port {
    let mut v = json!({"name": folder, "folder": folder, "source": "https://example.com/x.zip"});
    if let Some(s) = save {
        v["save"] = s;
    }
    if let Some(s) = save2 {
        v["save2"] = s;
    }
    port_from_value(&v).unwrap()
}

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// Every file under `root` with its bytes, by '/'-separated relative path.
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, rel: &str, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let name = e.file_name().to_string_lossy().into_owned();
            let rel = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            if e.file_type().unwrap().is_dir() {
                walk(&e.path(), &rel, out);
            } else {
                out.insert(rel, fs::read(e.path()).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    if root.exists() {
        walk(root, "", &mut out);
    }
    out
}

fn fill_saves(dir: &Path, tag: u8) {
    write(&dir.join("slot1.sav"), &[tag; 300]);
    write(&dir.join("profile/settings.ini"), format!("volume={tag}\n").as_bytes());
    write(&dir.join("profile/deep/x.bin"), &(0..=255u8).map(|b| b ^ tag).collect::<Vec<_>>());
}

fn set_age(path: &Path, seconds_ago: u64) {
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(seconds_ago)).unwrap();
}

/// Hidden `.<name>.restoring-*` / `.before-restore-*` folders left beside a save folder.
fn leftovers_beside(folder: &Path) -> Vec<String> {
    let name = folder.file_name().unwrap().to_string_lossy().into_owned();
    fs::read_dir(folder.parent().unwrap())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(&format!(".{name}.")))
        .collect()
}

// -- Listing ---------------------------------------------------------------------------------------

#[test]
fn lists_update_and_all_ports_backups_newest_first_with_their_folders() {
    let fx = Fixture::new();
    let game = port("MyGame", Some(json!("Save")), Some(json!("Save2")));
    let game_dir = fx.game_dir(&game);
    fill_saves(&game_dir.join("Save"), 1);
    write(&game_dir.join("Save2/other.dat"), b"second folder");

    let update_zip = fx.update_backup(&game, "2026-10-20_12-00-00");
    set_age(&update_zip, 3600);
    let other = port("Other", Some(json!("S")), None);
    write(&fx.game_dir(&other).join("S/o.sav"), b"other port");
    let summary = run_global_backup(&[game.clone(), other], &fx.library, &fx.saves_backup, "2026-10-21", &mut |_| {});
    assert_eq!(summary.copied, 3);

    let list = list_backups(&fx.saves_backup, "MyGame");
    let kinds: Vec<BackupKind> = list.iter().map(|b| b.kind).collect();
    assert_eq!(kinds, vec![BackupKind::AllPorts, BackupKind::BeforeUpdate]);
    for b in &list {
        assert_eq!(b.fields, vec![SaveField::Save, SaveField::Save2], "{}", b.name);
        assert_eq!(b.files, 4);
        assert_eq!(b.size, 300 + "volume=1\n".len() as u64 + 256 + "second folder".len() as u64);
        assert!(b.problem.is_none());
    }
    assert_eq!(list[0].prefix, "MyGame/");
    assert_eq!(list[1].prefix, "");
    assert_eq!(list[1].name, "2026-10-20_12-00-00");

    let all = list_all_backups(&fx.saves_backup);
    assert_eq!(all.keys().cloned().collect::<Vec<_>>(), vec!["MyGame".to_string(), "Other".to_string()]);
    assert_eq!(all["MyGame"].len(), 2);
    assert_eq!(all["Other"].len(), 1);
    assert_eq!(all["Other"][0].fields, vec![SaveField::Save]);
}

#[test]
fn an_unreadable_backup_is_listed_with_its_problem() {
    let fx = Fixture::new();
    write(&fx.saves_backup.join(UPDATE_BACKUPS_DIR).join("MyGame/2026-10-20_12-00-00.zip"), b"not a zip at all");
    let list = list_backups(&fx.saves_backup, "MyGame");
    assert_eq!(list.len(), 1);
    assert!(list[0].problem.as_deref().unwrap().contains("cannot be read"));
}

// -- Save folders ----------------------------------------------------------------------------------

#[test]
fn a_port_with_no_save_folder_for_this_system_gets_no_target() {
    let fx = Fixture::new();
    // Through the app's own resolver (O1's rules), which only refuses other systems' paths on Linux.
    #[cfg(target_os = "linux")]
    for value in [json!({"android": "/sdcard/A/saves"}), json!("%DPM_TEST_UNSET_VARIABLE%/B/saves"), json!({"windows": "%APPDATA%\\C"}), json!({"windows": "C:\\Games\\D"})] {
        let p = port("A", Some(value.clone()), None);
        let targets = save_targets(&p, &fx.game_dir(&p), &fx.saves_backup, None);
        assert_eq!(targets, vec![SaveTarget { field: SaveField::Save, target: Target::NotForThisSystem }], "{value}");
    }
    assert!(save_targets(&port("E", None, None), &fx.root, &fx.saves_backup, None).is_empty());
}

/// Backup and restore resolve the save folder through the same function, so what an update backed up
/// is put back in the same folder, also for in-game folders written with `\`.
#[test]
fn a_backup_made_before_an_update_restores_into_the_folder_it_came_from() {
    for (folder, value, parts) in [("WinRel", json!({"windows": "saves\\slots"}), ["saves", "slots"]), ("PlainRel", json!("save\\card0"), ["save", "card0"])] {
        let fx = Fixture::new();
        let game = port(folder, Some(value.clone()), None);
        let dir = fx.game_dir(&game);
        let save = dir.join(parts[0]).join(parts[1]);
        fill_saves(&save, 1);
        let original = snapshot(&save);
        fx.update_backup(&game, "2026-10-20_12-00-00");
        fill_saves(&save, 2);
        write(&save.join("since.sav"), b"after the backup");

        let backup = list_backups(&fx.saves_backup, folder).into_iter().find(|b| b.kind == BackupKind::BeforeUpdate).unwrap();
        assert_eq!(backup.fields, vec![SaveField::Save], "{value}");
        assert_eq!(backup.files, original.len(), "{value}");
        restore(&fx.input(&game, &dir, &backup, "s"), &mut |_| {}).unwrap();
        assert_eq!(snapshot(&save), original, "{value}");
        let stray: Vec<String> = fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.contains('\\')).collect();
        assert!(stray.is_empty(), "{value}: {stray:?}");
    }
}

#[test]
fn a_windows_only_relative_save_folder_counts_on_every_system_and_restores() {
    let fx = Fixture::new();
    let game = port("RelGame", Some(json!({"windows": "saves\\slots"})), Some(json!({"windows": "..\\Outside"})));
    let dir = fx.game_dir(&game);
    let targets = save_targets(&game, &dir, &fx.saves_backup, None);
    let save = dir.join("saves").join("slots");
    assert_eq!(targets[0].target, Target::Folder(save.clone()));
    // Leading out of the game folder: never a folder to restore into (O1 calls it unknown on Linux; the
    // outside-the-game-folder guard refuses it on Windows).
    assert!(matches!(targets[1].target, Target::Refused(_) | Target::NotForThisSystem), "{:?}", targets[1].target);
    let dot = port("DotGame", Some(json!({"windows": "."})), None);
    assert!(matches!(save_targets(&dot, &fx.game_dir(&dot), &fx.saves_backup, None)[0].target, Target::Refused(_)));

    write(&save.join("slot1.sav"), b"later");
    let zip = fx.saves_backup.join(UPDATE_BACKUPS_DIR).join("RelGame").join("2026-10-20_12-00-00.zip");
    fs::create_dir_all(zip.parent().unwrap()).unwrap();
    write_stored_zip(&zip, &[("save_folder/slot1.sav", b"from the backup"), ("save_folder/sub/a.dat", b"a")]);
    let backup = list_backups(&fx.saves_backup, "RelGame").remove(0);
    let done = restore(&fx.input(&game, &dir, &backup, "s"), &mut |_| {}).unwrap();
    assert_eq!(done.folders.len(), 1);
    assert_eq!(snapshot(&save), BTreeMap::from([("slot1.sav".to_string(), b"from the backup".to_vec()), ("sub/a.dat".to_string(), b"a".to_vec())]));
}

#[test]
fn a_windows_only_appdata_save_folder_is_not_restorable_on_linux() {
    #[cfg(not(windows))]
    {
        let fx = Fixture::new();
        let game = port("AppDataGame", Some(json!({"windows": "%APPDATA%\\AppDataGame"})), None);
        let dir = fx.game_dir(&game);
        assert_eq!(save_targets(&game, &dir, &fx.saves_backup, None)[0].target, Target::NotForThisSystem);
        let zip = fx.saves_backup.join(UPDATE_BACKUPS_DIR).join("AppDataGame").join("2026-10-20_12-00-00.zip");
        fs::create_dir_all(zip.parent().unwrap()).unwrap();
        write_stored_zip(&zip, &[("save_folder/slot1.sav", b"from the backup")]);
        let backup = list_backups(&fx.saves_backup, "AppDataGame").remove(0);
        let err = restore(&fx.input(&game, &dir, &backup, "s"), &mut |_| {}).unwrap_err();
        assert!(matches!(&err, RestoreError::NothingToRestore(why) if why.contains("no save folder known for Linux")), "{err:?}");
        assert!(!fx.saves_backup.join(RESTORE_BACKUPS_DIR).exists());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0, "nothing written in the game folder");
    }
}

#[test]
fn folders_that_are_too_broad_are_refused() {
    let fx = Fixture::new();
    let home = fx.root.join("home");
    fs::create_dir_all(&home).unwrap();
    let cases = [
        port("Dot", Some(json!(".")), None),
        port("Up", Some(json!("../Elsewhere")), None),
        port("Home", Some(json!(home.to_string_lossy())), None),
        port("Backups", Some(json!(fx.saves_backup.join("x").to_string_lossy())), None),
    ];
    for p in &cases {
        let targets = save_targets(p, &fx.game_dir(p), &fx.saves_backup, Some(&home));
        assert!(matches!(targets[0].target, Target::Refused(_)), "{}: {:?}", p.folder, targets[0].target);
    }
    let fine = port("Fine", Some(json!("saves")), Some(json!(home.join(".local/share/fine").to_string_lossy())));
    let dir = fx.game_dir(&fine);
    let targets = save_targets(&fine, &dir, &fx.saves_backup, Some(&home));
    assert_eq!(targets[0].target, Target::Folder(dir.join("saves")));
    assert_eq!(targets[1].target, Target::Folder(home.join(".local/share/fine")));
}

#[test]
fn restore_refuses_when_no_folder_is_known_for_this_system() {
    let fx = Fixture::new();
    let real = port("MyGame", Some(json!("Save")), None);
    let dir = fx.game_dir(&real);
    fill_saves(&dir.join("Save"), 1);
    fx.update_backup(&real, "2026-10-20_12-00-00");
    let backup = list_backups(&fx.saves_backup, "MyGame").remove(0);

    let elsewhere = port("MyGame", Some(json!({"android": "/sdcard/MyGame"})), None);
    let err = restore(&fx.input(&elsewhere, &dir, &backup, "s"), &mut |_| {}).unwrap_err();
    assert!(matches!(&err, RestoreError::NothingToRestore(why) if why.contains("no save folder known")), "{err:?}");
    assert!(!fx.saves_backup.join(RESTORE_BACKUPS_DIR).exists());
}

// -- Restoring -------------------------------------------------------------------------------------

#[test]
fn restore_round_trip_is_byte_identical_and_can_be_undone() {
    let fx = Fixture::new();
    let game = port("MyGame", Some(json!("Save")), None);
    let dir = fx.game_dir(&game);
    let save = dir.join("Save");
    fill_saves(&save, 1);
    let original = snapshot(&save);
    fx.update_backup(&game, "2026-10-20_12-00-00");

    // Play on: change a file, add one, delete one.
    write(&save.join("slot1.sav"), b"later progress");
    write(&save.join("slot2.sav"), b"a new slot");
    fs::remove_file(save.join("profile/deep/x.bin")).unwrap();
    let later = snapshot(&save);
    assert_ne!(later, original);

    let backup = list_backups(&fx.saves_backup, "MyGame").remove(0);
    let mut steps = Vec::new();
    let done = restore(&fx.input(&game, &dir, &backup, "2026-10-21_09-00-00"), &mut |s| steps.push(s.to_string())).unwrap();
    assert_eq!(snapshot(&save), original);
    assert_eq!(done.folders.len(), 1);
    assert_eq!(done.folders[0].files, 3);
    assert_eq!(done.folders[0].kept_old, None);
    assert_eq!(steps.first().map(String::as_str), Some("Backing up your current saves..."));
    assert!(leftovers_beside(&save).is_empty(), "{:?}", leftovers_beside(&save));

    // The safety backup was taken before the swap: it holds the "later" saves, and restoring it undoes the restore.
    let safety = done.safety_backup.clone().unwrap();
    assert_eq!(safety, fx.saves_backup.join(RESTORE_BACKUPS_DIR).join("MyGame").join("2026-10-21_09-00-00.zip"));
    let undo = list_backups(&fx.saves_backup, "MyGame").into_iter().find(|b| b.kind == BackupKind::BeforeRestore).unwrap();
    assert_eq!(undo.files, later.len());
    restore(&fx.input(&game, &dir, &undo, "2026-10-21_09-05-00"), &mut |_| {}).unwrap();
    assert_eq!(snapshot(&save), later);
}

#[test]
fn restore_is_refused_and_nothing_changes_when_the_safety_backup_fails() {
    let fx = Fixture::new();
    let game = port("MyGame", Some(json!("Save")), None);
    let dir = fx.game_dir(&game);
    let save = dir.join("Save");
    fill_saves(&save, 1);
    fx.update_backup(&game, "2026-10-20_12-00-00");
    write(&save.join("slot1.sav"), b"later progress");
    let later = snapshot(&save);

    // A file where the "Restore Backups" folder must go: the safety backup cannot be written.
    write(&fx.saves_backup.join(RESTORE_BACKUPS_DIR), b"blocks the folder");
    let backup = list_backups(&fx.saves_backup, "MyGame").remove(0);
    let err = restore(&fx.input(&game, &dir, &backup, "s"), &mut |_| {}).unwrap_err();
    assert!(matches!(err, RestoreError::SafetyBackupFailed(_)), "{err:?}");
    assert!(err.to_string().contains("nothing was changed"));
    assert_eq!(snapshot(&save), later);
    assert!(leftovers_beside(&save).is_empty());
}

#[test]
fn restore_is_refused_while_the_game_runs() {
    let fx = Fixture::new();
    let game = port("MyGame", Some(json!("Save")), None);
    let dir = fx.game_dir(&game);
    let save = dir.join("Save");
    fill_saves(&save, 1);
    fx.update_backup(&game, "2026-10-20_12-00-00");
    write(&save.join("slot1.sav"), b"later progress");
    let later = snapshot(&save);

    let backup = list_backups(&fx.saves_backup, "MyGame").remove(0);
    let mut input = fx.input(&game, &dir, &backup, "s");
    input.game_running = true;
    let err = restore(&input, &mut |_| {}).unwrap_err();
    assert!(matches!(&err, RestoreError::GameRunning(what) if what.contains("started from this app")), "{err:?}");
    assert_eq!(snapshot(&save), later);
    assert!(!fx.saves_backup.join(RESTORE_BACKUPS_DIR).exists(), "no safety backup either");
}

fn zip_slip_backup(fx: &Fixture, bad_name: &str) -> PathBuf {
    let zip = fx.saves_backup.join(UPDATE_BACKUPS_DIR).join("MyGame").join("2026-10-20_12-00-00.zip");
    fs::create_dir_all(zip.parent().unwrap()).unwrap();
    write_stored_zip(&zip, &[("save_folder/slot1.sav", b"from the zip"), (bad_name, b"escaped")]);
    zip
}

#[test]
fn zip_slip_entries_refuse_the_whole_backup() {
    let escape_abs = if cfg!(windows) { "C:/dpm-escape.txt" } else { "/tmp/dpm-escape-test.txt" };
    for bad in ["../../escape.txt", "save_folder/../../../escape.txt", escape_abs, "save_folder\\..\\..\\escape.txt"] {
        let fx = Fixture::new();
        let game = port("MyGame", Some(json!("Save")), None);
        let dir = fx.game_dir(&game);
        let save = dir.join("Save");
        fill_saves(&save, 1);
        let before = snapshot(&save);
        zip_slip_backup(&fx, bad);

        let backup = list_backups(&fx.saves_backup, "MyGame").remove(0);
        assert!(backup.problem.as_deref().is_some_and(|p| p.contains("points outside")), "{bad}: {:?}", backup.problem);
        let err = restore(&fx.input(&game, &dir, &backup, "s"), &mut |_| {}).unwrap_err();
        assert!(matches!(err, RestoreError::UnusableBackup(_)), "{bad}: {err:?}");

        // Even when the list was read before the file turned bad, the restore reads the names again.
        let mut stale = backup.clone();
        stale.problem = None;
        let err = restore(&fx.input(&game, &dir, &stale, "s"), &mut |_| {}).unwrap_err();
        assert!(matches!(err, RestoreError::UnusableBackup(_)), "{bad}: {err:?}");

        assert_eq!(snapshot(&save), before, "{bad}");
        assert!(!fx.saves_backup.join(RESTORE_BACKUPS_DIR).exists(), "{bad}: refused before the safety backup");
        for spot in [fx.root.join("escape.txt"), fx.library.join("escape.txt"), fx.saves_backup.join("escape.txt")] {
            assert!(!spot.exists(), "{bad}: {}", spot.display());
        }
        assert!(!Path::new(escape_abs).exists());
    }
}

#[test]
fn a_save_folder_that_does_not_exist_yet_is_created() {
    let fx = Fixture::new();
    let external = fx.root.join("home/.local/share/mygame");
    let game = port("MyGame", Some(json!(external.to_string_lossy())), None);
    let dir = fx.game_dir(&game);
    fill_saves(&external, 7);
    let original = snapshot(&external);
    fx.update_backup(&game, "2026-10-20_12-00-00");
    fs::remove_dir_all(fx.root.join("home/.local")).unwrap();

    let backup = list_backups(&fx.saves_backup, "MyGame").remove(0);
    let done = restore(&fx.input(&game, &dir, &backup, "s"), &mut |_| {}).unwrap();
    assert_eq!(snapshot(&external), original);
    assert_eq!(done.safety_backup, None, "nothing to back up first");
    assert!(leftovers_beside(&external).is_empty());
}

#[test]
fn both_save_folders_are_restored_and_only_the_ones_the_backup_holds() {
    let fx = Fixture::new();
    let game = port("MyGame", Some(json!("Save")), Some(json!("Save2")));
    let dir = fx.game_dir(&game);
    let (save, save2) = (dir.join("Save"), dir.join("Save2"));
    fill_saves(&save, 1);
    fill_saves(&save2, 2);
    let (orig1, orig2) = (snapshot(&save), snapshot(&save2));
    fx.update_backup(&game, "2026-10-20_12-00-00");
    fill_saves(&save, 3);
    fill_saves(&save2, 4);
    let backup = list_backups(&fx.saves_backup, "MyGame").remove(0);
    let done = restore(&fx.input(&game, &dir, &backup, "s"), &mut |_| {}).unwrap();
    assert_eq!((snapshot(&save), snapshot(&save2)), (orig1, orig2.clone()));
    assert_eq!(done.folders.iter().map(|f| f.field).collect::<Vec<_>>(), vec![SaveField::Save, SaveField::Save2]);

    // A backup holding only the first folder leaves the second alone.
    let only_first = fx.saves_backup.join(UPDATE_BACKUPS_DIR).join("MyGame").join("2026-10-22_08-00-00.zip");
    write_stored_zip(&only_first, &[("save_folder/slot1.sav", b"only the first")]);
    let backup = list_backups(&fx.saves_backup, "MyGame").into_iter().find(|b| b.name == "2026-10-22_08-00-00").unwrap();
    assert_eq!(backup.fields, vec![SaveField::Save]);
    restore(&fx.input(&game, &dir, &backup, "s2"), &mut |_| {}).unwrap();
    assert_eq!(snapshot(&save), BTreeMap::from([("slot1.sav".to_string(), b"only the first".to_vec())]));
    assert_eq!(snapshot(&save2), orig2);
}

#[test]
fn an_all_ports_backup_restores_only_this_port() {
    let fx = Fixture::new();
    let game = port("MyGame", Some(json!("Save")), Some(json!("Save2")));
    let other = port("Other", Some(json!("S")), None);
    let dir = fx.game_dir(&game);
    let other_save = fx.game_dir(&other).join("S");
    fill_saves(&dir.join("Save"), 1);
    fill_saves(&dir.join("Save2"), 2);
    write(&other_save.join("o.sav"), b"other then");
    let (orig1, orig2) = (snapshot(&dir.join("Save")), snapshot(&dir.join("Save2")));
    run_global_backup(&[game.clone(), other.clone()], &fx.library, &fx.saves_backup, "2026-10-21", &mut |_| {});

    fill_saves(&dir.join("Save"), 5);
    fs::remove_dir_all(dir.join("Save2")).unwrap();
    write(&other_save.join("o.sav"), b"other now");

    let backup = list_backups(&fx.saves_backup, "MyGame").remove(0);
    assert_eq!(backup.kind, BackupKind::AllPorts);
    restore(&fx.input(&game, &dir, &backup, "s"), &mut |_| {}).unwrap();
    assert_eq!(snapshot(&dir.join("Save")), orig1);
    assert_eq!(snapshot(&dir.join("Save2")), orig2);
    assert_eq!(fs::read(other_save.join("o.sav")).unwrap(), b"other now");
}

#[test]
fn a_save_folder_that_is_a_link_keeps_the_link() {
    #[cfg(unix)]
    {
        let fx = Fixture::new();
        let game = port("MyGame", Some(json!("Save")), None);
        let dir = fx.game_dir(&game);
        let real = fx.root.join("sdcard/MyGameSaves");
        fill_saves(&real, 1);
        std::os::unix::fs::symlink(&real, dir.join("Save")).unwrap();
        let original = snapshot(&real);
        fx.update_backup(&game, "2026-10-20_12-00-00");
        fill_saves(&real, 9);
        let backup = list_backups(&fx.saves_backup, "MyGame").remove(0);
        restore(&fx.input(&game, &dir, &backup, "s"), &mut |_| {}).unwrap();
        assert!(fs::symlink_metadata(dir.join("Save")).unwrap().file_type().is_symlink());
        assert_eq!(snapshot(&real), original);
    }
}

#[test]
fn when_one_folder_cannot_be_prepared_no_folder_changes() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let fx = Fixture::new();
        let locked_parent = fx.root.join("locked");
        let save2 = locked_parent.join("save2");
        let game = port("MyGame", Some(json!("Save")), Some(json!(save2.to_string_lossy())));
        let dir = fx.game_dir(&game);
        let save = dir.join("Save");
        fill_saves(&save, 1);
        fill_saves(&save2, 2);
        fx.update_backup(&game, "2026-10-20_12-00-00");
        fill_saves(&save, 3);
        let (later1, later2) = (snapshot(&save), snapshot(&save2));

        fs::set_permissions(&locked_parent, fs::Permissions::from_mode(0o555)).unwrap();
        // Running as root ignores the lock; the check is meaningless then.
        let root_user = fs::write(locked_parent.join("probe"), b"x").is_ok();
        let backup = list_backups(&fx.saves_backup, "MyGame").remove(0);
        let result = restore(&fx.input(&game, &dir, &backup, "s"), &mut |_| {});
        fs::set_permissions(&locked_parent, fs::Permissions::from_mode(0o755)).unwrap();
        if root_user {
            return;
        }
        assert!(matches!(result, Err(RestoreError::Failed(_))), "{result:?}");
        assert_eq!(snapshot(&save), later1, "the first folder was not touched");
        assert_eq!(snapshot(&save2), later2);
        assert!(leftovers_beside(&save).is_empty(), "{:?}", leftovers_beside(&save));
    }
}

#[test]
fn plan_says_which_folders_are_skipped_and_why() {
    let targets = vec![
        SaveTarget { field: SaveField::Save, target: Target::Folder(PathBuf::from("/x/save")) },
        SaveTarget { field: SaveField::Save2, target: Target::NotForThisSystem },
    ];
    let backup = Backup {
        kind: BackupKind::BeforeUpdate,
        zip: PathBuf::from("/b.zip"),
        prefix: String::new(),
        name: "b".into(),
        modified: SystemTime::UNIX_EPOCH,
        size: 1,
        files: 2,
        fields: vec![SaveField::Save, SaveField::Save2],
        problem: None,
    };
    let plan = plan_restore(&targets, &backup);
    assert_eq!(plan.replace, vec![(SaveField::Save, PathBuf::from("/x/save"))]);
    assert_eq!(plan.skipped, vec![(SaveField::Save2, format!("no save folder known for {}", system_name()))]);
}

// -- Review follow-ups -----------------------------------------------------------------------------

const CENTRAL: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];

/// Byte offsets of every central directory entry in a zip written by `write_stored_zip`.
fn central_entries(bytes: &[u8]) -> Vec<usize> {
    bytes.windows(4).enumerate().filter(|(_, w)| *w == CENTRAL).map(|(i, _)| i).collect()
}

/// A port with `Save`, an update backup of `fill_saves(1)`, and later saves `fill_saves(3)` on disk.
fn played_on(fx: &Fixture) -> (Port, PathBuf, PathBuf) {
    let game = port("MyGame", Some(json!("Save")), None);
    let dir = fx.game_dir(&game);
    let save = dir.join("Save");
    fill_saves(&save, 1);
    fx.update_backup(&game, "2026-10-20_12-00-00");
    fill_saves(&save, 3);
    write(&save.join("since.sav"), b"saved after the backup");
    (game, dir, save)
}

fn staging_left(fx: &Fixture) -> Vec<String> {
    fs::read_dir(&fx.saves_backup)
        .map(|r| r.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("_restore_")).collect())
        .unwrap_or_default()
}

#[test]
fn a_link_entry_refuses_the_backup_end_to_end() {
    let fx = Fixture::new();
    let (game, dir, save) = played_on(&fx);
    let later = snapshot(&save);
    let zip = fx.saves_backup.join(UPDATE_BACKUPS_DIR).join("MyGame").join("2026-10-22_08-00-00.zip");
    write_stored_zip(&zip, &[("save_folder/slot1.sav", b"from the zip"), ("save_folder/link", b"/etc/passwd")]);
    let mut bytes = fs::read(&zip).unwrap();
    let cd = central_entries(&bytes)[1];
    bytes[cd + 5] = 3; // made by: Unix
    bytes[cd + 38..cd + 42].copy_from_slice(&(0o120_777u32 << 16).to_le_bytes());
    fs::write(&zip, bytes).unwrap();

    let backup = list_backups(&fx.saves_backup, "MyGame").into_iter().find(|b| b.name == "2026-10-22_08-00-00").unwrap();
    assert!(backup.problem.as_deref().is_some_and(|p| p.contains("link")), "{:?}", backup.problem);
    let mut stale = backup.clone();
    stale.problem = None;
    let err = restore(&fx.input(&game, &dir, &stale, "s"), &mut |_| {}).unwrap_err();
    assert!(matches!(err, RestoreError::UnusableBackup(_)), "{err:?}");
    assert_eq!(snapshot(&save), later);
    assert!(!fx.saves_backup.join(RESTORE_BACKUPS_DIR).exists());
}

#[test]
fn a_size_that_differs_from_the_table_refuses_the_restore_and_leaves_nothing() {
    let fx = Fixture::new();
    let (game, dir, save) = played_on(&fx);
    let later = snapshot(&save);
    let zip = fx.saves_backup.join(UPDATE_BACKUPS_DIR).join("MyGame").join("2026-10-22_08-00-00.zip");
    write_stored_zip(&zip, &[("save_folder/slot1.sav", b"twelve bytes")]);
    let mut bytes = fs::read(&zip).unwrap();
    let cd = central_entries(&bytes)[0];
    bytes[cd + 24..cd + 28].copy_from_slice(&13u32.to_le_bytes()); // the table says 13, the data is 12
    fs::write(&zip, bytes).unwrap();

    let backup = list_backups(&fx.saves_backup, "MyGame").into_iter().find(|b| b.name == "2026-10-22_08-00-00").unwrap();
    let err = restore(&fx.input(&game, &dir, &backup, "s"), &mut |_| {}).unwrap_err();
    assert!(matches!(err, RestoreError::Failed(_) | RestoreError::UnusableBackup(_)), "{err:?}");
    assert_eq!(snapshot(&save), later);
    assert!(leftovers_beside(&save).is_empty(), "{:?}", leftovers_beside(&save));
    assert!(staging_left(&fx).is_empty(), "{:?}", staging_left(&fx));
}

#[test]
fn when_the_second_swap_fails_the_first_folder_is_put_back() {
    let fx = Fixture::new();
    let game = port("MyGame", Some(json!("Save")), Some(json!("Save2")));
    let dir = fx.game_dir(&game);
    let (save, save2) = (dir.join("Save"), dir.join("Save2"));
    fill_saves(&save, 1);
    fill_saves(&save2, 2);
    fx.update_backup(&game, "2026-10-20_12-00-00");
    fill_saves(&save, 3);
    write(&save2.join("new.sav"), b"new");
    let (later1, later2) = (snapshot(&save), snapshot(&save2));

    let backup = list_backups(&fx.saves_backup, "MyGame").remove(0);
    FAIL_SWAP_AT.with(|f| f.set(Some(1)));
    let result = restore(&fx.input(&game, &dir, &backup, "s"), &mut |_| {});
    FAIL_SWAP_AT.with(|f| f.set(None));
    assert!(matches!(result, Err(RestoreError::Failed(_))), "{result:?}");
    assert_eq!(snapshot(&save), later1, "the first folder was swapped, then put back");
    assert_eq!(snapshot(&save2), later2);
    assert!(leftovers_beside(&save).is_empty(), "{:?}", leftovers_beside(&save));
    assert!(leftovers_beside(&save2).is_empty(), "{:?}", leftovers_beside(&save2));
    assert!(staging_left(&fx).is_empty());
}

#[test]
fn a_link_to_a_folder_inside_a_save_folder_keeps_the_old_folder() {
    #[cfg(unix)]
    {
        let fx = Fixture::new();
        let (game, dir, save) = played_on(&fx);
        let elsewhere = fx.root.join("elsewhere");
        write(&elsewhere.join("big.dat"), b"not ours to touch");
        std::os::unix::fs::symlink(&elsewhere, save.join("linked")).unwrap();
        let backup = list_backups(&fx.saves_backup, "MyGame").remove(0);
        let done = restore(&fx.input(&game, &dir, &backup, "s"), &mut |_| {}).unwrap();
        let kept = done.folders[0].kept_old.clone().expect("the old folder is kept: its link was not in the safety zip");
        assert!(fs::symlink_metadata(kept.join("linked")).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(elsewhere.join("big.dat")).unwrap(), b"not ours to touch");
        assert!(!save.join("linked").exists());
        assert!(view::done_text("MyGame", &done).contains("was kept at"));
    }
}

#[test]
fn hidden_files_and_empty_folders_come_back() {
    let fx = Fixture::new();
    let (game, dir, save) = played_on(&fx);
    let zip = fx.saves_backup.join(UPDATE_BACKUPS_DIR).join("MyGame").join("2026-10-22_08-00-00.zip");
    write_stored_zip(&zip, &[("save_folder/", b""), ("save_folder/.hidden", b"dot"), ("save_folder/empty/", b""), ("save_folder/sub/.cfg", b"c")]);
    let backup = list_backups(&fx.saves_backup, "MyGame").into_iter().find(|b| b.name == "2026-10-22_08-00-00").unwrap();
    restore(&fx.input(&game, &dir, &backup, "s"), &mut |_| {}).unwrap();
    assert_eq!(snapshot(&save), BTreeMap::from([(".hidden".to_string(), b"dot".to_vec()), ("sub/.cfg".to_string(), b"c".to_vec())]));
    assert!(save.join("empty").is_dir());
}

#[test]
fn folders_left_by_an_interrupted_restore_are_reported_kept_and_do_not_block_a_restore() {
    let fx = Fixture::new();
    let (game, dir, save) = played_on(&fx);
    let before = dir.join(".Save.before-restore-999");
    let restoring = dir.join(".Save.restoring-999");
    write(&before.join("slot1.sav"), b"saves from before the crash");
    write(&restoring.join("slot1.sav"), b"half a copy");
    write(&dir.join(".Other.before-restore-1/x"), b"belongs to another folder, not listed");

    let targets = save_targets(&game, &dir, &fx.saves_backup, None);
    let found = interrupted_restores(&targets);
    assert_eq!(found.iter().map(|i| (i.path.clone(), i.holds_previous)).collect::<Vec<_>>(), vec![(before.clone(), true), (restoring.clone(), false)]);
    assert!(view::interrupted_text(&found[0]).contains(&before.display().to_string()));
    assert!(view::interrupted_row(&found[0]).starts_with("A restore was interrupted"));

    let backup = list_backups(&fx.saves_backup, "MyGame").remove(0);
    restore(&fx.input(&game, &dir, &backup, "s"), &mut |_| {}).unwrap();
    assert_eq!(fs::read(before.join("slot1.sav")).unwrap(), b"saves from before the crash");
    assert_eq!(fs::read(restoring.join("slot1.sav")).unwrap(), b"half a copy");
    assert_eq!(leftovers_beside(&save).len(), 2, "only the two that were there");
}

#[test]
fn stale_scratch_folders_are_swept_and_nothing_else() {
    let fx = Fixture::new();
    let fresh = fx.saves_backup.join("_restore_fresh");
    let other = fx.saves_backup.join("_update_backup_x");
    write(&fresh.join("a"), b"a");
    write(&other.join("a"), b"a");
    let now = SystemTime::now();
    sweep_stale_staging(&fx.saves_backup, now);
    assert!(fresh.exists(), "a scratch folder under a day old may belong to a running restore");
    sweep_stale_staging(&fx.saves_backup, now + Duration::from_secs(2 * 24 * 60 * 60));
    assert!(!fresh.exists());
    assert!(other.exists(), "not ours");
}

#[test]
fn an_empty_save_value_does_not_say_where_the_saves_are() {
    let fx = Fixture::new();
    for value in [json!(""), json!(null), json!({"linux": "", "windows": "%APPDATA%\\x"}), json!({"windows": ""})] {
        let p = port("Empty", Some(value.clone()), None);
        let targets = save_targets(&p, &fx.game_dir(&p), &fx.saves_backup, None);
        // On Windows the object with a filled "windows" entry is that system's own value.
        if !(cfg!(windows) && value.get("linux").is_some()) {
            assert_eq!(targets[0].target, Target::NotListed, "{value}");
        }
    }
    let p = port("Empty", Some(json!("")), None);
    let targets = save_targets(&p, &fx.game_dir(&p), &fx.saves_backup, None);
    assert!(view::nothing_restorable_text("Empty", &targets).contains("does not say where it keeps its saves"));
}

#[test]
fn a_program_running_from_the_game_folder_refuses_the_restore() {
    #[cfg(target_os = "linux")]
    {
        let fx = Fixture::new();
        let (game, dir, save) = played_on(&fx);
        let later = snapshot(&save);
        let mut child = std::process::Command::new("sleep").arg("30").current_dir(&dir).spawn().unwrap();
        let backup = list_backups(&fx.saves_backup, "MyGame").remove(0);
        let result = restore(&fx.input(&game, &dir, &backup, "s"), &mut |_| {});
        let _ = child.kill();
        let _ = child.wait();
        let err = result.unwrap_err();
        assert!(matches!(&err, RestoreError::GameRunning(what) if what.starts_with("sleep (process ")), "{err:?}");
        assert!(err.to_string().contains("Quit it first"));
        assert_eq!(snapshot(&save), later);
        assert!(!fx.saves_backup.join(RESTORE_BACKUPS_DIR).exists());
    }
}

#[test]
fn a_known_full_drive_is_a_reason_to_refuse_and_an_unknown_one_is_not() {
    let place = Path::new("/x");
    assert!(space_problem(1_000, None, place).is_none());
    assert!(space_problem(1_000, Some(u64::MAX), place).is_none());
    assert!(space_problem(1_000, Some(1_000), place).is_some_and(|p| p.contains("not enough free space")));
}
