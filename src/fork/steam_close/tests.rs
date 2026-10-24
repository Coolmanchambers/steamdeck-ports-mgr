use super::*;
use crate::core::models::{port_from_value, Port};
use crate::fork::backups;
use crate::fork::settings::ForkSettings;
use crate::fork::shortcuts;
use crate::fork::steam_queue::{ArtChoice, QueuedChange, QueuedKind, SteamQueue};
use crate::fork::store::ShortcutsFile;
use crate::fork::vdf::fixtures::{file_of, steam_entry};
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fs;

/// A Steam that answers the process check from a script (the last answer repeats) and counts what it was asked.
struct FakeSteam {
    answers: RefCell<VecDeque<io::Result<Vec<String>>>>,
    last: RefCell<Option<Vec<String>>>,
    checks: Cell<usize>,
    shutdowns: Cell<usize>,
    starts: Cell<usize>,
    shutdown_error: Option<String>,
}

impl FakeSteam {
    fn new(answers: Vec<Option<&[&str]>>) -> FakeSteam {
        FakeSteam {
            answers: RefCell::new(answers.into_iter().map(|a| a.map(|p| p.iter().map(|s| s.to_string()).collect()).ok_or_else(|| io::Error::other("no /proc"))).collect()),
            last: RefCell::new(None),
            checks: Cell::new(0),
            shutdowns: Cell::new(0),
            starts: Cell::new(0),
            shutdown_error: None,
        }
    }

    fn up() -> FakeSteam {
        FakeSteam::new(vec![Some(&["4242 steam", "4300 steamwebhelper"])])
    }
}

impl SteamControl for FakeSteam {
    fn running(&self) -> io::Result<Vec<String>> {
        self.checks.set(self.checks.get() + 1);
        let next = self.answers.borrow_mut().pop_front();
        match next {
            Some(Ok(p)) => {
                *self.last.borrow_mut() = Some(p.clone());
                Ok(p)
            }
            Some(Err(e)) => Err(e),
            None => self.last.borrow().clone().ok_or_else(|| io::Error::other("no answer")),
        }
    }
    fn request_shutdown(&self) -> Result<(), String> {
        self.shutdowns.set(self.shutdowns.get() + 1);
        self.shutdown_error.clone().map_or(Ok(()), Err)
    }
    fn start(&self) -> Result<(), String> {
        self.starts.set(self.starts.get() + 1);
        Ok(())
    }
}

fn secs(s: f64) -> Duration {
    Duration::from_secs_f64(s)
}

fn up() -> io::Result<Vec<String>> {
    Ok(vec!["4242 steam".into()])
}

fn gone() -> io::Result<Vec<String>> {
    Ok(Vec::new())
}

// -- finding Steam and who started us ----------------------------------------------------------------

// PATH lists and absolute paths are the Linux ones.
#[cfg(unix)]
#[test]
fn steam_is_found_on_path_then_in_the_usual_places_then_as_a_flatpak() {
    let have = |files: &'static [&'static str]| move |p: &Path| files.iter().any(|f| Path::new(f) == p);
    let path = std::ffi::OsString::from("/home/deck/bin:/usr/local/bin:relative");
    let c = find_steam(Some(path.as_os_str()), None, &have(&["/usr/local/bin/steam", "/usr/bin/steam"])).unwrap();
    assert_eq!((c.program.as_path(), c.shutdown_args(), c.start_args()), (Path::new("/usr/local/bin/steam"), vec!["-shutdown".to_string()], vec![]));
    let c = find_steam(Some(path.as_os_str()), None, &have(&["/usr/bin/steam"])).unwrap();
    assert_eq!(c.program, PathBuf::from("/usr/bin/steam"), "SteamOS keeps it in /usr/bin even with a PATH that misses it");
    assert_eq!(find_steam(None, None, &have(&["/usr/games/steam"])).unwrap().program, PathBuf::from("/usr/games/steam"));
    // Flatpak, per user or system wide, only with flatpak itself on PATH.
    let home = Path::new("/home/deck");
    let fp = find_steam(Some(path.as_os_str()), Some(home), &have(&["/usr/local/bin/flatpak", "/home/deck/.local/share/flatpak/app/com.valvesoftware.Steam/current/active/metadata"])).unwrap();
    assert_eq!((fp.program.clone(), fp.shutdown_args()), (PathBuf::from("/usr/local/bin/flatpak"), vec!["run".to_string(), "com.valvesoftware.Steam".to_string(), "-shutdown".to_string()]));
    assert_eq!(fp.start_args(), ["run", "com.valvesoftware.Steam"]);
    assert!(find_steam(Some(path.as_os_str()), Some(home), &have(&["/var/lib/flatpak/app/com.valvesoftware.Steam/current/active/metadata", "/usr/local/bin/flatpak"])).is_some());
    assert!(find_steam(Some(path.as_os_str()), Some(home), &have(&["/var/lib/flatpak/app/com.valvesoftware.Steam/current/active/metadata"])).is_none(), "no flatpak program");
    assert!(find_steam(Some(path.as_os_str()), Some(home), &have(&[])).is_none());
    // A relative PATH entry is never used.
    assert!(find_steam(Some(std::ffi::OsStr::new("relative")), None, &|p: &Path| p == Path::new("relative/steam")).is_none());
}

fn fake_proc(root: &Path, procs: &[(u32, &str, u32)]) {
    for (pid, comm, ppid) in procs {
        let dir = root.join(pid.to_string());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("stat"), format!("{pid} ({comm}) S {ppid} {pid} {pid} 0 -1 4194560 0 0\n")).unwrap();
    }
}

#[test]
fn an_app_started_by_steam_or_its_reaper_is_seen_and_one_started_from_the_desktop_is_not() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    // Desktop Mode, started from the menu: plasmashell > ports_launcher.
    fake_proc(root, &[(900, "ports_launcher", 800), (800, "plasmashell", 700), (700, "systemd", 1)]);
    assert!(!started_by_steam(root, 900));
    // Started from Steam: steam > reaper > sh > ports_launcher.
    fake_proc(root, &[(500, "ports_launcher", 400), (400, "sh", 300), (300, "reaper", 200), (200, "steam", 1)]);
    assert!(started_by_steam(root, 500));
    // A name with spaces and brackets is read whole; this app's own name never counts.
    fake_proc(root, &[(60, "steam", 50), (50, "my (odd) shell", 40), (40, "kwin_wayland", 1)]);
    assert!(!started_by_steam(root, 60), "the app itself is not its own parent");
    fake_proc(root, &[(61, "x", 51), (51, "a) (b", 30), (30, "steamwebhelper", 1)]);
    assert!(started_by_steam(root, 61));
    // A broken chain (no stat for the parent) ends the walk.
    fake_proc(root, &[(70, "ports_launcher", 69)]);
    assert!(!started_by_steam(root, 70));
    assert!(!started_by_steam(&root.join("missing"), 1));
}

// -- the flow, with a fake Steam ------------------------------------------------------------------------

#[test]
fn in_game_mode_steam_is_never_asked_to_close_or_even_looked_at() {
    let steam = FakeSteam::up();
    assert!(matches!(begin(4, true, false, &steam, secs(0.0)), Start::Blocked(Blocked::GameMode)));
    assert_eq!((steam.checks.get(), steam.shutdowns.get(), steam.starts.get()), (0, 0, 0));
    let p = blocked_page(&Blocked::GameMode, 4);
    assert_eq!(p.title, "Finish in Desktop Mode");
    assert_eq!(p.paragraphs[0], GAME_MODE_TEXT);
    assert!(p.paragraphs[1].contains("4 Steam changes are waiting"));
    assert!(p.options.iter().all(|o| !matches!(o.action, CloseAction::CloseSteam | CloseAction::StartSteam)));
}

#[test]
fn nothing_is_asked_of_steam_without_a_queue_when_started_by_steam_or_when_steam_cannot_be_checked() {
    let steam = FakeSteam::up();
    assert!(matches!(begin(0, false, false, &steam, secs(0.0)), Start::Blocked(Blocked::NothingQueued)));
    assert!(matches!(begin(3, false, true, &steam, secs(0.0)), Start::Blocked(Blocked::StartedBySteam)));
    assert_eq!(steam.shutdowns.get(), 0);
    let broken = FakeSteam::new(vec![None]);
    assert!(matches!(begin(3, false, false, &broken, secs(0.0)), Start::Blocked(Blocked::CheckFailed(_))));
    assert_eq!(broken.shutdowns.get(), 0);
    let mut refuses = FakeSteam::up();
    refuses.shutdown_error = Some("the steam program was not found".into());
    match begin(3, false, false, &refuses, secs(0.0)) {
        Start::Blocked(Blocked::ShutdownFailed(e)) => assert!(e.contains("not found")),
        other => panic!("{other:?}"),
    }
    let p = blocked_page(&Blocked::StartedBySteam, 3);
    assert!(p.paragraphs[0].contains("started from Steam") && p.paragraphs[1].contains("3 Steam changes are still waiting"), "{:?}", p.paragraphs);
}

#[test]
fn steam_already_closed_applies_at_once_without_asking_it_anything() {
    let steam = FakeSteam::new(vec![Some(&[])]);
    assert!(matches!(begin(2, false, false, &steam, secs(0.0)), Start::ApplyNow(_)));
    assert_eq!(steam.shutdowns.get(), 0);
}

#[test]
fn steam_counts_as_closed_only_after_it_stays_gone_and_a_return_starts_the_wait_over() {
    let steam = FakeSteam::up();
    let Start::Closing(mut closer) = begin(2, false, false, &steam, secs(10.0)) else { panic!() };
    assert_eq!(steam.shutdowns.get(), 1, "asked once, the normal way");
    assert!(matches!(closer.poll(up(), secs(10.5)), Poll::Waiting { seconds: 0 }));
    assert!(matches!(closer.poll(gone(), secs(12.0)), Poll::Waiting { seconds: 2 }));
    // A helper process comes back for a moment: the settle time starts again.
    assert!(matches!(closer.poll(up(), secs(13.0)), Poll::Waiting { .. }));
    assert!(matches!(closer.poll(gone(), secs(14.0)), Poll::Waiting { .. }));
    assert!(matches!(closer.poll(gone(), secs(16.9)), Poll::Waiting { seconds: 6 }));
    assert!(matches!(closer.poll(gone(), secs(17.0)), Poll::Closed(_)));
    assert!(matches!(closer.poll(Err(io::Error::other("x")), secs(18.0)), Poll::CheckFailed(_)));
}

#[test]
fn the_wait_gives_up_at_the_timeout_while_steam_is_still_up() {
    let mut closer = Closer::new(secs(0.0));
    assert!(matches!(closer.poll(up(), TIMEOUT - secs(0.5)), Poll::Waiting { .. }));
    match closer.poll(up(), TIMEOUT) {
        Poll::TimedOut { still } => assert_eq!(still, ["4242 steam"]),
        other => panic!("{other:?}"),
    }
}

// -- end to end on a fake home: queue -> close -> apply --------------------------------------------------

struct Fixture {
    _dir: tempfile::TempDir,
    home: PathBuf,
    library: PathBuf,
    vdf: PathBuf,
    queue: PathBuf,
    settings: PathBuf,
}

fn catalog() -> Vec<Port> {
    [json!({"name": "Ship of Harkinian", "folder": "SoH", "source": "https://github.com/a/soh", "executable": "soh.appimage"})].iter().map(|v| port_from_value(v).unwrap()).collect()
}

/// A Steam account whose "Ship of Harkinian" shortcut still starts the old copy in `/old`, the port moved
/// into the ports folder, and the repoint for it waiting in the queue: Tim's Deck after an import.
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let user = home.join(".local/share/Steam/userdata/1234/config");
    fs::create_dir_all(&user).unwrap();
    let vdf = user.join("shortcuts.vdf");
    fs::write(&vdf, file_of(&[steam_entry(0, 0xAAAA_0001, "Ship of Harkinian", "\"/old/SoH/soh.appimage\"", "\"/old/SoH/\"", &[])])).unwrap();
    let library = dir.path().join("sd").join("Ports");
    let exe = library.join("SoH").join("soh.appimage");
    fs::create_dir_all(exe.parent().unwrap()).unwrap();
    fs::write(&exe, b"x").unwrap();
    let queue = dir.path().join("app/steam-queue.json");
    fs::create_dir_all(queue.parent().unwrap()).unwrap();
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
        queued_at: "20261024-100000".into(),
        repoint: Some(("/old/SoH".into(), library.join("SoH").to_string_lossy().into_owned())),
    });
    steam_queue::save(&queue, &q).unwrap();
    let settings = dir.path().join("app/fork-settings.json");
    Fixture { _dir: dir, home, library, vdf, queue, settings }
}

impl Fixture {
    fn apply(&self, closed: SteamClosed, check: &dyn SteamProcessCheck) -> QueueApplyOutcome {
        let settings = ForkSettings::load(&self.settings);
        let cat = catalog();
        let inputs = ApplyInputs { home: &self.home, library_dir: &self.library, catalog: &cat, favorite_exe: &|_| None, cache_dir: None, settings: &settings, game_mode: false };
        apply(closed, &self.queue, &inputs, check, SystemTime::now())
    }
}

#[test]
fn running_then_asked_to_close_then_gone_then_the_queue_is_applied_after_a_backup() {
    let f = fixture();
    let before = fs::read(&f.vdf).unwrap();
    let steam = FakeSteam::up();
    let Start::Closing(mut closer) = begin(1, false, false, &steam, secs(0.0)) else { panic!() };
    assert_eq!(steam.shutdowns.get(), 1);
    assert_eq!(fs::read(&f.vdf).unwrap(), before, "nothing is written while Steam closes");
    assert!(matches!(closer.poll(up(), secs(1.0)), Poll::Waiting { .. }));
    assert!(matches!(closer.poll(gone(), secs(2.0)), Poll::Waiting { .. }));
    let Poll::Closed(closed) = closer.poll(gone(), secs(5.0)) else { panic!("closed") };
    let out = f.apply(closed, &|| gone());
    assert!(out.wrote && !out.refused && out.done == 1 && out.dropped == 0, "{out:?}");
    assert!(!f.queue.exists(), "the queue is empty once applied");
    let list = backups::list_backups(&f.vdf).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(fs::read(&list[0].path).unwrap(), before, "the old list is backed up byte for byte");
    let saved = shortcuts::list(&ShortcutsFile::load(&f.vdf).unwrap().doc);
    assert_eq!(saved[0].exe_path(), f.library.join("SoH").join("soh.appimage").to_str().unwrap(), "the shortcut starts the new copy");
    assert_eq!(saved[0].app_id, Some(0xAAAA_0001), "same shortcut, same app ID (play time and art kept)");
    let p = result_page(&out, true);
    assert_eq!(p.title, "Steam shortcuts updated");
    assert_eq!(p.facts[..2], ["1 Steam change made.", "Your old shortcut list was backed up first."]);
    assert_eq!(p.options.iter().map(|o| o.action).collect::<Vec<_>>(), [CloseAction::StartSteam, CloseAction::Done]);
    assert!(p.options[0].recommended && p.initial == Focus::Option(0));
    assert_eq!(steam.starts.get(), 0, "Steam is started only when the user picks it");
}

#[test]
fn a_timeout_writes_nothing_and_keeps_the_queue_byte_for_byte() {
    let f = fixture();
    let (vdf, queue) = (fs::read(&f.vdf).unwrap(), fs::read(&f.queue).unwrap());
    let steam = FakeSteam::up();
    let Start::Closing(mut closer) = begin(1, false, false, &steam, secs(0.0)) else { panic!() };
    let mut now = secs(0.0);
    let polled = loop {
        now += POLL;
        match closer.poll(steam.running(), now) {
            Poll::Waiting { .. } => continue,
            other => break other,
        }
    };
    assert!(matches!(polled, Poll::TimedOut { .. }), "{polled:?}");
    assert!(now >= TIMEOUT && now < TIMEOUT + secs(1.0));
    assert_eq!(fs::read(&f.vdf).unwrap(), vdf);
    assert_eq!(fs::read(&f.queue).unwrap(), queue);
    assert!(backups::list_backups(&f.vdf).unwrap().is_empty());
    assert_eq!(steam.shutdowns.get(), 1, "asked once, never forced");
    let p = timed_out_page(1);
    assert!(p.paragraphs[0].contains("nothing was changed") && p.paragraphs[0].contains("1 Steam change is still waiting"), "{:?}", p.paragraphs);
    assert_eq!(p.options.iter().map(|o| o.action).collect::<Vec<_>>(), [CloseAction::TryAgain, CloseAction::NotNow]);
}

#[test]
fn steam_starting_again_before_the_write_is_refused_and_the_page_offers_try_again() {
    let f = fixture();
    let before = fs::read(&f.vdf).unwrap();
    let steam = FakeSteam::new(vec![Some(&[])]);
    let Start::ApplyNow(closed) = begin(1, false, false, &steam, secs(0.0)) else { panic!() };
    let out = f.apply(closed, &|| up());
    assert!(out.refused && !out.wrote && out.done == 0, "{out:?}");
    assert_eq!(fs::read(&f.vdf).unwrap(), before);
    assert!(f.queue.exists());
    let p = result_page(&out, false);
    assert_eq!(p.title, "Nothing was changed");
    assert!(p.paragraphs[0].starts_with("1 Steam change is still waiting"), "{:?}", p.paragraphs);
    assert_eq!(p.options.iter().map(|o| o.action).collect::<Vec<_>>(), [CloseAction::TryAgain, CloseAction::NotNow]);
    // Closed by this flow: Steam can still be started again from the failure page.
    let p = result_page(&out, true);
    assert!(p.options.iter().any(|o| o.action == CloseAction::StartSteam && !o.recommended));
}

// -- pages ------------------------------------------------------------------------------------------------

fn all_pages() -> Vec<ClosePage> {
    let ok = QueueApplyOutcome { text: "Ship of Harkinian: 1 shortcut(s) now point at /x.".into(), remaining: SteamQueue::default(), wrote: true, refused: false, done: 4, dropped: 1 };
    let refused = QueueApplyOutcome { refused: true, wrote: false, done: 0, dropped: 0, ..ok.clone_for_test() };
    vec![
        ask_page(4, None, false),
        ask_page(1, Some("Queued 1 change."), true),
        game_mode_page(4),
        closing_page(12),
        timed_out_page(4),
        blocked_page(&Blocked::NothingQueued, 0),
        blocked_page(&Blocked::StartedBySteam, 4),
        blocked_page(&Blocked::CheckFailed("x".into()), 4),
        blocked_page(&Blocked::ShutdownFailed("y".into()), 4),
        result_page(&ok, true),
        result_page(&ok, false),
        result_page(&refused, true),
        result_page(&refused, false),
        start_failed_page("sh exited with 1"),
        details_page("a\n\nb"),
    ]
}

impl QueueApplyOutcome {
    fn clone_for_test(&self) -> QueueApplyOutcome {
        QueueApplyOutcome { text: self.text.clone(), remaining: self.remaining.clone(), wrote: self.wrote, refused: self.refused, done: self.done, dropped: self.dropped }
    }
}

#[test]
fn b_on_every_page_is_the_safe_way_out_and_never_closes_or_starts_steam() {
    for p in all_pages() {
        let b = back_action(&p);
        assert!(!matches!(b, CloseAction::CloseSteam | CloseAction::StartSteam | CloseAction::TryAgain), "{}: {b:?}", p.window_title);
        let offered = p.options.iter().map(|o| o.action).chain(p.buttons.iter().flatten().map(|b| b.action)).collect::<Vec<_>>();
        assert!(offered.contains(&b) || b == CloseAction::Done, "{}: B maps to something on the page", p.window_title);
        assert!(p.window_title.starts_with("Update Steam: "), "{}", p.window_title);
        assert!(p.options.len() <= 3 && p.paragraphs.len() <= 3, "{}", p.window_title);
        assert!(!crate::fork::page::focusables(&p).is_empty(), "{}", p.window_title);
    }
}

#[test]
fn no_page_but_details_shows_a_path() {
    for p in all_pages().into_iter().filter(|p| p.title != "Details") {
        let text = [p.title.clone(), p.paragraphs.join(" "), p.facts.join(" "), p.options.iter().map(|o| format!("{} {}", o.title, o.detail)).collect::<Vec<_>>().join(" ")].join(" ");
        assert!(!text.contains('/') && !text.contains("shortcuts.vdf"), "{}: {text}", p.window_title);
    }
}

#[test]
fn the_ask_page_starts_on_close_unless_it_came_up_by_itself() {
    let p = ask_page(4, None, false);
    assert_eq!(p.options[0].title, CLOSE_LABEL);
    assert!(p.options[0].recommended);
    assert_eq!(p.initial, Focus::Option(0));
    assert!(p.paragraphs[0].starts_with("4 Steam changes are waiting"));
    let p = ask_page(1, Some("Queued 1 change."), true);
    assert_eq!(p.initial, Focus::Option(1), "a stray A on a page that popped up does not close Steam");
    assert_eq!((p.paragraphs[0].as_str(), p.paragraphs[1].as_str()), ("Queued 1 change.", "1 Steam change is waiting. Steam has to be closed to make it."));
}

#[test]
fn the_closing_page_counts_the_wait_and_offers_only_stop_waiting() {
    let p = closing_page(12);
    assert_eq!(p.facts, [format!("Waited 12 s of up to {} s.", TIMEOUT.as_secs())]);
    assert_eq!(p.options.iter().map(|o| o.action).collect::<Vec<_>>(), [CloseAction::StopWaiting]);
    assert_eq!(back_action(&p), CloseAction::StopWaiting);
}

#[test]
fn dropped_changes_are_told_and_the_details_hold_the_full_text() {
    let out = QueueApplyOutcome { text: "first\n\nsecond".into(), remaining: SteamQueue::default(), wrote: false, refused: false, done: 0, dropped: 2 };
    let p = result_page(&out, false);
    assert_eq!(p.facts, ["0 Steam changes made.", "2 changes could not be made any more: Details says why."]);
    assert_eq!(p.options.iter().map(|o| o.action).collect::<Vec<_>>(), [CloseAction::Done]);
    assert_eq!(details_page(&out.text).paragraphs, ["first", "second"]);
}

#[cfg(unix)]
#[test]
fn the_real_steam_command_runs_detached_with_the_shutdown_argument() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let log = d.path().join("args.txt");
    let script = d.path().join("steam");
    fs::write(&script, format!("#!/bin/sh\necho \"$@\" > '{}.tmp' && mv '{}.tmp' '{}'\n", log.display(), log.display(), log.display())).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let steam = SystemSteam { command: Some(SteamCommand { program: script.clone(), prefix: Vec::new() }), check: SystemSteamCheck::new() };
    steam.request_shutdown().unwrap();
    for _ in 0..100 {
        if log.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(fs::read_to_string(&log).unwrap().trim(), "-shutdown");
    fs::remove_file(&log).unwrap();
    steam.start().unwrap();
    for _ in 0..100 {
        if log.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(fs::read_to_string(&log).unwrap().trim(), "", "started with no arguments");
    let none = SystemSteam { command: None, check: SystemSteamCheck::new() };
    assert!(none.request_shutdown().unwrap_err().contains("not found"));
}
