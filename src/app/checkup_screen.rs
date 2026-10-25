//! "Fix problems" on screen: the pages of `crate::fork::checkup::pages` in the guided-page window, the facts gathered
//! for them, the hand-offs to the existing fix screens and the way back, and the cached answer Home's tile and
//! Settings' card read.
//!
//! **Looking changes nothing.** The facts are read from the disk, Steam's files and the caches Home keeps. The disk
//! part runs on a worker thread (`Background`), the part that needs the app's state is read on the UI thread from
//! the same cached answers Home uses. Every fix is an existing screen that asks on its own page (or, for an update and
//! a backup, a page of ours that asks first); the checkup itself never writes.
//!
//! **Hand-off and return.** A fix closes the checkup page and opens the existing screen. Those screens do not know
//! about the checkup, so (like the setup wizard) it waits: a half-second watcher notices that no dialog is open for a
//! moment and comes back, and the screens that lead back to a Settings menu (`resume_instead_of_menu`) come back to the
//! checkup instead. Coming back looks again and shows that line, green or not (`after_page`).
//!
//! **The cache.** Home's tile and Settings' card show the last answer (`CACHE`), refreshed in the background at most
//! every few minutes and after every checkup; opening Home or Settings never waits for it.

use super::dialogs::{close_current_dialog, dialog_is_open};
use super::page_dialog::{show_page, Handlers};
use super::state::AppState;
use crate::core::models::Port;
use crate::fork::checkup::pages::{self, Act, CheckupPage};
use crate::fork::checkup::{self, Check, CopyFact, CopyState, Facts, Fix, Folder, Named, Outcome, QueuedFact, Results};
use crate::fork::folder_picker::mounts::{default_media_roots, system_mounts};
use crate::fork::port_mover::old_copies;
use crate::fork::rom_screens::job::Background;
use crate::fork::setup_wizard::files_state::FilesState;
use crate::fork::{ports_folder, steam_icons, steam_queue};
use crate::ui::gamepad_router::GamepadRouter;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
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
    /// The app part of the facts read when the background look started.
    pending: Option<AppPart>,
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
    /// The checks with a problem when "Fix them one by one" was chosen, and where the run is.
    order: RefCell<Vec<Check>>,
    pos: Cell<usize>,
    /// The checks the run went through, in order (for the last page).
    visited: RefCell<Vec<Check>>,
    /// The fix is being opened right now: a menu it opens must show, not hand control back.
    entering: Cell<bool>,
    quiet: Cell<u32>,
    job: RefCell<Option<Rc<Background<SysFacts>>>>,
    app_part: RefCell<Option<AppPart>>,
    /// Bumped whenever a look is started or cancelled; a result that arrives for an older one is dropped.
    looking: Cell<u64>,
    /// What to do when the look in progress ends.
    then: RefCell<Option<Rc<dyn Fn(&Rc<AppState>, &Router)>>>,
}

fn session() -> Option<Rc<Session>> {
    SESSION.with(|s| s.borrow().clone())
}

// -- Facts ----------------------------------------------------------------------------------------------------

/// What the worker thread reads: only paths and plain values, so it never touches the UI's state.
struct SysInput {
    installed: Vec<Port>,
    favorites: HashMap<String, String>,
    home: Option<PathBuf>,
    library: PathBuf,
    config_dir: PathBuf,
    saves_backup_dir: PathBuf,
    default_library: PathBuf,
}

struct SysFacts {
    folder: Folder,
    steam_found: bool,
    queue: Result<Vec<QueuedFact>, String>,
    shortcuts: Result<Vec<steam_icons::IconFinding>, String>,
    programs: Vec<Named>,
    old_copies: Result<Vec<CopyFact>, String>,
    backup_days: Option<u64>,
}

/// What needs the app's state: read on the UI thread, from the caches Home uses.
#[derive(Clone)]
struct AppPart {
    installed: Vec<Named>,
    files: Vec<(Named, FilesState)>,
    saves: Vec<Named>,
    updates: Vec<Named>,
    cloud_problem: bool,
    unfinished_moves: Vec<String>,
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

/// `fresh`: do not reuse the answers Home remembers for a few seconds (after a fix, the line must show what is true now).
fn app_part(app: &AppState, fresh: bool) -> AppPart {
    let installed = installed_ports(app);
    if fresh {
        super::save_finder_screen::forget_states();
    }
    let states = super::home_screen::file_states(app, &installed, fresh);
    let mut files = Vec::new();
    let mut saves = Vec::new();
    let mut updates = Vec::new();
    for p in &installed {
        match states.get(p.key()).copied().flatten() {
            Some(s @ (FilesState::Missing | FilesState::Unverifiable)) => files.push((named(p), s)),
            _ => {}
        }
        if super::save_finder_screen::saves_warning(app, p) {
            saves.push(named(p));
        }
        if super::home_screen::update_ready(app, p.key()) {
            updates.push(named(p));
        }
    }
    AppPart {
        installed: installed.iter().map(named).collect(),
        files,
        saves,
        updates,
        cloud_problem: super::cloud_screen::home_status(app).0 == 5,
        unfinished_moves: super::ports_folder_screen::unfinished_switches(app),
        game_mode: steam_queue::game_mode_from_env(),
    }
}

fn sys_input(app: &AppState) -> SysInput {
    let installed = installed_ports(app);
    let favorites = {
        let state = app.state.borrow();
        installed.iter().filter_map(|p| state.get(p.key()).and_then(|i| i.favorite_exe.clone()).map(|f| (p.key().to_string(), f))).collect()
    };
    SysInput {
        installed,
        favorites,
        home: home_dir(),
        library: app.paths.library_dir(),
        config_dir: app.paths.config_dir.clone(),
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
    let queue_file = steam_queue::load(&steam_queue::queue_path(&i.config_dir));
    let queue = match queue_file.problem {
        Some(_) if queue_file.unreadable => Err("The list of waiting Steam changes could not be opened".to_string()),
        _ => Ok(queue_file.queue.entries.iter().map(|e| QueuedFact { port_key: e.port_key.clone(), name: e.name.clone(), kind: e.kind }).collect()),
    };
    let steam_found = i.home.as_deref().is_some_and(|h| !crate::fork::port_mover::steam::device_accounts(h).is_empty());
    let shortcuts = match i.home.as_deref() {
        Some(h) => steam_icons::diagnose(h, &i.library, &i.config_dir, &i.installed),
        None => Err("Could not find your home folder".to_string()),
    };
    let programs = if folder == Folder::Here { programs_to_choose(&i) } else { Vec::new() };
    let old_copies = old_copy_facts(&i);
    let backup_days = {
        let dir = i.saves_backup_dir.join(crate::fork::save_restore::GLOBAL_BACKUPS_DIR);
        let now = chrono::Utc::now().timestamp();
        crate::fork::settings_menu::newest_zip(&dir).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| crate::fork::settings_menu::days_ago(d.as_secs() as i64, now))
    };
    SysFacts { folder, steam_found, queue, shortcuts, programs, old_copies, backup_days }
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

/// Each kept old copy and what it needs. Read only: `readiness_now`, which notes a first sighting in the record, is not
/// used here.
fn old_copy_facts(i: &SysInput) -> Result<Vec<CopyFact>, String> {
    let list = old_copies::load(&i.config_dir)?;
    let home = i.home.clone().unwrap_or_default();
    Ok(list
        .iter()
        .map(|c| {
            let state = if std::fs::symlink_metadata(&c.old_path).is_err() {
                CopyState::Gone
            } else {
                CopyState::of(old_copies::readiness(c, &old_copies::steam_view(c, &home, &i.config_dir)))
            };
            CopyFact { name: c.name.clone(), state }
        })
        .collect())
}

fn assemble(app: &AppPart, sys: SysFacts) -> Facts {
    Facts {
        game_mode: app.game_mode,
        installed: app.installed.clone(),
        folder: sys.folder,
        unfinished_moves: app.unfinished_moves.clone(),
        steam_found: sys.steam_found,
        queue: sys.queue,
        shortcuts: sys.shortcuts,
        programs: sys.programs,
        files: app.files.clone(),
        saves: app.saves.clone(),
        old_copies: sys.old_copies,
        updates: app.updates.clone(),
        backup_days: sys.backup_days,
        cloud_problem: app.cloud_problem,
    }
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

/// The tile Home shows (`None`: nothing to fix, or no answer yet).
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
    if CACHE.with(|c| c.borrow().job.is_some()) {
        return;
    }
    let part = app_part(app, false);
    let job = Rc::new(Background::spawn({
        let input = sys_input(app);
        move |_| sys_gather(input)
    }));
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.job = Some(job);
        c.pending = Some(part);
    });
    let app = app.clone();
    BACKGROUND_POLL.with(|t| {
        t.start(slint::TimerMode::Repeated, Duration::from_millis(300), move || {
            let done = CACHE.with(|c| c.borrow().job.as_ref().and_then(|j| j.take()));
            let Some(done) = done else { return };
            BACKGROUND_POLL.with(slint::Timer::stop);
            let part = CACHE.with(|c| {
                let mut c = c.borrow_mut();
                c.job = None;
                c.pending.take()
            });
            match (done, part) {
                (Ok(sys), Some(part)) => {
                    let results = checkup::run(&assemble(&part, sys));
                    eprintln!("checkup: background look: {}", results.summary());
                    let changed = store(results);
                    // Redrawn even under a dialog: Home is only drawn behind it, and the tile must be there when it closes.
                    if changed && app.home.is_active() {
                        app.refresh_current_view();
                    }
                }
                (Err(e), _) => {
                    eprintln!("checkup: the background look failed ({e})");
                    CACHE.with(|c| c.borrow_mut().at = Some(Instant::now()));
                }
                _ => {}
            }
        });
    });
}

// -- The checkup ----------------------------------------------------------------------------------------------

/// Settings > Fix problems, and Home's tile. Closes whatever page is open, then looks.
pub(crate) fn open(app: &Rc<AppState>, router: &Router) {
    close_current_dialog(app, router);
    let s = Rc::new(Session {
        stage: Cell::new(Stage::Looking),
        results: RefCell::new(Results { items: Vec::new() }),
        order: RefCell::new(Vec::new()),
        pos: Cell::new(0),
        visited: RefCell::new(Vec::new()),
        entering: Cell::new(false),
        quiet: Cell::new(0),
        job: RefCell::new(None),
        app_part: RefCell::new(None),
        looking: Cell::new(0),
        then: RefCell::new(None),
    });
    SESSION.with(|x| *x.borrow_mut() = Some(s.clone()));
    eprintln!("checkup: opened");
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

fn show(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, stage: Stage, page: CheckupPage) {
    s.stage.set(stage);
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

/// Shows the wait, gathers the facts (disk on a worker, the rest here), and runs `then` with the answer stored.
fn look(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, then: Rc<dyn Fn(&Rc<AppState>, &Router)>) {
    let first = s.results.borrow().items.is_empty();
    let mut wait = pages::looking_page();
    if !first {
        wait.title = "Checking again".to_string();
    }
    show(app, router, s, Stage::Looking, wait);
    let id = s.looking.get() + 1;
    s.looking.set(id);
    *s.app_part.borrow_mut() = Some(app_part(app, true));
    let input = sys_input(app);
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
            let part = s.app_part.borrow_mut().take();
            let then = s.then.borrow_mut().take();
            match (done, part) {
                (Ok(sys), Some(part)) => {
                    let results = checkup::run(&assemble(&part, sys));
                    eprintln!("checkup: looked ({})", results.summary());
                    store(results.clone());
                    *s.results.borrow_mut() = results;
                    if let Some(then) = then {
                        then(&app, &router);
                    }
                }
                (Err(e), _) => {
                    eprintln!("checkup: the look failed ({e})");
                    show(&app, &router, &s, Stage::Finished, pages::failed_page(&e));
                }
                _ => end(&app, &router, &s),
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
            eprintln!("checkup: fixing {} problem(s) one by one", order.len());
            *s.order.borrow_mut() = order;
            s.pos.set(0);
            s.visited.borrow_mut().clear();
            show_step(app, router, s);
        }
        Act::Fix | Act::TryAgain => open_fix(app, router, s),
        Act::Skip | Act::Next => advance(app, router, s),
        Act::CheckAgain => {
            let in_run = !s.order.borrow().is_empty() && s.stage.get() == Stage::After;
            let then: Rc<dyn Fn(&Rc<AppState>, &Router)> = if in_run { Rc::new(show_after) } else { Rc::new(show_overview) };
            look(app, router, s, then);
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
    let visited: Vec<(Check, Outcome)> = s.visited.borrow().iter().filter_map(|c| results.outcome(*c).map(|o| (*c, o.clone()))).collect();
    let page = if visited.is_empty() { pages::overview_page(&results) } else { pages::finished_page(&visited) };
    let stage = if visited.is_empty() { Stage::Overview } else { Stage::Finished };
    show(app, router, s, stage, page);
}

/// After a fix: the look is done, show that check's line.
fn show_after(app: &Rc<AppState>, router: &Router) {
    let Some(s) = session() else { return };
    let Some(check) = current(&s) else { return show_overview(app, router) };
    let outcome = s.results.borrow().outcome(check).cloned().unwrap_or_else(|| Outcome::Unknown("not looked at".into()));
    let order = s.order.borrow().clone();
    let last = !order.iter().skip(s.pos.get() + 1).any(|c| s.results.borrow().outcome(*c).is_some_and(Outcome::is_problem));
    let page = pages::after_page(check, &outcome, s.pos.get() + 1, order.len(), last);
    show(app, router, &s, Stage::After, page);
}

// -- Fixes ----------------------------------------------------------------------------------------------------

fn port_by_key(app: &AppState, key: &str) -> Option<Port> {
    app.catalog.borrow().iter().find(|p| p.key() == key).cloned()
}

fn open_fix(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    let Some(check) = current(s) else { return show_overview(app, router) };
    let Some(problem) = problem_of(s, check) else { return show_after(app, router) };
    {
        let mut visited = s.visited.borrow_mut();
        if !visited.contains(&check) {
            visited.push(check);
        }
    }
    eprintln!("checkup: fix for {}: {:?}", check.title(), problem.fix);
    let (n, total) = (s.pos.get() + 1, s.order.borrow().len());
    match &problem.fix {
        Fix::Update(key) => {
            let name = port_by_key(app, key).map_or_else(|| key.clone(), |p| p.display_name().to_string());
            let page = pages::confirm_page(check, &format!("Update {name} to the newest version?"), vec!["This is the same update you get from the port's own page.".to_string()], "Update", n, total);
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
    hand_off(app, router, s, problem.fix);
}

fn hand_off(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, fix: Fix) {
    close_current_dialog(app, router);
    s.stage.set(Stage::Fixing);
    s.quiet.set(0);
    s.entering.set(true);
    let back_here: Rc<dyn Fn(&Rc<AppState>, &Router)> = Rc::new(|app, router| resume(app, router));
    match fix {
        Fix::PortsFolderGone => super::ports_folder_screen::show_missing(app, router, false),
        Fix::PortsFolderMenu => super::ports_folder_screen::open_menu(app, router),
        Fix::CloseSteam => {
            let then: super::steam_close_screen::Then = Rc::new(|app, router, _| resume(app, router));
            super::steam_close_screen::open(app, router, super::steam_close_screen::Opts { ask_first: true, ..Default::default() }, then);
        }
        Fix::SteamIcons => super::steam_icons_screen::open(app, router, back_here),
        Fix::AddToSteam => super::steam_screens::open_steam_screen(app, router),
        Fix::PickProgram(key) => match port_by_key(app, &key) {
            Some(port) => super::install_launch::open_favorite_exe_picker(app, router, port),
            None => {}
        },
        Fix::GameFiles => super::rom_screens::open_game_files(app, router),
        Fix::FindSaves(key) => match port_by_key(app, &key) {
            Some(port) => super::save_finder_screen::open(app, router, port, back_here),
            None => {}
        },
        Fix::OldCopies => super::ports_folder_screen::open_old_copies(app, router),
        Fix::Update(key) => match port_by_key(app, &key) {
            Some(port) => super::install_launch::start_install(app, router, port, None, None),
            None => {}
        },
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
    if dialog_is_open(app) || super::import_screen::import_running() || s.entering.get() {
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

/// A menu (Settings, Ports folder, Game files) is about to open. When the checkup is waiting for the screen it opened,
/// and this menu is where that screen leads when the user backs out, come back to the checkup instead. True = handled.
/// A menu the checkup itself is opening as the fix is left alone.
pub(crate) fn resume_instead_of_menu(app: &Rc<AppState>, router: &Router) -> bool {
    let Some(s) = session() else { return false };
    if s.stage.get() != Stage::Fixing || s.entering.get() {
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
            home: Some(root.join("home")),
            library: root.join("Library"),
            config_dir: root.join("config"),
            saves_backup_dir: root.join("Saves Backup"),
            default_library: root.join("elsewhere"),
        }
    }

    #[test]
    fn looking_reads_the_disk_and_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("home")).unwrap();
        std::fs::create_dir_all(root.path().join("config")).unwrap();
        let before = std::fs::read_dir(root.path()).unwrap().count();
        let facts = sys_gather(input(root.path(), vec![port("a")]));
        assert_eq!(facts.folder, Folder::Gone, "a chosen ports folder that is not there");
        assert!(!facts.steam_found, "an empty home has no Steam folder");
        assert!(matches!(&facts.queue, Ok(q) if q.is_empty()));
        assert!(matches!(&facts.shortcuts, Ok(s) if s.is_empty()));
        assert!(facts.programs.is_empty(), "the folder is not there: no walk");
        assert_eq!(facts.backup_days, None);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), before, "nothing was created next to the app's folders");
        assert_eq!(std::fs::read_dir(root.path().join("config")).unwrap().count(), 0, "and nothing in the config folder");
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
        let part = AppPart { installed: Vec::new(), files: Vec::new(), saves: Vec::new(), updates: Vec::new(), cloud_problem: false, unfinished_moves: Vec::new(), game_mode: false };
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
}
