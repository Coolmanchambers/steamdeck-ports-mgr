use super::*;
use crate::core::models::port_from_value;

fn put(path: &Path, data: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

fn port(folder: &str) -> Port {
    port_from_value(&serde_json::json!({"name": "n", "folder": folder, "source": "https://example.com/x.zip"})).unwrap()
}

fn guard(tmp: &Path, folder: &str) -> Guard {
    let dest = tmp.join("Library").join(folder);
    Guard::protecting(Protect::for_port(&port(folder), &dest, None).unwrap(),&tmp.join("Saves Backup"), folder, &tmp.join("Library"), "2026-11-04_10-00-00")
}

fn plain_move(from: &Path, to: &Path) -> io::Result<()> {
    uninstall_keep::move_path(from, to)
}

// D7: a failure part way through puts every file back where it was: the old program files, the user's files and the
// release's copies. Nothing is lost and nothing of the release is left behind.
#[test]
fn a_merge_that_fails_part_way_puts_everything_back() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("Library").join("game");
    let src = tmp.path().join("release");
    put(&dest.join("a.bin"), b"old a");
    put(&dest.join("b.o2r"), b"old archive");
    put(&dest.join("c.sav"), b"my save");
    put(&dest.join("keep.txt"), b"not in the release");
    put(&src.join("a.bin"), b"new a");
    put(&src.join("b.o2r"), b"new archive");
    put(&src.join("c.sav"), b"release save");
    put(&src.join("sub").join("d.bin"), b"new d");
    put(&src.join("z.bin"), b"new z");
    let before: Vec<(PathBuf, Vec<u8>)> = ["a.bin", "b.o2r", "c.sav", "keep.txt"].iter().map(|n| (dest.join(n), fs::read(dest.join(n)).unwrap())).collect();
    let mut calls = 0;
    let mut mover = |from: &Path, to: &Path| {
        calls += 1;
        if calls == 6 { Err(io::Error::other("disk full")) } else { plain_move(from, to) }
    };

    let err = merge(&src, &dest, &guard(tmp.path(), "game"), &mut mover).unwrap_err();

    assert!(err.to_string().contains("disk full") && err.to_string().contains("put back"), "{err}");
    for (path, bytes) in before {
        assert_eq!(fs::read(&path).unwrap(), bytes, "{}", path.display());
    }
    assert!(!dest.join("sub").exists() && !dest.join("z.bin").exists(), "nothing of the release is left in the port folder");
    assert!(!dest.join(PREVIOUS).join("update-copies-2026-11-04_10-00-00").exists());
    assert!(fs::read_dir(tmp.path().join("Library")).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with("_displaced_")));
    assert_eq!(read(&src.join("a.bin")), "new a", "the release's files are back in the staging folder");
}

#[test]
fn a_merge_that_works_replaces_program_files_and_removes_the_old_ones() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("Library").join("game");
    let src = tmp.path().join("release");
    put(&dest.join("game.x86_64"), b"old");
    put(&src.join("game.x86_64"), b"new");
    let report = merge(&src, &dest, &guard(tmp.path(), "game"), &mut plain_move).unwrap();
    assert!(report.is_empty());
    assert_eq!(read(&dest.join("game.x86_64")), "new");
    assert!(fs::read_dir(tmp.path().join("Library")).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with("_displaced_")), "the old program file goes once the release is in");
    assert!(report.line("update", &|p: &Path| p.display().to_string()).is_none());
}

// A folder of the user's where the release has a file is kept in Replaced by update, never deleted.
#[test]
fn a_folder_the_release_replaces_with_a_file_is_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("Library").join("game");
    let src = tmp.path().join("release");
    put(&dest.join("config").join("mine.ini"), b"my settings");
    put(&src.join("config"), b"now a file");
    let report = merge(&src, &dest, &guard(tmp.path(), "game"), &mut plain_move).unwrap();
    assert_eq!(read(&dest.join("config")), "now a file");
    let kept = tmp.path().join("Saves Backup").join(REPLACED_BY_UPDATE).join("game").join("2026-11-04_10-00-00");
    assert_eq!(read(&kept.join("config").join("mine.ini")), "my settings");
    assert_eq!(report.replaced, [PathBuf::from("config")]);
    assert_eq!(report.replaced_dir.as_deref(), Some(kept.as_path()));
}

// A file of the user's (a dump) where the release has a folder cannot stay: it is kept in Replaced by update.
#[test]
fn a_users_file_where_the_release_has_a_folder_is_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("Library").join("game");
    let src = tmp.path().join("release");
    put(&dest.join("bios.bin"), b"my bios");
    put(&src.join("bios.bin").join("readme.txt"), b"a folder now");
    merge(&src, &dest, &guard(tmp.path(), "game"), &mut plain_move).unwrap();
    assert_eq!(read(&dest.join("bios.bin").join("readme.txt")), "a folder now");
    let kept = tmp.path().join("Saves Backup").join(REPLACED_BY_UPDATE).join("game").join("2026-11-04_10-00-00");
    assert_eq!(read(&kept.join("bios.bin")), "my bios");
}

// F-D3: a subfolder the user replaced with a link stays a link; the release's files go through it.
#[cfg(unix)]
#[test]
fn a_linked_subfolder_stays_a_link() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("Library").join("game");
    let src = tmp.path().join("release");
    let elsewhere = tmp.path().join("SD").join("mods");
    put(&elsewhere.join("old.pak"), b"my mod");
    fs::create_dir_all(&dest).unwrap();
    std::os::unix::fs::symlink(&elsewhere, dest.join("mods")).unwrap();
    put(&src.join("mods").join("base.pak"), b"release mod");
    merge(&src, &dest, &guard(tmp.path(), "game"), &mut plain_move).unwrap();
    assert!(fs::symlink_metadata(dest.join("mods")).unwrap().file_type().is_symlink(), "the link is kept");
    assert_eq!(read(&elsewhere.join("base.pak")), "release mod");
    assert_eq!(read(&elsewhere.join("old.pak")), "my mod");
}

// The plain guard (upstream's merge, tests only) still replaces whatever is there.
#[test]
fn the_plain_guard_replaces_everything() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("game");
    let src = tmp.path().join("release");
    put(&dest.join("x.sav"), b"old");
    put(&src.join("x.sav"), b"new");
    let report = merge(&src, &dest, &Guard::plain(tmp.path()), &mut plain_move).unwrap();
    assert_eq!(read(&dest.join("x.sav")), "new");
    assert!(report.is_empty());
}

// Review S1: a release's own `save*` program files are updated; a save extension or a `save` name on generic data is
// still the user's. Review C3: a track a cue sheet names is the user's too.
#[test]
fn only_real_save_files_and_named_tracks_stay_the_users() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("Library").join("game");
    let src = tmp.path().join("release");
    for (name, mine, theirs) in [
        ("save_icon.png", "old icon", "new icon"),
        ("savemanager.lua", "old script", "new script"),
        ("save.json", "my save in json", "release default"),
        ("game.x86_64", "old program", "new program"),
        ("savegame.dat", "my save", "release save"),
        ("slot.srm", "my srm", "release srm"),
        ("SAVE1", "my save1", "release save1"),
        ("disc/Game (Track 2).bin", "my track", "release track"),
    ] {
        put(&dest.join(name), mine.as_bytes());
        put(&src.join(name), theirs.as_bytes());
    }
    put(&dest.join("disc/Game.cue"), b"FILE \"Game (Track 2).bin\" BINARY\n");
    let report = merge(&src, &dest, &guard(tmp.path(), "game"), &mut plain_move).unwrap();
    assert_eq!(read(&dest.join("save_icon.png")), "new icon");
    assert_eq!(read(&dest.join("savemanager.lua")), "new script");
    assert_eq!(read(&dest.join("savegame.dat")), "my save");
    assert_eq!(read(&dest.join("slot.srm")), "my srm");
    assert_eq!(read(&dest.join("SAVE1")), "my save1");
    assert_eq!(read(&dest.join("disc/Game (Track 2).bin")), "my track");
    let mut kept: Vec<String> = report.kept_yours.iter().map(|p| p.to_string_lossy().replace('\\', "/")).collect();
    kept.sort();
    assert_eq!(kept, ["SAVE1", "disc/Game (Track 2).bin", "savegame.dat", "slot.srm"]);
    // Recheck 3: a `save*` file the update does replace is kept in Replaced by update, never deleted; a plain program
    // file is replaced as before.
    assert_eq!(read(&dest.join("save.json")), "release default");
    let aside = tmp.path().join("Saves Backup").join(REPLACED_BY_UPDATE).join("game").join("2026-11-04_10-00-00");
    assert_eq!(read(&aside.join("save.json")), "my save in json");
    assert_eq!(read(&aside.join("save_icon.png")), "old icon");
    assert_eq!(read(&aside.join("savemanager.lua")), "old script");
    assert!(!aside.join("game.x86_64").exists());
    assert_eq!(read(&dest.join("game.x86_64")), "new program");
    let mut replaced: Vec<String> = report.replaced.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    replaced.sort();
    assert_eq!(replaced, ["save.json", "save_icon.png", "savemanager.lua"]);
}

// Review C4: with a save folder that is the whole port folder, every file a release replaces is kept, not deleted.
#[test]
fn with_the_whole_folder_as_save_folder_every_replaced_file_is_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("Library").join("game");
    let src = tmp.path().join("release");
    put(&dest.join("progress.dat"), b"my progress");
    put(&src.join("progress.dat"), b"release default");
    let port = port_from_value(&serde_json::json!({"name": "n", "folder": "game", "source": "https://example.com/x.zip", "save": "."})).unwrap();
    let g = Guard::protecting(Protect::for_port(&port, &dest, None).unwrap(), &tmp.path().join("Saves Backup"), "game", &tmp.path().join("Library"), "s");
    let report = merge(&src, &dest, &g, &mut plain_move).unwrap();
    assert_eq!(read(&dest.join("progress.dat")), "release default");
    assert_eq!(read(&tmp.path().join("Saves Backup").join(REPLACED_BY_UPDATE).join("game").join("s").join("progress.dat")), "my progress");
    assert_eq!(report.replaced, [PathBuf::from("progress.dat")]);
}

// Review C2: an unreadable record of connected game files refuses the guard (the install stops before anything changes).
#[test]
fn an_unreadable_record_refuses_the_guard() {
    let tmp = tempfile::tempdir().unwrap();
    put(&tmp.path().join("rom_library.json"), b"{ damaged");
    assert!(Protect::for_port(&port("game"), &tmp.path().join("game"), Some(tmp.path())).is_err());
}

// Review C6: a move that fails after its copy landed is undone too (or named as stuck), never left behind.
#[test]
fn a_move_that_fails_after_landing_is_undone() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("Library").join("game");
    let src = tmp.path().join("release");
    put(&src.join("new.bin"), b"new");
    let mut mover = |from: &Path, to: &Path| -> io::Result<()> {
        fs::rename(from, to)?;
        Err(io::Error::other("failed after the rename"))
    };
    assert!(merge(&src, &dest, &guard(tmp.path(), "game"), &mut mover).is_err());
    assert!(!dest.join("new.bin").exists(), "the landed file was moved back");
    assert_eq!(read(&src.join("new.bin")), "new");
}

#[test]
fn the_report_is_taken_once_per_port_folder() {
    let dir = PathBuf::from("/nowhere/take-once");
    record(&dir, MergeReport::default());
    assert_eq!(take(&dir), None, "an empty report is not recorded");
    // Review S2: a newer merge with nothing to say clears an older merge's report.
    record(&dir, MergeReport { kept_yours: vec!["old.sav".into()], ..MergeReport::default() });
    record(&dir, MergeReport::default());
    assert_eq!(take(&dir), None, "no stale line from an earlier merge");
    let s3 = MergeReport { saves_set_aside: Some(PathBuf::from("/b/Replaced by restore/game/stamp")), saves_put_back: true, ..MergeReport::default() };
    assert_eq!(s3.line("update", &|p: &Path| p.display().to_string()).as_deref(), Some("Saves kept from an earlier uninstall were put back; the saves that were there were moved to /b/Replaced by restore/game first."));
    // Recheck 1: when the kept copy could not go back, the line never says it did.
    let failed = MergeReport { saves_put_back: false, ..s3 };
    assert_eq!(
        failed.line("update", &|p: &Path| p.display().to_string()).as_deref(),
        Some("Saves kept from an earlier uninstall could not be put back; they stay in /b/Pending Restore/game. The saves that were there were moved to /b/Replaced by restore/game.")
    );
    record(&dir, MergeReport { kept_yours: vec!["a.sav".into()], ..MergeReport::default() });
    assert_eq!(take(&dir).map(|r| r.kept_yours.len()), Some(1));
    assert_eq!(take(&dir), None);
    let one = MergeReport { kept_yours: vec!["a.sav".into()], ..MergeReport::default() };
    assert_eq!(one.line("extras", &|_| String::new()).as_deref(), Some("Kept your files: 1 file in the extras had the same name as yours; yours is still in place."));
}
