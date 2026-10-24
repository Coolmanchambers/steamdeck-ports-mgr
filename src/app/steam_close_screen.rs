//! "Close Steam and update shortcuts" on screen: the pages of `crate::fork::steam_close` in the guided-page
//! window, and the half-second timer that watches Steam close. One flow for every place that applies queued
//! Steam changes: the import's finished page, the setup wizard's Steam step, Settings > Add to Steam, and the
//! queue offer that comes up by itself.
//!
//! [`open`] runs it; when the user leaves (whichever way: done, not now, B, the X) the page closes and the
//! caller's `then` runs with what was applied and what still waits, so the caller can draw its own page again.

use super::dialogs::close_current_dialog;
use super::page_dialog::{show_page_handle, Handlers, PageHandle};
use super::state::AppState;
use crate::fork::settings::ForkSettings;
use crate::fork::steam_close::{self, back_action, Blocked, CloseAction, ClosePage, Closer, Poll, Start, SteamClosed, SteamControl, SystemSteam};
use crate::fork::steam_queue::{self, ApplyInputs};
use crate::fork::store::{SteamProcessCheck, SystemSteamCheck};
use crate::ui::gamepad_router::GamepadRouter;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Instant, SystemTime};

type Router = Rc<RefCell<GamepadRouter>>;

/// What the flow did, for the caller.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Finished {
    /// Queued changes made (written, or found already in Steam).
    pub applied: usize,
    /// Changes still waiting when the flow ended.
    pub waiting: usize,
}

pub(crate) type Then = Rc<dyn Fn(&Rc<AppState>, &Router, Finished)>;

/// How the flow starts.
#[derive(Clone, Default)]
pub(crate) struct Opts {
    /// Ask "Update your Steam shortcuts?" first. Off when the page that sent the user here already had the
    /// "Close Steam and update shortcuts" choice on it.
    pub ask_first: bool,
    /// A line put before the question ("Queued 2 changes.").
    pub lead: Option<String>,
    /// The question came up by itself: the cursor starts on Not now.
    pub stray_safe: bool,
    /// Drawn as the setup wizard's Steam step.
    pub wizard: bool,
}

struct Run {
    opts: Opts,
    then: Then,
    started: Instant,
    applied: Cell<usize>,
    /// This flow asked Steam to close (so it offers to start it again).
    closed_by_us: Cell<bool>,
    /// The page on screen, so B and the X can do what it says.
    current: RefCell<ClosePage>,
    /// The text behind Details on the page on screen.
    details: RefCell<String>,
    /// The page Details goes back to.
    before_details: RefCell<Option<ClosePage>>,
}

thread_local! {
    static TIMER: slint::Timer = slint::Timer::default();
}

fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"].iter().find_map(|v| std::env::var_os(v)).map(PathBuf::from).filter(|p| p.is_absolute())
}

fn queue_path(app: &AppState) -> PathBuf {
    steam_queue::queue_path(&app.paths.config_dir)
}

fn waiting_now(app: &AppState) -> usize {
    steam_queue::load(&queue_path(app)).queue.len()
}

/// Runs the flow. Replaces whatever dialog is open.
pub(crate) fn open(app: &Rc<AppState>, router: &Router, opts: Opts, then: Then) {
    TIMER.with(|t| t.stop());
    // Covers for any adds in the queue are fetched while the pages are up.
    super::steam_screens::prefetch_art(app);
    let run = Rc::new(Run {
        opts,
        then,
        started: Instant::now(),
        applied: Cell::new(0),
        closed_by_us: Cell::new(false),
        current: RefCell::new(ClosePage::default()),
        details: RefCell::new(String::new()),
        before_details: RefCell::new(None),
    });
    let loaded = steam_queue::load(&queue_path(app));
    let game_mode = steam_queue::game_mode_from_env();
    if game_mode {
        return show(app, router, &run, steam_close::game_mode_page(loaded.queue.len()));
    }
    if run.opts.ask_first && loaded.problem.is_none() && !loaded.queue.is_empty() {
        let page = steam_close::ask_page(loaded.queue.len(), run.opts.lead.as_deref(), run.opts.stray_safe);
        return show(app, router, &run, page);
    }
    start_close(app, router, &run);
}

fn finish(app: &Rc<AppState>, router: &Router, run: &Rc<Run>) {
    TIMER.with(|t| t.stop());
    close_current_dialog(app, router);
    let done = Finished { applied: run.applied.get(), waiting: waiting_now(app) };
    (run.then)(app, router, done);
}

fn decorate(run: &Run, page: ClosePage) -> ClosePage {
    if run.opts.wizard { page.in_wizard(crate::fork::setup_wizard::flow::Step::Steam.number(), crate::fork::setup_wizard::flow::Step::COUNTED) } else { page }
}

fn handlers(run: &Rc<Run>) -> Handlers<CloseAction> {
    let on_action: Rc<dyn Fn(&Rc<AppState>, &Router, CloseAction)> = {
        let run = run.clone();
        Rc::new(move |app, router, action| act(app, router, &run, action))
    };
    // B and the X both take the page's safe way out.
    let on_back: Rc<dyn Fn(&Rc<AppState>, &Router)> = {
        let run = run.clone();
        Rc::new(move |app, router| {
            let action = back_action(&run.current.borrow());
            act(app, router, &run, action);
        })
    };
    Handlers { on_action, on_back: on_back.clone(), on_exit: on_back, on_input: None }
}

fn show(app: &Rc<AppState>, router: &Router, run: &Rc<Run>, page: ClosePage) {
    show_handle(app, router, run, page);
}

fn show_handle(app: &Rc<AppState>, router: &Router, run: &Rc<Run>, page: ClosePage) -> Option<PageHandle<CloseAction>> {
    *run.current.borrow_mut() = page.clone();
    let handle = show_page_handle(app, router, decorate(run, page), handlers(run));
    if handle.is_none() {
        // The window could not be made: nothing was changed, the caller goes on.
        finish(app, router, run);
    }
    handle
}

fn act(app: &Rc<AppState>, router: &Router, run: &Rc<Run>, action: CloseAction) {
    match action {
        CloseAction::CloseSteam | CloseAction::TryAgain => start_close(app, router, run),
        CloseAction::NotNow | CloseAction::Done | CloseAction::StopWaiting => finish(app, router, run),
        CloseAction::StartSteam => match SystemSteam::new().start() {
            Ok(()) => {
                eprintln!("steam close: Steam started again");
                finish(app, router, run)
            }
            Err(e) => show(app, router, run, steam_close::start_failed_page(&e)),
        },
        CloseAction::Details => {
            *run.before_details.borrow_mut() = Some(run.current.borrow().clone());
            let text = run.details.borrow().clone();
            show(app, router, run, steam_close::details_page(&text));
        }
        CloseAction::Back => {
            let before = run.before_details.borrow_mut().take();
            match before {
                Some(page) => show(app, router, run, page),
                None => finish(app, router, run),
            }
        }
    }
}

/// Looks at Steam and either applies now, starts waiting for it to close, or says why it can't.
fn start_close(app: &Rc<AppState>, router: &Router, run: &Rc<Run>) {
    TIMER.with(|t| t.stop());
    let loaded = steam_queue::load(&queue_path(app));
    let waiting = loaded.queue.len();
    if let Some(problem) = loaded.problem.filter(|_| loaded.unreadable || waiting == 0) {
        return show(app, router, run, steam_close::blocked_page(&Blocked::QueueProblem(problem), 0));
    }
    let steam = SystemSteam::new();
    let by_steam = steam_close::started_by_steam(Path::new("/proc"), std::process::id());
    eprintln!("steam close: {waiting} waiting, steam command {:?}, started by Steam: {by_steam}", steam.command.as_ref().map(|c| c.program.display().to_string()));
    match steam_close::begin(waiting, steam_queue::game_mode_from_env(), by_steam, &steam, run.started.elapsed()) {
        Start::ApplyNow(closed) => apply_now(app, router, run, closed),
        Start::Blocked(blocked) => {
            eprintln!("steam close: not closing Steam: {blocked:?}");
            show(app, router, run, steam_close::blocked_page(&blocked, waiting));
        }
        Start::Closing(closer) => {
            eprintln!("steam close: asked Steam to shut down");
            run.closed_by_us.set(true);
            let Some(handle) = show_handle(app, router, run, steam_close::closing_page(0)) else { return };
            watch(app, router, run, closer, handle);
        }
    }
}

fn watch(app: &Rc<AppState>, router: &Router, run: &Rc<Run>, closer: Closer, handle: PageHandle<CloseAction>) {
    let (app, router, run) = (app.clone(), router.clone(), run.clone());
    let closer = RefCell::new(closer);
    let shown = Cell::new(0u64);
    TIMER.with(|t| {
        t.start(slint::TimerMode::Repeated, steam_close::POLL, move || {
            // The page was replaced (Stop waiting, B, another dialog): stop looking.
            if !handle.is_open() {
                return TIMER.with(|t| t.stop());
            }
            let poll = closer.borrow_mut().poll(SystemSteamCheck::new().running_steam_processes(), run.started.elapsed());
            match poll {
                Poll::Waiting { seconds } => {
                    if seconds != shown.get() {
                        shown.set(seconds);
                        let page = steam_close::closing_page(seconds);
                        *run.current.borrow_mut() = page.clone();
                        handle.update(decorate(&run, page));
                    }
                }
                Poll::Closed(closed) => {
                    TIMER.with(|t| t.stop());
                    eprintln!("steam close: Steam is closed");
                    apply_now(&app, &router, &run, closed);
                }
                Poll::TimedOut { still } => {
                    TIMER.with(|t| t.stop());
                    eprintln!("steam close: timed out, still running: {}", still.join(", "));
                    *run.details.borrow_mut() = format!("Still running after {} seconds: {}", steam_close::TIMEOUT.as_secs(), still.join(", "));
                    show(&app, &router, &run, steam_close::timed_out_page(waiting_now(&app)));
                }
                Poll::CheckFailed(e) => {
                    TIMER.with(|t| t.stop());
                    show(&app, &router, &run, steam_close::blocked_page(&Blocked::CheckFailed(e), waiting_now(&app)));
                }
            }
        });
    });
}

fn apply_now(app: &Rc<AppState>, router: &Router, run: &Rc<Run>, closed: SteamClosed) {
    let Some(home) = home_dir() else {
        return show(app, router, run, steam_close::blocked_page(&Blocked::CheckFailed("no home folder".into()), waiting_now(app)));
    };
    let settings = ForkSettings::load(&super::steam_screens::settings_path(app));
    let outcome = {
        let catalog = app.catalog.borrow();
        let state = app.state.borrow();
        let favorite = |key: &str| state.get(key).and_then(|i| i.favorite_exe.clone());
        let inputs = ApplyInputs {
            home: &home,
            library_dir: &app.paths.library_dir(),
            catalog: &catalog,
            favorite_exe: &favorite,
            cache_dir: Some(app.paths.cache_dir.clone()),
            settings: &settings,
            game_mode: steam_queue::game_mode_from_env(),
        };
        steam_close::apply(closed, &queue_path(app), &inputs, &SystemSteamCheck::new(), SystemTime::now())
    };
    eprintln!("steam close: applied {} (dropped {}, still waiting {}, refused {})", outcome.done, outcome.dropped, outcome.remaining.len(), outcome.refused);
    run.applied.set(run.applied.get() + outcome.done);
    *run.details.borrow_mut() = outcome.text.clone();
    show(app, router, run, steam_close::result_page(&outcome, run.closed_by_us.get()));
}
