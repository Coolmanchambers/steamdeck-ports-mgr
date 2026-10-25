//! "Fix problems" on screen: the pages of `crate::fork::checkup::pages` in the guided-page window, the facts gathered
//! for them, the hand-offs to the existing fix screens and the way back, and the cached answer Home's tile and
//! Settings' card read.
//!
//! **Looking changes nothing.** Every read of the disk (Steam's files, the waiting list, old copies, the game files and
//! saves of each port, the program walk, the backup folder) runs on a worker thread (`Background`); only what is in
//! memory (the installed list, the update badges, the cloud status) is read here, and the worker's answer is joined with
//! a fresh read of that when it arrives. Every fix is an existing screen that asks on its own page (or, for an update and
//! a backup, a page of ours that asks first); the checkup itself never writes, except that "Skip this one" is remembered
//! in `fork-settings.json`.
//!
//! **Hand-off and return.** A fix closes the checkup page and opens the existing screen. Those screens do not know
//! about the checkup, so (like the setup wizard) it waits: a half-second watcher notices that no dialog is open for a
//! moment and comes back, and the menus the chosen fix leads back to (`Fix::returns_to`, asked by
//! `resume_instead_of_menu`) come back to the checkup instead. Coming back looks again and shows that line, green or not
//! (`after_page`).
//!
//! **The cache.** Home's tile and Settings' card show the last answer (`CACHE`), refreshed in the background at most
//! every few minutes (never while a move, install, restore or import runs) and after every checkup; opening Home or
//! Settings never waits for it.

use super::dialogs::{close_current_dialog, dialog_is_open};
use super::page_dialog::{show_page, Handlers};
use super::state::AppState;
use crate::core::models::Port;
use crate::fork::checkup::pages::{self, Act, CheckupPage};
use crate::fork::checkup::{self, Check, CopyFact, CopyState, Facts, Fix, Folder, Named, Outcome, QueuedFact, Results, Skips};
use crate::fork::folder_picker::mounts::{default_media_roots, system_mounts};
use crate::fork::port_mover::old_copies;
use crate::fork::rom_screens::find::FindRequest;
use crate::fork::rom_screens::job::Background;
use crate::fork::save_finder::{self as finder, SaveState};
use crate::fork::settings::ForkSettings;
use crate::fork::setup_wizard::files_state::FilesState;
use crate::fork::{ports_folder, steam_icons, steam_queue};
use crate::ui::gamepad_router::GamepadRouter;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

type Router = Rc<RefCell<GamepadRouter>>;

/// How often the watcher looks for "no dialog is open" while a fix screen is up.
const TICK: Duration = Duration::from_millis(250);
/// How many quiet ticks in a row bring the checkup back (a page swap inside one callback never shows a gap; a
/// dialog closed and the next opened from a zero-delay timer does for a moment).
const QUIET_TICKS: u32 = 2;
/// The cached answer is refreshed when older than this.
const CACHE_FRESH: Duration = Duration::from_secs(180);
/// The first look after start waits a little, so the first screen is up before any of this reads the disk.
const START_DELAY: Duration = Duration::from_secs(4);
/// A cloud backup that copied within this long counts as the saves being safe.
const CLOUD_RECENT_SECS: u64 = 3 * 86_400;
/// `fork-settings.json` key of the problems the user skipped (check key to the problem's fingerprint).
const KEY_SKIPS: &str = "checkup_skips";

thread_local! {
    static SESSION: RefCell<Option<Rc<Session>>> = const { RefCell::new(None) };
    static POLL: slint::Timer = slint::Timer::default();
    static WATCH: slint::Timer = slint::Timer::default();
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
    static BACKGROUND_POLL: slint::Timer = slint::Timer::default();
    static PERIODIC: slint::Timer = slint::Timer::default();
}

#[derive(Default)]
struct Cache {
    results: Option<Results>,
    at: Option<Instant>,
    job: Option<Rc<Background<SysFacts>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Looking,
    Overview,
    Step,
    Confirm,
    After,
    Finished,
    /// An existing screen is open; the checkup waits.
    Fixing,
}

struct Session {
    stage: Cell<Stage>,
    results: RefCell<Results>,
    /// The checks with a problem when "Fix them one by one" was chosen, where the run is, and what the results were then
    /// (choosing it again with nothing changed picks up where it left off).
    order: RefCell<Vec<Check>>,
    pos: Cell<usize>,
    order_results: RefCell<Option<Results>>,
    /// The fix that is open now, and how many ports its problem was about (to tell "one of several done" from "nothing").
    fix: RefCell<Option<Fix>>,
    fix_ports: Cell<usize>,
    /// What went wrong while a fix ran (a shortcut Steam has not finished setting up), shown on the page after it.
    notes: RefCell<Vec<String>>,
    /// The fix is being opened right now: a menu it opens must show, not hand control back.
    entering: Cell<bool>,
    quiet: Cell<u32>,
    job: RefCell<Option<Rc<Background<SysFacts>>>>,
    /// Bumped whenever a look is started or cancelled; a result that arrives for an older one is dropped.
    looking: Cell<u64>,
    /// What to do when the look in progress ends.
    then: RefCell<Option<Rc<dyn Fn(&Rc<AppState>, &Router)>>>,
    /// Fork (port page): the checkup for this one port only (`checkup::run_for_port`), and where leaving it goes.
    scope: Option<String>,
    done: Option<Rc<dyn Fn(&Rc<AppState>, &Router)>>,
}

fn session() -> Option<Rc<Session>> {
    SESSION.with(|s| s.borrow().clone())
}

// -- Facts ----------------------------------------------------------------------------------------------------

/// What the worker thread reads from: only paths and plain values, so it never touches the UI's state.
struct SysInput {
    installed: Vec<Port>,
    favorites: HashMap<String, String>,
    /// The installed ports that were ever played (a game never run has no saves to miss yet).
    played: HashSet<String>,
    /// The folder names of every catalog port (a switch cut short counts only when its new copy is one of them).
    catalog_folders: Vec<String>,
    home: Option<PathBuf>,
    library: PathBuf,
    config_dir: PathBuf,
    settings_path: PathBuf,
    saves_backup_dir: PathBuf,
    default_library: PathBuf,
}

struct SysFacts {
    folder: Folder,
    steam_found: bool,
    queue: Result<Vec<QueuedFact>, String>,
    shortcuts: Result<Vec<steam_icons::IconFinding>, String>,
    programs: Vec<Named>,
    files: Vec<(Named, FilesState)>,
    saves: Vec<Named>,
    saves_known: usize,
    unfinished_moves: Vec<String>,
    old_copies: Result<Vec<CopyFact>, String>,
    backup_days: Option<u64>,
    skips: Skips,
}

/// What is in memory: read here, quickly, when the worker's answer arrives.
struct AppPart {
    installed: Vec<Named>,
    updates: Vec<Named>,
    cloud_problem: bool,
    cloud_ok: bool,
    game_mode: bool,
}

fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"].iter().find_map(|v| std::env::var_os(v)).map(PathBuf::from).filter(|p| p.is_absolute())
}

fn installed_ports(app: &AppState) -> Vec<Port> {
    app.catalog.borrow().iter().filter(|p| !p.is_android_only() && app.is_installed(p)).cloned().collect()
}

fn named(p: &Port) -> Named {
    Named::new(p.key(), p.display_name())
}

fn settings_path(app: &AppState) -> PathBuf {
    app.paths.config_dir.join(crate::fork::settings::FILE_NAME)
}

/// Nothing that touches the disk beyond two small status files.
fn app_part(app: &AppState) -> AppPart {
    let installed = installed_ports(app);
    let updates = installed.iter().filter(|p| super::home_screen::update_ready(app, p.key())).map(named).collect();
    AppPart {
        installed: installed.iter().map(named).collect(),
        updates,
        cloud_problem: super::cloud_screen::home_status(app).0 == 5,
        cloud_ok: super::cloud_screen::recent_success(app, CLOUD_RECENT_SECS),
        game_mode: steam_queue::game_mode_from_env(),
    }
}

fn sys_input(app: &AppState) -> SysInput {
    sys_input_for(app, None)
}

/// The facts to read, for every installed port or (`scope`) for one.
fn sys_input_for(app: &AppState, scope: Option<&str>) -> SysInput {
    let installed: Vec<Port> = installed_ports(app).into_iter().filter(|p| scope.is_none_or(|k| p.key() == k)).collect();
    let (favorites, played) = {
        let state = app.state.borrow();
        let favorites = installed.iter().filter_map(|p| state.get(p.key()).and_then(|i| i.favorite_exe.clone()).map(|f| (p.key().to_string(), f))).collect();
        let played = installed.iter().filter(|p| state.get(p.key()).is_some_and(|i| i.playtime_seconds > 0 || !i.last_played_at.is_empty())).map(|p| p.key().to_string()).collect();
        (favorites, played)
    };
    SysInput {
        installed,
        favorites,
        played,
        catalog_folders: app.catalog.borrow().iter().map(|p| p.folder.clone()).collect(),
        home: home_dir(),
        library: app.paths.library_dir(),
        config_dir: app.paths.config_dir.clone(),
        settings_path: settings_path(app),
        saves_backup_dir: app.paths.saves_backup_dir.clone(),
        default_library: ports_folder::default_dir(&app.paths.config_dir),
    }
}

/// Reads the disk. Nothing here writes, and none of it waits on the network.
fn sys_gather(i: SysInput) -> SysFacts {
    let folder = if i.library == i.default_library {
        // The default folder next to the app is simply made again, as upstream always did.
        Folder::Here
    } else {
        Folder::from_missing(ports_folder::availability(&i.library, &system_mounts(), &default_media_roots()).as_ref())
    };
    // Read only: `steam_queue::load` moves a file it cannot understand aside, and looking must not.
    let queue = steam_queue::peek(&steam_queue::queue_path(&i.config_dir)).map(|q| q.entries.iter().map(|e| QueuedFact { port_key: e.port_key.clone(), name: e.name.clone(), kind: e.kind }).collect()).map_err(|why| {
        eprintln!("checkup: the waiting list of Steam changes cannot be read: {why}");
        checkup::QUEUE_UNREADABLE.to_string()
    });
    let steam_found = i.home.as_deref().is_some_and(|h| !crate::fork::port_mover::steam::device_accounts(h).is_empty());
    let shortcuts = match i.home.as_deref() {
        Some(h) => steam_icons::diagnose(h, &i.library, &i.config_dir, &i.installed),
        None => Err("Could not find your home folder".to_string()),
    };
    let here = folder == Folder::Here;
    let programs = if here { programs_to_choose(&i) } else { Vec::new() };
    let files = if here { game_files_missing(&i) } else { Vec::new() };
    let (saves, saves_known) = if here { saves_not_found(&i) } else { (Vec::new(), 0) };
    let old_copies = old_copy_facts(&i);
    let unfinished_moves = unfinished_switches(&i);
    let backup_days = {
        let dir = i.saves_backup_dir.join(crate::fork::save_restore::GLOBAL_BACKUPS_DIR);
        let now = chrono::Utc::now().timestamp();
        crate::fork::settings_menu::newest_zip(&dir).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| crate::fork::settings_menu::days_ago(d.as_secs() as i64, now))
    };
    let skips = checkup::skips_from_json(ForkSettings::load(&i.settings_path).get(KEY_SKIPS));
    SysFacts { folder, steam_found, queue, shortcuts, programs, files, saves, saves_known, unfinished_moves, old_copies, backup_days, skips }
}

/// The ports with more than one program to start and none remembered (the same question Play asks at the first start).
fn programs_to_choose(i: &SysInput) -> Vec<Named> {
    let mut out = Vec::new();
    for p in &i.installed {
        let Ok(dir) = crate::core::path_safety::safe_join(&i.library, &p.folder) else { continue };
        let favorite_works = i.favorites.get(p.key()).and_then(|f| crate::core::path_safety::safe_join(&dir, f).ok()).is_some_and(|f| f.exists());
        let named_by_catalog = p.executable.as_ref().is_some_and(crate::core::platform_resolve::is_truthy);
        if favorite_works || named_by_catalog {
            continue;
        }
        if checkup::shallow_programs(&dir).is_ok_and(|found| found.len() > 1) {
            out.push(named(p));
        }
    }
    out
}

/// The ports whose game files are missing (a named file that is not there) or only named by title: the same answer Home's
/// badge gives (`rom_screens::port_files_state`), read here without the app's state.
fn game_files_missing(i: &SysInput) -> Vec<(Named, FilesState)> {
    let locals = crate::fork::rom_library::load_local_entries(&i.config_dir.join("ports.local.json"));
    let mut out = Vec::new();
    for p in &i.installed {
        if !FindRequest::new(p.clone(), true, locals.get(&p.folder), Vec::new()).needs_files() {
            continue;
        }
        let state = match crate::core::path_safety::safe_join(&i.library, &p.folder) {
            Ok(dir) => crate::fork::setup_wizard::files_state::files_state(p, &dir, &i.config_dir),
            Err(_) => FilesState::Missing,
        };
        if matches!(state, FilesState::Missing | FilesState::Unverifiable) {
            out.push((named(p), state));
        }
    }
    out
}

/// The ports whose saves the app cannot find (`SaveState::warns`, the save finder's badge) and how many ports have a save
/// folder with files in it (what "Back up saves now" can copy).
fn saves_not_found(i: &SysInput) -> (Vec<Named>, usize) {
    let overrides = finder::overrides(&ForkSettings::load(&i.settings_path));
    let (mut missing, mut known) = (Vec::new(), 0);
    for p in &i.installed {
        let Ok(dir) = crate::core::path_safety::safe_join(&i.library, &p.folder) else { continue };
        if !dir.is_dir() {
            continue;
        }
        let state = finder::state_of(p, &dir, overrides.get(&p.folder));
        if state == SaveState::Found {
            known += 1;
        }
        if state.warns(i.played.contains(p.key())) {
            missing.push(named(p));
        }
    }
    (missing, known)
}

/// The switches cut short (the app closed between the ports folder setting and Steam) whose new copy is in use: what
/// Settings > Ports folder offers to finish. A read of the records.
fn unfinished_switches(i: &SysInput) -> Vec<String> {
    let active: Vec<PathBuf> = i.catalog_folders.iter().filter_map(|f| crate::core::path_safety::safe_join(&i.library, f).ok()).collect();
    old_copies::load(&i.config_dir).unwrap_or_default().into_iter().filter(|o| !o.switched && active.contains(&o.new_path)).map(|o| o.name).collect()
}

/// Each kept old copy and what it needs. Read only: `readiness_now`, which notes a first sighting in the record, is not
/// used here (`readiness_read_only` counts a Steam start after the switch without writing anything).
fn old_copy_facts(i: &SysInput) -> Result<Vec<CopyFact>, String> {
    let list = old_copies::load(&i.config_dir)?;
    let home = i.home.clone().unwrap_or_default();
    Ok(list
        .iter()
        .map(|c| {
            let state = if std::fs::symlink_metadata(&c.old_path).is_err() { CopyState::Gone } else { CopyState::of(old_copies::readiness_read_only(c, &home, &i.config_dir)) };
            CopyFact { name: c.name.clone(), state }
        })
        .collect())
}

fn assemble(app: &AppPart, sys: SysFacts) -> Facts {
    Facts {
        game_mode: app.game_mode,
        installed: app.installed.clone(),
        folder: sys.folder,
        unfinished_moves: sys.unfinished_moves,
        steam_found: sys.steam_found,
        queue: sys.queue,
        shortcuts: sys.shortcuts,
        programs: sys.programs,
        files: sys.files,
        saves: sys.saves,
        old_copies: sys.old_copies,
        updates: app.updates.clone(),
        backup_days: sys.backup_days,
        cloud_problem: app.cloud_problem,
        cloud_ok: app.cloud_ok,
        saves_known: sys.saves_known,
        skips: sys.skips,
    }
}

/// A move of ports, an install, a restore or an import is running: what the disk shows is half way, so nothing looks now.
fn busy(app: &AppState) -> bool {
    super::ports_folder_screen::job_running() || super::import_screen::import_running() || super::save_restore_screen::restore_running() || !app.install_runtime.installing.borrow().is_empty()
}

// -- One port's line, for the Port page -----------------------------------------------------------------------

/// Fork (port page): what one port's checkup reads, taken on the UI thread; `gather` it on a worker.
pub(crate) struct PortLook(SysInput);

/// What the worker read for one port's checkup.
pub(crate) struct PortSys(SysFacts);

pub(crate) fn port_look(app: &AppState, key: &str) -> PortLook {
    PortLook(sys_input_for(app, Some(key)))
}

impl PortLook {
    /// Reads the disk (a worker thread); writes nothing.
    pub(crate) fn gather(self) -> PortSys {
        PortSys(sys_gather(self.0))
    }
}

/// The port's Fix problems line ("Everything looks fine", "2 things to fix") and how many problems, joined with what is in
/// memory now. `None` while a move, install, restore or import runs (what the disk shows is half way).
pub(crate) fn port_line(app: &AppState, key: &str, sys: PortSys) -> Option<(String, usize)> {
    if busy(app) {
        return None;
    }
    let results = checkup::run_for_port(&assemble(&app_part(app), sys.0), key);
    Some((results.summary(), results.problem_count()))
}

// -- The cached answer (Home's tile, Settings' card) --------------------------------------------------------

/// Called once at start: the first look, after the first screen is up.
pub(crate) fn start(app: &Rc<AppState>) {
    let app = app.clone();
    slint::Timer::single_shot(START_DELAY, move || {
        refresh_in_background(&app);
        // Then once a minute, when the answer has grown old and nothing is open on top of Home.
        PERIODIC.with(|t| {
            let app = app.clone();
            t.start(slint::TimerMode::Repeated, Duration::from_secs(60), move || {
                if session().is_none() && !dialog_is_open(&app) {
                    ensure_fresh(&app);
                }
            });
        });
    });
}

/// The tile Home shows (`None`: nothing worth a tile, or no answer yet).
pub(crate) fn home_tile() -> Option<checkup::HomeTile> {
    CACHE.with(|c| c.borrow().results.as_ref().and_then(Results::home_tile))
}

/// The line Settings shows on its Fix problems card; `None` before the first look finished.
pub(crate) fn summary() -> Option<String> {
    CACHE.with(|c| c.borrow().results.as_ref().map(Results::summary))
}

/// Looks again in the background when the answer is older than a few minutes (or there is none). Never blocks; Home
/// is redrawn when the new answer differs.
pub(crate) fn ensure_fresh(app: &Rc<AppState>) {
    let stale = CACHE.with(|c| c.borrow().at.is_none_or(|at| at.elapsed() >= CACHE_FRESH));
    if stale {
        refresh_in_background(app);
    }
}

fn store(results: Results) -> bool {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let changed = c.results.as_ref() != Some(&results);
        c.results = Some(results);
        c.at = Some(Instant::now());
        changed
    })
}

fn refresh_in_background(app: &Rc<AppState>) {
    if CACHE.with(|c| c.borrow().job.is_some()) || busy(app) {
        return;
    }
    let job = Rc::new(Background::spawn({
        let input = sys_input(app);
        move |_| sys_gather(input)
    }));
    CACHE.with(|c| c.borrow_mut().job = Some(job));
    let app = app.clone();
    BACKGROUND_POLL.with(|t| {
        t.start(slint::TimerMode::Repeated, Duration::from_millis(300), move || {
            let done = CACHE.with(|c| c.borrow().job.as_ref().and_then(|j| j.take()));
            let Some(done) = done else { return };
            BACKGROUND_POLL.with(slint::Timer::stop);
            CACHE.with(|c| c.borrow_mut().job = None);
            match done {
                // What is in memory is read now, not when the worker started: ports may have come or gone meanwhile.
                Ok(sys) if !busy(&app) => {
                    let results = checkup::run(&assemble(&app_part(&app), sys));
                    eprintln!("checkup: background look: {}", results.summary());
                    let changed = store(results);
                    // Redrawn even under a dialog: Home is only drawn behind it, and the tile must be there when it closes.
                    if changed && app.home.is_active() {
                        app.refresh_current_view();
                    }
                }
                Ok(_) => eprintln!("checkup: a job started meanwhile; this look is dropped"),
                Err(e) => {
                    eprintln!("checkup: the background look failed ({e})");
                    CACHE.with(|c| c.borrow_mut().at = Some(Instant::now()));
                }
            }
        });
    });
}

// -- Skips ----------------------------------------------------------------------------------------------------

fn load_skips(app: &AppState) -> Skips {
    checkup::skips_from_json(ForkSettings::load(&settings_path(app)).get(KEY_SKIPS))
}

fn save_skips(app: &AppState, skips: &Skips) {
    let mut settings = ForkSettings::load(&settings_path(app));
    let result = if skips.is_empty() { settings.remove(KEY_SKIPS) } else { settings.set(KEY_SKIPS, checkup::skips_to_json(skips)) };
    if let Err(e) = result {
        eprintln!("checkup: could not remember what was skipped ({e})");
    }
}

/// "Skip this one": remembered until the problem reads differently, and out of the way meanwhile.
fn skip_current(app: &Rc<AppState>, s: &Rc<Session>) {
    let Some(check) = current(s) else { return };
    let Some(problem) = problem_of(s, check) else { return };
    let mut skips = load_skips(app);
    skips.insert(check.key().to_string(), problem.fingerprint());
    save_skips(app, &skips);
    eprintln!("checkup: skipped {}", check.title());
    let results = s.results.borrow().clone().with_skips(&skips);
    if s.scope.is_none() {
        store(results.clone());
    }
    *s.results.borrow_mut() = results;
}

// -- The checkup ----------------------------------------------------------------------------------------------

/// Settings > Fix problems, and Home's tile. Closes whatever page is open, then looks.
pub(crate) fn open(app: &Rc<AppState>, router: &Router) {
    open_scoped(app, router, None, None);
}

/// Fork (port page): the checkup for one port, from the Port page's Fix problems card. Only the checks that can be about
/// a port (`checkup::Check::PORT`), its lines and step pages as the full checkup has them; leaving it runs `done` (the
/// Port page again).
pub(crate) fn open_for_port(app: &Rc<AppState>, router: &Router, key: &str, done: Rc<dyn Fn(&Rc<AppState>, &Router)>) {
    open_scoped(app, router, Some(key.to_string()), Some(done));
}

fn open_scoped(app: &Rc<AppState>, router: &Router, scope: Option<String>, done: Option<Rc<dyn Fn(&Rc<AppState>, &Router)>>) {
    close_current_dialog(app, router);
    let s = Rc::new(Session {
        stage: Cell::new(Stage::Looking),
        results: RefCell::new(Results { items: Vec::new() }),
        order: RefCell::new(Vec::new()),
        pos: Cell::new(0),
        order_results: RefCell::new(None),
        fix: RefCell::new(None),
        fix_ports: Cell::new(0),
        notes: RefCell::new(Vec::new()),
        entering: Cell::new(false),
        quiet: Cell::new(0),
        job: RefCell::new(None),
        looking: Cell::new(0),
        then: RefCell::new(None),
        scope,
        done,
    });
    SESSION.with(|x| *x.borrow_mut() = Some(s.clone()));
    eprintln!("checkup: opened");
    if busy(app) {
        return show(app, router, &s, Stage::Finished, pages::failed_page("A move, an install, a restore or an import is running. Try again when it has finished: looking now would show it half way."));
    }
    look(app, router, &s, Rc::new(show_overview));
}

fn handlers(s: &Rc<Session>) -> Handlers<Act> {
    let (s_act, s_back, s_exit) = (s.clone(), s.clone(), s.clone());
    Handlers {
        on_action: Rc::new(move |app, router, act| act_on(app, router, &s_act, act)),
        on_back: Rc::new(move |app, router| back(app, router, &s_back)),
        on_exit: Rc::new(move |app, router| leave(app, router, &s_exit)),
        on_input: None,
    }
}

fn show(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, stage: Stage, mut page: CheckupPage) {
    s.stage.set(stage);
    // Fork (port page): one port's checkup says whose it is.
    if let Some(key) = &s.scope {
        let name = port_by_key(app, key).map_or_else(|| key.clone(), |p| p.display_name().to_string());
        page.window_title = format!("{}: {name}", page.window_title);
        page.header = name;
    }
    if !show_page(app, router, page, handlers(s)) {
        eprintln!("checkup: the page window could not be made");
        end(app, router, s);
    }
}

/// B: the safe way out of the page on screen.
fn back(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    match s.stage.get() {
        Stage::Looking | Stage::Overview => leave(app, router, s),
        Stage::Step => show_overview(app, router),
        Stage::Confirm | Stage::After => advance(app, router, s),
        Stage::Finished => leave(app, router, s),
        Stage::Fixing => {}
    }
}

fn leave(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    end(app, router, s);
    // What the checkup found last is what Home and Settings show.
    refresh_in_background(app);
    // Fork (port page): one port's checkup goes back to its Port page.
    if let Some(done) = &s.done {
        done(app, router);
    }
}

fn end(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    s.looking.set(s.looking.get() + 1);
    POLL.with(slint::Timer::stop);
    WATCH.with(slint::Timer::stop);
    SESSION.with(|x| *x.borrow_mut() = None);
    close_current_dialog(app, router);
    app.refresh_current_view();
}

fn show_overview(app: &Rc<AppState>, router: &Router) {
    let Some(s) = session() else { return };
    let page = pages::overview_page(&s.results.borrow());
    show(app, router, &s, Stage::Overview, page);
}

/// Shows the wait, gathers the facts (the disk on a worker, then what is in memory), and runs `then` with the answer stored.
fn look(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, then: Rc<dyn Fn(&Rc<AppState>, &Router)>) {
    let first = s.results.borrow().items.is_empty();
    let mut wait = pages::looking_page();
    if !first {
        wait.title = "Checking again".to_string();
    }
    show(app, router, s, Stage::Looking, wait);
    let id = s.looking.get() + 1;
    s.looking.set(id);
    let input = sys_input_for(app, s.scope.as_deref());
    *s.job.borrow_mut() = Some(Rc::new(Background::spawn(move |_| sys_gather(input))));
    *s.then.borrow_mut() = Some(then);
    let (app, router, s) = (app.clone(), router.clone(), s.clone());
    POLL.with(|t| {
        t.start(slint::TimerMode::Repeated, Duration::from_millis(150), move || {
            if s.looking.get() != id {
                return POLL.with(slint::Timer::stop);
            }
            let done = s.job.borrow().as_ref().and_then(|j| j.take());
            let Some(done) = done else { return };
            POLL.with(slint::Timer::stop);
            *s.job.borrow_mut() = None;
            let then = s.then.borrow_mut().take();
            match done {
                Ok(sys) => {
                    let facts = assemble(&app_part(&app), sys);
                    let results = match s.scope.as_deref() {
                        Some(key) => checkup::run_for_port(&facts, key),
                        None => checkup::run(&facts),
                    };
                    eprintln!("checkup: looked ({}){}", results.summary(), if s.scope.is_some() { " for one port" } else { "" });
                    // Home's tile and Settings' card show the whole checkup, never one port's.
                    if s.scope.is_none() {
                        store(results.clone());
                    }
                    *s.results.borrow_mut() = results;
                    if let Some(then) = then {
                        then(&app, &router);
                    }
                }
                Err(e) => {
                    eprintln!("checkup: the look failed ({e})");
                    show(&app, &router, &s, Stage::Finished, pages::failed_page(&e));
                }
            }
        });
    });
}

fn act_on(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, act: Act) {
    match act {
        Act::NotNow | Act::Done => leave(app, router, s),
        Act::Overview => show_overview(app, router),
        Act::FixAll => {
            let order: Vec<Check> = s.results.borrow().problems().iter().map(|(c, _)| *c).collect();
            // Choosing it again with nothing changed picks up where it left off.
            let same = *s.order.borrow() == order && s.order_results.borrow().as_ref() == Some(&*s.results.borrow());
            eprintln!("checkup: fixing {} problem(s) one by one{}", order.len(), if same { " (where it left off)" } else { "" });
            if !same || s.pos.get() >= order.len() {
                s.pos.set(0);
            }
            *s.order.borrow_mut() = order;
            *s.order_results.borrow_mut() = Some(s.results.borrow().clone());
            show_step(app, router, s);
        }
        Act::Fix | Act::TryAgain => open_fix(app, router, s),
        Act::Skip => {
            skip_current(app, s);
            advance(app, router, s);
        }
        Act::Next => advance(app, router, s),
        Act::CheckAgain => {
            let in_run = !s.order.borrow().is_empty() && s.stage.get() == Stage::After;
            let then: Rc<dyn Fn(&Rc<AppState>, &Router)> = if in_run { Rc::new(show_after) } else { Rc::new(show_overview) };
            look(app, router, s, then);
        }
        Act::Unskip => {
            save_skips(app, &Skips::new());
            eprintln!("checkup: skipped problems brought back");
            look(app, router, s, Rc::new(show_overview));
        }
        Act::Yes => confirmed(app, router, s),
    }
}

fn current(s: &Session) -> Option<Check> {
    s.order.borrow().get(s.pos.get()).copied()
}

fn problem_of(s: &Session, check: Check) -> Option<checkup::Problem> {
    match s.results.borrow().outcome(check) {
        Some(Outcome::Problem(p)) => Some(p.clone()),
        _ => None,
    }
}

/// The step page of the check the run is at; one that is no longer a problem is passed over.
fn show_step(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    loop {
        let Some(check) = current(s) else { return show_finished(app, router, s) };
        match problem_of(s, check) {
            Some(p) => {
                let (n, total) = (s.pos.get() + 1, s.order.borrow().len());
                return show(app, router, s, Stage::Step, pages::step_page(check, &p, n, total));
            }
            None => s.pos.set(s.pos.get() + 1),
        }
    }
}

fn advance(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    s.pos.set(s.pos.get() + 1);
    show_step(app, router, s);
}

fn show_finished(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    let results = s.results.borrow().clone();
    // Every check the run set out to fix and every problem there is now (one can appear meanwhile: a repair queued while
    // Steam runs shows up under Steam changes), as it stands: fixed ones green, the rest counted.
    let mut checks: Vec<Check> = s.order.borrow().clone();
    for (c, _) in results.problems() {
        if !checks.contains(&c) {
            checks.push(c);
        }
    }
    checks.sort_by_key(|c| Check::ALL.iter().position(|x| x == c));
    let went_through: Vec<(Check, Outcome)> = checks.iter().filter_map(|c| results.outcome(*c).map(|o| (*c, o.clone()))).collect();
    let page = if went_through.is_empty() { pages::overview_page(&results) } else { pages::finished_page(&went_through) };
    let stage = if went_through.is_empty() { Stage::Overview } else { Stage::Finished };
    show(app, router, s, stage, page);
}

/// After a fix: the look is done, show that check's line.
fn show_after(app: &Rc<AppState>, router: &Router) {
    let Some(s) = session() else { return };
    let Some(check) = current(&s) else { return show_overview(app, router) };
    let outcome = s.results.borrow().outcome(check).cloned().unwrap_or_else(|| Outcome::Unknown("not looked at".into()));
    let order = s.order.borrow().clone();
    let last = !order.iter().skip(s.pos.get() + 1).any(|c| s.results.borrow().outcome(*c).is_some_and(Outcome::is_problem));
    let progress = matches!(&outcome, Outcome::Problem(p) if s.fix_ports.get() > 1 && p.ports.len() < s.fix_ports.get());
    let page = pages::after_page(check, &outcome, s.pos.get() + 1, order.len(), last, progress);
    let page = pages::with_notes(page, &std::mem::take(&mut *s.notes.borrow_mut()));
    show(app, router, &s, Stage::After, page);
}

// -- Fixes ----------------------------------------------------------------------------------------------------

fn port_by_key(app: &AppState, key: &str) -> Option<Port> {
    app.catalog.borrow().iter().find(|p| p.key() == key).cloned()
}

fn open_fix(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    let Some(check) = current(s) else { return show_overview(app, router) };
    let (n, total) = (s.pos.get() + 1, s.order.borrow().len());
    let problem = match s.results.borrow().outcome(check).cloned() {
        Some(Outcome::Problem(p)) => p,
        // A repair that waits for Steam to close: the way to finish it is the Close Steam flow.
        Some(Outcome::Queued { .. }) => {
            eprintln!("checkup: finishing the queued {} repair", check.title());
            return hand_off(app, router, s, Fix::CloseSteam);
        }
        _ => return show_after(app, router),
    };
    s.fix_ports.set(problem.ports.len());
    eprintln!("checkup: fix for {}: {:?}", check.title(), problem.fix);
    match &problem.fix {
        Fix::Update(key) => {
            let name = port_by_key(app, key).map_or_else(|| key.clone(), |p| p.display_name().to_string());
            let page = pages::confirm_page(check, &format!("Update {name} to the newest version?"), vec!["This is the same update you get from the port's own page.".to_string()], "Update", n, total);
            show(app, router, s, Stage::Confirm, page);
        }
        Fix::Repoint => {
            let names = checkup::names(&problem.ports.iter().map(|k| port_by_key(app, k).map_or_else(|| k.clone(), |p| p.display_name().to_string())).collect::<Vec<_>>());
            let page = pages::confirm_page(check, &format!("Point Steam at the new copy of {names}?"), vec!["Its name, art, play time and launch options stay. Only where it starts changes.".to_string()], "Point Steam at it", n, total);
            show(app, router, s, Stage::Confirm, page);
        }
        Fix::BackUpNow => {
            let page = pages::confirm_page(check, "Back up every port's saves now?", vec!["The saves are copied into a zip. Nothing is deleted or replaced.".to_string()], "Back up now", n, total);
            show(app, router, s, Stage::Confirm, page);
        }
        fix => hand_off(app, router, s, fix.clone()),
    }
}

/// "Yes" on a page of ours that asked first.
fn confirmed(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    let Some(check) = current(s) else { return };
    let Some(problem) = problem_of(s, check) else { return show_after(app, router) };
    if problem.fix == Fix::Repoint {
        return repoint(app, router, s, &problem.ports);
    }
    hand_off(app, router, s, problem.fix);
}

/// "Point Steam at it" was confirmed: every shortcut of these ports that still starts a recorded old copy is repointed at the new
/// copy (`port_mover::steam::repoint_everywhere`: every account, the app ID kept, written now when Steam is closed, else queued).
/// A queued change goes on to the Close Steam flow; either way the checkup looks again.
fn repoint(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, keys: &[String]) {
    let Some((queued, notes)) = repoint_ports(app, keys) else {
        return look(app, router, s, Rc::new(show_after));
    };
    *s.notes.borrow_mut() = notes;
    if queued > 0 {
        hand_off(app, router, s, Fix::CloseSteam);
    } else {
        look(app, router, s, Rc::new(show_after));
    }
}

/// Fork (port page): points every shortcut of these ports that still starts a recorded old copy at the new copy (the
/// checkup's `Fix::Repoint`, shared with the Port page's Steam card). How many changes were queued for Steam to close,
/// and the notes to show; `None` when where Steam and the old copies are could not be read (nothing was changed).
pub(crate) fn repoint_ports(app: &Rc<AppState>, keys: &[String]) -> Option<(usize, Vec<String>)> {
    use crate::fork::port_mover::steam::{repoint_everywhere, SteamCtx};
    let (Some(home), Ok(records)) = (home_dir(), old_copies::load(&app.paths.config_dir)) else {
        eprintln!("checkup: could not read where Steam and the old copies are; nothing was changed");
        return None;
    };
    let settings = ForkSettings::load(&settings_path(app));
    let steam = crate::fork::store::SystemSteamCheck::new();
    let ctx = SteamCtx { home: &home, config_dir: &app.paths.config_dir, settings: &settings, steam: &steam, game_mode: steam_queue::game_mode_from_env(), now: std::time::SystemTime::now() };
    let mut queued = 0;
    let mut notes = Vec::new();
    for key in keys {
        let name = port_by_key(app, key).map_or_else(|| key.clone(), |p| p.display_name().to_string());
        // Only a copy whose switch was made and whose new folder is there.
        for r in records.iter().filter(|r| r.switched && r.port_key == *key && r.new_path.is_dir()) {
            let report = repoint_everywhere(key, &name, &r.old_path, &r.new_path, &ctx);
            eprintln!("checkup: repoint {name}: wrote {}, queued {}, left alone {}, failed {}", report.changed_count(), report.queued.len(), report.skipped.len(), report.failed.len());
            queued += report.queued.len();
            // The record learns of it the way a move's does: when it was written, so only a Steam start after it counts as the
            // new copy's; or that it is queued, so the queue's apply stamps the time.
            if report.changed_count() > 0 || !report.queued.is_empty() {
                let at = chrono::Utc::now().timestamp();
                if let Err(e) = old_copies::note_repoint(&app.paths.config_dir, &r.old_path, !report.queued.is_empty(), at) {
                    eprintln!("checkup: could not note the repoint in the old copy's record ({e})");
                }
            }
            for (shortcut, why) in &report.skipped {
                notes.push(if why == crate::fork::port_mover::steam::NO_APP_ID { format!("Steam has not finished setting up {shortcut}; open and quit Steam once, then try again.") } else { format!("{shortcut} was left alone: {why}.") });
            }
            notes.extend(report.failed.iter().cloned());
        }
    }
    Some((queued, notes))
}

fn hand_off(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, fix: Fix) {
    close_current_dialog(app, router);
    s.stage.set(Stage::Fixing);
    s.quiet.set(0);
    *s.fix.borrow_mut() = Some(fix.clone());
    s.entering.set(true);
    let back_here: Rc<dyn Fn(&Rc<AppState>, &Router)> = Rc::new(|app, router| resume(app, router));
    match fix {
        Fix::PortsFolderGone => super::ports_folder_screen::show_missing(app, router, false),
        Fix::PortsFolderMenu => super::ports_folder_screen::open_menu(app, router),
        Fix::CloseSteam => {
            let then: super::steam_close_screen::Then = Rc::new(|app, router, _| resume(app, router));
            super::steam_close_screen::open(app, router, super::steam_close_screen::Opts { ask_first: true, ..Default::default() }, then);
        }
        // Fork (port page): one port's checkup fixes that port only.
        Fix::SteamIcons => match &s.scope {
            Some(key) => super::steam_icons_screen::open_for(app, router, Some(key.clone()), back_here),
            None => super::steam_icons_screen::open(app, router, back_here),
        },
        Fix::AddToSteam => super::steam_screens::open_steam_screen_for(app, router, s.scope.as_deref()),
        // Asked and done on the checkup's own pages (`repoint`), never handed off.
        Fix::Repoint => {}
        Fix::PickProgram(key) => {
            if let Some(port) = port_by_key(app, &key) {
                super::install_launch::open_favorite_exe_picker(app, router, port);
            }
        }
        Fix::GameFiles => match &s.scope {
            Some(key) => super::rom_screens::find_for_port(app, router, key, back_here),
            None => super::rom_screens::open_game_files(app, router),
        },
        Fix::FindSaves(key) => {
            if let Some(port) = port_by_key(app, &key) {
                super::save_finder_screen::open(app, router, port, back_here);
            }
        }
        Fix::OldCopies => super::ports_folder_screen::open_old_copies(app, router),
        Fix::Update(key) => {
            if let Some(port) = port_by_key(app, &key) {
                super::install_launch::start_install(app, router, port, None, None);
            }
        }
        Fix::BackUpNow => super::dialogs::start_save_backup(app, router),
        Fix::Cloud => super::cloud_screen::open_from_settings(app, router),
    }
    s.entering.set(false);
    start_watch(app, router);
}

fn start_watch(app: &Rc<AppState>, router: &Router) {
    let (app, router) = (app.clone(), router.clone());
    WATCH.with(|t| t.start(slint::TimerMode::Repeated, TICK, move || tick(&app, &router)));
}

/// While an existing screen is open the checkup waits. When no dialog has been open for a moment it comes back and
/// looks again.
fn tick(app: &Rc<AppState>, router: &Router) {
    let Some(s) = session() else { return WATCH.with(slint::Timer::stop) };
    if s.stage.get() != Stage::Fixing {
        return WATCH.with(slint::Timer::stop);
    }
    if dialog_is_open(app) || busy(app) || s.entering.get() {
        s.quiet.set(0);
        return;
    }
    s.quiet.set(s.quiet.get() + 1);
    if s.quiet.get() >= QUIET_TICKS {
        resume(app, router);
    }
}

/// A fix screen ended: look again and show that line. Does nothing when the checkup is not waiting.
fn resume(app: &Rc<AppState>, router: &Router) {
    let Some(s) = session() else { return };
    if s.stage.get() != Stage::Fixing {
        return;
    }
    WATCH.with(slint::Timer::stop);
    s.quiet.set(0);
    // Whatever page is left over from the fix screen goes first.
    close_current_dialog(app, router);
    look(app, router, &s, Rc::new(show_after));
}

/// A menu is about to open. When the checkup is waiting for the screen it opened and this menu is where that screen leads
/// when the user backs out (`Fix::returns_to`), come back to the checkup instead. True = handled. Any other menu (the ports
/// folder flow handing over to Game files, say), and a menu the checkup itself is opening as the fix, open as always.
pub(crate) fn resume_instead_of_menu(app: &Rc<AppState>, router: &Router, menu: checkup::Menu) -> bool {
    let Some(s) = session() else { return false };
    if s.stage.get() != Stage::Fixing || s.entering.get() {
        return false;
    }
    if !s.fix.borrow().as_ref().is_some_and(|f| f.returns_to(menu)) {
        return false;
    }
    resume(app, router);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(folder: &str) -> Port {
        crate::core::models::port_from_value(&serde_json::json!({"name": folder, "folder": folder, "source": "https://example.com/x.zip"})).unwrap()
    }

    fn input(root: &std::path::Path, installed: Vec<Port>) -> SysInput {
        SysInput {
            installed,
            favorites: HashMap::new(),
            played: HashSet::new(),
            catalog_folders: Vec::new(),
            home: Some(root.join("home")),
            library: root.join("Library"),
            config_dir: root.join("config"),
            settings_path: root.join("config").join("fork-settings.json"),
            saves_backup_dir: root.join("Saves Backup"),
            default_library: root.join("elsewhere"),
        }
    }

    fn snapshot(root: &std::path::Path) -> Vec<(PathBuf, Vec<u8>)> {
        fn walk(dir: &std::path::Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
            let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().filter_map(Result::ok).collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                let p = e.path();
                if p.is_dir() {
                    out.push((p.clone(), Vec::new()));
                    walk(&p, out);
                } else {
                    out.push((p.clone(), std::fs::read(&p).unwrap()));
                }
            }
        }
        let mut out = Vec::new();
        walk(root, &mut out);
        out
    }

    #[test]
    fn looking_reads_the_disk_and_writes_nothing_even_with_a_corrupt_waiting_list() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("home")).unwrap();
        std::fs::create_dir_all(root.path().join("config")).unwrap();
        // A waiting list the app cannot understand: `steam_queue::load` would move it aside; looking must leave it.
        std::fs::write(root.path().join("config").join(steam_queue::FILE_NAME), "{ not json").unwrap();
        let before = snapshot(root.path());
        let facts = sys_gather(input(root.path(), vec![port("a")]));
        assert_eq!(facts.folder, Folder::Gone, "a chosen ports folder that is not there");
        assert!(!facts.steam_found, "an empty home has no Steam folder");
        assert_eq!(facts.queue.as_ref().err().map(String::as_str), Some(checkup::QUEUE_UNREADABLE), "not 'nothing waiting'");
        assert!(matches!(&facts.shortcuts, Ok(s) if s.is_empty()));
        assert!(facts.programs.is_empty() && facts.files.is_empty() && facts.saves.is_empty(), "the folder is not there: no walks");
        assert_eq!(facts.backup_days, None);
        assert_eq!(snapshot(root.path()), before, "not a file made, moved or changed (no .corrupt-* copy of the list either)");
        let unreadable = checkup::check(Check::SteamWaiting, &assemble_for_test(facts));
        assert_eq!(unreadable, Outcome::Unknown("Steam's change list cannot be read".into()));
    }

    #[test]
    fn a_waiting_list_that_cannot_be_opened_is_could_not_check_not_empty() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("Library")).unwrap();
        std::fs::create_dir_all(root.path().join("config").join(steam_queue::FILE_NAME)).unwrap();
        let facts = sys_gather(input(root.path(), Vec::new()));
        assert_eq!(facts.folder, Folder::Here);
        assert!(facts.queue.is_err(), "a directory where the list should be: say so, do not call it empty");
        assert!(matches!(checkup::check(Check::SteamWaiting, &assemble_for_test(facts)), Outcome::Unknown(_)));
    }

    fn assemble_for_test(sys: SysFacts) -> Facts {
        let part = AppPart { installed: Vec::new(), updates: Vec::new(), cloud_problem: false, cloud_ok: false, game_mode: false };
        assemble(&part, sys)
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_port_with_two_programs_and_no_remembered_choice_is_asked_about_once() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("Library").join("two");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.sh"), b"#!/bin/sh\n").unwrap();
        std::fs::write(dir.join("b.sh"), b"#!/bin/sh\n").unwrap();
        let one = root.path().join("Library").join("one");
        std::fs::create_dir_all(&one).unwrap();
        std::fs::write(one.join("a.sh"), b"#!/bin/sh\n").unwrap();
        let mut i = input(root.path(), vec![port("two"), port("one")]);
        assert_eq!(programs_to_choose(&i), vec![Named::new("two", "two")], "a single program needs no choice");
        i.favorites.insert("two".to_string(), "b.sh".to_string());
        assert!(programs_to_choose(&i).is_empty(), "a remembered choice that still exists settles it");
        i.favorites.insert("two".to_string(), "gone.sh".to_string());
        assert_eq!(programs_to_choose(&i).len(), 1, "a remembered program that is gone asks again");
    }

    #[test]
    fn an_old_copy_is_gone_or_waits_for_its_first_start() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(root.path().join("home")).unwrap();
        let kept = root.path().join("old").join("kept");
        std::fs::create_dir_all(&kept).unwrap();
        let record = |name: &str, old: &std::path::Path| {
            serde_json::json!({"port_key": name, "name": name, "old_path": old, "new_path": root.path().join("Library").join(name), "bytes": 1,
                "moved_at": "20261024-120000", "link_only": false, "switched": true, "repoint_queued": false})
        };
        let file = serde_json::json!({"version": 1, "old_copies": [record("kept", &kept), record("vanished", &root.path().join("old").join("vanished"))]});
        std::fs::write(config.join(old_copies::FILE_NAME), file.to_string()).unwrap();
        let i = input(root.path(), Vec::new());
        let copies = old_copy_facts(&i).unwrap();
        assert_eq!(copies.iter().map(|c| (c.name.as_str(), c.state)).collect::<Vec<_>>(), [("kept", CopyState::StartOnce), ("vanished", CopyState::Gone)]);
        let record_after = std::fs::read_to_string(config.join(old_copies::FILE_NAME)).unwrap();
        assert_eq!(record_after, file.to_string(), "looking did not note anything in the record (readiness_now would have)");
    }

    #[test]
    fn a_switch_cut_short_counts_only_when_its_new_copy_is_a_catalog_folder_in_use() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        std::fs::create_dir_all(&config).unwrap();
        let lib = root.path().join("Library");
        let rec = |name: &str, new: &std::path::Path, switched: bool| {
            serde_json::json!({"port_key": name, "name": name, "old_path": root.path().join("old").join(name), "new_path": new, "bytes": 1, "moved_at": "20261024-120000", "link_only": false, "switched": switched})
        };
        let file = serde_json::json!({"version": 1, "old_copies": [rec("a", &lib.join("a"), false), rec("b", &lib.join("b"), true), rec("c", &root.path().join("elsewhere").join("c"), false)]});
        std::fs::write(config.join(old_copies::FILE_NAME), file.to_string()).unwrap();
        let mut i = input(root.path(), Vec::new());
        i.catalog_folders = vec!["a".into(), "b".into(), "c".into()];
        assert_eq!(unfinished_switches(&i), vec!["a".to_string()], "only the one cut short whose new copy is in the ports folder");
    }

    #[test]
    fn the_disk_part_reads_saves_and_game_files_for_each_port_without_the_app() {
        let root = tempfile::tempdir().unwrap();
        let lib = root.path().join("Library");
        std::fs::create_dir_all(lib.join("a")).unwrap();
        std::fs::create_dir_all(root.path().join("config")).unwrap();
        // Told "no saves" by the user: not missing, not known.
        std::fs::write(root.path().join("config").join("fork-settings.json"), serde_json::json!({"save_overrides": {"a": {"no_saves": true}}}).to_string()).unwrap();
        let i = input(root.path(), vec![port("a"), port("b")]);
        let (missing, known) = saves_not_found(&i);
        assert_eq!(known, 0);
        assert!(missing.is_empty(), "a is answered, b has no folder here: {missing:?}");
        std::fs::create_dir_all(lib.join("b")).unwrap();
        let (missing, _) = saves_not_found(&i);
        assert_eq!(missing, vec![Named::new("b", "b")], "no save folder is known for b");
        assert!(game_files_missing(&i).is_empty(), "these ports name no game files");
    }
}
