//! The Port page on screen: one installed port's options as cards (`crate::fork::port_page` says what they show), drawn
//! in the guided-page window (`page_dialog::show_page`). Opens from a right-click (or a long press) on a port's tile on
//! Home, a right-click on an installed row in All ports or Deck-ready, and X on Home.
//!
//! **Looking changes nothing.** What the app holds in memory is read at once; everything that reads the disk (the
//! programs, the save folders and backups, the settings files, Steam's shortcuts, the folder size, the port's checkup)
//! runs on a worker (`Background`), and the cards say "Looking..." until it answers. Every change is the existing flow
//! behind a card, with its own guards: the save backup (`save_backup::backup_before_update`), Restore saves, the save
//! finder, Add to Steam, the icon repair, the checkup's repoint, the Close Steam flow, the update and version picker, the
//! uninstall (`installer::uninstall_port_keeping` on a worker, with `install_launch::may_uninstall`'s guards).
//!
//! **For the install wizard** (the next branch): `open` (the whole page), `open_card` (one card's page, B going wherever
//! the caller says), `open_link` (the project page with its QR code) and `rom_screens::find_for_port` (the game files
//! pages) are the reusable pieces. Each takes a `Back` that runs when the user leaves.

use super::dialogs::{close_current_dialog, dialog_is_open, open_extra_install_confirm_dialog, open_info_dialog, open_reset_playtime_dialog};
use super::install_launch::{activate_port, mark_uninstalled, may_uninstall, open_version_picker, start_install};
use super::page_dialog::{show_page, show_page_handle, Handlers, PageHandle};
use super::playtime::{format_last_played, format_playtime, is_port_running, LastPlayed};
use super::state::AppState;
use crate::core::models::{Port, SourceType};
use crate::core::path_safety::{is_within, safe_join};
use crate::fork::art::thumbs::ArtRef;
use crate::fork::page::{Mark, Page};
use crate::fork::port_page::paths::Places;
use crate::fork::port_page::settings_files::{self, SettingsFile};
use crate::fork::port_page::{self as model, Act, Card, DiskFacts, LinkAct, PortFacts, SaveFolder, SaveSource, SteamFacts, SteamState, UpdateCheck};
use crate::fork::rom_screens::job::Background;
use crate::fork::settings::ForkSettings;
use crate::ui::gamepad_router::GamepadRouter;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

type Router = Rc<RefCell<GamepadRouter>>;
/// What runs when the user leaves a page (B on the first page, or on a card opened by itself).
pub(crate) type Back = Rc<dyn Fn(&Rc<AppState>, &Router)>;

const POLL: Duration = Duration::from_millis(150);
/// The files counted in a save folder, and the entries walked to size the port's folder.
const SAVE_FILES_CAP: usize = 5000;
const SIZE_ENTRIES_CAP: usize = 300_000;

thread_local! {
    static SESSION: RefCell<Option<Rc<Session>>> = const { RefCell::new(None) };
    static DISK_POLL: slint::Timer = slint::Timer::default();
    static WORK_POLL: slint::Timer = slint::Timer::default();
    static WORK: RefCell<Pollers> = RefCell::new(Pollers::default());
}

/// Jobs waiting for their answer (Check now, Back up now, Uninstall), each asked on every tick until it says it is done.
/// One timer serves them all, so one job finishing never stops another's polling.
#[derive(Default)]
struct Pollers(Vec<Box<dyn FnMut() -> bool>>);

/// One tick: every job is asked once; the ones done are dropped. A job's answer may start another job (it is added to
/// the same list, not lost). Returns whether any are left.
fn tick_pollers(cell: &RefCell<Pollers>) -> bool {
    let mut now = std::mem::take(&mut *cell.borrow_mut());
    now.0.retain_mut(|done| !done());
    let mut list = cell.borrow_mut();
    let added = std::mem::take(&mut list.0);
    now.0.extend(added);
    *list = now;
    !list.0.is_empty()
}

/// Polls `done` until it returns true (it has handled the job's answer).
fn run_work(done: impl FnMut() -> bool + 'static) {
    WORK.with(|w| w.borrow_mut().0.push(Box::new(done)));
    WORK_POLL.with(|t| {
        if !t.running() {
            t.start(slint::TimerMode::Repeated, Duration::from_millis(200), || {
                if !WORK.with(tick_pollers) {
                    WORK_POLL.with(slint::Timer::stop);
                }
            });
        }
    });
}

/// What is on screen now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    First,
    Card(Card),
    /// A settings file's View page.
    View(usize),
    /// A card waiting for the worker's facts.
    Wait(Card),
    /// A message under a card (a backup made); its button goes back to the card.
    Notice(Card),
    /// Work running under a card (a backup); B waits.
    Working(Card),
    /// The port is gone (uninstalled): the result page's button closes the Port page.
    Done(Card),
}

struct Session {
    key: String,
    back: Back,
    /// Opened straight on a card (the install wizard): B on that card leaves through `back`.
    direct: bool,
    places: Places,
    disk: RefCell<Option<Rc<DiskFacts>>>,
    job: RefCell<Option<Rc<Background<(DiskFacts, super::checkup_screen::PortSys)>>>>,
    update_check: RefCell<UpdateCheck>,
    screen: Cell<Screen>,
    handle: RefCell<Option<PageHandle<Act>>>,
}

fn current() -> Option<Rc<Session>> {
    SESSION.with(|s| s.borrow().clone())
}

fn is_current(s: &Rc<Session>) -> bool {
    current().is_some_and(|c| Rc::ptr_eq(&c, s))
}

fn port_of(app: &AppState, key: &str) -> Option<Port> {
    app.catalog.borrow().iter().find(|p| p.key() == key).cloned()
}

fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"].iter().find_map(|v| std::env::var_os(v)).map(PathBuf::from).filter(|p| p.is_absolute())
}

fn game_dir(app: &AppState, port: &Port) -> Option<PathBuf> {
    safe_join(&app.paths.library_dir(), &port.folder).ok()
}

// -- Opening --------------------------------------------------------------------------------------------------

/// The Port page of an installed port. `back` runs on B (Home: just close; the cursor stays on the tile).
pub(crate) fn open(app: &Rc<AppState>, router: &Router, key: &str, back: Back) {
    if let Some(s) = start(app, key, back, false) {
        show(app, router, &s, Screen::First);
    }
}

/// Home's X and right-click, and a right-click on an installed row: the Port page; B closes it.
pub(crate) fn open_from_list(app: &Rc<AppState>, router: &Router, port: &Port) {
    if dialog_is_open(app) || !app.is_installed(port) || port.is_android_only() {
        return;
    }
    open(app, router, port.key(), Rc::new(|app, router| close_current_dialog(app, router)));
}

/// A right-click on a row of All ports (or a card in big mode): the Port page of an installed port; for one that is not
/// installed, its project page, so it can be looked over before installing.
pub(crate) fn open_options_for(app: &Rc<AppState>, router: &Router, port: &Port) {
    if dialog_is_open(app) {
        return;
    }
    if app.is_installed(port) {
        return open_from_list(app, router, port);
    }
    if let Some(url) = model::project_url(port) {
        open_link(app, router, port.display_name(), &url, None);
    }
}

/// One card's page by itself (the install wizard's steps reuse them): B leaves through `back`. Play, Fix problems and
/// Game files do what their cards do.
pub(crate) fn open_card(app: &Rc<AppState>, router: &Router, key: &str, card: Card, back: Back) {
    if let Some(s) = start(app, key, back, true) {
        open_card_in(app, router, &s, card);
    }
}

fn start(app: &Rc<AppState>, key: &str, back: Back, direct: bool) -> Option<Rc<Session>> {
    let port = port_of(app, key)?;
    if !app.is_installed(&port) {
        return None;
    }
    let s = Rc::new(Session {
        key: key.to_string(),
        back,
        direct,
        places: Places::system(),
        disk: RefCell::new(None),
        job: RefCell::new(None),
        update_check: RefCell::new(UpdateCheck::NotAsked),
        screen: Cell::new(Screen::First),
        handle: RefCell::new(None),
    });
    SESSION.with(|x| *x.borrow_mut() = Some(s.clone()));
    eprintln!("port page: opened for {}", port.display_name());
    read_disk(app, &s);
    Some(s)
}

// -- Facts ----------------------------------------------------------------------------------------------------

fn port_facts(app: &AppState, s: &Session, port: &Port) -> PortFacts {
    let info = app.state.borrow().get(port.key()).cloned();
    let (badge, badge_mark) = super::home_screen::badge_of(app, port.key()).unwrap_or_else(|| ("Installed".to_string(), Mark::Done));
    let last_played = info.as_ref().and_then(|i| match format_last_played(&i.last_played_at) {
        LastPlayed::Never => None,
        LastPlayed::Today => Some("today".to_string()),
        LastPlayed::Date(d) => Some(format!("on {d}")),
    });
    let seconds = info.as_ref().map_or(0, |i| i.playtime_seconds);
    PortFacts {
        key: port.key().to_string(),
        name: port.display_name().to_string(),
        art: Some(ArtRef::of_port(port)),
        badge,
        badge_mark,
        last_played,
        playtime: (seconds > 0).then(|| format_playtime(seconds)),
        running: is_port_running(app, port.key()),
        busy: app.install_runtime.installing.borrow().contains(port.key()),
        user_managed: port.user_managed,
        has_playtime: seconds > 0,
        update_on: info.as_ref().is_none_or(|i| i.update),
        tracks_releases: matches!(port.source_type, SourceType::Github | SourceType::Gitlab) && port.repo.is_some(),
        has_extra: port.extra.is_some(),
        update_ready: super::home_screen::update_ready(app, port.key()),
        version: info.and_then(|i| i.installed_tag),
        game_mode: crate::fork::steam_queue::game_mode_from_env(),
        update_check: s.update_check.borrow().clone(),
        project_url: model::project_url(port),
    }
}

/// What the worker reads from: paths and plain values only.
struct DiskInput {
    port: Port,
    game_dir: PathBuf,
    library: PathBuf,
    config_dir: PathBuf,
    saves_backup_dir: PathBuf,
    settings_path: PathBuf,
    home: Option<PathBuf>,
    favorite: Option<String>,
    local_entry: Option<Value>,
    look: super::checkup_screen::PortLook,
}

fn read_disk(app: &Rc<AppState>, s: &Rc<Session>) {
    let Some(port) = port_of(app, &s.key) else { return };
    let Some(dir) = game_dir(app, &port) else { return };
    let input = DiskInput {
        favorite: app.state.borrow().get(port.key()).and_then(|i| i.favorite_exe.clone()),
        local_entry: crate::fork::rom_library::load_local_entries(&app.paths.config_dir.join("ports.local.json")).remove(&port.folder),
        look: super::checkup_screen::port_look(app, port.key()),
        game_dir: dir,
        library: app.paths.library_dir(),
        config_dir: app.paths.config_dir.clone(),
        saves_backup_dir: app.paths.saves_backup_dir.clone(),
        settings_path: app.paths.config_dir.join(crate::fork::settings::FILE_NAME),
        home: home_dir(),
        port,
    };
    let job = Rc::new(Background::spawn(move |_| gather(input)));
    *s.job.borrow_mut() = Some(job.clone());
    let (app, s) = (app.clone(), s.clone());
    DISK_POLL.with(|t| {
        t.start(slint::TimerMode::Repeated, POLL, move || {
            if !is_current(&s) || !s.job.borrow().as_ref().is_some_and(|j| Rc::ptr_eq(j, &job)) {
                return DISK_POLL.with(slint::Timer::stop);
            }
            let Some(done) = job.take() else { return };
            DISK_POLL.with(slint::Timer::stop);
            *s.job.borrow_mut() = None;
            match done {
                Ok((mut disk, sys)) => {
                    disk.checkup = super::checkup_screen::port_line(&app, &s.key, sys);
                    let busy = disk.checkup.is_none();
                    *s.disk.borrow_mut() = Some(Rc::new(disk));
                    disk_arrived(&app, &s);
                    if busy {
                        look_again_later(&app, &s);
                    }
                }
                Err(e) => eprintln!("port page: reading the port's files failed ({e})"),
            }
        });
    });
}

/// The checkup could not look (another job runs, the card says "Busy, look again later"): read again in a while, as long
/// as this Port page is still open on a page that shows the line.
fn look_again_later(app: &Rc<AppState>, s: &Rc<Session>) {
    let (app, s) = (app.clone(), s.clone());
    slint::Timer::single_shot(Duration::from_secs(10), move || {
        let open = s.handle.borrow().as_ref().is_some_and(PageHandle::is_open);
        if is_current(&s) && open && matches!(s.screen.get(), Screen::First | Screen::Card(_)) && s.job.borrow().is_none() {
            read_disk(&app, &s);
        }
    });
}

/// The worker's facts are in: the page on screen is drawn again with them (a card that was waiting for them opens).
fn disk_arrived(app: &Rc<AppState>, s: &Rc<Session>) {
    let Some(router) = ROUTER.with(|r| r.borrow().clone()) else { return };
    let open = s.handle.borrow().as_ref().is_some_and(PageHandle::is_open);
    if !open {
        return;
    }
    match s.screen.get() {
        Screen::Wait(card) => show(app, &router, s, Screen::Card(card)),
        screen @ (Screen::First | Screen::Card(_)) => redraw(app, s, screen),
        _ => {}
    }
}

thread_local! {
    /// The router the pages were opened with (the worker's answer arrives on a timer, outside any callback).
    static ROUTER: RefCell<Option<Router>> = const { RefCell::new(None) };
}

fn gather(i: DiskInput) -> (DiskFacts, super::checkup_screen::PortSys) {
    let port = &i.port;
    let dir = &i.game_dir;
    // Program to start.
    let candidates = if dir.is_dir() { crate::core::executable_detect::list_candidates(dir) } else { Vec::new() };
    let favorite_found = i.favorite.as_ref().and_then(|f| safe_join(dir, f).ok()).is_some_and(|p| p.exists());
    let catalog_exe = port.executable.as_ref().and_then(crate::core::platform_resolve::resolve_per_platform).and_then(|v| v.as_str().map(str::to_string)).filter(|s| !s.trim().is_empty());
    let program = model::ProgramFacts { game_dir: dir.clone(), candidates, favorite: i.favorite.clone(), favorite_found, catalog_exe };

    // Saves: the folder the user chose, else the catalog's.
    let overrides = crate::fork::save_finder::overrides(&ForkSettings::load(&i.settings_path));
    let choice = overrides.get(&port.folder);
    let no_saves = matches!(choice, Some(crate::fork::save_finder::Override::NoSaves));
    let mut found: Vec<(PathBuf, SaveSource)> = Vec::new();
    match choice {
        Some(c @ crate::fork::save_finder::Override::Folder(_)) => found.extend(crate::fork::save_finder::folder_of(c, dir).map(|p| (p, SaveSource::Finder))),
        _ if !no_saves => {
            for v in [port.save.as_ref(), port.save2.as_ref()].into_iter().flatten() {
                found.extend(crate::core::platform_resolve::resolve_save_folder(v, dir).map(|p| (p, SaveSource::Catalog)));
            }
        }
        _ => {}
    }
    // A "save folder" that holds the whole game ("save": ".") is not one to count or back up.
    found.retain(|(p, _)| !is_within(dir, p));
    let folders: Vec<SaveFolder> = found
        .into_iter()
        .map(|(path, source)| {
            let exists = path.is_dir();
            let (files, capped) = if exists { count_files(&path, SAVE_FILES_CAP) } else { (0, false) };
            SaveFolder { path, source, exists, files, capped }
        })
        .collect();
    let (backups, newest) = port_backups(&i.saves_backup_dir, &port.folder);
    let now = std::time::SystemTime::now();
    let saves = model::SavesFacts {
        folders,
        no_saves,
        last_backup_days: newest.map(|t| now.duration_since(t).map_or(0, |d| d.as_secs() / 86_400)),
        backups,
        unknown_on_linux: crate::core::save_backup::save_folder_unknown_on_linux(port, dir),
    };

    // Settings files.
    let spec = crate::fork::rom_library::RomSpec::for_port(port, i.local_entry.as_ref());
    let game_data: Vec<String> = spec.requirement.slots.iter().flat_map(|s| s.alternatives.iter().map(|a| a.display.clone())).collect();
    let save_dirs: Vec<PathBuf> = saves.folders.iter().filter(|f| f.exists).map(|f| f.path.clone()).collect();
    let settings = settings_files::discover(&settings_files::Input { game_dir: dir, save_dirs: &save_dirs, named: i.local_entry.as_ref().and_then(|e| e.get("settings")), game_data: &game_data });

    // Game files.
    let needs = crate::fork::rom_screens::find::FindRequest::new(port.clone(), true, i.local_entry.as_ref(), Vec::new()).needs_files();
    let mappings = crate::fork::rom_library::RomLibraryStore::load(&i.config_dir.join("rom_library.json")).map(|s| s.mappings().to_vec()).unwrap_or_default();
    let mine: Vec<&crate::fork::rom_library::Mapping> = mappings.iter().filter(|m| m.port_key == port.key()).collect();
    let game_files = model::GameFilesFacts {
        state: needs.then(|| crate::fork::setup_wizard::files_state::files_state(port, dir, &i.config_dir)),
        connected: mine.iter().map(|m| file_name(&m.source)).collect(),
    };

    // Steam.
    let steam = steam_facts(&i);

    // Files.
    let (size, size_capped) = if dir.is_dir() { crate::fork::save_restore::tree_size_capped(dir, u64::MAX / 2, SIZE_ENTRIES_CAP) } else { (0, false) };
    let mut previous: Vec<(String, u64)> = std::fs::read_dir(dir.join(crate::fork::versioned::PREVIOUS))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| (e.file_name().to_string_lossy().into_owned(), crate::fork::save_restore::tree_size_capped(&e.path(), u64::MAX / 2, 50_000).0))
        .collect();
    previous.sort();
    // What an uninstall keeps (Saves Backup/Kept files): the same plan the uninstall runs.
    let (keep, keep_error) = match crate::fork::uninstall_keep::plan(port, dir, Some(&i.config_dir)) {
        Ok(items) => (crate::fork::uninstall_keep::KeepSummary::of(&items, dir), None),
        Err(e) => (Default::default(), Some(e)),
    };
    let kept_dir = crate::fork::uninstall_keep::kept_dir_for(&i.saves_backup_dir, &port.folder);
    let files = model::FilesFacts { dir: dir.clone(), size: Some(size), size_capped, previous, keep, kept_dir, keep_error };

    let disk = DiskFacts { checkup: None, program, saves, settings, game_files, steam, files };
    (disk, i.look.gather())
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Files in a folder and below, counted up to `cap`.
fn count_files(dir: &Path, cap: usize) -> (usize, bool) {
    let mut n = 0usize;
    let mut stack = vec![dir.to_path_buf()];
    let mut seen = 0usize;
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            seen += 1;
            if seen > cap * 4 || n >= cap {
                return (n, true);
            }
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(t) if t.is_file() => n += 1,
                _ => {}
            }
        }
    }
    (n, false)
}

/// The backups that hold this port's saves (its own zips, and the all-ports zips that have its folder) and when the newest
/// was made. Only reads: no stale staging is swept here.
fn port_backups(saves_backup_dir: &Path, folder: &str) -> (usize, Option<std::time::SystemTime>) {
    use crate::fork::save_restore::{GLOBAL_BACKUPS_DIR, RESTORE_BACKUPS_DIR, UPDATE_BACKUPS_DIR};
    let zips = |d: &Path| -> Vec<(PathBuf, std::time::SystemTime)> {
        std::fs::read_dir(d)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")))
            .filter_map(|p| std::fs::metadata(&p).and_then(|m| m.modified()).ok().map(|t| (p, t)))
            .collect()
    };
    let mut times: Vec<std::time::SystemTime> = Vec::new();
    for d in [UPDATE_BACKUPS_DIR, RESTORE_BACKUPS_DIR] {
        if let Ok(port_dir) = safe_join(&saves_backup_dir.join(d), folder) {
            times.extend(zips(&port_dir).into_iter().map(|(_, t)| t));
        }
    }
    let prefix = format!("{folder}/");
    for (zip, t) in zips(&saves_backup_dir.join(GLOBAL_BACKUPS_DIR)) {
        if crate::fork::save_restore::zip_index::read_index(&zip).is_ok_and(|entries| entries.iter().any(|e| e.name.starts_with(&prefix))) {
            times.push(t);
        }
    }
    (times.len(), times.into_iter().max())
}

fn steam_facts(i: &DiskInput) -> SteamFacts {
    use crate::fork::steam_sync::Action;
    let Some(home) = i.home.as_deref() else { return SteamFacts { state: Err("Could not find your home folder.".into()), ..SteamFacts::default() } };
    let settings = ForkSettings::load(&i.settings_path);
    let favorite = i.favorite.clone();
    let screen = crate::fork::screens::SteamScreen::load(home, &i.library, std::slice::from_ref(&i.port), &settings, &|_| favorite.clone());
    let state = match (&screen.load_error, screen.rows.first()) {
        (Some(e), _) => Err(e.clone()),
        (None, None) => Err("Choose your Steam account in Add to Steam first.".to_string()),
        (None, Some(row)) => Ok(match (&row.wanted, row.plan.as_ref().map(|p| &p.action)) {
            (Err(why), _) => SteamState::CannotAdd(why.clone()),
            (_, Some(Action::Add)) => SteamState::NotInSteam,
            (_, Some(Action::Update { changes, by_name: true, .. })) => {
                SteamState::OtherProgram(changes.iter().find(|c| c.field == "program").map_or_else(|| "another program".to_string(), |c| file_name(Path::new(c.from.trim_matches('"')))))
            }
            (_, Some(Action::Update { changes, .. })) => SteamState::NeedsUpdate(changes.iter().map(|c| c.field).collect::<Vec<_>>().join(", ")),
            (_, Some(Action::UpToDate { .. })) => SteamState::InSteam,
            (_, Some(Action::Blocked(why))) => SteamState::CannotAdd(why.clone()),
            (_, None) => SteamState::CannotAdd("it cannot be added".to_string()),
        }),
    };
    let findings = crate::fork::steam_icons::diagnose(home, &i.library, &i.config_dir, std::slice::from_ref(&i.port)).unwrap_or_default();
    let icon_problem = findings.iter().find(|f| f.state.needs_fix()).map(|f| crate::fork::checkup::icon_words(&f.state).to_lowercase());
    let at_old_copy = findings.iter().any(|f| f.at_old_copy);
    let queued = crate::fork::steam_queue::peek(&crate::fork::steam_queue::queue_path(&i.config_dir)).is_ok_and(|q| q.entries.iter().any(|e| e.port_key == i.port.key()));
    SteamFacts { state, icon_problem, at_old_copy, queued }
}

// -- Pages ----------------------------------------------------------------------------------------------------

fn page_for(app: &AppState, s: &Session, screen: Screen) -> Option<Page<Act>> {
    let port = port_of(app, &s.key)?;
    let p = port_facts(app, s, &port);
    let disk = s.disk.borrow().clone();
    let d = disk.as_deref();
    let places = &s.places;
    Some(match screen {
        Screen::First => model::first_page(&p, d, places),
        Screen::Card(card) => match (card, d) {
            (Card::Updates, _) => model::updates_page(&p),
            (Card::More, _) => model::more_page(&p),
            (Card::Uninstall, d) => model::uninstall_page(&p, d.map(|d| &d.files), d.map(|d| &d.saves), places),
            (Card::Program, Some(d)) => model::program_page(&p, &d.program, places),
            (Card::Saves, Some(d)) => model::saves_page(&p, &d.saves, places),
            (Card::Settings, Some(d)) => model::settings_page(&p, &d.settings, places, std::time::SystemTime::now()),
            (Card::Steam, Some(d)) => model::steam_page(&p, &d.steam),
            (Card::Files, Some(d)) => model::files_page(&p, &d.files, places),
            (card, _) => wait_page(&p, card),
        },
        Screen::Wait(card) => wait_page(&p, card),
        Screen::View(i) => {
            let f: SettingsFile = d?.settings.get(i)?.clone();
            // A settings file is at most 1 MB and only its first 48 KB are read: small enough for here.
            let text = settings_files::view_text(&f.path);
            model::view_page(&p, i, &f, &text, places)
        }
        Screen::Notice(_) | Screen::Working(_) | Screen::Done(_) => return None,
    })
}

fn wait_page(p: &PortFacts, card: Card) -> Page<Act> {
    model::notice_page(p, card, "Looking...", vec![format!("Reading {}'s files. This takes a moment.", p.name)]).with_progress(None)
}

fn handlers(s: &Rc<Session>) -> Handlers<Act> {
    let (s1, s2, s3) = (s.clone(), s.clone(), s.clone());
    Handlers {
        on_action: Rc::new(move |app, router, act| on_act(app, router, &s1, act)),
        on_back: Rc::new(move |app, router| on_act(app, router, &s2, Act::Back)),
        on_exit: Rc::new(move |app, router| leave(app, router, &s3)),
        on_input: None,
    }
}

fn show(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, screen: Screen) {
    ROUTER.with(|r| *r.borrow_mut() = Some(router.clone()));
    let Some(page) = page_for(app, s, screen) else { return leave(app, router, s) };
    show_built(app, router, s, screen, page);
}

fn show_built(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, screen: Screen, page: Page<Act>) {
    ROUTER.with(|r| *r.borrow_mut() = Some(router.clone()));
    SESSION.with(|x| *x.borrow_mut() = Some(s.clone()));
    s.screen.set(screen);
    *s.handle.borrow_mut() = show_page_handle(app, router, page, handlers(s));
}

/// The page on screen again with fresh facts, the cursor where it was.
fn redraw(app: &Rc<AppState>, s: &Rc<Session>, screen: Screen) {
    let Some(page) = page_for(app, s, screen) else { return };
    if let Some(h) = s.handle.borrow().as_ref() {
        h.update(page);
    }
}

fn notice(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, card: Card, title: &str, text: Vec<String>) {
    let Some(port) = port_of(app, &s.key) else { return };
    let page = model::notice_page(&port_facts(app, s, &port), card, title, text);
    show_built(app, router, s, Screen::Notice(card), page);
}

/// Back to a card's page after a hand-off: the facts are read again (the hand-off may have changed them).
fn back_to(s: &Rc<Session>, screen: Screen) -> Back {
    let s = s.clone();
    Rc::new(move |app, router| {
        if !app.catalog.borrow().iter().any(|p| p.key() == s.key && app.is_installed(p)) {
            return close_current_dialog(app, router);
        }
        read_disk(app, &s);
        show(app, router, &s, screen);
    })
}

fn leave(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    if is_current(s) {
        SESSION.with(|x| *x.borrow_mut() = None);
    }
    DISK_POLL.with(slint::Timer::stop);
    close_current_dialog(app, router);
}

fn open_card_in(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, card: Card) {
    let Some(port) = port_of(app, &s.key) else { return };
    match card.route() {
        model::Route::Start => {
            close_current_dialog(app, router);
            activate_port(app, router, &port);
        }
        model::Route::Checkup => super::checkup_screen::open_for_port(app, router, &s.key, back_to(s, Screen::First)),
        model::Route::GameFiles => {
            let back = if s.direct { s.back.clone() } else { back_to(s, Screen::First) };
            super::rom_screens::find_for_port(app, router, &s.key, back);
        }
        model::Route::Page if card.needs_disk() && s.disk.borrow().is_none() => show(app, router, s, Screen::Wait(card)),
        model::Route::Page => show(app, router, s, Screen::Card(card)),
    }
}

fn on_act(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, act: Act) {
    let Some(port) = port_of(app, &s.key) else { return leave(app, router, s) };
    let disk = s.disk.borrow().clone();
    match act {
        Act::Back => match s.screen.get() {
            Screen::First => {
                SESSION.with(|x| *x.borrow_mut() = None);
                (s.back)(app, router);
            }
            Screen::Card(_) | Screen::Wait(_) if s.direct => (s.back)(app, router),
            Screen::Card(_) | Screen::Wait(_) => show(app, router, s, Screen::First),
            Screen::View(_) => show(app, router, s, Screen::Card(Card::Settings)),
            Screen::Notice(card) => show(app, router, s, Screen::Card(card)),
            // A backup or an uninstall runs: B waits for its answer.
            Screen::Working(_) => {}
            Screen::Done(_) => leave(app, router, s),
        },
        Act::Open(card) => open_card_in(app, router, s, card),
        Act::AskEveryTime | Act::UseProgram(_) => {
            let Some(d) = disk else { return };
            let pick = match act {
                Act::UseProgram(i) => d.program.candidates.get(i).and_then(|c| crate::fork::program_detect::relative_pick(&d.program.game_dir, c)),
                _ => None,
            };
            if matches!(act, Act::UseProgram(_)) && pick.is_none() {
                return;
            }
            app.state.borrow_mut().set_favorite_exe(&s.key, pick.clone());
            let mut next = (*d).clone();
            next.program.favorite_found = pick.is_some();
            next.program.favorite = pick;
            *s.disk.borrow_mut() = Some(Rc::new(next));
            redraw(app, s, Screen::Card(Card::Program));
        }
        Act::BackUpNow => back_up(app, router, s, &port),
        Act::Restore => super::save_restore_screen::show_port(app, router, &port, back_to(s, Screen::Card(Card::Saves))),
        Act::FindSaves => super::save_finder_screen::open(app, router, port, back_to(s, Screen::Card(Card::Saves))),
        Act::OpenSaveFolder => {
            if let Some(f) = disk.as_ref().and_then(|d| d.saves.folders.iter().find(|f| f.exists).map(|f| f.path.clone())) {
                crate::core::launch::open_folder(&f);
            }
        }
        Act::View(i) => show(app, router, s, Screen::View(i)),
        Act::OpenFileFolder(i) | Act::OpenInEditor(i) => {
            let Some(f) = disk.as_ref().and_then(|d| d.settings.get(i).map(|f| f.path.clone())) else { return };
            if matches!(act, Act::OpenInEditor(_)) {
                crate::core::launch::open_file(&f);
            } else if let Some(parent) = f.parent() {
                crate::core::launch::open_folder(parent);
            }
        }
        Act::SteamAdd => {
            close_current_dialog(app, router);
            super::steam_screens::open_steam_screen_for(app, router, Some(&s.key));
        }
        Act::SteamRepoint => match super::checkup_screen::repoint_ports(app, std::slice::from_ref(&s.key)) {
            None => notice(app, router, s, Card::Steam, "Nothing was changed", vec!["Where Steam and the old copies are could not be read.".into()]),
            Some((queued, notes)) if queued > 0 => {
                eprintln!("port page: repoint queued {queued} change(s) for Steam to close");
                let back = back_to(s, Screen::Card(Card::Steam));
                let then: super::steam_close_screen::Then = Rc::new(move |app, router, _| back(app, router));
                let lead = (!notes.is_empty()).then(|| notes.join(" "));
                super::steam_close_screen::open(app, router, super::steam_close_screen::Opts { ask_first: true, lead, ..Default::default() }, then);
            }
            Some((_, notes)) => {
                read_disk(app, s);
                let mut text = vec!["Steam now starts the new copy. Its name, art, play time and launch options stayed.".to_string()];
                text.extend(notes);
                notice(app, router, s, Card::Steam, "Done", text);
            }
        },
        Act::SteamFixIcon => super::steam_icons_screen::open_for(app, router, Some(s.key.clone()), back_to(s, Screen::Card(Card::Steam))),
        Act::SteamFinish => {
            let back = back_to(s, Screen::Card(Card::Steam));
            let then: super::steam_close_screen::Then = Rc::new(move |app, router, _| back(app, router));
            super::steam_close_screen::open(app, router, super::steam_close_screen::Opts { ask_first: true, ..Default::default() }, then);
        }
        Act::OpenInstallFolder => {
            if let Some(dir) = game_dir(app, &port) {
                crate::core::launch::open_folder(&dir);
            }
        }
        Act::CheckNow => check_now(app, s, &port),
        Act::UpdateNow => {
            close_current_dialog(app, router);
            start_install(app, router, port, None, None);
        }
        Act::ToggleUpdates => {
            let on = app.state.borrow().get(&s.key).is_none_or(|i| i.update);
            app.state.borrow_mut().set_port_update(&s.key, !on);
            app.refresh_current_view();
            redraw(app, s, Screen::Card(Card::Updates));
        }
        Act::SelectVersion => {
            close_current_dialog(app, router);
            open_version_picker(app, router, port);
        }
        Act::InstallExtras => open_extra_install_confirm_dialog(app, router, port),
        Act::UninstallYes => uninstall(app, router, s, &port),
        Act::ResetPlaytime => open_reset_playtime_dialog(app, router, port),
        Act::AllDetails => open_info_dialog(app, router, &port),
        Act::ProjectPage => {
            if let Some(url) = model::project_url(&port) {
                open_link(app, router, port.display_name(), &url, Some(back_to(s, Screen::Card(Card::More))));
            }
        }
    }
}

// -- The actions that run work --------------------------------------------------------------------------------

/// "Back up this port now": the save finder's backup (`backup_before_update`, into Saves Backup / Update Backups), on a
/// worker; B waits for its answer.
fn back_up(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, port: &Port) {
    if super::save_restore_screen::refuse_while_restoring(app, router) {
        return;
    }
    let Some(dir) = game_dir(app, port) else { return };
    let (backups, port2) = (app.paths.saves_backup_dir.clone(), port.clone());
    let stamp = crate::fork::store::backup_stamp();
    let job = Rc::new(Background::spawn(move |ctl| crate::core::save_backup::backup_before_update(&port2, &backups, &dir, &stamp, &mut |t| ctl.set_progress(t.to_string()))));
    let mut page = model::notice_page(&port_facts(app, s, port), Card::Saves, "Backing up...", vec!["Copying the saves into a zip. Nothing is changed.".into()]).with_progress(None);
    page.no_back = true;
    page.buttons = [None, None, None, None];
    show_built(app, router, s, Screen::Working(Card::Saves), page);
    let (app, router, s, folder) = (app.clone(), router.clone(), s.clone(), port.folder.clone());
    run_work(move || {
        let Some(result) = job.take() else { return false };
        super::cloud_screen::trigger(&app, crate::fork::cloud::hooks::Why::FinderBackup);
        let (title, text) = match &result {
            Ok(Ok(Some(_))) => ("Backup made", format!("Backed up to Saves Backup > Update Backups > {folder}. Restore lists it as \"Before update\".")),
            Ok(Ok(None)) => ("Nothing to back up", "The save folder holds no saves yet, so there was nothing to copy.".to_string()),
            Ok(Err(detail)) | Err(detail) => {
                eprintln!("port page backup: {detail}");
                ("The backup did not work", format!("{detail}\n\nNothing was changed. Your saves are as they were."))
            }
        };
        if is_current(&s) {
            read_disk(&app, &s);
            let paragraphs = text.split("\n\n").map(str::to_string).collect();
            notice(&app, &router, &s, Card::Saves, title, paragraphs);
        }
        true
    });
}

/// "Check for an update now": the same question Play asks (the newest release against the installed one), on a thread;
/// the answer also goes to Home's badge.
fn check_now(app: &Rc<AppState>, s: &Rc<Session>, port: &Port) {
    let info = app.state.borrow().get(port.key()).cloned().unwrap_or_default();
    let (gh, gl) = app.api_tokens();
    let port2 = port.clone();
    let job = Rc::new(Background::spawn(move |_| latest_release(&port2, info.installed_tag.as_deref(), &info.installed_at, gh.as_deref(), gl.as_deref())));
    *s.update_check.borrow_mut() = UpdateCheck::Checking;
    redraw(app, s, Screen::Card(Card::Updates));
    let (app, s) = (app.clone(), s.clone());
    run_work(move || {
        let Some(result) = job.take() else { return false };
        let answer = match result.and_then(|r| r) {
            Ok((true, tag)) => {
                super::home_screen::note_update_check(&app, &s.key, true);
                UpdateCheck::Ready(tag)
            }
            Ok((false, tag)) => {
                super::home_screen::note_update_check(&app, &s.key, false);
                UpdateCheck::UpToDate(tag)
            }
            Err(e) => UpdateCheck::Failed(e),
        };
        eprintln!("port page: update check for {}: {answer:?}", s.key);
        *s.update_check.borrow_mut() = answer;
        app.refresh_current_view();
        if is_current(&s) && s.screen.get() == Screen::Card(Card::Updates) {
            redraw(&app, &s, Screen::Card(Card::Updates));
        }
        true
    });
}

/// Whether a newer release than the installed one is out, and its name: Play's own rule (`jobs::update_decision`, the
/// release asset the installer would pick).
fn latest_release(port: &Port, installed_tag: Option<&str>, installed_at: &str, github: Option<&str>, gitlab: Option<&str>) -> Result<(bool, String), String> {
    let repo = port.repo.as_deref().unwrap_or_default();
    let preferred = port.preferred_asset.as_ref().and_then(crate::core::platform_resolve::resolve_preferred_asset);
    let (tag, date) = match port.source_type {
        SourceType::Github => crate::core::github_api::fetch_latest_tag_and_date_preferring(repo, github, preferred.as_deref()),
        SourceType::Gitlab => crate::core::gitlab_api::fetch_latest_tag_and_date_preferring(repo, gitlab, preferred.as_deref()),
        SourceType::DirectUrl | SourceType::None => return Err("this port does not follow releases".into()),
    }
    .map_err(|e| e.message().to_string())?;
    Ok((crate::core::jobs::update_decision(installed_tag, installed_at, &tag, date.as_deref()), tag))
}

/// "Uninstall" confirmed: upstream's guards (`may_uninstall`: not while it runs or installs, never a port you added
/// yourself), then on a worker the saves are copied to Saves Backup, game files, keys and older versions are moved to
/// Kept files (`fork::uninstall_keep`; across drives that is a copy) and the rest of the folder is removed. The port is
/// held as busy meanwhile, so nothing installs, starts or moves it. B waits; the result page names Kept files.
fn uninstall(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, port: &Port) {
    if !may_uninstall(app, port) {
        return notice(app, router, s, Card::Uninstall, "Nothing was removed", vec![format!("{} is running, installing or restoring right now. Try again when it has finished.", port.display_name())]);
    }
    let (library, backups, config, port2) = (app.paths.library_dir(), app.paths.saves_backup_dir.clone(), app.paths.config_dir.clone(), port.clone());
    app.install_runtime.installing.borrow_mut().insert(port.key().to_string());
    let job = Rc::new(Background::spawn(move |_| crate::core::installer::uninstall_port_keeping(&port2, &library, &backups, Some(&config))));
    let mut page = model::notice_page(
        &port_facts(app, s, port),
        Card::Uninstall,
        "Keeping game files...",
        vec!["Copying the saves to Saves Backup and moving the game files to Kept files, then removing the rest of the folder. Big games on another drive take a while.".into()],
    )
    .with_progress(None);
    page.no_back = true;
    page.buttons = [None, None, None, None];
    show_built(app, router, s, Screen::Working(Card::Uninstall), page);
    let (app, router, s, port) = (app.clone(), router.clone(), s.clone(), port.clone());
    run_work(move || {
        let Some(result) = job.take() else { return false };
        app.install_runtime.installing.borrow_mut().remove(port.key());
        let (title, text, gone) = match result.and_then(|r| r) {
            Ok(kept) => {
                mark_uninstalled(&app, &port);
                eprintln!("port page: uninstalled {}", port.display_name());
                let mut text = vec![kept.map_or_else(|| "Its folder was removed. It held no game files to keep.".to_string(), |k| k.line())];
                text.push("Steam shortcuts are left as they are.".to_string());
                (format!("{} was uninstalled", port.display_name()), text, true)
            }
            Err(message) => {
                eprintln!("port page: uninstall of {} stopped: {message}", port.display_name());
                ("The uninstall did not finish".to_string(), vec![message], false)
            }
        };
        if is_current(&s) {
            let facts = port_facts(&app, &s, &port);
            let page = if gone { model::done_page(&facts, Card::Uninstall, &title, text) } else { model::notice_page(&facts, Card::Uninstall, &title, text) };
            show_built(&app, &router, &s, if gone { Screen::Done(Card::Uninstall) } else { Screen::Notice(Card::Uninstall) }, page);
        } else if !dialog_is_open(&app) {
            super::dialogs::open_message_dialog(&app, &router, &title, &text.join("\n\n"));
        }
        true
    });
}

// -- The project page -----------------------------------------------------------------------------------------

/// The project page of a port: its address as text first and a QR code (Game Mode has no browser); in Desktop Mode one
/// card opens it in the browser. `back`: where B goes (`None`: close). Only http(s) addresses are shown.
pub(crate) fn open_link(app: &Rc<AppState>, router: &Router, name: &str, url: &str, back: Option<Back>) {
    if !crate::core::launch::is_web_url(url) {
        return;
    }
    let page = model::link_page(name, url, crate::fork::steam_queue::game_mode_from_env());
    let url = url.to_string();
    let leave: Back = back.unwrap_or_else(|| Rc::new(|app, router| close_current_dialog(app, router)));
    let (l1, l2, l3) = (leave.clone(), leave.clone(), leave);
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| match act {
            LinkAct::OpenInBrowser => {
                eprintln!("port page: opening the project page {url}");
                crate::core::launch::open_url(&url);
            }
            LinkAct::Back => l1(app, router),
        }),
        on_back: Rc::new(move |app, router| l2(app, router)),
        on_exit: Rc::new(move |app, router| l3(app, router)),
        on_input: None,
    };
    show_page(app, router, page, handlers);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Check now and Back up now used to share one timer: the first to finish stopped the other's polling, and its answer
    /// never arrived. Every job is now asked until it is done, and a job started from another's answer is kept.
    #[test]
    fn one_job_finishing_never_stops_another_and_new_jobs_join() {
        let cell = Rc::new(RefCell::new(Pollers::default()));
        let log = Rc::new(RefCell::new(Vec::<&str>::new()));
        let ticks = Rc::new(Cell::new(0));
        let (l1, l2, l3, t2, c) = (log.clone(), log.clone(), log.clone(), ticks.clone(), cell.clone());
        cell.borrow_mut().0.push(Box::new(move || {
            l1.borrow_mut().push("check");
            // Its answer starts another job (a re-read): it must not be lost.
            let l3 = l3.clone();
            c.borrow_mut().0.push(Box::new(move || {
                l3.borrow_mut().push("reread");
                true
            }));
            true
        }));
        cell.borrow_mut().0.push(Box::new(move || {
            t2.set(t2.get() + 1);
            if t2.get() < 3 {
                return false;
            }
            l2.borrow_mut().push("backup");
            true
        }));
        assert!(tick_pollers(&cell), "the backup still runs after the check is done");
        assert_eq!(*log.borrow(), ["check"]);
        assert!(tick_pollers(&cell));
        assert_eq!(*log.borrow(), ["check", "reread"]);
        assert!(!tick_pollers(&cell), "nothing left once the backup answers");
        assert_eq!(*log.borrow(), ["check", "reread", "backup"]);
    }
}
