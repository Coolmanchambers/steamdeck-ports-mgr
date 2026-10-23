//! "Import ports..." (Settings menu, and step c of the setup wizard): finds ports installed outside the
//! ports folder and moves them in. It replaces "Adopt installed ports".
//!
//! The user sees cards, not lists: a page of what was found, one card per port to confirm, and one page to
//! start the move (`import_cards_screen`, drawn by `page_dialog`; what they say is `crate::fork::import::cards`).
//! This file runs the rest: the scan and the import on a worker thread behind a progress list with a Cancel
//! row, the plan list behind Details, the scan folders list behind Details of the found page, the game
//! file question while the worker waits, and one page about all the old copies afterwards
//! (`ports_folder_screen::prompt_old_copies_batch_then`, Keep them preselected). The import itself is the
//! port mover (`crate::fork::port_mover`) through `crate::fork::import::real_engine`. The text is English
//! only; it is not part of upstream's translation tables.
//!
//! Entry point for the setup wizard: `open(app, router, from_wizard)`, or `open_with(app, router, true,
//! Some(finish))`. `finish` runs exactly once, whichever way the user leaves: "Continue setup", the end
//! of an import (after the old-copy page and the game file search), B on the found page, B on the
//! scan's progress list, or a notice. Quitting the app is the one exit that does not call it.
//! Back inside the flow (B on a card, on the move page, on the scan folders or plan list) returns to the
//! previous page, not out of the flow.

use super::dialogs::{
    build_picker_dialog, close_current_dialog, dialog_is_open, finish_dialog_open, open_confirm_dialog, open_message_dialog, set_picker_items, DialogSlot,
};
use super::folder_picker_screen::open_folder_picker;
use super::ports_folder_screen::{blocked_reason, prompt_old_copies_batch_then};
use super::state::AppState;
use crate::fork::folder_picker::{PickerRequest, Purpose};
use crate::fork::import::engine::{move_selected, GameFileChoice, ImportSummary, Prompter};
use crate::fork::import::fallback;
use crate::fork::import::folders::{AddOutcome, ScanFolders};
use crate::fork::import::plan::{build_requests, plan_all_with, steam_write_mode, MoveRequest, PlanGuards, PortPlan};
use crate::fork::import::{ports_folder, Once};
use crate::fork::import::real_engine::engine_for_import;
use crate::fork::import::cards::{Cards};
use crate::fork::import::scan::{read_shortcuts, run_scan, Choices, ScanInput};
use crate::fork::import::screen::{blocked_text, plan_labels, plan_text, space_blocked, FolderLine, FoldersScreen, ImportScreen};
use crate::fork::import::targets;
use crate::fork::port_mover::game_files::Issue;
use crate::fork::port_mover::old_copies;
use crate::fork::ports_folder_view::{game_file_issues, Act};
use crate::fork::rom_screens::job::{Background, Ctl};
use crate::fork::rom_screens::util::{ellipsize, free_bytes};
use crate::fork::screens::short_path;
use crate::fork::settings::{ForkSettings, FILE_NAME};
use crate::fork::steam_queue;
use crate::fork::store::SystemSteamCheck;
use crate::ui::gamepad_router::GamepadRouter;
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Router = Rc<RefCell<GamepadRouter>>;
/// Called when the wizard's step is over.
pub(crate) type Finish = Rc<dyn Fn(&Rc<AppState>, &Router)>;
pub(super) type Step = Rc<dyn Fn(&Rc<AppState>, &Router)>;

pub(super) const TITLE: &str = "Import ports";

/// What opened the screen: the Settings menu, or the setup wizard (which wants to be told when to go on).
#[derive(Clone)]
pub(super) struct Ctx {
    pub(super) from_wizard: bool,
    finish: Option<Finish>,
    /// Makes `finish` run once, whichever exit is taken.
    finished: Once,
    /// What the user said about folders (which port, or not a port), kept while the screen is open.
    pub(super) choices: Rc<RefCell<Choices>>,
}

impl Ctx {
    pub(super) fn new(from_wizard: bool, finish: Option<Finish>) -> Ctx {
        Ctx { from_wizard, finish, finished: Once::default(), choices: Rc::default() }
    }

    /// Whether the wizard is waiting for this screen.
    pub(super) fn waited_on(&self) -> bool {
        self.from_wizard || self.finish.is_some()
    }

    /// Runs the wizard's `finish` once, whatever dialog is on screen now being closed first.
    fn finish(&self, app: &Rc<AppState>, router: &Router) {
        close_current_dialog(app, router);
        self.finish_quiet(app, router);
    }

    /// As `finish`, for when the dialog on screen is already gone (or must stay).
    pub(super) fn finish_quiet(&self, app: &Rc<AppState>, router: &Router) {
        self.finished.run(|| {
            if let Some(finish) = &self.finish {
                finish(app, router);
            }
        });
    }

    pub(super) fn finisher(&self) -> Step {
        let ctx = self.clone();
        Rc::new(move |app, router| ctx.finish_quiet(app, router))
    }
}

/// The dialogs size themselves from a text estimate that comes out a line short for wrapped text, which
/// clips the last line; one spare line keeps it readable.
fn padded(text: &str) -> String {
    format!("{text}\n")
}

pub(super) fn message(app: &Rc<AppState>, router: &Router, title: &str, text: &str) {
    open_message_dialog(app, router, title, &padded(text));
}

/// A confirm dialog whose Cancel (and B) runs `on_cancel` instead of only closing, optionally starting on
/// Cancel so a stray press does not do the work.
#[allow(clippy::too_many_arguments)]
pub(super) fn confirm_back(app: &Rc<AppState>, router: &Router, title: &str, text: &str, label: &str, preselect_cancel: bool, on_confirm: impl Fn(&Rc<AppState>, &Router) + 'static, on_cancel: Step) {
    open_confirm_dialog(app, router, title.into(), padded(text).into(), label.into(), on_confirm);
    if let DialogSlot::Confirm(d) = &*app.dialog_nav.dialogs.borrow() {
        if preselect_cancel {
            d.set_selected_index(1);
            app.dialog_nav.confirm_nav_index.set(1);
        }
        let (app, router) = (app.clone(), router.clone());
        let (b_app, b_router, b_cancel) = (app.clone(), router.clone(), on_cancel.clone());
        d.on_close_requested(move || {
            close_current_dialog(&b_app, &b_router);
            b_cancel(&b_app, &b_router);
        });
        // The title bar's X only hides the window; it goes where B goes.
        d.window().on_close_requested(move || {
            close_current_dialog(&app, &router);
            on_cancel(&app, &router);
            slint::CloseRequestResponse::HideWindow
        });
    }
}

/// A notice inside the flow: OK (or B) goes to `then`.
pub(super) fn notice_back(app: &Rc<AppState>, router: &Router, text: &str, then: Step) {
    let on_ok = then.clone();
    confirm_back(app, router, TITLE, text, "OK", false, move |app, router| on_ok(app, router), then);
}

/// A notice that ends the flow: a message, or for the wizard a Continue dialog (B too) that finishes it.
pub(super) fn end_notice(app: &Rc<AppState>, router: &Router, ctx: &Ctx, title: &str, text: &str) {
    if !ctx.waited_on() {
        return message(app, router, title, text);
    }
    let ctx2 = ctx.clone();
    confirm_back(app, router, title, text, "Continue setup", false, move |app, router| ctx2.finish_quiet(app, router), ctx.finisher());
}

pub(super) fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"].iter().find_map(|v| std::env::var_os(v)).map(PathBuf::from).filter(|p| p.is_absolute())
}

pub(super) fn settings_path(app: &AppState) -> PathBuf {
    app.paths.config_dir.join(FILE_NAME)
}

// -- Entry points ----------------------------------------------------------------------------------

/// Settings > Import ports... (`from_wizard` false), or the wizard's import step (true).
pub(crate) fn open(app: &Rc<AppState>, router: &Router, from_wizard: bool) {
    open_with(app, router, from_wizard, None);
}

/// Like `open`; `finish` runs once when the user leaves the screen, whichever way (see the module docs).
pub(crate) fn open_with(app: &Rc<AppState>, router: &Router, from_wizard: bool, finish: Option<Finish>) {
    // The wizard has its own dialog up when it calls this; the Settings entry must not open on top of one.
    if !from_wizard && dialog_is_open(app) {
        return;
    }
    let folders = ScanFolders::load(&ForkSettings::load(&settings_path(app)));
    start_scan(app, router, &Ctx::new(from_wizard, finish), folders);
}

// -- Background jobs ---------------------------------------------------------------------------------

thread_local! {
    static JOB_TIMER: slint::Timer = slint::Timer::default();
    static JOB_RUNNING: Cell<bool> = const { Cell::new(false) };
    /// Cancels the import that is running (Close and Quit ask for this first).
    static IMPORT_CANCEL: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
    static QUIT_AFTER_IMPORT: Cell<bool> = const { Cell::new(false) };
}

/// True while the scan or the import runs on its worker thread, including after B closed its progress list and the
/// worker is still stopping (removing a half copy can take seconds): its late result and questions still come.
/// The setup wizard waits for this before it takes control back.
pub(crate) fn import_running() -> bool {
    JOB_RUNNING.with(Cell::get)
}

/// Close and Quit ask this: a running import is cancelled (the copy in progress is removed, the ports
/// already moved stay moved) and the app quits when the worker has stopped.
pub(crate) fn quit_waits_for_import(app: &Rc<AppState>, router: &Router) -> bool {
    let Some(cancel) = IMPORT_CANCEL.with(|c| c.borrow().clone()) else { return false };
    QUIT_AFTER_IMPORT.with(|q| q.set(true));
    cancel();
    message(app, router, TITLE, "Stopping the import first (the copy in progress is removed; ports already moved stay moved). The app closes when that is done.");
    true
}

fn stop_job_timer() {
    JOB_TIMER.with(|t| t.stop());
    JOB_RUNNING.with(|r| r.set(false));
    IMPORT_CANCEL.with(|c| *c.borrow_mut() = None);
}

fn job_rows(text: &str, cancel_label: &str) -> Vec<String> {
    let mut rows: Vec<String> = text.lines().filter(|l| !l.is_empty()).map(|l| ellipsize(l, 80)).collect();
    rows.push(cancel_label.to_string());
    rows
}

/// A game file question from the worker, waiting for the user.
struct PromptRequest {
    port: String,
    issues: Vec<Issue>,
    reply: mpsc::Sender<GameFileChoice>,
}

/// Asks the user on the UI thread while the worker waits: the worker queues the question and blocks; the
/// job timer shows it. No answer (the app is gone) counts as Cancel.
#[derive(Clone, Default)]
struct UiPrompter {
    queue: Arc<Mutex<VecDeque<PromptRequest>>>,
}

impl Prompter for UiPrompter {
    fn game_file_problem(&self, port: &str, issues: &[Issue]) -> GameFileChoice {
        let (reply, answer) = mpsc::channel();
        self.queue.lock().unwrap_or_else(|e| e.into_inner()).push_back(PromptRequest { port: port.to_string(), issues: issues.to_vec(), reply });
        answer.recv().unwrap_or(GameFileChoice::Cancel)
    }
}

/// The progress list of a running job.
struct JobView {
    title: &'static str,
    cancel_label: &'static str,
    cancelling: Cell<bool>,
    /// The user closed the list with B: the job is cancelled.
    closed: Cell<bool>,
    /// A question is on screen instead of the list.
    prompting: Cell<bool>,
    /// What the question was about, and whether a problem's full text is on screen instead of it: if that
    /// text is closed with the title bar's X, the question comes back (it is not an answer).
    prompt_data: RefCell<Option<(Rc<String>, Rc<Vec<Issue>>)>>,
    in_detail: Cell<bool>,
    /// The list must be shown again (a question was answered).
    reshow: Cell<bool>,
    text: RefCell<String>,
    cancel_row: Cell<usize>,
    weak: RefCell<Option<slint::Weak<crate::ListPickerDialog>>>,
    /// Where the answer to the question on screen goes.
    pending_reply: RefCell<Option<mpsc::Sender<GameFileChoice>>>,
}

impl JobView {
    /// The answer to the question on screen goes to the worker; the progress list comes back.
    fn answer_prompt(&self, choice: GameFileChoice) {
        if let Some(reply) = self.pending_reply.borrow_mut().take() {
            let _ = reply.send(choice);
        }
        self.prompting.set(false);
        self.reshow.set(true);
    }
}

fn show_list(app: &Rc<AppState>, router: &Router, view: &Rc<JobView>, cancel: &Rc<dyn Fn()>) {
    let rows = job_rows(&view.text.borrow(), view.cancel_label);
    view.cancel_row.set(rows.len() - 1);
    let Some(dialog) = build_picker_dialog(app, router, view.title, &rows) else { return };
    {
        let (view, cancel, weak) = (view.clone(), cancel.clone(), dialog.as_weak());
        dialog.on_item_selected(move |index| {
            if index as usize == view.cancel_row.get() && !view.cancelling.get() {
                cancel();
                view.cancelling.set(true);
                if let Some(d) = weak.upgrade() {
                    set_picker_items(&d, &job_rows(&view.text.borrow(), "Cancelling (stops the copy in progress)..."));
                }
            }
        });
    }
    *view.weak.borrow_mut() = Some(dialog.as_weak());
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
}

/// The game file question: the problems, then Fix in Game files / Move anyway / Cancel. A problem row opens
/// its full text and comes back here. `reply` is the worker's answer channel (only the first time).
fn show_prompt(app: &Rc<AppState>, router: &Router, view: &Rc<JobView>, port: Rc<String>, issues: Rc<Vec<Issue>>, reply: Option<mpsc::Sender<GameFileChoice>>) {
    if let Some(reply) = reply {
        *view.pending_reply.borrow_mut() = Some(reply);
    }
    *view.prompt_data.borrow_mut() = Some((port.clone(), issues.clone()));
    view.in_detail.set(false);
    let answer = {
        let view = view.clone();
        Rc::new(move |choice: GameFileChoice| view.answer_prompt(choice))
    };
    let named: Vec<(String, Issue)> = issues.iter().map(|i| (port.as_ref().clone(), i.clone())).collect();
    let model = game_file_issues(&named);
    let labels: Vec<String> = model
        .labels
        .iter()
        .enumerate()
        .map(|(i, label)| match model.act(i) {
            Act::Detail(n) => format!("  {}", ellipsize(&issues[n].line(), 90)),
            Act::FixInGameFiles => "Fix in Game files (then import again)".to_string(),
            Act::Cancel => "Cancel (stops the whole import here)".to_string(),
            _ => label.clone(),
        })
        .collect();
    let Some(dialog) = build_picker_dialog(app, router, "Check the game files", &labels) else {
        // The list could not be built: nothing is copied for this port and the import stops, rather
        // than the worker waiting for an answer nobody can give.
        answer(GameFileChoice::Cancel);
        return;
    };
    dialog.set_selected_index(model.select as i32);
    app.dialog_nav.picker_index.set(model.select as i32);
    {
        let (answer, app, router, view, port, issues) = (answer.clone(), app.clone(), router.clone(), view.clone(), port.clone(), issues.clone());
        dialog.on_item_selected(move |index| match model.act(index as usize) {
            Act::FixInGameFiles => answer(GameFileChoice::FixInGameFiles),
            Act::MoveAnyway => answer(GameFileChoice::MoveAnyway),
            Act::Cancel => answer(GameFileChoice::Cancel),
            Act::Detail(n) => {
                view.in_detail.set(true);
                let again: Step = {
                    let (view, port, issues) = (view.clone(), port.clone(), issues.clone());
                    Rc::new(move |app, router| show_prompt(app, router, &view, port.clone(), issues.clone(), None))
                };
                let text = format!("{}\n\n{}", port, issues[n].line());
                let on_ok = again.clone();
                confirm_back(&app, &router, "Check the game files", &text, "Back to the question", false, move |app, router| on_ok(app, router), again);
            }
            _ => {}
        });
    }
    {
        let answer = answer.clone();
        dialog.on_close_requested(move || answer(GameFileChoice::Cancel));
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
}

/// Runs `work` on a worker thread behind a list of progress rows (one per line of the worker's progress
/// text) and a Cancel row. Closing the list (B) cancels; the result is then dropped (and `leave` runs)
/// unless `deliver_after_close` (an import must still report what it did). `leave` is also what a notice
/// goes on to when the job cannot start because another is running. `prompts` is where the worker's
/// questions arrive.
pub(super) struct JobSpec<T> {
    pub(super) title: &'static str,
    pub(super) initial: String,
    pub(super) cancel_label: &'static str,
    pub(super) deliver_after_close: bool,
    pub(super) leave: Option<Step>,
    pub(super) prompts: Option<UiPrompter>,
    pub(super) is_import: bool,
    pub(super) work: Box<dyn FnOnce(&Ctl) -> T + Send>,
}

pub(super) fn run_job<T: Send + 'static>(app: &Rc<AppState>, router: &Router, spec: JobSpec<T>, done: impl FnOnce(&Rc<AppState>, &Router, Result<T, String>) + 'static) {
    if JOB_RUNNING.with(Cell::get) {
        let text = "A scan or an import is still running. Wait for it to finish, then try again.";
        return match &spec.leave {
            Some(step) => notice_back(app, router, text, step.clone()),
            None => message(app, router, TITLE, text),
        };
    }
    let job = Rc::new(Background::spawn(spec.work));
    JOB_RUNNING.with(|r| r.set(true));
    let view = Rc::new(JobView {
        title: spec.title,
        cancel_label: spec.cancel_label,
        cancelling: Cell::new(false),
        closed: Cell::new(false),
        prompting: Cell::new(false),
        prompt_data: RefCell::new(None),
        in_detail: Cell::new(false),
        reshow: Cell::new(false),
        text: RefCell::new(spec.initial),
        cancel_row: Cell::new(0),
        weak: RefCell::new(None),
        pending_reply: RefCell::new(None),
    });
    let cancel: Rc<dyn Fn()> = {
        let (job, view) = (job.clone(), view.clone());
        Rc::new(move || {
            job.cancel();
            // A question on screen is answered "Cancel" so the worker is not left waiting.
            if view.prompting.get() {
                view.answer_prompt(GameFileChoice::Cancel);
            }
        })
    };
    if spec.is_import {
        IMPORT_CANCEL.with(|c| *c.borrow_mut() = Some(cancel.clone()));
    }
    show_list(app, router, &view, &cancel);

    let done: RefCell<Option<Box<dyn FnOnce(&Rc<AppState>, &Router, Result<T, String>)>>> = RefCell::new(Some(Box::new(done)));
    let (app, router) = (app.clone(), router.clone());
    let (prompts, deliver_after_close, leave) = (spec.prompts, spec.deliver_after_close, spec.leave);
    JOB_TIMER.with(|timer| {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
            // The list was closed with B (not while a question stands in for it, nor between a question
            // and the list coming back): the work is cancelled.
            if !view.prompting.get() && !view.reshow.get() && !view.closed.get() && !view.weak.borrow().as_ref().and_then(slint::Weak::upgrade).is_some_and(|d| d.window().is_visible()) {
                cancel();
                view.closed.set(true);
            }
            if !view.prompting.get() {
                let next = prompts.as_ref().and_then(|p| p.queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front());
                if let Some(request) = next {
                    // Cancelling or closed: a question that was queued meanwhile is not asked.
                    if view.closed.get() || view.cancelling.get() {
                        let _ = request.reply.send(GameFileChoice::Cancel);
                        return;
                    }
                    view.prompting.set(true);
                    return show_prompt(&app, &router, &view, Rc::new(request.port), Rc::new(request.issues), Some(request.reply));
                }
            }
            if view.prompting.get() {
                // Nothing is on screen: the title bar's X hid it. If it was a problem's full text, the question
                // comes back; if it was the question itself, that is Cancel.
                if !dialog_is_open(&app) {
                    let data = view.prompt_data.borrow().clone();
                    match data {
                        Some((port, issues)) if view.in_detail.get() => show_prompt(&app, &router, &view, port, issues, None),
                        _ => view.answer_prompt(GameFileChoice::Cancel),
                    }
                }
                return;
            }
            if let Some(result) = job.take() {
                stop_job_timer();
                if QUIT_AFTER_IMPORT.with(Cell::get) {
                    let _ = slint::quit_event_loop();
                    return;
                }
                if view.closed.get() && !deliver_after_close {
                    if let Some(leave) = &leave {
                        leave(&app, &router);
                    }
                    return;
                }
                if let Some(done) = done.borrow_mut().take() {
                    done(&app, &router, result);
                }
                return;
            }
            if view.reshow.get() {
                view.reshow.set(false);
                return show_list(&app, &router, &view, &cancel);
            }
            if view.closed.get() {
                return;
            }
            let text = job.progress();
            if text.is_empty() || text == *view.text.borrow() {
                return;
            }
            *view.text.borrow_mut() = text.clone();
            view.cancel_row.set(job_rows(&text, view.cancel_label).len() - 1);
            if let Some(d) = view.weak.borrow().as_ref().and_then(slint::Weak::upgrade) {
                let label = if view.cancelling.get() { "Cancelling (stops the copy in progress)..." } else { view.cancel_label };
                set_picker_items(&d, &job_rows(&text, label));
            }
        });
    });
}

// -- Scanning ----------------------------------------------------------------------------------------

pub(super) fn start_scan(app: &Rc<AppState>, router: &Router, ctx: &Ctx, folders: ScanFolders) {
    scan(app, router, ctx, folders, ctx.waited_on().then(|| ctx.finisher()));
}

/// The scan, with what B on its progress list does: leave the step (the first scan), or for a rescan from the
/// main list go back to that list.
pub(super) fn scan(app: &Rc<AppState>, router: &Router, ctx: &Ctx, folders: ScanFolders, leave: Option<Step>) {
    let Some(home) = home_dir() else {
        return end_notice(app, router, ctx, TITLE, "Could not find your home folder, so there is nowhere to look.");
    };
    let settings = ForkSettings::load(&settings_path(app));
    let ports = ports_folder(&app.paths.library_dir());
    let roots = folders.roots(&home, &ports);
    let catalog = app.catalog.borrow().clone();
    let saved_index = super::deck_ready_screen::index_for_import(app);
    let cache_dir = app.paths.cache_dir.clone();
    let choices = ctx.choices.borrow().clone();
    let (work_home, work_ports, deeper) = (home.clone(), ports.clone(), folders.look_deeper);
    let ctx2 = ctx.clone();
    run_job(
        app,
        router,
        JobSpec {
            title: "Looking for ports",
            initial: "Looking in your scan folders...".into(),
            cancel_label: "Cancel",
            deliver_after_close: false,
            leave,
            prompts: None,
            is_import: false,
            work: Box::new(move |ctl| {
                let shortcuts = read_shortcuts(&work_home, &settings);
                // The Deck-ready list adds download names and projects the catalog lacks; fetched once if
                // it was never saved (a failure only means matching by the catalog alone).
                let index = saved_index.or_else(|| fetch_deck_ready(&cache_dir, ctl));
                let deck_ready_note = index.is_none().then(|| {
                    "The Deck-ready list couldn't be loaded, so ports only on that list (like Severed Chains) aren't recognised. Connect to the internet and Scan again.".to_string()
                });
                let t = targets::build(&catalog, index.as_ref(), &work_ports);
                let input = ScanInput {
                    roots,
                    catalog: t.catalog,
                    library_dir: work_ports,
                    home: work_home.clone(),
                    deeper,
                    shortcuts,
                    hints: t.hints,
                    new_entries: t.new_entries,
                    choices,
                    deck_ready_note,
                };
                run_scan(&input, &|| ctl.cancelled(), &|t| ctl.set_progress(t))
            }),
        },
        move |app, router, result| match result {
            Ok(scan) => {
                // What the scan matched becomes the cards; folders the user said are not ports are left out.
                let ignored = crate::fork::import::cards::ignored_paths(&ForkSettings::load(&settings_path(app)));
                let cards = Cards::from_scan(&scan, &ignored, &ports);
                let screen = Rc::new(RefCell::new(ImportScreen::new(&home, &ports, folders, scan, cards, ctx2.from_wizard)));
                super::import_cards_screen::show_found(app, router, &ctx2, screen);
            }
            Err(e) => end_notice(app, router, &ctx2, TITLE, &e),
        },
    );
}

/// Fetches the Deck-ready list once (it was never saved), waiting at most about 10 s; Cancel stops the wait
/// at once (the request then ends by itself and only saves the list for next time).
fn fetch_deck_ready(cache_dir: &std::path::Path, ctl: &Ctl) -> Option<crate::fork::deck_ready::index::Index> {
    use crate::fork::deck_ready::sync;
    ctl.set_progress("Getting the Deck-ready list...".into());
    let (tx, rx) = mpsc::channel();
    let dir = cache_dir.to_path_buf();
    std::thread::spawn(move || {
        let outcome = sync::refresh(&dir, chrono::Utc::now(), |etag| sync::fetch_from_github_within(etag, Duration::from_secs(10)));
        let _ = tx.send(outcome.index);
    });
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(index) => return index,
            Err(mpsc::RecvTimeoutError::Timeout) if !ctl.cancelled() => {}
            Err(_) => return None,
        }
    }
}

// -- Scan folders ----------------------------------------------------------------------------------------

pub(super) struct FoldersState {
    main: Rc<RefCell<ImportScreen>>,
    screen: RefCell<FoldersScreen>,
    /// The folder list changed, so going back scans again.
    changed: Cell<bool>,
}

impl FoldersState {
    pub(super) fn new(main: Rc<RefCell<ImportScreen>>) -> Rc<FoldersState> {
        let screen = FoldersScreen::new(&main.borrow().home, main.borrow().folders.clone());
        Rc::new(FoldersState { main, screen: RefCell::new(screen), changed: Cell::new(false) })
    }
}

pub(super) fn save_folders(app: &AppState, folders: &ScanFolders) -> Option<String> {
    let mut settings = ForkSettings::load(&settings_path(app));
    folders.save(&mut settings).err().map(|e| format!("Could not save the folder list: {e}"))
}

/// Back from the scan folders: a new scan when the list changed, else the found page as it was.
fn folders_back(app: &Rc<AppState>, router: &Router, ctx: &Ctx, state: &Rc<FoldersState>) {
    if state.changed.get() {
        let folders = state.screen.borrow().folders.clone();
        return start_scan(app, router, ctx, folders);
    }
    super::import_cards_screen::show_found(app, router, ctx, state.main.clone());
}

pub(super) fn show_folders(app: &Rc<AppState>, router: &Router, ctx: &Ctx, state: Rc<FoldersState>) {
    let labels = state.screen.borrow().labels();
    let Some(dialog) = build_picker_dialog(app, router, "Scan folders", &labels) else { return ctx.finish_quiet(app, router) };
    // The cursor starts on the first real row, not on the note.
    let first = state.screen.borrow().first_row() as i32;
    dialog.set_selected_index(first);
    app.dialog_nav.picker_index.set(first);
    {
        let (app, router, ctx, state) = (app.clone(), router.clone(), ctx.clone(), state.clone());
        dialog.on_close_requested(move || {
            close_current_dialog(&app, &router);
            folders_back(&app, &router, &ctx, &state);
        });
    }
    {
        let (app, router, ctx) = (app.clone(), router.clone(), ctx.clone());
        dialog.on_item_selected(move |index| {
            let line = state.screen.borrow().lines().get(index as usize).copied();
            let mut changed = false;
            match line {
                Some(FolderLine::Note) | None => {}
                Some(FolderLine::Add) => return add_folder(&app, &router, &ctx, state.clone()),
                Some(FolderLine::Default(i)) => {
                    state.screen.borrow_mut().folders.toggle_default(crate::fork::adopt::WELL_KNOWN[i]);
                    changed = true;
                }
                Some(FolderLine::User(i)) => {
                    let text = state.screen.borrow().remove_text(i);
                    let (on_state, on_ctx) = (state.clone(), ctx.clone());
                    let cancel: Step = {
                        let (state, ctx) = (state.clone(), ctx.clone());
                        Rc::new(move |app, router| show_folders(app, router, &ctx, state.clone()))
                    };
                    return confirm_back(&app, &router, "Scan folder", &text, "Remove", false, move |app, router| {
                        {
                            let mut screen = on_state.screen.borrow_mut();
                            screen.folders.remove_user(i);
                            screen.note = save_folders(app, &screen.folders);
                        }
                        on_state.changed.set(true);
                        show_folders(app, router, &on_ctx, on_state.clone());
                    }, cancel);
                }
                Some(FolderLine::Deeper) => {
                    state.screen.borrow_mut().folders.toggle_deeper();
                    changed = true;
                }
                Some(FolderLine::Back) => return folders_back(&app, &router, &ctx, &state),
            }
            if changed {
                state.changed.set(true);
                let mut screen = state.screen.borrow_mut();
                screen.note = save_folders(&app, &screen.folders);
            }
            // The note row comes and goes, so the list is rebuilt, not just relabelled.
            show_folders(&app, &router, &ctx, state.clone());
        });
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
}

/// "Add a folder to scan...": the in-app folder picker (`folder_picker_screen`), then back to the list.
fn add_folder(app: &Rc<AppState>, router: &Router, ctx: &Ctx, state: Rc<FoldersState>) {
    let start = home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let (chosen_state, chosen_ctx) = (state.clone(), ctx.clone());
    let cancel_ctx = ctx.clone();
    open_folder_picker(
        app,
        router,
        PickerRequest::new("Add a folder to scan", &start, Purpose::ScanFolder),
        move |app, router, path| {
            let home = home_dir().unwrap_or_default();
            {
                let mut screen = chosen_state.screen.borrow_mut();
                let outcome = screen.folders.add_user(&path, &home);
                screen.note = match &outcome {
                    AddOutcome::Added => save_folders(app, &screen.folders).or_else(|| Some(format!("Added {}", short_path(&path, &home)))),
                    AddOutcome::DefaultTurnedOn(name) => save_folders(app, &screen.folders).or_else(|| Some(format!("~/{name} is back on"))),
                    AddOutcome::AlreadyListed => Some("That folder is already in the list".to_string()),
                    AddOutcome::NotAFolder => Some("That is not a folder".to_string()),
                };
                if matches!(outcome, AddOutcome::Added | AddOutcome::DefaultTurnedOn(_)) {
                    chosen_state.changed.set(true);
                }
            }
            show_folders(app, router, &chosen_ctx, chosen_state.clone());
        },
        move |app, router| show_folders(app, router, &cancel_ctx, state.clone()),
    );
}

// -- Plans and the import ---------------------------------------------------------------------------------

pub(super) fn build_plans(app: &AppState, screen: &ImportScreen) -> (Vec<PortPlan>, Option<u64>) {
    let requests = build_requests(&screen.selected_found(), &screen.ports_folder);
    let free = free_bytes(&screen.ports_folder);
    let write = steam_write_mode(&SystemSteamCheck::new(), steam_queue::game_mode_from_env());
    let guards = PlanGuards { home: screen.home.clone(), config_dir: app.paths.config_dir.clone(), scan_roots: screen.scan.roots_scanned.clone() };
    (plan_all_with(&requests, free, write, &guards), free)
}

pub(super) fn back_to_confirm(ctx: &Ctx, screen: &Rc<RefCell<ImportScreen>>) -> Step {
    let (ctx, screen) = (ctx.clone(), screen.clone());
    Rc::new(move |app, router| super::import_cards_screen::show_confirm(app, router, &ctx, screen.clone()))
}

pub(super) fn show_plans(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Rc<RefCell<ImportScreen>>) {
    let (plans, _) = build_plans(app, &screen.borrow());
    let mut labels = plan_labels(&plans);
    let back = labels.len();
    labels.push("Back".to_string());
    let Some(dialog) = build_picker_dialog(app, router, "What importing will do", &labels) else { return ctx.finish_quiet(app, router) };
    {
        let (app, router, ctx, screen) = (app.clone(), router.clone(), ctx.clone(), screen.clone());
        dialog.on_close_requested(move || {
            close_current_dialog(&app, &router);
            super::import_cards_screen::show_confirm(&app, &router, &ctx, screen.clone());
        });
    }
    {
        let (app, router, ctx) = (app.clone(), router.clone(), ctx.clone());
        dialog.on_item_selected(move |index| {
            let index = index as usize;
            if index == back {
                return super::import_cards_screen::show_confirm(&app, &router, &ctx, screen.clone());
            }
            let Some(plan) = plans.get(index) else { return };
            let home = screen.borrow().home.clone();
            let text = plan_text(plan, &home);
            let again: Step = {
                let (screen, ctx) = (screen.clone(), ctx.clone());
                Rc::new(move |app, router| show_plans(app, router, &ctx, screen.clone()))
            };
            let on_ok = again.clone();
            confirm_back(&app, &router, &plan.request.port_name, &text, "Back to the list", false, move |app, router| on_ok(app, router), again);
        });
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
}

/// "Move 4 ports" was chosen on the move page: refuses while the ports folder is missing, offers to link in
/// place the ports that do not fit, and otherwise starts the copies. The move page was the confirmation.
pub(super) fn start_move(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Rc<RefCell<ImportScreen>>) {
    let back = back_to_confirm(ctx, &screen);
    // The ports folder is missing (card out) or a move is running: nothing may be written there.
    if let Some(why) = blocked_reason(app) {
        return notice_back(app, router, &why, back);
    }
    let (plans, _) = build_plans(app, &screen.borrow());
    let home = screen.borrow().home.clone();
    if plans.iter().all(|p| !p.can_run()) {
        // Only the reasons; and where the one reason is room, linking the port where it stands is offered.
        let text = blocked_text(&plans, &home);
        let linkable = space_blocked(&plans);
        if linkable.is_empty() {
            return notice_back(app, router, &text, back);
        }
        let ctx2 = ctx.clone();
        let then: Rc<dyn Fn(&Rc<AppState>, &Router, String, Vec<String>)> = Rc::new(move |app, router, text, keys| finish_after(app, router, &ctx2, text, VecDeque::new(), keys, false));
        return offer_link(app, router, linkable, text, then, back);
    }
    run_import(app, router, ctx, &screen, plans, home);
}

fn run_import(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: &Rc<RefCell<ImportScreen>>, plans: Vec<PortPlan>, home: PathBuf) {
    let back = back_to_confirm(ctx, screen);
    if let Some(why) = blocked_reason(app) {
        return notice_back(app, router, &why, back);
    }
    if !app.install_runtime.installing.borrow().is_empty() {
        return notice_back(app, router, "An install, update or save restore is running. Wait for it to finish, then try again.", back);
    }
    // Deck-ready projects the catalog lacks are added to the user's port list first (built and checked before
    // anything is written, the file backed up), as the Deck-ready screen's "Add and install" does.
    let added = match add_new_entries(app, &plans) {
        Ok(added) => added,
        Err(why) => return notice_back(app, router, &format!("Nothing was imported: {why}"), back),
    };
    // While it runs these ports count as busy, so nothing installs or updates them underneath it.
    let keys: Vec<String> = plans.iter().map(|p| p.request.port_key.clone()).collect();
    for k in &keys {
        app.install_runtime.installing.borrow_mut().insert(k.clone());
    }
    let engine = engine_for_import(home.clone(), app.paths.config_dir.clone(), app.catalog.borrow().clone(), steam_queue::game_mode_from_env());
    let prompter = UiPrompter::default();
    let worker_prompter = prompter.clone();
    let initial = plans.iter().map(|p| format!("{}: waiting", p.request.port_name)).collect::<Vec<_>>().join("\n");
    let ctx = ctx.clone();
    run_job(
        app,
        router,
        JobSpec {
            title: "Importing ports",
            initial,
            cancel_label: "Cancel (the copy in progress is removed; finished ports stay)",
            deliver_after_close: true,
            leave: Some(back.clone()),
            prompts: Some(prompter),
            is_import: true,
            work: Box::new(move |ctl| move_selected(engine.as_ref(), plans, ctl.flag(), &|t| ctl.set_progress(t), &worker_prompter)),
        },
        move |app, router, result| {
            for k in &keys {
                app.install_runtime.installing.borrow_mut().remove(k);
            }
            match result {
                Ok(summary) => {
                    let moved: Vec<String> = summary.moved().map(|i| i.plan.request.port_key.clone()).collect();
                    let note = take_off_unmoved(app, &added, &moved);
                    show_result(app, router, &ctx, Rc::new(summary), &home, note)
                }
                Err(e) => {
                    let note = take_off_unmoved(app, &added, &[]).map_or(String::new(), |n| format!("\n{n}"));
                    end_notice(app, router, &ctx, "Import problem", &format!("The import stopped: {e}\nThe old folders were not changed by the app.{note}"))
                }
            }
        },
    );
}

/// A port list entry this import added: its port key, name and the entry as written.
struct Added {
    key: String,
    name: String,
    value: serde_json::Value,
}

/// Adds the port list entries the runnable plans need (Deck-ready projects the catalog lacks) and puts them in
/// the app's catalog. On a failure the ones added before it are taken off again, and nothing is imported.
fn add_new_entries(app: &Rc<AppState>, plans: &[PortPlan]) -> Result<Vec<Added>, String> {
    let local_json = app.paths.config_dir.join("ports.local.json");
    let mut added: Vec<Added> = Vec::new();
    for plan in plans.iter().filter(|p| p.can_run()) {
        let Some(new) = &plan.request.new_entry else { continue };
        if app.catalog.borrow().iter().any(|p| p.key() == plan.request.port_key) {
            continue;
        }
        let built = crate::core::models::port_from_value(&new.value).map_err(|_| format!("the entry for {} could not be built", plan.request.port_name));
        let written = built.and_then(|port| {
            crate::fork::deck_ready::entry::add_to_local_file(&local_json, new.value.clone(), &crate::fork::store::backup_stamp()).map(|_| port).map_err(|e| format!("{}: {e}", plan.request.port_name))
        });
        let mut port = match written {
            Ok(port) => port,
            Err(why) => {
                let _ = take_off_unmoved(app, &added, &[]);
                return Err(why);
            }
        };
        port.user_managed = true;
        {
            let mut catalog = app.catalog.borrow_mut();
            let old = std::mem::take(&mut *catalog);
            *catalog = crate::core::config::merge_local_catalog(old, vec![port]);
        }
        added.push(Added { key: plan.request.port_key.clone(), name: plan.request.port_name.clone(), value: new.value.clone() });
    }
    Ok(added)
}

/// Takes the entries this import added off the port list again for the ports that did not move (cancelled,
/// refused, failed), so a port that never came in does not stay listed. Returns what to tell the user.
fn take_off_unmoved(app: &Rc<AppState>, added: &[Added], moved: &[String]) -> Option<String> {
    let local_json = app.paths.config_dir.join("ports.local.json");
    let mut lines = Vec::new();
    for a in added.iter().filter(|a| !moved.contains(&a.key)) {
        match crate::fork::import::targets::remove_new_entry(&local_json, &a.value, &crate::fork::store::backup_stamp()) {
            Ok(_) => {
                app.catalog.borrow_mut().retain(|p| !(p.user_managed && p.key() == a.key));
                lines.push(format!("{} was not imported, so it was taken off your port list again.", a.name));
            }
            Err(e) => lines.push(format!("{} stays in your port list (it could not be taken off: {e}).", a.name)),
        }
    }
    if !lines.is_empty() {
        app.refresh_current_view();
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Marks what moved as installed, redraws, then continues: the link offer for ports that did not fit,
/// then "Remove the old copy now?" for each port that moved, then the game file search.
fn show_result(app: &Rc<AppState>, router: &Router, ctx: &Ctx, summary: Rc<ImportSummary>, home: &std::path::Path, note: Option<String>) {
    {
        let dir = app.paths.library_dir();
        let mut state = app.state.borrow_mut();
        for item in summary.moved() {
            let key = &item.plan.request.port_key;
            if state.get(key).is_none() {
                if let Some(port) = app.catalog.borrow().iter().find(|p| p.key() == key) {
                    if crate::core::installer::is_installed(port, &dir) {
                        state.mark_installed(key, None);
                    }
                }
            }
        }
    }
    app.refresh_current_view();
    app.refresh_tray_recent_games();

    let moved_from: Vec<PathBuf> = summary.moved().map(|i| i.plan.request.from.clone()).collect();
    let copies: VecDeque<old_copies::OldCopy> = old_copies::load(&app.paths.config_dir).unwrap_or_default().into_iter().filter(|o| moved_from.contains(&o.old_path)).collect();
    let mut text = summary.text(home);
    if let Some(note) = note {
        text.push_str(&format!("\n\n{note}"));
    }
    let candidates = summary.link_candidates();
    let moved_keys: Vec<String> = summary.moved().map(|i| i.plan.request.port_key.clone()).collect();
    let wants_game_files = summary.wants_game_files();
    if candidates.is_empty() {
        return finish_after(app, router, ctx, text, copies, moved_keys, wants_game_files);
    }
    let requests: Vec<MoveRequest> = candidates.iter().map(|&i| summary.items[i].plan.request.clone()).collect();
    let ctx2 = ctx.clone();
    // Linking is offered after the summary; saying no goes on with the rest of the flow just the same.
    let (carry_text, carry_copies, carry_keys) = (text.clone(), copies.clone(), moved_keys.clone());
    let skip: Step = {
        let ctx = ctx.clone();
        Rc::new(move |app, router| finish_after(app, router, &ctx, carry_text.clone(), carry_copies.clone(), carry_keys.clone(), wants_game_files))
    };
    let base = text.clone();
    let then: Rc<dyn Fn(&Rc<AppState>, &Router, String, Vec<String>)> = Rc::new(move |app, router, link_text, linked_keys| {
        let mut keys = moved_keys.clone();
        keys.extend(linked_keys);
        finish_after(app, router, &ctx2, format!("{base}\n\n{link_text}"), copies.clone(), keys, wants_game_files);
    });
    offer_link(app, router, requests, text, then, skip);
}

/// "These ports did not fit: link them where they stand instead?" `preface` is what came before (the
/// summary or the reasons). Link instead runs the link and `then(text, keys)`; Cancel or B runs `on_cancel`.
fn offer_link(app: &Rc<AppState>, router: &Router, requests: Vec<MoveRequest>, preface: String, then: Rc<dyn Fn(&Rc<AppState>, &Router, String, Vec<String>)>, on_cancel: Step) {
    let refs: Vec<&MoveRequest> = requests.iter().collect();
    let question = format!("{preface}\n\n{}", fallback::confirm_text(&refs));
    confirm_back(app, router, TITLE, &question, "Link instead", true, move |app, router| {
        // Refused (the ports folder is missing or a move runs): the flow goes on with the reason, not a dead end.
        if let Some(why) = blocked_reason(app) {
            return then(app, router, format!("Nothing was linked: {why}"), Vec::new());
        }
        let local_json = app.paths.config_dir.join("ports.local.json");
        let catalog = app.catalog.borrow().clone();
        let refs: Vec<&MoveRequest> = requests.iter().collect();
        let done = fallback::link_instead(&refs, &catalog, &app.paths.library_dir(), &local_json);
        let mut keys = Vec::new();
        for port in done.linked {
            keys.push(port.key().to_string());
            app.state.borrow_mut().mark_installed(port.key(), None);
            let mut catalog = app.catalog.borrow_mut();
            let old = std::mem::take(&mut *catalog);
            *catalog = crate::core::config::merge_local_catalog(old, vec![port]);
        }
        app.refresh_current_view();
        then(app, router, done.text, keys);
    }, on_cancel);
}

/// The last steps of an import: the old-copy questions, then the game file search for the ports that came
/// in (`rom_screens::after_ports_added_then`, which shows the summary at the top of its list when it has
/// something to look for), else the summary alone, or Continue in the wizard.
fn finish_after(app: &Rc<AppState>, router: &Router, ctx: &Ctx, text: String, copies: VecDeque<old_copies::OldCopy>, keys: Vec<String>, wants_game_files: bool) {
    let ctx = ctx.clone();
    prompt_old_copies_batch_then(app, router, copies, text, move |app, router, text| match text {
        Some(text) => conclude(app, router, &ctx, text, &keys, wants_game_files),
        // The questions were abandoned because another dialog took the screen: nothing more is shown here.
        None => ctx.finish_quiet(app, router),
    });
}

fn conclude(app: &Rc<AppState>, router: &Router, ctx: &Ctx, text: String, keys: &[String], wants_game_files: bool) {
    let next: Step = {
        let (ctx, text, keys) = (ctx.clone(), text.clone(), keys.to_vec());
        Rc::new(move |app, router| {
            if !keys.is_empty() && super::rom_screens::after_ports_added_then(app, router, &keys, &text, Some(ctx.finisher())) {
                return;
            }
            end_notice(app, router, &ctx, "Import finished", &text);
        })
    };
    // Ports stopped for their game files: offer Game files first (the wizard just continues, its own step
    // for game files comes later); "Not now" goes on to the search for the ports that came in.
    if wants_game_files && !ctx.waited_on() {
        let go_next = next.clone();
        return confirm_back(app, router, "Import finished", &text, "Open Game files...", false, |app, router| super::rom_screens::open_game_files(app, router), go_next);
    }
    next(app, router);
}
