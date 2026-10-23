//! "Import ports..." (Settings menu, and step c of the setup wizard): finds ports installed outside the
//! ports folder and moves them in. It replaces "Adopt installed ports".
//!
//! Every screen is a list built from the picker dialog the app already has, so it works with the
//! controller, keyboard and mouse. What the lists say and do is decided in `crate::fork::import` (tested
//! without any UI); this file draws it and runs the slow parts (the scan, the import) on a worker thread
//! with a Cancel row. The import itself is the port mover (`crate::fork::port_mover`) through
//! `crate::fork::import::real_engine`; this file only asks the user what it needs to ask: the game file
//! problem question while the worker waits, and "Remove the old copy now?" per port afterwards
//! (`ports_folder_screen::prompt_old_copies`, Keep for now preselected). The text is English only; it is
//! not part of upstream's translation tables.
//!
//! Entry point for the setup wizard: `open(app, router, from_wizard)`, or `open_with` to be called back
//! when the user chooses "Continue setup" (or finishes an import).

use super::dialogs::{
    build_picker_dialog, close_current_dialog, dialog_is_open, finish_dialog_open, open_confirm_dialog, open_message_dialog, set_picker_items, DialogSlot,
};
use super::folder_picker_screen::open_folder_picker;
use super::ports_folder_screen::{blocked_reason, prompt_old_copies_then, refuse_when_blocked};
use super::state::AppState;
use crate::fork::folder_picker::{Purpose, PickerRequest};
use crate::fork::import::engine::{move_selected, GameFileChoice, ImportSummary, Prompter};
use crate::fork::import::fallback;
use crate::fork::import::folders::{AddOutcome, ScanFolders};
use crate::fork::import::plan::{build_requests, plan_all, steam_write_mode, PortPlan};
use crate::fork::import::real_engine::engine_for_import;
use crate::fork::import::scan::{read_shortcuts, run_scan, ScanInput};
use crate::fork::import::screen::{confirm_text, plan_labels, plan_text, FolderLine, FoldersScreen, ImportLine, ImportScreen};
use crate::fork::import::ports_folder;
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

const TITLE: &str = "Import ports";

/// What opened the screen: the Settings menu, or the setup wizard (which wants to be told when to go on).
#[derive(Clone)]
struct Ctx {
    from_wizard: bool,
    finish: Option<Finish>,
}

impl Ctx {
    fn finish(&self, app: &Rc<AppState>, router: &Router) {
        close_current_dialog(app, router);
        if let Some(finish) = &self.finish {
            finish(app, router);
        }
    }
}

/// The dialogs size themselves from a text estimate that comes out a line short for wrapped text, which
/// clips the last line; one spare line keeps it readable.
fn padded(text: &str) -> String {
    format!("{text}\n")
}

fn message(app: &Rc<AppState>, router: &Router, title: &str, text: &str) {
    open_message_dialog(app, router, title, &padded(text));
}

fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"].iter().find_map(|v| std::env::var_os(v)).map(PathBuf::from).filter(|p| p.is_absolute())
}

fn settings_path(app: &AppState) -> PathBuf {
    app.paths.config_dir.join(FILE_NAME)
}

// -- Entry points ----------------------------------------------------------------------------------

/// Settings > Import ports... (`from_wizard` false), or the wizard's import step (true).
pub(crate) fn open(app: &Rc<AppState>, router: &Router, from_wizard: bool) {
    open_with(app, router, from_wizard, None);
}

/// Like `open`; `finish` is called after "Continue setup" is chosen or an import has been reported.
pub(crate) fn open_with(app: &Rc<AppState>, router: &Router, from_wizard: bool, finish: Option<Finish>) {
    // The wizard has its own dialog up when it calls this; the Settings entry must not open on top of one.
    if !from_wizard && dialog_is_open(app) {
        return;
    }
    let folders = ScanFolders::load(&ForkSettings::load(&settings_path(app)));
    start_scan(app, router, &Ctx { from_wizard, finish }, folders);
}

// -- Background jobs ---------------------------------------------------------------------------------

thread_local! {
    static JOB_TIMER: slint::Timer = slint::Timer::default();
    static JOB_RUNNING: Cell<bool> = const { Cell::new(false) };
    /// Cancels the import that is running (Close and Quit ask for this first).
    static IMPORT_CANCEL: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
    static QUIT_AFTER_IMPORT: Cell<bool> = const { Cell::new(false) };
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
    /// The list must be shown again (a question was answered).
    reshow: Cell<bool>,
    text: RefCell<String>,
    cancel_row: Cell<usize>,
    weak: RefCell<Option<slint::Weak<crate::ListPickerDialog>>>,
    /// Where the answer to the question on screen goes.
    pending_reply: RefCell<Option<mpsc::Sender<GameFileChoice>>>,
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
                    set_picker_items(&d, &job_rows(&view.text.borrow(), "Cancelling (stops after the current step)..."));
                }
            }
        });
    }
    *view.weak.borrow_mut() = Some(dialog.as_weak());
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
}

/// The game file question: the problems, then Fix in Game files / Move anyway / Cancel.
fn show_prompt(app: &Rc<AppState>, router: &Router, view: &Rc<JobView>, request: PromptRequest) {
    let named: Vec<(String, Issue)> = request.issues.iter().map(|i| (request.port.clone(), i.clone())).collect();
    let model = game_file_issues(&named);
    let labels: Vec<String> = model
        .labels
        .iter()
        .enumerate()
        .map(|(i, label)| match model.act(i) {
            Act::Detail(n) => format!("  {}", ellipsize(&request.issues[n].line(), 90)),
            _ => label.clone(),
        })
        .collect();
    let Some(dialog) = build_picker_dialog(app, router, "Check the game files", &labels) else {
        let _ = request.reply.send(GameFileChoice::Cancel);
        return;
    };
    dialog.set_selected_index(model.select as i32);
    app.dialog_nav.picker_index.set(model.select as i32);
    *view.pending_reply.borrow_mut() = Some(request.reply);
    let answer = {
        let view = view.clone();
        Rc::new(move |choice: GameFileChoice| {
            if let Some(reply) = view.pending_reply.borrow_mut().take() {
                let _ = reply.send(choice);
            }
            view.prompting.set(false);
            view.reshow.set(true);
        })
    };
    {
        let answer = answer.clone();
        dialog.on_item_selected(move |index| match model.act(index as usize) {
            Act::FixInGameFiles => answer(GameFileChoice::FixInGameFiles),
            Act::MoveAnyway => answer(GameFileChoice::MoveAnyway),
            Act::Cancel => answer(GameFileChoice::Cancel),
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
/// text) and a Cancel row. Closing the list (B) cancels; the result is then dropped unless
/// `deliver_after_close` (an import must still report what it did). `prompts` is where the worker's
/// questions arrive; `on_cancel` is called besides raising the worker's cancel flag.
struct JobSpec<T> {
    title: &'static str,
    initial: String,
    cancel_label: &'static str,
    deliver_after_close: bool,
    prompts: Option<UiPrompter>,
    is_import: bool,
    work: Box<dyn FnOnce(&Ctl) -> T + Send>,
}

fn run_job<T: Send + 'static>(app: &Rc<AppState>, router: &Router, spec: JobSpec<T>, done: impl FnOnce(&Rc<AppState>, &Router, Result<T, String>) + 'static) {
    if JOB_RUNNING.with(Cell::get) {
        return message(app, router, TITLE, "A scan or an import is still running. Wait for it to finish, then try again.");
    }
    let job = Rc::new(Background::spawn(spec.work));
    JOB_RUNNING.with(|r| r.set(true));
    let view = Rc::new(JobView {
        title: spec.title,
        cancel_label: spec.cancel_label,
        cancelling: Cell::new(false),
        closed: Cell::new(false),
        prompting: Cell::new(false),
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
            if let Some(reply) = view.pending_reply.borrow_mut().take() {
                let _ = reply.send(GameFileChoice::Cancel);
                view.prompting.set(false);
                view.reshow.set(true);
            }
        })
    };
    if spec.is_import {
        IMPORT_CANCEL.with(|c| *c.borrow_mut() = Some(cancel.clone()));
    }
    show_list(app, router, &view, &cancel);

    let done: RefCell<Option<Box<dyn FnOnce(&Rc<AppState>, &Router, Result<T, String>)>>> = RefCell::new(Some(Box::new(done)));
    let (app, router) = (app.clone(), router.clone());
    let (prompts, deliver_after_close) = (spec.prompts, spec.deliver_after_close);
    JOB_TIMER.with(|timer| {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
            if !view.prompting.get() {
                let next = prompts.as_ref().and_then(|p| p.queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front());
                if let Some(request) = next {
                    view.prompting.set(true);
                    return show_prompt(&app, &router, &view, request);
                }
            }
            if view.prompting.get() {
                return;
            }
            if let Some(result) = job.take() {
                stop_job_timer();
                if QUIT_AFTER_IMPORT.with(Cell::get) {
                    let _ = slint::quit_event_loop();
                    return;
                }
                if view.closed.get() && !deliver_after_close {
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
            if !view.closed.get() && !view.weak.borrow().as_ref().and_then(slint::Weak::upgrade).is_some_and(|d| d.window().is_visible()) {
                cancel();
                view.closed.set(true);
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
                let label = if view.cancelling.get() { "Cancelling (stops after the current step)..." } else { view.cancel_label };
                set_picker_items(&d, &job_rows(&text, label));
            }
        });
    });
}

// -- Scanning ----------------------------------------------------------------------------------------

fn start_scan(app: &Rc<AppState>, router: &Router, ctx: &Ctx, folders: ScanFolders) {
    let Some(home) = home_dir() else {
        return message(app, router, TITLE, "Could not find your home folder, so there is nowhere to look.");
    };
    let settings = ForkSettings::load(&settings_path(app));
    let ports = ports_folder(&app.paths.library_dir());
    let roots = folders.roots(&home, &ports);
    let catalog = app.catalog.borrow().clone();
    let (work_home, work_ports, deeper) = (home.clone(), ports.clone(), folders.look_deeper);
    let ctx = ctx.clone();
    run_job(
        app,
        router,
        JobSpec {
            title: "Looking for ports",
            initial: "Looking in your scan folders...".into(),
            cancel_label: "Cancel",
            deliver_after_close: false,
            prompts: None,
            is_import: false,
            work: Box::new(move |ctl| {
                let shortcuts = read_shortcuts(&work_home, &settings);
                let input = ScanInput { roots, catalog, library_dir: work_ports, deeper, shortcuts };
                run_scan(&input, &|| ctl.cancelled(), &|t| ctl.set_progress(t))
            }),
        },
        move |app, router, result| match result {
            Ok(scan) => show_main(app, router, &ctx, Rc::new(RefCell::new(ImportScreen::new(&home, &ports, folders, scan, ctx.from_wizard)))),
            Err(e) => message(app, router, TITLE, &e),
        },
    );
}

// -- The main list -------------------------------------------------------------------------------------

fn show_main(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Rc<RefCell<ImportScreen>>) {
    let labels = screen.borrow().labels();
    let Some(dialog) = build_picker_dialog(app, router, TITLE, &labels) else { return };
    {
        let (app, router, ctx, weak) = (app.clone(), router.clone(), ctx.clone(), dialog.as_weak());
        dialog.on_item_selected(move |index| {
            let line = screen.borrow().lines().get(index as usize).copied();
            match line {
                Some(ImportLine::Found(i)) => screen.borrow_mut().toggle(i),
                Some(ImportLine::Folders) => {
                    let state = Rc::new(FoldersState { main: screen.clone(), screen: RefCell::new(FoldersScreen::new(&screen.borrow().home, screen.borrow().folders.clone())), changed: Cell::new(false) });
                    return show_folders(&app, &router, &ctx, state);
                }
                Some(ImportLine::Rescan) => {
                    let folders = screen.borrow().folders.clone();
                    return start_scan(&app, &router, &ctx, folders);
                }
                Some(ImportLine::Preview) => {
                    if screen.borrow().selected_count() > 0 {
                        return show_plans(&app, &router, &ctx, screen.clone());
                    }
                }
                Some(ImportLine::Import) => {
                    if screen.borrow().selected_count() > 0 {
                        return confirm_import(&app, &router, &ctx, screen.clone());
                    }
                }
                Some(ImportLine::Continue) => return ctx.finish(&app, &router),
                Some(ImportLine::Intro | ImportLine::PortsFolder | ImportLine::Info(_)) | None => {}
            }
            if let Some(d) = weak.upgrade() {
                set_picker_items(&d, &screen.borrow().labels());
            }
        });
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
}

// -- Scan folders ----------------------------------------------------------------------------------------

struct FoldersState {
    main: Rc<RefCell<ImportScreen>>,
    screen: RefCell<FoldersScreen>,
    /// The folder list changed, so going back scans again.
    changed: Cell<bool>,
}

fn save_folders(app: &AppState, folders: &ScanFolders) -> Option<String> {
    let mut settings = ForkSettings::load(&settings_path(app));
    folders.save(&mut settings).err().map(|e| format!("Could not save the folder list: {e}"))
}

fn show_folders(app: &Rc<AppState>, router: &Router, ctx: &Ctx, state: Rc<FoldersState>) {
    let labels = state.screen.borrow().labels();
    let Some(dialog) = build_picker_dialog(app, router, "Scan folders", &labels) else { return };
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
                    let (state, ctx) = (state.clone(), ctx.clone());
                    return open_confirm_dialog(&app, &router, "Scan folder".into(), padded(&text).into(), "Remove".into(), move |app, router| {
                        {
                            let mut screen = state.screen.borrow_mut();
                            screen.folders.remove_user(i);
                            screen.note = save_folders(app, &screen.folders);
                        }
                        state.changed.set(true);
                        show_folders(app, router, &ctx, state.clone());
                    });
                }
                Some(FolderLine::Deeper) => {
                    state.screen.borrow_mut().folders.toggle_deeper();
                    changed = true;
                }
                Some(FolderLine::Back) => {
                    if state.changed.get() {
                        let folders = state.screen.borrow().folders.clone();
                        return start_scan(&app, &router, &ctx, folders);
                    }
                    return show_main(&app, &router, &ctx, state.main.clone());
                }
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

fn build_plans(screen: &ImportScreen) -> (Vec<PortPlan>, Option<u64>) {
    let requests = build_requests(&screen.selected_found(), &screen.ports_folder);
    let free = free_bytes(&screen.ports_folder);
    let write = steam_write_mode(&SystemSteamCheck::new(), steam_queue::game_mode_from_env());
    (plan_all(&requests, free, write), free)
}

fn show_plans(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Rc<RefCell<ImportScreen>>) {
    let (plans, _) = build_plans(&screen.borrow());
    let mut labels = plan_labels(&plans);
    let back = labels.len();
    labels.push("Back".to_string());
    let Some(dialog) = build_picker_dialog(app, router, "What importing will do", &labels) else { return };
    {
        let (app, router, ctx) = (app.clone(), router.clone(), ctx.clone());
        dialog.on_item_selected(move |index| {
            let index = index as usize;
            if index == back {
                return show_main(&app, &router, &ctx, screen.clone());
            }
            let Some(plan) = plans.get(index) else { return };
            let home = screen.borrow().home.clone();
            let text = padded(&plan_text(plan, &home));
            let (screen, ctx) = (screen.clone(), ctx.clone());
            open_confirm_dialog(&app, &router, plan.request.port_name.clone().into(), text.into(), "Back to the list".into(), move |app, router| {
                show_plans(app, router, &ctx, screen.clone());
            });
        });
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
}

fn confirm_import(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Rc<RefCell<ImportScreen>>) {
    // The ports folder is missing (card out) or a move is running: nothing may be written there.
    if let Some(why) = blocked_reason(app) {
        return message(app, router, TITLE, &why);
    }
    let (plans, free) = build_plans(&screen.borrow());
    let (home, ports) = (screen.borrow().home.clone(), screen.borrow().ports_folder.clone());
    let text = padded(&confirm_text(&plans, &ports, &home, free, false));
    if plans.iter().all(|p| !p.can_run()) {
        return message(app, router, TITLE, &text);
    }
    let ctx = ctx.clone();
    open_confirm_dialog(app, router, TITLE.into(), text.into(), "Import".into(), move |app, router| run_import(app, router, &ctx, plans.clone(), home.clone()));
}

fn run_import(app: &Rc<AppState>, router: &Router, ctx: &Ctx, plans: Vec<PortPlan>, home: PathBuf) {
    if let Some(why) = blocked_reason(app) {
        return message(app, router, TITLE, &why);
    }
    if !app.install_runtime.installing.borrow().is_empty() {
        return message(app, router, TITLE, "An install, update or save restore is running. Wait for it to finish, then try again.");
    }
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
            cancel_label: "Cancel (stops after the current port)",
            deliver_after_close: true,
            prompts: Some(prompter),
            is_import: true,
            work: Box::new(move |ctl| move_selected(engine.as_ref(), plans, ctl.flag(), &|t| ctl.set_progress(t), &worker_prompter)),
        },
        move |app, router, result| {
            for k in &keys {
                app.install_runtime.installing.borrow_mut().remove(k);
            }
            match result {
                Ok(summary) => show_result(app, router, &ctx, Rc::new(summary), &home),
                Err(e) => message(app, router, "Import problem", &format!("The import stopped: {e}\nThe old folders were not changed by the app.")),
            }
        },
    );
}

/// Marks what moved as installed, redraws, then continues: the link offer for ports that did not fit,
/// then "Remove the old copy now?" for each port that moved.
fn show_result(app: &Rc<AppState>, router: &Router, ctx: &Ctx, summary: Rc<ImportSummary>, home: &std::path::Path) {
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
    let text = summary.text(home);
    let candidates = summary.link_candidates();
    let moved_keys: Vec<String> = summary.moved().map(|i| i.plan.request.port_key.clone()).collect();
    if candidates.is_empty() {
        return finish_after(app, router, ctx, text, copies, moved_keys, summary.wants_game_files());
    }
    let requests: Vec<_> = candidates.iter().map(|&i| summary.items[i].plan.request.clone()).collect();
    let refs: Vec<_> = requests.iter().collect();
    let mut question = format!("{text}\n\n{}", fallback::confirm_text(&refs));
    if !copies.is_empty() {
        question.push_str("\n\nIf you cancel here, the old copies of the ports that moved stay listed under Settings > Ports folder > Old copies.");
    }
    let (ctx, wants_game_files) = (ctx.clone(), summary.wants_game_files());
    open_confirm_dialog(app, router, summary.title().into(), padded(&question).into(), "Link instead".into(), move |app, router| {
        if refuse_when_blocked(app) {
            return;
        }
        let local_json = app.paths.config_dir.join("ports.local.json");
        let catalog = app.catalog.borrow().clone();
        let refs: Vec<_> = requests.iter().collect();
        let done = fallback::link_instead(&refs, &catalog, &app.paths.library_dir(), &local_json);
        let mut keys = moved_keys.clone();
        for port in done.linked {
            keys.push(port.key().to_string());
            app.state.borrow_mut().mark_installed(port.key(), None);
            let mut catalog = app.catalog.borrow_mut();
            let old = std::mem::take(&mut *catalog);
            *catalog = crate::core::config::merge_local_catalog(old, vec![port]);
        }
        app.refresh_current_view();
        finish_after(app, router, &ctx, format!("{text}\n\n{}", done.text), copies.clone(), keys, wants_game_files);
    });
}

/// The last steps of an import: the old-copy questions, then the game file search for the ports that came
/// in (`rom_screens::after_ports_added`, which shows the summary at the top of its list when it has
/// something to look for), else the summary alone.
fn finish_after(app: &Rc<AppState>, router: &Router, ctx: &Ctx, text: String, copies: VecDeque<old_copies::OldCopy>, keys: Vec<String>, wants_game_files: bool) {
    let ctx = ctx.clone();
    prompt_old_copies_then(app, router, copies, text, move |app, router, text| conclude(app, router, &ctx, text, &keys, wants_game_files));
}

fn conclude(app: &Rc<AppState>, router: &Router, ctx: &Ctx, text: String, keys: &[String], wants_game_files: bool) {
    if !keys.is_empty() && super::rom_screens::after_ports_added(app, router, keys, &text) {
        return;
    }
    if wants_game_files {
        return open_confirm_dialog(app, router, "Import finished".into(), padded(&text).into(), "Open Game files...".into(), |app, router| super::rom_screens::open_game_files(app, router));
    }
    if ctx.from_wizard {
        let ctx = ctx.clone();
        return open_confirm_dialog(app, router, "Import finished".into(), padded(&text).into(), "Continue setup".into(), move |app, router| ctx.finish(app, router));
    }
    message(app, router, "Import finished", &text);
}
