
//! The "Game files" pages (Settings > Ports > Game files) and the search for a port's game files after it installs.
//!
//! Fork (game files settings): every page is a guided page (`page_dialog::show_page`), no picker lists. What the pages
//! say is decided in `crate::fork::rom_screens` (`settings_pages` for the Game files page and the pages under it, `pages`
//! for one port's pages; tested without any UI); this file shows them, runs the slow parts on a worker thread (a look
//! through the folder can read thousands of files, a copy gigabytes) and opens the next page. The text is English only;
//! it is not part of upstream's translation tables.
//!
//! Hard rules, kept: the game folder is only read, a copy is made only after the user confirms it
//! with its size shown, and forgetting a connection removes the record and no files.

use super::dialogs::{close_current_dialog, open_message_dialog, open_progress_dialog, update_progress_dialog, DialogSlot};
use super::page_dialog::{show_page, show_page_handle, Handlers};
use super::state::AppState;
use crate::fork::setup_wizard::progress::{progress_page, ProgressAction};
use crate::core::path_safety::safe_join;
use crate::fork::rom_library::{Approval, RomLibraryStore, System};
use crate::fork::folder_picker::{PickerRequest, Purpose};
use crate::fork::port_page::paths::{human_path, Places};
use crate::fork::rom_screens::browse;
use super::folder_picker_screen::open_folder_picker;
use crate::fork::rom_screens::connect_screen::{self, ConnectScreen, PlanOutput};
use crate::fork::rom_screens::connections::{connection_groups, ConnGroup};
use crate::fork::rom_screens::job::{Background, Ctl};
use crate::fork::rom_screens::pages::GfAct;
use crate::fork::rom_screens::report::{scan_time, Report, ReportInput};
use crate::fork::rom_screens::scan_work::{self, ScanOutput};
use crate::fork::rom_screens::settings_pages::{self, GsAct};
use crate::ui::gamepad_router::GamepadRouter;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

type Router = Rc<RefCell<GamepadRouter>>;
type Back = Rc<dyn Fn(&Rc<AppState>, &Router)>;

const TITLE: &str = "Game files";
const STORE_FILE: &str = "rom_library.json";

// The "find this port's game files" flow (after an install, the Port page, Settings, and for other screens).
mod find_flow;
#[allow(unused_imports)]
pub(crate) use find_flow::{after_install, find_for_port, open_from_info, port_files_state, port_missing_game_files, port_needs_game_files, search_ports_now};

/// What to do once a folder is chosen (and where Back goes).
#[derive(Clone)]
enum Then {
    Results,
    /// From the setup wizard: the scan's result goes back to the wizard's page, not to the results pages.
    Wizard,
    /// Look for this port's game files (`Port::key()`); `back` is where its flow returns; `ask_how` asks how the file
    /// reaches the port (Settings).
    Find { key: String, back: Back, ask_how: bool },
}

/// Leave the folder chooser without choosing.
fn leave(app: &Rc<AppState>, router: &Router, then: &Then) {
    match then {
        Then::Results | Then::Wizard => menu(app, router),
        Then::Find { key, back, ask_how } => find_flow::resume(app, router, key, back, *ask_how),
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

fn store_path(app: &AppState) -> PathBuf {
    app.paths.config_dir.join(STORE_FILE)
}

fn load_store(app: &Rc<AppState>, router: &Router) -> Option<RomLibraryStore> {
    match RomLibraryStore::load(&store_path(app)) {
        Ok(store) => Some(store),
        Err(e) => {
            message(app, router, TITLE, &e.to_string());
            None
        }
    }
}

fn report_input(app: &AppState, store: &RomLibraryStore) -> ReportInput {
    let catalog = app.catalog.borrow().clone();
    let installed = catalog.iter().filter(|p| app.is_installed(p)).map(|p| p.key().to_string()).collect();
    ReportInput::new(catalog, installed, &app.paths.config_dir.join("ports.local.json"), store.mappings().to_vec())
}

fn local_date(time: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(time).format("%Y-%m-%d %H:%M").to_string()
}

// -- Background jobs ------------------------------------------------------------------------------

thread_local! {
    static JOB_TIMER: slint::Timer = slint::Timer::default();
    static JOB_RUNNING: Cell<bool> = const { Cell::new(false) };
}

enum JobUi {
    /// A dialog with a fixed status line; the job cannot be cancelled.
    Progress { title: &'static str, status: String },
    /// The setup wizard's scan page (one line, and Cancel; B cancels too). Outside the wizard it is a [`JobUi::Page`] with
    /// a Cancel card.
    Cancellable { title: &'static str },
    /// Fork (port page): a guided page showing how far the job got, for the game files pages. With `cancellable` it has a
    /// Cancel card (B cancels too); without, it only waits.
    Page { title: &'static str, cancellable: bool },
}

thread_local! {
    /// Run when the user closes the progress list of the next job with B (the job is cancelled and its result
    /// dropped): lets a flow that waits for that job (the import screen's, in the setup wizard) still go on.
    /// Set right before `run_job`, taken when that job ends either way.
    static ABANDON_HOOK: RefCell<Option<Back>> = const { RefCell::new(None) };
}

/// The game files scan as a wizard page: what it is reading now, and Cancel.
fn scan_progress_page(title: &str, text: &str, cancelling: bool) -> crate::fork::page::Page<ProgressAction> {
    let mut page = progress_page(title, "Only files your ports could need are read; nothing is changed or moved.", 3, &text.lines().take(3).collect::<Vec<_>>().join("\n"), "Skip for now (the folder is kept)", cancelling);
    page.window_title = format!("Setup wizard: {}", title.to_lowercase());
    page
}

/// The one-port game files flow's wait page (`JobUi::Page`).
fn work_page(title: &str, text: &str, cancellable: bool, cancelling: bool) -> crate::fork::page::Page<ProgressAction> {
    let mut page = if cancellable {
        progress_page(title, "Your game files are only read; nothing is changed or moved.", 3, &text.lines().take(3).collect::<Vec<_>>().join("
"), "Cancel (nothing is changed)", cancelling)
    } else {
        let mut p = progress_page(title, "This takes a moment.", 3, &text.lines().take(3).collect::<Vec<_>>().join("
"), "", false);
        p.options.clear();
        p.no_back = true;
        p
    };
    page.window_title = format!("Game files: {}", title.to_lowercase());
    page
}

fn stop_job_timer() {

    JOB_TIMER.with(|t| t.stop());
    JOB_RUNNING.with(|r| r.set(false));
}

fn run_job<T: Send + 'static>(
    app: &Rc<AppState>,
    router: &Router,
    ui: JobUi,
    work: impl FnOnce(&Ctl) -> T + Send + 'static,
    done: impl FnOnce(&Rc<AppState>, &Router, Result<T, String>) + 'static,
) {
    if JOB_RUNNING.with(Cell::get) {
        // The job does not start, so a hook set for it would be left for an unrelated job.
        ABANDON_HOOK.with(|h| h.borrow_mut().take());
        return message(app, router, TITLE, "Another game files task is still running. Wait for it to finish, then try again.");
    }
    // Fork (game files settings): outside the setup wizard a job that can be cancelled waits on a page with a Cancel card,
    // never on the old list with a "Cancel (nothing is changed)" row.
    let ui = match ui {
        JobUi::Cancellable { title } if !super::setup_wizard_screen::active() => JobUi::Page { title, cancellable: true },
        other => other,
    };
    let job = Rc::new(Background::spawn(work));
    JOB_RUNNING.with(|r| r.set(true));
    let cancelling = Rc::new(Cell::new(false));
    // In the setup wizard the scan's progress is a page (one line, and Cancel) instead of a list.
    let mut page_handle = None;
    let page_closed = Rc::new(Cell::new(false));
    let own_page = matches!(ui, JobUi::Page { .. });
    let progress_mode = match &ui {
        JobUi::Page { title, cancellable } => {
            let (cancel_job, closed_job) = (job.clone(), job.clone());
            let (cancelling_now, closed, closed_exit) = (cancelling.clone(), page_closed.clone(), page_closed.clone());
            let cancellable = *cancellable;
            let handlers = Handlers {
                on_action: Rc::new(move |_, _, _: ProgressAction| {
                    cancel_job.cancel();
                    cancelling_now.set(true);
                }),
                on_back: Rc::new(move |_, _| {
                    if cancellable {
                        closed_job.cancel();
                        closed.set(true);
                    }
                }),
                on_exit: Rc::new(move |_, _| closed_exit.set(true)),
                on_input: None,
            };
            page_handle = show_page_handle(app, router, work_page(title, "", cancellable, false).with_progress(None), handlers);
            false
        }
        JobUi::Progress { title, status } => {
            open_progress_dialog(app, router, title, status);
            true
        }
        JobUi::Cancellable { title } => {
            let (cancel_job, closed_job) = (job.clone(), job.clone());
            let (cancelling_now, closed) = (cancelling.clone(), page_closed.clone());
            let handlers = Handlers {
                on_action: Rc::new(move |_, _, _: ProgressAction| {
                    cancel_job.cancel();
                    cancelling_now.set(true);
                }),
                on_back: Rc::new({
                    let closed = closed.clone();
                    move |_, _| {
                        closed_job.cancel();
                        closed.set(true);
                    }
                }),
                on_exit: Rc::new(move |app, router| {
                    closed.set(true);
                    super::setup_wizard_screen::exit_wizard(app, router);
                }),
                on_input: None,
            };
            page_handle = show_page_handle(app, router, scan_progress_page(title, "", false).with_progress(None), handlers);
            false
        }
    };

    let title_text = match &ui {
        JobUi::Cancellable { title } | JobUi::Progress { title, .. } | JobUi::Page { title, .. } => *title,
    };
    let done: RefCell<Option<Box<dyn FnOnce(&Rc<AppState>, &Router, Result<T, String>)>>> = RefCell::new(Some(Box::new(done)));
    let (app, router) = (app.clone(), router.clone());

    let last = RefCell::new(String::new());
    let abandoned = Cell::new(false);
    JOB_TIMER.with(|timer| {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
            // The user left the page with B: stop the work and drop its result. The timer keeps running until the worker
            // has really stopped, so a new job cannot overlap it.
            if page_closed.get() && !abandoned.get() {
                job.cancel();
                abandoned.set(true);
            }
            if let Some(result) = job.take() {
                stop_job_timer();
                let hook = ABANDON_HOOK.with(|h| h.borrow_mut().take());
                if abandoned.get() {
                    if let Some(hook) = hook {
                        hook(&app, &router);
                    } else if page_handle.is_some() && super::setup_wizard_screen::active() && !own_page {
                        // B on the wizard's scan page: the page goes, and the wizard's game files page comes back
                        // (opening the Game files menu is what the wizard takes over while it waits).
                        close_current_dialog(&app, &router);
                        open_game_files(&app, &router);
                    } else if own_page {
                        close_current_dialog(&app, &router);
                    }
                    return;
                }

                // Only close what this job opened: the user may have closed it and opened something else.
                let own_dialog = !progress_mode || matches!(&*app.dialog_nav.dialogs.borrow(), DialogSlot::Progress(_));
                if own_dialog {
                    close_current_dialog(&app, &router);
                }
                if let Some(done) = done.borrow_mut().take() {
                    done(&app, &router, result);
                }
                return;
            }
            if abandoned.get() {
                return;
            }
            let text = job.progress();
            if text.is_empty() || text == *last.borrow() {
                return;
            }
            *last.borrow_mut() = text.clone();
            if progress_mode {
                if let DialogSlot::Progress(d) = &*app.dialog_nav.dialogs.borrow() {
                    update_progress_dialog(&app, d, &text);
                }
            } else if let (Some(handle), JobUi::Page { cancellable, .. }) = (&page_handle, &ui) {
                handle.update(work_page(title_text, &text, *cancellable, cancelling.get()).with_progress(job.fraction()));
            } else if let Some(handle) = &page_handle {
                handle.update(scan_progress_page(title_text, &text, cancelling.get()).with_progress(job.fraction()));
            }
        });
    });
}

// -- Menu -----------------------------------------------------------------------------------------

thread_local! {
    /// Where B (and Back) on the Game files page goes: the Ports page of Settings, or closing the page (Fix problems, the
    /// ports folder screens and Import ports, which come back by themselves when it closes).
    static MENU_BACK: RefCell<Option<Back>> = const { RefCell::new(None) };
}

/// The Game files page, from Fix problems, the ports folder screens and Import ports: B on it closes it.
pub(crate) fn open_game_files(app: &Rc<AppState>, router: &Router) {
    open_game_files_with(app, router, Rc::new(|app, router| close_current_dialog(app, router)));
}

/// Settings > Ports > Game files: B on the page runs `back` (up to the Ports page).
pub(crate) fn open_game_files_with(app: &Rc<AppState>, router: &Router, back: Back) {
    MENU_BACK.with(|b| *b.borrow_mut() = Some(back));
    menu(app, router);
}

/// The Game files page again (Back and B on the pages under it, a cancelled look, a folder not chosen).
fn menu(app: &Rc<AppState>, router: &Router) {
    // No "dialog already open" guard: the pages under it come back here while they are still up, and showing the page
    // replaces whatever is open.
    // Fork (setup wizard): backing out of a screen the wizard opened lands here; go back to the wizard.
    if super::setup_wizard_screen::resume_instead_of_menu(app, router, crate::fork::setup_wizard::flow::Menu::GameFiles) {
        return;
    }
    let Some(store) = load_store(app, router) else { return };
    show_menu(app, router, store);
}

fn menu_back(app: &Rc<AppState>, router: &Router) {
    match MENU_BACK.with(|b| b.borrow().clone()) {
        Some(back) => back(app, router),
        None => close_current_dialog(app, router),
    }
}

/// Handlers for a page of Settings > Game files: `act` says what each choice does, B goes `back`, the title bar's X
/// closes the page.
fn gs_handlers(act: impl Fn(&Rc<AppState>, &Router, GsAct) + 'static, back: impl Fn(&Rc<AppState>, &Router) + 'static) -> Handlers<GsAct> {
    Handlers { on_action: Rc::new(act), on_back: Rc::new(back), on_exit: Rc::new(|app, router| close_current_dialog(app, router)), on_input: None }
}

/// Something to say on a page of its own, then back to the Game files page.
fn notice(app: &Rc<AppState>, router: &Router, title: &str, text: &str) {
    show_page(app, router, settings_pages::notice_page(title, text), gs_handlers(|app, router, _| menu(app, router), menu));
}

/// Fork (setup wizard): the folder choice of this screen with no menu in front of it. After the scan
/// the results list shows what was found.
pub(crate) fn open_folder_chooser(app: &Rc<AppState>, router: &Router) {
    choose_folder(app, router, Then::Wizard);
}

thread_local! {
    /// What the last scan run for the wizard found for the installed ports that need game files.
    static WIZARD_RESULT: RefCell<Option<crate::fork::setup_wizard::game_files::Results>> = const { RefCell::new(None) };
}

thread_local! {
    /// The report of the last wizard scan, for "Connect them now".
    static WIZARD_REPORT: RefCell<Option<Rc<Report>>> = const { RefCell::new(None) };
    /// What connecting would do, planned and waiting for the answer about copies.
    static WIZARD_PLAN: RefCell<Option<Rc<PlanOutput>>> = const { RefCell::new(None) };
    /// How the last connect went: how many ports got a file, or why not.
    static WIZARD_CONNECT: RefCell<Option<Result<usize, String>>> = const { RefCell::new(None) };
}

/// The requests for every installed port in the scan whose files were found in the game files folder, are not
/// connected yet and are not already inside the port's own folder: the same requests Settings > Game files builds for
/// one port (each port's own rules: use in place, link or copy), for every port at once. A port that needs a choice
/// between several files is left to Settings > Game files. Returns the requests and the ports they are for.
fn wizard_requests(app: &AppState, report: &Report) -> (Vec<crate::fork::rom_library::ConnectRequest>, Vec<String>) {
    let Some(store) = RomLibraryStore::load(&store_path(app)).ok() else { return (Vec::new(), Vec::new()) };
    let library = app.paths.library_dir();
    let mut requests = Vec::new();
    let mut ports = Vec::new();
    for row in report.groups.iter().flat_map(|g| g.ports.iter()).filter(|r| r.installed && r.can_connect() && !r.connected) {
        let Ok(port_dir) = safe_join(&library, &row.port.folder) else { continue };
        let own = crate::fork::setup_wizard::game_files::in_folder_keys(std::iter::once((&row.port, port_dir.clone())), &app.paths.config_dir);
        if own.contains(row.port.key()) {
            continue;
        }
        if let Ok(mut r) = ConnectScreen::new(row, port_dir, store.mappings()).requests() {
            ports.push(row.port.key().to_string());
            requests.append(&mut r);
        }
    }
    (requests, ports)
}

/// A wizard scan finished: what it found for the ports that need files is read out and kept for the wizard page.
fn wizard_scan_done(app: &AppState, report: Report) {
    let library = app.paths.library_dir();
    let rows: Vec<&crate::fork::rom_screens::report::PortRow> = report.groups.iter().flat_map(|g| g.ports.iter()).filter(|r| r.installed).collect();
    let dirs = rows.iter().filter_map(|r| safe_join(&library, &r.port.folder).ok().map(|d| (&r.port, d)));
    let in_folder = crate::fork::setup_wizard::game_files::in_folder_keys(dirs, &app.paths.config_dir);
    let connectable: std::collections::HashSet<String> = wizard_requests(app, &report).1.into_iter().collect();
    WIZARD_RESULT.with(|r| *r.borrow_mut() = Some(crate::fork::setup_wizard::game_files::Results::from_report(&report, &in_folder, &connectable)));
    WIZARD_PLAN.with(|p| *p.borrow_mut() = None);
    WIZARD_REPORT.with(|r| *r.borrow_mut() = Some(Rc::new(report)));
}

/// What "Connect them now" would do (reads the disk, writes nothing). A copy is refused when the drive is too full.
pub(crate) fn wizard_plan(app: &AppState) -> Result<crate::fork::setup_wizard::flow::GamePlan, String> {
    let report = WIZARD_REPORT.with(|r| r.borrow().clone()).ok_or("Look in the game files folder first.")?;
    let (requests, _) = wizard_requests(app, &report);
    if requests.is_empty() {
        return Err("There is nothing to connect: every file that was found is already connected or needs a choice that Settings > Game files asks you for.".to_string());
    }
    let ports: std::collections::BTreeSet<String> = requests.iter().map(|r| r.port_key.clone()).collect();
    let plan = connect_screen::make_plan(requests)?;
    let copy_ports: std::collections::BTreeSet<&str> = plan.plans.iter().filter(|p| p.needs_confirmation).map(|p| p.request.port_key.as_str()).collect();
    let info = crate::fork::setup_wizard::flow::GamePlan { ports: ports.len(), copy_ports: copy_ports.len(), copy_bytes: plan.extra_bytes };
    WIZARD_PLAN.with(|p| *p.borrow_mut() = Some(Rc::new(plan)));
    Ok(info)
}

/// The wizard asked for this from inside one of its own presses, which holds it busy: the answer is handed back on
/// the next turn of the event loop, so it is not lost (and the question page is not left standing with dead choices).
fn resume_later(app: &Rc<AppState>, router: &Router) {
    let (app, router) = (app.clone(), router.clone());
    slint::Timer::single_shot(Duration::from_millis(0), move || super::setup_wizard_screen::resume_after_scan(&app, &router));
}

/// Connects the planned files in the background: links and in-place use always; the copies only when `copies` (the
/// user saw their size). The result waits for the wizard (`take_wizard_connect`).
pub(crate) fn wizard_connect(app: &Rc<AppState>, router: &Router, copies: bool) {
    let Some(plan) = WIZARD_PLAN.with(|p| p.borrow_mut().take()) else {
        WIZARD_CONNECT.with(|c| *c.borrow_mut() = Some(Err("The plan is gone; try again.".to_string())));
        return resume_later(app, router);
    };
    let plans: Vec<_> = plan.plans.iter().filter(|p| copies || !p.needs_confirmation).cloned().collect();
    let approval = if copies && plans.iter().any(|p| p.needs_confirmation) { Approval::CopyAllowed } else { Approval::None };
    if plans.is_empty() {
        WIZARD_CONNECT.with(|c| *c.borrow_mut() = Some(Ok(0)));
        return resume_later(app, router);
    }
    let status = if approval == Approval::CopyAllowed { "Copying game files. This can take a while and cannot be cancelled." } else { "Connecting game files..." };
    let path = store_path(app);
    run_job(
        app,
        router,
        JobUi::Progress { title: TITLE, status: status.into() },
        move |_| connect_screen::apply_plans(&path, &plans, approval),
        move |app, router, result| {
            let outcome = match result.and_then(|r| r) {
                Ok(mappings) => Ok(mappings.iter().map(|m| m.port_key.as_str()).collect::<std::collections::BTreeSet<_>>().len()),
                Err(e) => Err(e),
            };
            WIZARD_CONNECT.with(|c| *c.borrow_mut() = Some(outcome));
            super::setup_wizard_screen::resume_after_scan(app, router);
        },
    );
}

/// How the last wizard connect went (once).
pub(crate) fn take_wizard_connect() -> Option<Result<usize, String>> {
    WIZARD_CONNECT.with(|c| c.borrow_mut().take())
}

/// What the saved scan of the game files folder says for the installed ports that need files, without reading the
/// folder: `None` when no folder is set or it was never scanned (by the wizard, Settings > Game files, an older build).
pub(crate) fn saved_results(app: &AppState) -> Option<crate::fork::setup_wizard::game_files::Results> {
    let store = RomLibraryStore::load(&store_path(app)).ok()?;
    let root = store.rom_dir()?.to_path_buf();
    let out = scan_work::run_report(&store, &root, &report_input(app, &store)).ok()?;
    let library = app.paths.library_dir();
    let rows: Vec<&crate::fork::rom_screens::report::PortRow> = out.report.groups.iter().flat_map(|g| g.ports.iter()).filter(|r| r.installed).collect();
    let dirs = rows.iter().filter_map(|r| safe_join(&library, &r.port.folder).ok().map(|d| (&r.port, d)));
    let in_folder = crate::fork::setup_wizard::game_files::in_folder_keys(dirs, &app.paths.config_dir);
    Some(crate::fork::setup_wizard::game_files::Results::from_report(&out.report, &in_folder, &Default::default()))
}

/// Takes what the last wizard scan found (once).
pub(crate) fn take_wizard_result() -> Option<crate::fork::setup_wizard::game_files::Results> {
    WIZARD_RESULT.with(|r| r.borrow_mut().take())
}

/// Fork (game files settings): the Game files page as cards (`fork::rom_screens::settings_pages::menu_page`), every place
/// named the human way, the "only read" note as text instead of a row.
fn show_menu(app: &Rc<AppState>, router: &Router, store: RomLibraryStore) {
    let need = find_flow::need_rows(app, &store);
    let connected: std::collections::BTreeSet<&str> = store.mappings().iter().map(|m| m.port_key.as_str()).collect();
    let facts = settings_pages::MenuFacts {
        folder: store.rom_dir().map(|d| human_path(d, &Places::system())),
        looked: scan_time(&store).map(local_date),
        need_total: need.len(),
        need_missing: need.iter().filter(|r| !r.connected).count(),
        connected: connected.len(),
    };
    let page = settings_pages::menu_page(&facts);
    let act = move |app: &Rc<AppState>, router: &Router, act: GsAct| match act {
        GsAct::ChooseFolder => choose_folder(app, router, Then::Results),
        GsAct::LookAgain => start_scan(app, router, Then::Results),
        GsAct::WhatWasFound => show_saved_results(app, router),
        GsAct::NeedFiles => find_flow::show_ports_needing_files(app, router),
        GsAct::Connected => show_connections(app, router),
        _ => menu_back(app, router),
    };
    show_page(app, router, page, gs_handlers(act, menu_back));
}

// -- Choosing the folder --------------------------------------------------------------------------

/// The game files folder picker as every screen opens it: the usual ROM folders (SD card, EmuDeck) first as quick
/// places, and it opens in the EmuDeck ROM folder when there is one (Use this folder is then one press), else the home
/// folder.
pub(super) fn game_files_picker() -> PickerRequest {
    let home = home_dir();
    let drives = browse::system_drives();
    let start = browse::emudeck_folder(home.as_deref(), &drives).or_else(|| home.clone()).unwrap_or_else(|| PathBuf::from("/"));
    let mut request = PickerRequest::new("Where are your game files?", &start, Purpose::ScanFolder);
    request.extra_places = browse::start_points(home.as_deref(), &drives);
    request.extra_first = true;
    request
}

fn choose_folder(app: &Rc<AppState>, router: &Router, then: Then) {
    let request = game_files_picker();
    let on_cancel = then.clone();
    open_folder_picker(
        app,
        router,
        request,
        move |app, router, path| {
            let Some(mut store) = load_store(app, router) else { return };
            match store.set_rom_dir(Some(path)) {
                // `Then::Find` goes back to the find flow through `start_scan`.
                Ok(()) => start_scan(app, router, then.clone()),
                Err(e) if matches!(then, Then::Results) => notice(app, router, "Could not use that folder", &format!("{e}\n\nNothing was changed.")),
                Err(e) => message(app, router, TITLE, &format!("Could not use that folder: {e}")),
            }
        },
        // Cancel returns to where the picker was opened from: the Game files page or the find prompt.
        move |app, router| leave(app, router, &on_cancel),
    );
}

// -- Looking through the folder -------------------------------------------------------------------

fn start_scan(app: &Rc<AppState>, router: &Router, then: Then) {
    if let Then::Find { key, back, ask_how } = &then {
        return find_flow::resume(app, router, key, back, *ask_how);
    }
    let Some(store) = load_store(app, router) else { return };
    let Some(root) = store.rom_dir().map(Path::to_path_buf) else { return choose_folder(app, router, then) };
    let input = report_input(app, &store);
    let for_wizard = matches!(then, Then::Wizard);
    if !for_wizard {
        // B on the waiting page goes back to the Game files page, as Cancel does.
        ABANDON_HOOK.with(|h| *h.borrow_mut() = Some(Rc::new(menu)));
    }
    run_job(
        app,
        router,
        // The wizard's page; outside it a page with a Cancel card (`run_job`).
        JobUi::Cancellable { title: if for_wizard { "Scanning game files" } else { "Looking through your game files" } },
        // The wizard reads only what the installed ports could need; Settings reads everything (its pages show it all).
        move |ctl| scan_work::run_scan(ctl, &store, &root, &input, for_wizard),
        move |app, router, result| match result.and_then(|r| r) {
            Err(e) if for_wizard => message(app, router, TITLE, &e),
            Err(e) => notice(app, router, "Could not look through the folder", &e),
            Ok(None) => menu(app, router),
            // The wizard reports what the scan found for the ports that need files; no pages of its own.
            Ok(Some(out)) if for_wizard => {
                wizard_scan_done(app, out.report);
                super::setup_wizard_screen::resume_after_scan(app, router);
            }
            Ok(Some(out)) => after_scan(app, router, out),
        },
    );
}

fn after_scan(app: &Rc<AppState>, router: &Router, out: ScanOutput) {
    if out.scan.files.is_empty() {
        let text = format!(
            "Nothing in {} looks like a game file.\n\n{} other files were left alone. Game files are looked for up to 6 folders deep; folder names like n64, snes or gc help tell the console.",
            human_path(&out.report.root, &Places::system()),
            out.report.ignored_files
        );
        return notice(app, router, "No game files were found", &text);
    }
    let mut report = out.report;
    report.note = out.cache_error.map(|_| "This look could not be saved, so it runs again next time.".to_string());
    show_systems(app, router, Rc::new(report));
}

/// What the last look found, without looking again: the saved look matched to the ports as they are now.
fn show_saved_results(app: &Rc<AppState>, router: &Router) {
    reread(app, router, None);
}

/// Reads the saved look again (connections may have changed since) and shows what was found: by console, or the ports
/// of `system` when it is still there.
fn reread(app: &Rc<AppState>, router: &Router, system: Option<System>) {
    let Some(store) = load_store(app, router) else { return };
    let Some(root) = store.rom_dir().map(Path::to_path_buf) else { return choose_folder(app, router, Then::Results) };
    if !store.load_scan_cache().is_some_and(|c| c.root == root) {
        return start_scan(app, router, Then::Results);
    }
    let input = report_input(app, &store);
    run_job(
        app,
        router,
        JobUi::Page { title: "Reading the last look", cancellable: false },
        move |_| scan_work::run_report(&store, &root, &input),
        move |app, router, result| match result.and_then(|r| r) {
            Ok(out) => {
                let report = Rc::new(out.report);
                match system.and_then(|s| report.groups.iter().position(|g| g.system == s && !g.ports.is_empty())) {
                    Some(group) => show_system(app, router, report, group),
                    None => show_systems(app, router, report),
                }
            }
            Err(e) => notice(app, router, "Could not read the last look", &e),
        },
    );
}

// -- What was found -------------------------------------------------------------------------------

fn show_systems(app: &Rc<AppState>, router: &Router, report: Rc<Report>) {
    // Consoles with ports first, then the ones that only have files nobody uses.
    let mut order: Vec<usize> = (0..report.groups.len()).collect();
    order.sort_by_key(|i| report.groups[*i].ports.is_empty());
    let facts = settings_pages::FoundFacts {
        place: human_path(&report.root, &Places::system()),
        total: report.total_files,
        unused: report.unused_files,
        ignored: report.ignored_files,
        problems: report.problems,
        note: report.note.clone(),
        systems: order.iter().map(|i| settings_pages::SystemLine::of(&report.groups[*i])).collect(),
    };
    let act = move |app: &Rc<AppState>, router: &Router, act: GsAct| match act {
        GsAct::System(i) => {
            if let Some(&group) = order.get(i) {
                show_system(app, router, report.clone(), group);
            }
        }
        GsAct::LookAgain => start_scan(app, router, Then::Results),
        _ => menu(app, router),
    };
    show_page(app, router, settings_pages::found_page(&facts), gs_handlers(act, menu));
}

fn show_system(app: &Rc<AppState>, router: &Router, report: Rc<Report>, group: usize) {
    let g = &report.groups[group];
    let lines: Vec<settings_pages::PortLine> = g.ports.iter().map(|r| settings_pages::PortLine { name: r.port.display_name().to_string(), words: settings_pages::port_words(r) }).collect();
    let page = settings_pages::system_page(g.system.label(), g.files, &lines);
    let up = {
        let report = report.clone();
        move |app: &Rc<AppState>, router: &Router| show_systems(app, router, report.clone())
    };
    let up_too = up.clone();
    let act = move |app: &Rc<AppState>, router: &Router, act: GsAct| match act {
        GsAct::Port(i) => open_port(app, router, &report, group, i),
        _ => up(app, router),
    };
    show_page(app, router, page, gs_handlers(act, up_too));
}

/// One port of a console: its own game files pages (found, which one, how, connected), the same ones its Port page
/// shows. A port that is not installed only says so.
fn open_port(app: &Rc<AppState>, router: &Router, report: &Rc<Report>, group: usize, port: usize) {
    let Some(row) = report.groups.get(group).and_then(|g| g.ports.get(port)) else { return };
    let system = report.groups[group].system;
    // Back comes to this console's ports again, read anew (a port may have been connected meanwhile).
    let back: Back = Rc::new(move |app, router| reread(app, router, Some(system)));
    let name = row.port.display_name().to_string();
    if !row.installed {
        let text = format!("Your copy for {name} is here, but {name} is not installed.\n\nInstall it first, then connect its game files.");
        let (b1, b2, b3) = (back.clone(), back.clone(), back);
        let handlers = Handlers { on_action: Rc::new(move |app, router, _: GfAct| b1(app, router)), on_back: Rc::new(move |app, router| b2(app, router)), on_exit: Rc::new(move |app, router| b3(app, router)), on_input: None };
        show_page(app, router, crate::fork::rom_screens::pages::result_page(&name, "Install it first", &text), handlers);
        return;
    }
    find_flow::find_for_port_asking(app, router, row.port.key(), back);
}

// -- Connected ports ------------------------------------------------------------------------------

fn show_connections(app: &Rc<AppState>, router: &Router) {
    let Some(store) = load_store(app, router) else { return };
    let names = |key: &str| app.find_port(key).map(|p| p.display_name().to_string());
    let groups = Rc::new(connection_groups(store.mappings(), &names));
    let page = settings_pages::connected_page(&groups);
    let act = move |app: &Rc<AppState>, router: &Router, act: GsAct| match act {
        GsAct::Connection(i) => {
            if let Some(group) = groups.get(i) {
                ask_forget(app, router, group.clone());
            }
        }
        _ => menu(app, router),
    };
    show_page(app, router, page, gs_handlers(act, menu));
}

/// "Forget this connection?": only the record goes; the game files and what is in the port's folder stay.
fn ask_forget(app: &Rc<AppState>, router: &Router, group: ConnGroup) {
    let page = settings_pages::forget_page(&group);
    let act = move |app: &Rc<AppState>, router: &Router, act: GsAct| {
        if act == GsAct::Forget {
            let Some(mut store) = load_store(app, router) else { return };
            if let Err(e) = store.forget(&group.port_key, &group.slot) {
                return notice(app, router, "Could not forget it", &format!("{e}\n\nNothing was changed."));
            }
        }
        show_connections(app, router);
    };
    show_page(app, router, page, gs_handlers(act, show_connections));
}

