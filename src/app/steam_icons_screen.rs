//! "Steam icons" on screen: the pages of `crate::fork::steam_icons` in the guided-page window. Opened from the
//! "Fix Steam icons..." row of Settings > Add to Steam.
//!
//! It looks at every Steam shortcut of the installed ports, says which icons show nothing and why, and on "Fix"
//! repairs only those icons. With Steam running the repairs go to the Steam queue and the shared "Close Steam and
//! update shortcuts" flow follows (in Game Mode that flow says to finish in Desktop Mode).

use super::dialogs::close_current_dialog;
use super::page_dialog::{show_page_handle, Handlers, PageHandle};
use super::state::AppState;
use super::steam_close_screen;
use crate::core::models::Port;
use crate::fork::settings::ForkSettings;
use crate::fork::steam_icons::pages::{self, IconAction, IconPage};
use crate::fork::steam_icons::{self, ApplyReport, IconFinding, Plan, PlanInputs, Replacement};
use crate::fork::steam_queue;
use crate::fork::store::{SteamProcessCheck, SystemSteamCheck};
use crate::ui::gamepad_router::GamepadRouter;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

type Router = Rc<RefCell<GamepadRouter>>;
/// What runs when the user leaves (the screen that opened this one draws itself again).
pub(crate) type Then = Rc<dyn Fn(&Rc<AppState>, &Router)>;

thread_local! {
    static FETCH_TIMER: slint::Timer = slint::Timer::default();
}

struct Run {
    ports: Vec<Port>,
    home: PathBuf,
    then: Then,
    findings: RefCell<Vec<IconFinding>>,
    plan: RefCell<Plan>,
    /// The user left while icons were being fetched.
    cancelled: Cell<bool>,
}

fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"].iter().find_map(|v| std::env::var_os(v)).map(PathBuf::from).filter(|p| p.is_absolute())
}

fn settings(app: &AppState) -> ForkSettings {
    ForkSettings::load(&super::steam_screens::settings_path(app))
}

pub(crate) fn open(app: &Rc<AppState>, router: &Router, then: Then) {
    let Some(home) = home_dir() else {
        show(app, router, None, pages::problem_page("Could not find your home folder, so there is nowhere to look."), then);
        return;
    };
    let ports: Vec<Port> = app.catalog.borrow().iter().filter(|p| !p.is_android_only() && app.is_installed(p)).cloned().collect();
    let run = Rc::new(Run { ports, home, then, findings: RefCell::new(Vec::new()), plan: RefCell::new(Plan::default()), cancelled: Cell::new(false) });
    scan(app, router, &run);
}

/// Looks at the shortcuts; fetches catalog icons first when a shortcut would need one that is not downloaded yet.
fn scan(app: &Rc<AppState>, router: &Router, run: &Rc<Run>) {
    let library = app.paths.library_dir();
    let findings = match steam_icons::diagnose(&run.home, &library, &app.paths.config_dir, &run.ports) {
        Ok(f) => f,
        Err(e) => {
            show(app, router, Some(run), pages::problem_page(&e), run.then.clone());
            return;
        }
    };
    let make_plan = |findings: &[IconFinding]| steam_icons::plan(&PlanInputs { config_dir: &app.paths.config_dir, cache_dir: &app.paths.cache_dir, library_dir: &library }, findings, &run.ports);
    let plan = make_plan(&findings);
    let wanted = to_fetch(app, run, &plan);
    *run.findings.borrow_mut() = findings.clone();
    *run.plan.borrow_mut() = plan;
    if wanted.is_empty() {
        return show_overview(app, router, run);
    }
    // Fetching is network work: off the UI thread, with a page that can be cancelled and a timer that waits for it.
    eprintln!("steam icons: fetching {} catalog icon(s)", wanted.len());
    let done = Arc::new(AtomicBool::new(false));
    {
        let (done, cache) = (done.clone(), app.paths.cache_dir.clone());
        std::thread::spawn(move || {
            for (folder, url) in wanted {
                crate::core::image_cache::cache_image(&url, &cache, &format!("{folder}_icon"));
            }
            done.store(true, Ordering::SeqCst);
        });
    }
    let handle = show(app, router, Some(run), pages::fetching_page(), run.then.clone());
    let (app2, router2, run2) = (app.clone(), router.clone(), run.clone());
    FETCH_TIMER.with(|t| {
        t.start(slint::TimerMode::Repeated, Duration::from_millis(250), move || {
            if run2.cancelled.get() || handle.as_ref().is_some_and(|h| !h.is_open()) {
                FETCH_TIMER.with(|t| t.stop());
                return;
            }
            if !done.load(Ordering::SeqCst) {
                return;
            }
            FETCH_TIMER.with(|t| t.stop());
            let library = app2.paths.library_dir();
            let findings = run2.findings.borrow().clone();
            let plan = steam_icons::plan(&PlanInputs { config_dir: &app2.paths.config_dir, cache_dir: &app2.paths.cache_dir, library_dir: &library }, &findings, &run2.ports);
            *run2.plan.borrow_mut() = plan;
            show_overview(&app2, &router2, &run2);
        });
    });
}

/// (folder, icon url) of the ports whose shortcuts could use the catalog icon if it were downloaded.
fn to_fetch(app: &AppState, run: &Run, plan: &Plan) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for item in &plan.items {
        if !matches!(item.replacement, Replacement::Nothing(_)) || item.finding.app_id.is_none() {
            continue;
        }
        let folder = &item.finding.port_folder;
        let cached = crate::core::image_cache::cached_image_path(&app.paths.cache_dir, &format!("{folder}_icon")).is_ok_and(|p| p.is_file());
        let url = run.ports.iter().find(|p| &p.folder == folder).and_then(|p| p.icon.clone());
        if let (false, Some(url)) = (cached, url) {
            if !out.iter().any(|(f, _)| f == folder) {
                out.push((folder.clone(), url));
            }
        }
    }
    out
}

fn steam_running() -> bool {
    SystemSteamCheck::new().running_steam_processes().map(|p| !p.is_empty()).unwrap_or(true)
}

fn show_overview(app: &Rc<AppState>, router: &Router, run: &Rc<Run>) {
    let page = pages::overview_page(&run.findings.borrow(), &run.plan.borrow(), steam_running(), steam_queue::game_mode_from_env());
    show(app, router, Some(run), page, run.then.clone());
}

fn leave(app: &Rc<AppState>, router: &Router, run: Option<&Rc<Run>>, then: &Then) {
    FETCH_TIMER.with(|t| t.stop());
    if let Some(run) = run {
        run.cancelled.set(true);
    }
    close_current_dialog(app, router);
    then(app, router);
}

fn show(app: &Rc<AppState>, router: &Router, run: Option<&Rc<Run>>, page: IconPage, then: Then) -> Option<PageHandle<IconAction>> {
    let run_for_handlers = run.cloned();
    let on_action: Rc<dyn Fn(&Rc<AppState>, &Router, IconAction)> = {
        let (run, then) = (run_for_handlers.clone(), then.clone());
        Rc::new(move |app, router, action| match action {
            IconAction::Fix => match &run {
                Some(run) => fix(app, router, run),
                None => leave(app, router, None, &then),
            },
            IconAction::NotNow | IconAction::Done => leave(app, router, run.as_ref(), &then),
        })
    };
    let on_back: Rc<dyn Fn(&Rc<AppState>, &Router)> = {
        let (run, then) = (run_for_handlers, then);
        Rc::new(move |app, router| leave(app, router, run.as_ref(), &then))
    };
    let handle = show_page_handle(app, router, page, Handlers { on_action, on_back: on_back.clone(), on_exit: on_back, on_input: None });
    if handle.is_none() {
        eprintln!("steam icons: the page window could not be made");
    }
    handle
}

/// Places the catalog icons, then writes the repairs (Steam closed) or queues them (Steam running, Game Mode).
fn fix(app: &Rc<AppState>, router: &Router, run: &Rc<Run>) {
    let resolved = steam_icons::resolve(&run.plan.borrow());
    let game_mode = steam_queue::game_mode_from_env();
    let mut report = steam_icons::apply(&resolved.fixes, &run.home, &SystemSteamCheck::new(), &settings(app), game_mode, SystemTime::now());
    if !report.queued.is_empty() {
        let entries = steam_icons::queue_entries(&report.queued, game_mode);
        match steam_queue::queue_changes(&steam_queue::queue_path(&app.paths.config_dir), entries, game_mode) {
            Ok(_) => eprintln!("steam icons: queued {} icon repair(s)", report.queued.len()),
            Err(e) => {
                report.failed.push(e.lines().next().unwrap_or("could not queue the icon repairs").to_string());
                report.queued.clear();
            }
        }
    }
    eprintln!("steam icons: wrote {}, queued {}, left alone {}, failed {}", report.written.len(), report.queued.len(), report.skipped.len() + resolved.left.len(), report.failed.len());
    if report.queued.is_empty() {
        return show_result(app, router, run, report, resolved);
    }
    // Steam has to be closed to change its list: the shared flow asks, closes Steam and applies the queue.
    let lead = format!("Queued {} icon repair{}.", report.queued.len(), if report.queued.len() == 1 { "" } else { "s" });
    let run2 = run.clone();
    let then: steam_close_screen::Then = Rc::new(move |app, router, finished| {
        let mut report = report.clone();
        if finished.waiting == 0 {
            // The queue is empty, but an entry can be dropped as stale: read the file to see what was really written.
            let queued: Vec<_> = report.queued.drain(..).collect();
            let (done, not) = steam_icons::verify_applied(&queued, &run2.home);
            report.written.extend(done.into_iter().map(|f| (f.name, f.app_id, f.from, f.to)));
            report.skipped.extend(not.into_iter().map(|(f, why)| (f.name, why)));
        }
        show_result(app, router, &run2, report, resolved.clone());
    });
    steam_close_screen::open(app, router, steam_close_screen::Opts { ask_first: true, lead: Some(lead), ..Default::default() }, then);
}

fn show_result(app: &Rc<AppState>, router: &Router, run: &Rc<Run>, report: ApplyReport, resolved: steam_icons::Resolved) {
    show(app, router, Some(run), pages::result_page(&report, &resolved), run.then.clone());
}
