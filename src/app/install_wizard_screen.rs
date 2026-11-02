//! The install wizard on screen: the pages of `crate::fork::install_wizard`, one question each, from every place that
//! installs (A on a port in All ports or Deck-ready, a Deck-ready project, Update on the Port page or in Fix problems).
//!
//! - [`open`] / [`open_entry`]: "Install this port?" (the same-game note is on it). Install starts `start_install`, which
//!   calls [`downloading`] for its page and its byte meter. Every install shows that page, even one with no question
//!   first (Play's update, a reinstall): upstream's bare progress dialog is never used for an install.
//! - The download goes on in a plain thread (`core::jobs::run_install`); a 250 ms timer reads its meter and redraws the
//!   page, and Home's tile while it is in the background. "Stop and remove the download" sets the installer's cancel
//!   flag (`DownloadCtl`); the install then fails with `installer::CANCELLED` and its temporary folder is gone.
//! - When the install ends (`events.rs`): [`end_run`], then [`after_install`] goes on with the pages that follow (kept
//!   files, game files, Steam, done), [`choose_asset`] when the release has several files, or [`install_failed`] ("Could
//!   not install" with one way back). An install sent to the background shows a short "finished" page (or its failure)
//!   when it ends and nothing else is open (or as soon as nothing is). A flow that ends before Done is dropped.
//!
//! Every change goes through its existing flow and its guards: the game files pages (`rom_screens::find_for_port`), the
//! setup wizard's one-button Steam add (`setup_wizard::steam::add_all`, which queues while Steam runs) and the Close
//! Steam flow, `uninstall_keep::restore_kept` (copies, never overwrites without a copy aside first).

use super::dialogs::{close_current_dialog, dialog_is_open};
use super::page_dialog::{show_page, show_page_handle, Handlers, PageHandle};
use super::state::AppState;
use crate::core::installer::{DownloadCtl, CANCELLED};
use crate::core::models::Port;
use crate::fork::art::thumbs::ArtRef;
use crate::fork::home::List;
use crate::fork::install_wizard::{self as wiz, InstallInfo, KeptOffer, Meter, SameNote, Snapshot, Speed, WizAct};
use crate::fork::page::{Mark, Page};
use crate::fork::port_page::paths::{human_path, Places};
use crate::fork::rom_screens::job::Background;
use crate::fork::same_game::{self, Game};
use crate::fork::steam_add::Ask;
use crate::fork::uninstall_keep;
use crate::ui::gamepad_router::GamepadRouter;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

type Router = Rc<RefCell<GamepadRouter>>;
/// Where "Not now" (and B) on the first page goes, and where a stopped download returns.
pub(crate) type Back = Rc<dyn Fn(&Rc<AppState>, &Router)>;
/// A Deck-ready project: writes the user's own entry and gives back the port to install (`None`: nothing was added, and
/// it has said why).
pub(crate) type Prepare = Rc<dyn Fn(&Rc<AppState>, &Router) -> Option<Port>>;

const TICK: Duration = Duration::from_millis(250);
const WORK_TICK: Duration = Duration::from_millis(150);

/// One install running now.
struct Run {
    key: String,
    name: String,
    art: Option<ArtRef>,
    meter: Arc<Meter>,
    started: Instant,
    speed: RefCell<Speed>,
    /// "Keep going in the background" (or the page was replaced by something else): Home's tile shows it.
    background: Cell<bool>,
    handle: RefCell<Option<PageHandle<WizAct>>>,
    /// The page last drawn, so the timer redraws only on a change.
    shown: RefCell<Option<Page<WizAct>>>,
    tile: RefCell<String>,
}

/// One install started from the first page: the pages after the download follow.
struct Flow {
    key: String,
    name: String,
    art: Option<ArtRef>,
    update: bool,
    back: Back,
    /// Lines for the Done page (kept files put back, Steam, a versioned file's note).
    notes: RefCell<Vec<(Mark, String)>>,
    was_background: Cell<bool>,
}

thread_local! {
    static RUNS: RefCell<HashMap<String, Rc<Run>>> = RefCell::new(HashMap::new());
    static FLOWS: RefCell<HashMap<String, Rc<Flow>>> = RefCell::new(HashMap::new());
    static TIMER: slint::Timer = slint::Timer::default();
    /// Background installs that ended while something else was on screen: their note waits for a free screen.
    static WAITING: RefCell<Vec<Waiting>> = const { RefCell::new(Vec::new()) };
    static ROUTER: RefCell<Option<Router>> = const { RefCell::new(None) };
    static WORK_TIMER: slint::Timer = slint::Timer::default();
    static WORK: RefCell<Vec<Box<dyn FnMut() -> bool>>> = RefCell::new(Vec::new());
    static HOME_DRAWN: Cell<Option<Instant>> = const { Cell::new(None) };
}

const HOME_EVERY: Duration = Duration::from_secs(1);

/// A background install's note that waits for a free screen.
enum Waiting {
    /// "<name> finished installing" (its flow has the rest).
    Finished(String),
    /// "Could not install": the port's key and what went wrong.
    Failed(String, String),
}

fn run_of(key: &str) -> Option<Rc<Run>> {
    RUNS.with(|r| r.borrow().get(key).cloned())
}

fn flow_of(key: &str) -> Option<Rc<Flow>> {
    FLOWS.with(|f| f.borrow().get(key).cloned())
}

fn drop_flow(key: &str) {
    FLOWS.with(|f| f.borrow_mut().remove(key));
}

fn close_back() -> Back {
    Rc::new(|app, router| close_current_dialog(app, router))
}

/// Polls `done` until it returns true (it has handled its job's answer); one timer for all.
fn when_done(done: impl FnMut() -> bool + 'static) {
    WORK.with(|w| w.borrow_mut().push(Box::new(done)));
    WORK_TIMER.with(|t| {
        if !t.running() {
            t.start(slint::TimerMode::Repeated, WORK_TICK, || {
                let mut now = WORK.with(|w| std::mem::take(&mut *w.borrow_mut()));
                now.retain_mut(|done| !done());
                let left = WORK.with(|w| {
                    let mut w = w.borrow_mut();
                    now.append(&mut w);
                    *w = now;
                    !w.is_empty()
                });
                if !left {
                    WORK_TIMER.with(slint::Timer::stop);
                }
            });
        }
    });
}

fn handlers(on_action: impl Fn(&Rc<AppState>, &Router, WizAct) + 'static, back: WizAct) -> Handlers<WizAct> {
    let on_action: Rc<dyn Fn(&Rc<AppState>, &Router, WizAct)> = Rc::new(on_action);
    let (b, x) = (on_action.clone(), on_action.clone());
    Handlers { on_action, on_back: Rc::new(move |app, router| b(app, router, back)), on_exit: Rc::new(move |app, router| x(app, router, back)), on_input: None }
}

// -- 1. Install <name>? -------------------------------------------------------------------------------------------

fn local_entry(app: &AppState, folder: &str) -> Option<serde_json::Value> {
    crate::fork::rom_library::load_local_entries(&app.paths.config_dir.join("ports.local.json")).remove(folder)
}

/// What the first page says about `port`. `game` is what the same-game check compares (a Deck-ready project's own label
/// for its system); `system` the label shown when the catalog's tags name none.
fn info_for(app: &AppState, port: &Port, game: &Game, system: Option<String>, version: Option<String>, own_entry: bool) -> InstallInfo {
    let update = app.is_installed(port);
    let name = port.display_name().to_string();
    let spec = crate::fork::rom_library::RomSpec::for_port(port, local_entry(app, &port.folder).as_ref());
    let same = if update {
        None
    } else {
        let owned = super::same_game_screen::Installed::read(app).owned;
        same_game::find(&owned, if own_entry { "" } else { port.key() }, game).map(|f| SameNote { verdict: f.verdict, owned_project: f.owned.project.clone() })
    };
    let installed_tag = app.state.borrow().get(port.key()).and_then(|i| i.installed_tag.clone());
    InstallInfo {
        art: Some(ArtRef::of_port(port)),
        system: same_game::system_from_tags(&port.tags_lower).map(|s| wiz::sys_label(&s)).or(system),
        version: if update { installed_tag } else { version },
        size: None,
        place: human_path(&app.paths.library_dir(), &Places::system()),
        needs: if port.is_android_only() { Vec::new() } else { wiz::needs_words(&spec, &name) },
        same,
        update,
        own_entry,
        has_project_page: crate::fork::port_page::project_url(port).is_some(),
        busy: false,
        name,
    }
}

/// The port is installing, checking for an update, or running: Install has to wait.
fn busy(app: &AppState, key: &str) -> bool {
    app.install_runtime.installing.borrow().contains(key) || super::playtime::is_port_running(app, key)
}

/// "Install <name>?" (or "Update <name>?") for a catalog port. `back`: where Not now and B go.
pub(crate) fn open(app: &Rc<AppState>, router: &Router, port: Port, back: Back) {
    if reopen(app, router, port.key()) {
        return;
    }
    let game = Game::of_port(&port.name, &port.tags_lower);
    let info = info_for(app, &port, &game, None, None, false);
    show_install(app, router, port, info, None, back);
}

/// [`open`] for a Deck-ready project the app does not have yet: `port` is the entry it would add (nothing is written
/// until Install), `prepare` writes it.
pub(crate) fn open_entry(app: &Rc<AppState>, router: &Router, port: Port, system: Option<String>, version: Option<String>, prepare: Prepare, back: Back) {
    let game = Game::of_entry(&port.name, system.as_deref());
    let info = info_for(app, &port, &game, system, version, true);
    show_install(app, router, port, info, Some(prepare), back);
}

/// Opens an install's first page from a list (A on a row): Not now closes it.
pub(crate) fn open_from_list(app: &Rc<AppState>, router: &Router, port: Port) {
    open(app, router, port, close_back());
}

fn show_install(app: &Rc<AppState>, router: &Router, port: Port, info: InstallInfo, prepare: Option<Prepare>, back: Back) {
    let page = wiz::install_page(&info);
    let (name, update) = (info.name.clone(), info.update);
    let on_action = move |app: &Rc<AppState>, router: &Router, act: WizAct| match act {
        WizAct::Install => {
            if busy(app, port.key()) {
                let info = InstallInfo { busy: true, ..info.clone() };
                return show_install(app, router, port.clone(), info, prepare.clone(), back.clone());
            }
            let port = match &prepare {
                Some(prepare) => match prepare(app, router) {
                    Some(port) => port,
                    None => return,
                },
                None => port.clone(),
            };
            let key = port.key().to_string();
            let flow = Flow { key: key.clone(), name: name.clone(), art: Some(ArtRef::of_port(&port)), update, back: back.clone(), notes: RefCell::new(Vec::new()), was_background: Cell::new(false) };
            FLOWS.with(|f| f.borrow_mut().insert(key.clone(), Rc::new(flow)));
            eprintln!("install wizard: {} {}", if update { "updating" } else { "installing" }, port.display_name());
            super::install_launch::start_install(app, router, port, None, None);
            // It did not start (the ports folder is missing, it is busy, it has nothing to download): no pages follow.
            if run_of(&key).is_none() {
                drop_flow(&key);
            }
        }
        WizAct::ProjectPage => {
            if let Some(url) = crate::fork::port_page::project_url(&port) {
                // The project page's B comes back to this page.
                let again: Back = {
                    let (port, info, prepare, back) = (port.clone(), info.clone(), prepare.clone(), back.clone());
                    Rc::new(move |app, router| show_install(app, router, port.clone(), info.clone(), prepare.clone(), back.clone()))
                };
                super::port_page_screen::open_link(app, router, &name, &url, Some(again));
            }
        }
        _ => back(app, router),
    };
    close_current_dialog(app, router);
    show_page(app, router, page, handlers(on_action, WizAct::NotNow));
}

// -- 2. Downloading -----------------------------------------------------------------------------------------------

/// `start_install`'s page and meter: shown at once, filled by the timer. Every install comes here.
pub(crate) fn downloading(app: &Rc<AppState>, router: &Router, port: &Port) -> (DownloadCtl, Arc<Meter>) {
    ROUTER.with(|r| *r.borrow_mut() = Some(router.clone()));
    let meter = Meter::new();
    let run = Rc::new(Run {
        key: port.key().to_string(),
        name: port.display_name().to_string(),
        art: Some(ArtRef::of_port(port)),
        meter: meter.clone(),
        started: Instant::now(),
        speed: RefCell::new(Speed::default()),
        background: Cell::new(false),
        handle: RefCell::new(None),
        shown: RefCell::new(None),
        tile: RefCell::new(String::new()),
    });
    RUNS.with(|r| r.borrow_mut().insert(run.key.clone(), run.clone()));
    show_downloading(app, router, &run);
    start_timer(app);
    let m = meter.clone();
    let ctl = DownloadCtl { on_bytes: Some(Arc::new(move |done, total| m.record(done, total))), cancel: Some(meter.cancel_flag()) };
    (ctl, meter)
}

fn snapshot(run: &Run) -> Snapshot {
    let (downloaded, total) = run.meter.bytes();
    Snapshot { phase: run.meter.phase(), downloaded, total, speed: run.speed.borrow().per_second(), stopping: run.meter.stopping() }
}

fn show_downloading(app: &Rc<AppState>, router: &Router, run: &Rc<Run>) {
    let page = wiz::downloading_page(&run.name, &run.art, &snapshot(run));
    let r = run.clone();
    let on_action = move |app: &Rc<AppState>, router: &Router, act: WizAct| match act {
        WizAct::Stop => {
            eprintln!("install wizard: stopping the download of {}", r.name);
            r.meter.stop();
            redraw(&r);
        }
        _ => to_background(app, router, &r),
    };
    run.background.set(false);
    *run.shown.borrow_mut() = Some(page.clone());
    *run.handle.borrow_mut() = show_page_handle(app, router, page, handlers(on_action, WizAct::Background));
}

/// "Keep going in the background" (B and the X too): the page closes, the install goes on, Home's tile shows it. Also
/// while stopping: the stop finishes on its own (a stalled connection takes up to the read timeout to notice it).
fn to_background(app: &Rc<AppState>, router: &Router, run: &Rc<Run>) {
    run.background.set(true);
    eprintln!("install wizard: {} goes on in the background", run.name);
    close_current_dialog(app, router);
    app.refresh_current_view();
}

fn redraw(run: &Rc<Run>) {
    let page = wiz::downloading_page(&run.name, &run.art, &snapshot(run));
    if run.shown.borrow().as_ref() == Some(&page) {
        return;
    }
    let open = run.handle.borrow().as_ref().is_some_and(|h| h.update(page.clone()));
    if open {
        *run.shown.borrow_mut() = Some(page);
    }
}

fn start_timer(app: &Rc<AppState>) {
    let app = app.clone();
    TIMER.with(|t| {
        if t.running() {
            return;
        }
        t.start(slint::TimerMode::Repeated, TICK, move || tick(&app));
    });
}

fn tick(app: &Rc<AppState>) {
    let runs: Vec<Rc<Run>> = RUNS.with(|r| r.borrow().values().cloned().collect());
    let mut tiles_changed = false;
    for run in &runs {
        let (done, total) = run.meter.bytes();
        run.speed.borrow_mut().add(run.started.elapsed().as_secs_f64(), done);
        let open = run.handle.borrow().as_ref().is_some_and(PageHandle::is_open);
        if open && !run.background.get() {
            redraw(run);
        } else if !run.background.get() {
            // Something else took the screen: the install goes on, as if sent to the background.
            run.background.set(true);
        }
        tiles_changed |= run.background.get() && *run.tile.borrow() != wiz::tile_note(run.meter.phase(), done, total);
    }
    // Home is redrawn at most once a second for its tiles (a redraw reads a few things from disk, cached). A tile's line
    // counts as shown only once Home was redrawn with it.
    let due = HOME_DRAWN.with(|d| d.get().is_none_or(|at| at.elapsed() >= HOME_EVERY));
    if tiles_changed && due && app.home.is_active() && !dialog_is_open(app) {
        HOME_DRAWN.with(|d| d.set(Some(Instant::now())));
        app.refresh_current_view();
        for run in &runs {
            let (done, total) = run.meter.bytes();
            *run.tile.borrow_mut() = wiz::tile_note(run.meter.phase(), done, total);
        }
    }
    // A background install's note, once nothing else is on screen.
    if !dialog_is_open(app) {
        let next = WAITING.with(|w| {
            let mut w = w.borrow_mut();
            (!w.is_empty()).then(|| w.remove(0))
        });
        if let (Some(next), Some(router)) = (next, ROUTER.with(|r| r.borrow().clone())) {
            match next {
                Waiting::Finished(key) => {
                    if let Some(flow) = flow_of(&key) {
                        show_finished(app, &router, &flow);
                    }
                }
                Waiting::Failed(key, message) => show_failed(app, &router, &key, None, &message),
            }
        }
    }
    if runs.is_empty() && WAITING.with(|w| w.borrow().is_empty()) {
        TIMER.with(slint::Timer::stop);
    }
}

/// Home's tile line for a port that is installing ("Downloading 42%"), `None` when it is not.
pub(crate) fn tile_note(key: &str) -> Option<String> {
    run_of(key).map(|r| {
        let (done, total) = r.meter.bytes();
        wiz::tile_note(r.meter.phase(), done, total)
    })
}

/// The ports installing now (Home shows a tile for each, installed yet or not).
pub(crate) fn installing_keys() -> Vec<String> {
    RUNS.with(|r| r.borrow().keys().cloned().collect())
}

/// A on a port that is installing (its Home tile, its row): its downloading page again. False when it is not installing.
pub(crate) fn reopen(app: &Rc<AppState>, router: &Router, key: &str) -> bool {
    let Some(run) = run_of(key) else { return false };
    if dialog_is_open(app) {
        return true;
    }
    show_downloading(app, router, &run);
    true
}

/// The install of `key` ended (whatever way): its run is gone. True when it was in the background, so the caller leaves
/// whatever is on screen alone.
pub(crate) fn end_run(key: &str) -> bool {
    let Some(run) = RUNS.with(|r| r.borrow_mut().remove(key)) else { return false };
    let background = run.background.get() || !run.handle.borrow().as_ref().is_some_and(PageHandle::is_open);
    if let Some(flow) = flow_of(key) {
        flow.was_background.set(background);
    }
    background
}

/// The install failed (its run has ended: `background` is [`end_run`]'s answer). "Stop and remove the download" has
/// nothing to report (the user asked for it; the first page's caller gets the screen back); anything else shows "Could
/// not install" with one way back, at once or, for an install in the background, once nothing else is open. `launch`:
/// Play started it as an update, and a failed update must not lock the player out of the game that is installed (not
/// from the background: the game never starts on its own while the user is elsewhere).
pub(crate) fn install_failed(app: &Rc<AppState>, router: &Router, key: &str, message: &str, background: bool, launch: bool) {
    let flow = FLOWS.with(|f| f.borrow_mut().remove(key));
    let stopped = message == CANCELLED;
    if stopped {
        eprintln!("install wizard: the download of {key} was stopped and removed");
    } else {
        eprintln!("install wizard: the install of {key} failed: {message}");
    }
    if background {
        // Home's tile still says "Downloading N%"; the tick that redraws it stops with the run.
        app.refresh_current_view();
    }
    if launch && !background {
        if let Some(port) = app.find_port(key) {
            super::install_launch::launch_installed(app, router, &port);
        }
        // launch_installed may have opened its own dialog (the program picker); the launch matters more than this page.
        if stopped || dialog_is_open(app) {
            return;
        }
    }
    match (stopped, background) {
        (true, true) => {}
        (true, false) => {
            if let Some(flow) = flow {
                (flow.back)(app, router);
            }
        }
        (false, true) if dialog_is_open(app) => {
            WAITING.with(|w| w.borrow_mut().push(Waiting::Failed(key.to_string(), message.to_string())));
            start_timer(app);
        }
        (false, true) => show_failed(app, router, key, None, message),
        (false, false) => show_failed(app, router, key, flow, message),
    }
}

/// "Could not install": the message and Back, which goes where the first page's Not now would (`flow`), or closes.
fn show_failed(app: &Rc<AppState>, router: &Router, key: &str, flow: Option<Rc<Flow>>, message: &str) {
    let port = app.find_port(key);
    let name = flow.as_ref().map(|f| f.name.clone()).or_else(|| port.as_ref().map(|p| p.display_name().to_string())).unwrap_or_else(|| key.to_string());
    let art = flow.as_ref().and_then(|f| f.art.clone()).or_else(|| port.as_ref().map(ArtRef::of_port));
    let update = flow.as_ref().map_or_else(|| port.as_ref().is_some_and(|p| app.is_installed(p)), |f| f.update);
    let back: Back = flow.map_or_else(close_back, |f| f.back.clone());
    let page = wiz::failed_page(&name, &art, update, message);
    show_page(app, router, page, handlers(move |app, router, _| back(app, router), WizAct::Back));
}

/// The release has several files that could be the one (`InstallAssetAmbiguous`): a card each. One starts the install
/// again with that file (the flow goes on); Not now drops the flow and goes back (`was_play`: Play started it, so the
/// installed game starts instead, as when its update fails).
pub(crate) fn choose_asset(app: &Rc<AppState>, router: &Router, key: &str, assets: Vec<Value>, release: Option<Value>, was_play: bool) {
    let Some(port) = app.find_port(key) else { return drop_flow(key) };
    let flow = flow_of(key);
    let list: Vec<(String, Option<u64>)> = assets.iter().map(|a| (a.get("name").and_then(Value::as_str).unwrap_or("?").to_string(), a.get("size").and_then(Value::as_u64))).collect();
    let name = flow.as_ref().map_or_else(|| port.display_name().to_string(), |f| f.name.clone());
    let page = wiz::asset_page(&name, &Some(ArtRef::of_port(&port)), &list);
    let on_action = move |app: &Rc<AppState>, router: &Router, act: WizAct| {
        let key = port.key().to_string();
        if let WizAct::Asset(i) = act {
            let Some(chosen) = assets.get(i).cloned() else { return };
            if was_play {
                app.install_runtime.pending_launch_after_install.borrow_mut().insert(key.clone());
            }
            super::install_launch::start_install(app, router, port.clone(), Some(chosen), release.clone());
            if run_of(&key).is_none() {
                drop_flow(&key);
                app.install_runtime.pending_launch_after_install.borrow_mut().remove(&key);
            }
            return;
        }
        drop_flow(&key);
        if was_play {
            close_current_dialog(app, router);
            super::install_launch::launch_installed(app, router, &port);
        } else if let Some(flow) = &flow {
            (flow.back)(app, router);
        } else {
            close_current_dialog(app, router);
        }
    };
    show_page(app, router, page, handlers(on_action, WizAct::NotNow));
}

/// Play's update (no first page, so no flow) ended in the background: instead of starting the game wherever the user
/// is now, it gets the finished note like any background install, and its pages lead to Play now.
pub(crate) fn finished_in_background(app: &Rc<AppState>, router: &Router, key: &str, note: Option<String>) {
    if flow_of(key).is_none() {
        let Some(port) = app.find_port(key) else { return };
        let flow = Flow { key: key.to_string(), name: port.display_name().to_string(), art: Some(ArtRef::of_port(&port)), update: true, back: close_back(), notes: RefCell::new(Vec::new()), was_background: Cell::new(true) };
        FLOWS.with(|f| f.borrow_mut().insert(key.to_string(), Rc::new(flow)));
    }
    after_install(app, router, key, note);
}

/// The install finished. True when it was started from the first page (its next pages follow, or its note waits).
/// `note`: what the versioned-file step had to say.
pub(crate) fn after_install(app: &Rc<AppState>, router: &Router, key: &str, note: Option<String>) -> bool {
    let Some(flow) = flow_of(key) else { return false };
    if let Some(n) = note {
        flow.notes.borrow_mut().push((Mark::None, n));
    }
    if flow.was_background.get() {
        if dialog_is_open(app) {
            WAITING.with(|w| w.borrow_mut().push(Waiting::Finished(key.to_string())));
            start_timer(app);
        } else {
            show_finished(app, router, &flow);
        }
    } else {
        kept_step(app, router, &flow);
    }
    true
}

fn show_finished(app: &Rc<AppState>, router: &Router, flow: &Rc<Flow>) {
    let page = wiz::finished_page(&flow.name, &flow.art, flow.update);
    let f = flow.clone();
    let on_action = move |app: &Rc<AppState>, router: &Router, act: WizAct| match act {
        WizAct::FinishSetup => kept_step(app, router, &f),
        _ => {
            drop_flow(&f.key);
            close_current_dialog(app, router);
        }
    };
    show_page(app, router, page, handlers(on_action, WizAct::Home));
}

fn port_of(app: &AppState, flow: &Flow) -> Option<Port> {
    app.find_port(&flow.key)
}

// -- 3. Kept files ------------------------------------------------------------------------------------------------

fn kept_step(app: &Rc<AppState>, router: &Router, flow: &Rc<Flow>) {
    let Some(port) = port_of(app, flow) else { return finish_quietly(app, router, flow) };
    let Ok(port_dir) = crate::core::path_safety::safe_join(&app.paths.library_dir(), &port.folder) else { return game_files_step(app, router, flow) };
    let backups = app.paths.saves_backup_dir.clone();
    let Some(set) = uninstall_keep::newest_kept(&backups, &port.folder) else { return game_files_step(app, router, flow) };
    let plan = uninstall_keep::restore_plan(&set, &port_dir);
    if plan.steps.is_empty() {
        return game_files_step(app, router, flow);
    }
    let places = Places::system();
    // The time is fixed now, so the page names the very folder a replaced file goes to.
    let stamp = crate::fork::store::backup_stamp();
    let replaced = uninstall_keep::replaced_dir_for(&backups, &port.folder);
    let offer = KeptOffer {
        when: set.when.clone(),
        files: plan.steps.iter().map(|s| (s.rel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), uninstall_keep::kind_words(&s.kind).to_string())).collect(),
        replaces: plan.steps.iter().filter(|s| s.replaces).count(),
        already: plan.already,
        kept_place: human_path(&uninstall_keep::kept_root(&backups), &places),
        replaced_place: human_path(&replaced.join(&stamp), &places),
    };
    let page = wiz::kept_page(&flow.name, &flow.art, &offer);
    let (f, set, steps) = (flow.clone(), Rc::new(set), Rc::new(plan.steps));
    let on_action = move |app: &Rc<AppState>, router: &Router, act: WizAct| match act {
        WizAct::PutBack => put_back(app, router, &f, set.clone(), steps.clone(), port_dir.clone(), replaced.clone(), stamp.clone()),
        _ => game_files_step(app, router, &f),
    };
    show_page(app, router, page, handlers(on_action, WizAct::KeepLater));
}

#[allow(clippy::too_many_arguments)]
fn put_back(app: &Rc<AppState>, router: &Router, flow: &Rc<Flow>, set: Rc<uninstall_keep::KeptSet>, plan: Rc<Vec<uninstall_keep::RestoreStep>>, port_dir: PathBuf, replaced: PathBuf, stamp: String) {
    let page = wiz::working_page(&flow.name, &flow.art, "Putting your files back", "Copying them from Kept files. Big files take a while.");
    show_page(app, router, page, handlers(|_, _, _| {}, WizAct::KeepLater));
    let (s, p) = ((*set).clone(), (*plan).clone());
    let job = Background::spawn(move |_| uninstall_keep::restore_kept(&s, &p, &port_dir, &replaced, &stamp));
    let (app, router, flow) = (app.clone(), router.clone(), flow.clone());
    when_done(move || {
        let Some(result) = job.take() else { return false };
        let note = match result.and_then(|r| r) {
            Ok(done) => {
                eprintln!("install wizard: put back {} kept file(s), {} copied aside first", done.copied, done.backed_up);
                let mut line = format!("Put back {} kept file{}", done.copied, if done.copied == 1 { "" } else { "s" });
                if let Some(dir) = &done.backup_dir {
                    line.push_str(&format!("; {} already there went to {} first", done.backed_up, human_path(dir, &Places::system())));
                }
                (Mark::Done, line)
            }
            Err(e) => {
                eprintln!("install wizard: putting kept files back stopped: {e}");
                (Mark::Problem, format!("Kept files: {e}"))
            }
        };
        flow.notes.borrow_mut().push(note);
        game_files_step(&app, &router, &flow);
        true
    });
}

// -- 4. Game files ------------------------------------------------------------------------------------------------

fn game_files_step(app: &Rc<AppState>, router: &Router, flow: &Rc<Flow>) {
    let Some(port) = port_of(app, flow) else { return finish_quietly(app, router, flow) };
    if !super::rom_screens::port_missing_game_files(app, &port) {
        return steam_step(app, router, flow);
    }
    let f = flow.clone();
    super::rom_screens::find_for_port(app, router, &flow.key, Rc::new(move |app, router| steam_step(app, router, &f)));
}

// -- 5. Steam -----------------------------------------------------------------------------------------------------

// Whether to ask, the artwork fetch and the add are shared with Fix problems and the Port page
// (`steam_add_screen`); this page and its words stay the wizard's own.

fn steam_step(app: &Rc<AppState>, router: &Router, flow: &Rc<Flow>) {
    let Some(port) = port_of(app, flow) else { return finish_quietly(app, router, flow) };
    // No account to add it to (Steam never ran here, or several and none chosen), already in Steam, waiting: the Done
    // page says so.
    let (running, game_mode) = match super::steam_add_screen::ask(app, std::slice::from_ref(&port)) {
        Ask::Skip(note) => {
            flow.notes.borrow_mut().extend(note);
            return done_step(app, router, flow);
        }
        Ask::Ask { running, game_mode } => (running, game_mode),
    };
    let page = wiz::steam_page(&flow.name, &flow.art, game_mode, running);
    let f = flow.clone();
    let on_action = move |app: &Rc<AppState>, router: &Router, act: WizAct| match act {
        WizAct::SteamYes => add_to_steam(app, router, &f, &port),
        _ => {
            f.notes.borrow_mut().push((Mark::Skipped, "Not in Steam: its Port page can add it".to_string()));
            done_step(app, router, &f);
        }
    };
    show_page(app, router, page, handlers(on_action, WizAct::SteamNo));
}

/// Yes: the shared one-port add (`steam_add_screen::add`): the cover and icon are fetched into the cache first, then
/// written now with Steam closed (`shortcuts.vdf` backed up first), else queued and, outside Game Mode, the Close Steam
/// flow (it asks first). Its notes go on the Done page.
fn add_to_steam(app: &Rc<AppState>, router: &Router, flow: &Rc<Flow>, port: &Port) {
    let working = wiz::working_page(&flow.name, &flow.art, "Getting the artwork", "Fetching its cover and icon for Steam.");
    let f = flow.clone();
    let then: super::steam_add_screen::Then = Rc::new(move |app, router, notes| {
        f.notes.borrow_mut().extend(notes);
        done_step(app, router, &f);
    });
    super::steam_add_screen::add(app, router, vec![port.clone()], working, then);
}

// -- 6. Done ------------------------------------------------------------------------------------------------------

fn done_step(app: &Rc<AppState>, router: &Router, flow: &Rc<Flow>) {
    let Some(port) = port_of(app, flow) else { return finish_quietly(app, router, flow) };
    let ready = !super::rom_screens::port_missing_game_files(app, &port);
    let page = wiz::done_page(&flow.name, &flow.art, ready, &flow.notes.borrow());
    let f = flow.clone();
    let on_action = move |app: &Rc<AppState>, router: &Router, act: WizAct| {
        if act == WizAct::AddGameFiles {
            // The game files pages, then this page again (ready now, or still not).
            let again = f.clone();
            return super::rom_screens::find_for_port(app, router, &f.key, Rc::new(move |app, router| done_step(app, router, &again)));
        }
        drop_flow(&f.key);
        close_current_dialog(app, router);
        match act {
            WizAct::PlayNow => super::install_launch::launch_installed(app, router, &port),
            _ => super::home_screen::show_list(app, router, List::Home),
        }
    };
    show_page(app, router, page, handlers(on_action, WizAct::Home));
}

/// The port went away mid-flow (removed from the catalog): the pages end.
fn finish_quietly(app: &Rc<AppState>, router: &Router, flow: &Rc<Flow>) {
    drop_flow(&flow.key);
    close_current_dialog(app, router);
}
