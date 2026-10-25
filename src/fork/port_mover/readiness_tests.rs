//! When an old copy may go: Waiting for Steam / Start it once / Ready, what records it, and that `remove` never
//! takes one that is not Ready.

use super::old_copies::{self, mark_started, mark_steam_switched, note_launch, readiness, readiness_now, steam_since, steam_view, OldCopy, Readiness, RemovalCtx, Started, StartedBy, SteamView};
use crate::fork::steam_queue::{self, ArtChoice, QueuedChange, QueuedKind, SteamQueue};
use crate::fork::vdf::fixtures::{file_of, steam_entry_played};
use std::fs;
use std::path::{Path, PathBuf};

fn copy(old: &Path, new: &Path) -> OldCopy {
    OldCopy {
        port_key: "a/soh".into(),
        name: "Ship of Harkinian".into(),
        old_path: old.to_path_buf(),
        new_path: new.to_path_buf(),
        bytes: 3,
        moved_at: "20261024-100000".into(),
        link_only: false,
        switched: true,
        import_folder: None,
        started: None,
        steam_at: None,
        repoint_queued: None,
    }
}

fn local(stamp: &str) -> i64 {
    chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%d-%H%M%S").unwrap().and_local_timezone(chrono::Local).single().unwrap().timestamp()
}

#[test]
fn each_status_follows_from_steam_and_the_starts() {
    // Moved with Steam closed: the move switched the shortcut, so its time is when Steam starts begin to count.
    let c = OldCopy { repoint_queued: Some(false), ..copy(Path::new("/home/deck/Games/SoH"), Path::new("/run/media/deck/SD/Ports/SoH")) };
    let moved = local("20261024-100000");
    let quiet = SteamView::default();
    assert_eq!(readiness(&c, &quiet), Readiness::StartOnce, "moved, Steam switched, never started");
    assert_eq!(readiness(&c, &SteamView { waiting: true, played_new_at: Some(moved + 99) }), Readiness::WaitingForSteam, "Steam first, whatever else");
    let started = OldCopy { started: Some(Started { by: StartedBy::User, at: 5 }), ..c.clone() };
    assert_eq!(readiness(&started, &quiet), Readiness::Ready(StartedBy::User));
    assert_eq!(readiness(&started, &SteamView { waiting: true, played_new_at: None }), Readiness::WaitingForSteam, "a confirm does not beat a shortcut that still starts the old copy");
    // Started from Steam after the move (the shortcut was switched with it).
    assert_eq!(readiness(&c, &SteamView { waiting: false, played_new_at: Some(moved + 60) }), Readiness::Ready(StartedBy::Steam));
    assert_eq!(readiness(&c, &SteamView { waiting: false, played_new_at: Some(moved - 60) }), Readiness::StartOnce, "a start from before the move is the old copy's");
    // A queued switch applied later: only a start after that counts.
    let switched_late = OldCopy { steam_at: Some(moved + 3600), ..c.clone() };
    assert_eq!(steam_since(&switched_late), Some(moved + 3600));
    assert_eq!(readiness(&switched_late, &SteamView { waiting: false, played_new_at: Some(moved + 60) }), Readiness::StartOnce, "started before its shortcut was switched: that was the old copy");
    assert_eq!(readiness(&switched_late, &SteamView { waiting: false, played_new_at: Some(moved + 3601) }), Readiness::Ready(StartedBy::Steam));
    // No move time known (an old record): only the app or the user can tell.
    let unknown = OldCopy { moved_at: String::new(), ..c.clone() };
    assert_eq!(readiness(&unknown, &SteamView { waiting: false, played_new_at: Some(moved + 60) }), Readiness::StartOnce);
    // The change was queued and never applied by this app (fixed by hand), or the record does not say: a Steam
    // start after the move could have been the old copy's, so it does not count.
    for queued in [Some(true), None] {
        let q = OldCopy { repoint_queued: queued, ..c.clone() };
        assert_eq!(steam_since(&q), None, "{queued:?}");
        assert_eq!(readiness(&q, &SteamView { waiting: false, played_new_at: Some(moved + 60) }), Readiness::StartOnce, "{queued:?}");
    }
    for (r, line) in [(Readiness::WaitingForSteam, "Waiting for Steam to update"), (Readiness::StartOnce, "Start it once from its new place"), (Readiness::Ready(StartedBy::App), "Ready to remove")] {
        assert_eq!(r.line(), line);
    }
}

struct Home {
    _dir: tempfile::TempDir,
    home: PathBuf,
    app: PathBuf,
    old: PathBuf,
    new: PathBuf,
    vdf: PathBuf,
}

/// A Steam account, an old copy and a new copy with the same files, and the record of the move.
fn home() -> Home {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let vdf = home.join(".local/share/Steam/userdata/1234/config/shortcuts.vdf");
    fs::create_dir_all(vdf.parent().unwrap()).unwrap();
    let app = dir.path().join("app");
    let old = home.join("Games").join("SoH");
    let new = dir.path().join("sd").join("Ports").join("SoH");
    for d in [&old, &new] {
        fs::create_dir_all(d).unwrap();
        fs::write(d.join("soh.appimage"), b"soh").unwrap();
    }
    fs::create_dir_all(&app).unwrap();
    old_copies::record(&app, copy(&old, &new)).unwrap();
    Home { _dir: dir, home, app, old, new, vdf }
}

impl Home {
    fn shortcut_at(&self, exe_dir: &Path, last_play_time: u32) {
        let exe = format!("\"{}\"", exe_dir.join("soh.appimage").display());
        let start = format!("\"{}\"", exe_dir.display());
        fs::write(&self.vdf, file_of(&[steam_entry_played(0, 0xAAAA_0001, "Ship of Harkinian", &exe, &start, &[], last_play_time)])).unwrap();
    }

    fn entry(&self) -> OldCopy {
        old_copies::load(&self.app).unwrap().remove(0)
    }

    fn entry_unchecked(&self) -> OldCopy {
        copy(&self.old, &self.new)
    }

    fn view(&self) -> SteamView {
        steam_view(&self.entry(), &self.home, &self.app)
    }

    fn queue_repoint(&self) {
        let mut q = SteamQueue::default();
        q.upsert(QueuedChange {
            account_id: 1234,
            account_label: String::new(),
            port_key: "a/soh".into(),
            name: "Ship of Harkinian".into(),
            kind: QueuedKind::Repoint,
            repoint_by_name: false,
            art: ArtChoice::None,
            from_game_mode: false,
            queued_at: String::new(),
            repoint: Some((self.old.to_string_lossy().into_owned(), self.new.to_string_lossy().into_owned())),
            icon: None,
        });
        steam_queue::save(&steam_queue::queue_path(&self.app), &q).unwrap();
    }
}

#[test]
fn steam_is_read_for_shortcuts_into_the_old_copy_queued_repoints_and_starts_of_the_new_one() {
    let h = home();
    let moved = local("20261024-100000") as u32;
    // Tim's Deck after the import: the shortcut still starts the old copy and its repoint waits in the queue.
    h.shortcut_at(&h.old, moved + 500);
    h.queue_repoint();
    assert_eq!(readiness(&h.entry(), &h.view()), Readiness::WaitingForSteam);
    // The queue applied (the shortcut now starts the new copy), not started since.
    fs::remove_file(steam_queue::queue_path(&h.app)).unwrap();
    mark_steam_switched(&h.app, "a/soh", &h.old, i64::from(moved)).unwrap();
    h.shortcut_at(&h.new, moved - 100);
    assert_eq!(h.view(), SteamView { waiting: false, played_new_at: Some(i64::from(moved) - 100) });
    assert_eq!(readiness(&h.entry(), &h.view()), Readiness::StartOnce);
    // Started from Steam after that.
    h.shortcut_at(&h.new, moved + 100);
    assert_eq!(readiness(&h.entry(), &h.view()), Readiness::Ready(StartedBy::Steam));
    // Only the queue left (shortcut fixed by hand, say): still waiting.
    h.queue_repoint();
    assert!(h.view().waiting);
    // A queue that cannot be read counts as waiting.
    fs::remove_file(steam_queue::queue_path(&h.app)).unwrap();
    fs::create_dir_all(steam_queue::queue_path(&h.app)).unwrap();
    assert!(h.view().waiting, "unreadable queue");
}

#[test]
fn a_switch_seen_without_a_recorded_time_is_noted_and_only_later_steam_starts_count() {
    let h = home();
    // A queued repoint, fixed by hand: Steam's shortcut already starts the new copy and says it was played after
    // the move, but that start may have been the old copy's.
    old_copies::record(&h.app, OldCopy { repoint_queued: Some(true), ..h.entry() }).unwrap();
    h.shortcut_at(&h.new, local("20261024-100000") as u32 + 100);
    assert_eq!(readiness_now(&h.entry(), &h.home, &h.app).unwrap(), Readiness::StartOnce);
    let noted = h.entry().steam_at.expect("the first look notes when Steam was seen switched");
    assert!((chrono::Utc::now().timestamp() - noted).abs() < 60);
    // A start after that counts.
    h.shortcut_at(&h.new, (noted + 30) as u32);
    assert_eq!(readiness_now(&h.entry(), &h.home, &h.app).unwrap(), Readiness::Ready(StartedBy::Steam));
    // The list cannot be read: an error, never the copy held in memory.
    fs::write(old_copies::path(&h.app), "not json").unwrap();
    assert!(readiness_now(&h.entry_unchecked(), &h.home, &h.app).is_err());
}

#[test]
fn a_steam_start_counts_only_with_a_recorded_switch_time_and_looking_writes_nothing() {
    let h = home();
    // The queue was applied long ago and no switch time was recorded (a record that never saw the old copies page); Steam now
    // starts the new copy, and applying left a backup of the shortcuts.
    old_copies::record(&h.app, OldCopy { repoint_queued: Some(true), ..h.entry() }).unwrap();
    h.shortcut_at(&h.new, 9_000);
    let backup = h.vdf.with_file_name("shortcuts.vdf.bak-20261024-110000");
    fs::write(&backup, b"x").unwrap();
    let applied = std::time::UNIX_EPOCH + std::time::Duration::from_secs(5_000);
    fs::File::options().write(true).open(&backup).unwrap().set_modified(applied).unwrap();
    let before = fs::read_to_string(old_copies::path(&h.app)).unwrap();
    // No recorded switch time: a backup's time is no stand-in, so the copy is "Start it once" (never closer than it is).
    assert_eq!(old_copies::readiness_read_only(&h.entry(), &h.home, &h.app), Readiness::StartOnce);
    assert_eq!(fs::read_to_string(old_copies::path(&h.app)).unwrap(), before, "looking wrote nothing into the record");
    // A recorded switch time is used as it is: a start after it is ready, one before it is not.
    old_copies::record(&h.app, OldCopy { steam_at: Some(2_000), repoint_queued: Some(true), ..h.entry() }).unwrap();
    assert_eq!(old_copies::readiness_read_only(&h.entry(), &h.home, &h.app), Readiness::Ready(StartedBy::Steam));
    h.shortcut_at(&h.new, 1_000);
    assert_eq!(old_copies::readiness_read_only(&h.entry(), &h.home, &h.app), Readiness::StartOnce);
}


#[test]
fn a_later_repoint_resets_when_a_steam_start_counts_and_the_page_and_the_checkup_agree() {
    let h = home();
    let moved = local("20261024-100000") as u32;
    // The move wrote its switch at once (so the move time would count), yet the shortcut was never switched, and the old copy was
    // played from Steam after the move.
    old_copies::record(&h.app, OldCopy { repoint_queued: Some(false), ..h.entry() }).unwrap();
    h.shortcut_at(&h.old, moved + 500);
    assert_eq!(readiness(&h.entry(), &h.view()), Readiness::WaitingForSteam, "it still starts the old copy");
    // The checkup repoints it now: its LastPlayTime (the old copy's play) is carried to the new path, as the real write does.
    let at = i64::from(moved) + 1_000;
    h.shortcut_at(&h.new, moved + 500);
    assert_eq!(old_copies::note_repoint(&h.app, &h.old, false, at).unwrap(), 1);
    assert_eq!(h.entry().steam_at, Some(at));
    for (name, ready) in [("the checkup", old_copies::readiness_read_only(&h.entry(), &h.home, &h.app)), ("the page", readiness_now(&h.entry(), &h.home, &h.app).unwrap())] {
        assert_eq!(ready, Readiness::StartOnce, "{name}: a play of the OLD copy before the repoint is not the new copy's start");
    }
    // A Steam start after the repoint is.
    h.shortcut_at(&h.new, (at + 100) as u32);
    for (name, ready) in [("the checkup", old_copies::readiness_read_only(&h.entry(), &h.home, &h.app)), ("the page", readiness_now(&h.entry(), &h.home, &h.app).unwrap())] {
        assert_eq!(ready, Readiness::Ready(StartedBy::Steam), "{name}");
    }
    // Queued instead (Steam running): no time yet, whatever is played; the queue's apply stamps it.
    old_copies::record(&h.app, OldCopy { steam_at: None, repoint_queued: Some(false), ..h.entry() }).unwrap();
    assert_eq!(old_copies::note_repoint(&h.app, &h.old, true, at).unwrap(), 1);
    assert_eq!((h.entry().steam_at, h.entry().repoint_queued), (None, Some(true)));
    h.shortcut_at(&h.new, (at + 100) as u32);
    assert_eq!(old_copies::readiness_read_only(&h.entry(), &h.home, &h.app), Readiness::StartOnce, "not before the queue is applied");
    mark_steam_switched(&h.app, "a/soh", &h.old, at + 200).unwrap();
    assert_eq!(old_copies::readiness_read_only(&h.entry(), &h.home, &h.app), Readiness::StartOnce, "a play from before the apply");
    h.shortcut_at(&h.new, (at + 300) as u32);
    assert_eq!(old_copies::readiness_read_only(&h.entry(), &h.home, &h.app), Readiness::Ready(StartedBy::Steam));
}

#[test]
fn a_queue_file_that_is_not_readable_is_never_moved_aside_by_looking() {
    let h = home();
    let q = steam_queue::queue_path(&h.app);
    fs::write(&q, "{ not json").unwrap();
    assert!(h.view().waiting, "an unreadable queue keeps the copy waiting");
    assert_eq!(fs::read_to_string(&q).unwrap(), "{ not json", "and the file is where it was");
    assert!(steam_queue::peek(&q).is_err());
    assert_eq!(fs::read_dir(&h.app).unwrap().filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().contains("corrupt")).count(), 0);
}

#[test]
fn starts_and_switches_are_recorded_once_and_survive_a_reload() {
    let h = home();
    assert_eq!(mark_started(&h.app, &h.old, StartedBy::User, 111).unwrap(), 1);
    assert_eq!(mark_started(&h.app, &h.old, StartedBy::App, 222).unwrap(), 0, "the first reason stays");
    assert_eq!(h.entry().started, Some(Started { by: StartedBy::User, at: 111 }));
    assert_eq!(mark_steam_switched(&h.app, "a/soh", &h.old, 333).unwrap(), 1);
    assert_eq!(h.entry().steam_at, Some(333));
    assert_eq!(mark_steam_switched(&h.app, "other/port", Path::new("/elsewhere"), 444).unwrap(), 0, "another port's switch changes nothing");
    let raw = fs::read_to_string(old_copies::path(&h.app)).unwrap();
    assert!(raw.contains("\"started_by\": \"user\"") && raw.contains("\"steam_at\": 333"), "{raw}");
}

#[test]
fn an_app_launch_counts_only_when_it_ran_long_enough_from_the_new_copy() {
    let h = home();
    assert_eq!(note_launch(&h.app, "a/soh", &h.new, 29, 1).unwrap(), 0, "a crash at start does not count");
    assert_eq!(note_launch(&h.app, "a/soh", &h.old, 600, 1).unwrap(), 0, "a run of the old copy does not count");
    assert_eq!(note_launch(&h.app, "b/other", &h.new, 600, 1).unwrap(), 0);
    assert!(h.entry().started.is_none());
    assert_eq!(note_launch(&h.app, "a/soh", &h.new, 30, 7).unwrap(), 1);
    assert_eq!(h.entry().started, Some(Started { by: StartedBy::App, at: 7 }));
    // A switch that never finished is not counted.
    let h2 = home();
    old_copies::record(&h2.app, OldCopy { switched: false, ..h2.entry() }).unwrap();
    assert_eq!(note_launch(&h2.app, "a/soh", &h2.new, 600, 1).unwrap(), 0);
}

fn running(_: &Path) -> Vec<String> {
    Vec::new()
}

#[test]
fn nothing_that_is_not_ready_can_be_removed_and_a_ready_one_still_goes_through_the_checks() {
    let h = home();
    let active = vec![h.new.clone()];
    let saves = |_: &OldCopy, _: &Path| Some(Vec::new());
    let ctx = RemovalCtx { home: &h.home, config_dir: &h.app, library_dir: h.new.parent().unwrap(), active_paths: &active, running: &running, save_folders: &saves };
    let never = |_: &OldCopy| -> Result<Option<PathBuf>, String> { panic!("nothing is zipped or removed for a copy that is not ready") };
    // Waiting for Steam: the shortcut still starts the old copy.
    h.shortcut_at(&h.old, 0);
    let why = old_copies::remove(&h.entry(), &ctx, &never).unwrap_err();
    assert!(why.contains("Steam still starts the old copy"), "{why}");
    // Even with a confirm on record.
    mark_started(&h.app, &h.old, StartedBy::User, 1).unwrap();
    assert!(old_copies::remove(&h.entry(), &ctx, &never).unwrap_err().contains("Steam still starts"));
    // Steam switched, but this record was never started (a second fixture without the confirm).
    let h2 = home();
    let active2 = vec![h2.new.clone()];
    let ctx2 = RemovalCtx { home: &h2.home, config_dir: &h2.app, library_dir: h2.new.parent().unwrap(), active_paths: &active2, running: &running, save_folders: &saves };
    h2.shortcut_at(&h2.new, 0);
    let stale = h2.entry();
    let why = old_copies::remove(&stale, &ctx2, &never).unwrap_err();
    assert!(why.contains("once from its new place"), "{why}");
    assert!(h2.old.is_dir());
    // Confirmed after the entry was read: removal reads the record again and goes on (saves zipped first).
    mark_started(&h2.app, &h2.old, StartedBy::User, 2).unwrap();
    let zipped = std::cell::Cell::new(false);
    old_copies::remove(&stale, &ctx2, &|_| {
        zipped.set(true);
        Ok(None)
    })
    .unwrap();
    assert!(zipped.get() && !h2.old.exists() && h2.new.is_dir());
    // Ready but the ordinary checks refuse (a program runs from it): kept.
    let h3 = home();
    h3.shortcut_at(&h3.new, 0);
    mark_started(&h3.app, &h3.old, StartedBy::App, 3).unwrap();
    let active3 = vec![h3.new.clone()];
    let busy = |_: &Path| vec!["soh".to_string()];
    let ctx3 = RemovalCtx { home: &h3.home, config_dir: &h3.app, library_dir: h3.new.parent().unwrap(), active_paths: &active3, running: &busy, save_folders: &saves };
    assert!(old_copies::remove(&h3.entry(), &ctx3, &never).unwrap_err().contains("running"));
    assert!(h3.old.is_dir());
}
