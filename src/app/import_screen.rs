//! "Import ports..." (Settings menu, and step c of the setup wizard): finds ports installed outside the
//! ports folder and moves them in. It replaces "Adopt installed ports".
//!
//! Every screen is a list built from the picker dialog the app already has, so it works with the
//! controller, keyboard and mouse. What the lists say and do is decided in `crate::fork::import` (tested
//! without any UI); this file draws it and runs the slow parts (the scan, the import) on a worker thread
//! with a Cancel row. The text is English only; it is not part of upstream's translation tables.
//!
//! Entry point for the setup wizard: `open(app, router, from_wizard)`, or `open_with` to be called back
//! when the user chooses "Continue setup" (or finishes an import).

use super::dialogs::{
    build_picker_dialog, close_current_dialog, dialog_is_open, finish_dialog_open, open_confirm_dialog, open_message_dialog, open_picker_dialog, set_picker_items,
    DialogSlot,
};
use super::state::AppState;
use crate::fork::import::engine::{engine_for_import, move_selected, ImportSummary, NoPrompts};
use crate::fork::import::fallback;
use crate::fork::import::folders::{AddOutcome, ScanFolders};
use crate::fork::import::plan::{build_requests, plan_all, steam_write_mode, PortPlan};
use crate::fork::import::scan::{read_shortcuts, run_scan, ScanInput};
use crate::fork::import::screen::{confirm_text, plan_labels, plan_text, FolderLine, FoldersScreen, ImportLine, ImportScreen};
use crate::fork::import::{picker, ports_folder};
use crate::fork::rom_screens::browse::{FolderBrowser, Line as BrowseLine};
use crate::fork::rom_screens::job::{Background, Ctl};
use crate::fork::rom_screens::util::{ellipsize, free_bytes};
use crate::fork::screens::short_path;
use crate::fork::settings::{ForkSettings, FILE_NAME};
use crate::fork::steam_queue;
use crate::fork::store::SystemSteamCheck;
use crate::ui::gamepad_router::GamepadRouter;
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
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
    let Some(folders) = load_folders(app) else { return };
    start_scan(app, router, &Ctx { from_wizard, finish }, folders);
}

fn load_folders(app: &AppState) -> Option<ScanFolders> {
    Some(ScanFolders::load(&ForkSettings::load(&settings_path(app))))
}

// -- Background jobs ---------------------------------------------------------------------------------

thread_local! {
    static JOB_TIMER: slint::Timer = slint::Timer::default();
    static JOB_RUNNING: Cell<bool> = const { Cell::new(false) };
}

fn stop_job_timer() {
    JOB_TIMER.with(|t| t.stop());
    JOB_RUNNING.with(|r| r.set(false));
}

fn job_rows(text: &str, cancel_label: &str) -> Vec<String> {
    let mut rows: Vec<String> = text.lines().filter(|l| !l.is_empty()).map(|l| ellipsize(l, 80)).collect();
    rows.push(cancel_label.to_string());
    rows
}

/// Runs `work` on a worker thread behind a list of progress rows (one per line of the worker's progress
/// text) and a Cancel row. `initial` is shown until the worker says more. Closing the list (B) cancels;
/// the result is then dropped unless `deliver_after_close` (an import must still report what it did).
#[allow(clippy::too_many_arguments)]
fn run_job<T: Send + 'static>(
    app: &Rc<AppState>,
    router: &Router,
    title: &'static str,
    initial: &str,
    cancel_label: &'static str,
    deliver_after_close: bool,
    work: impl FnOnce(&Ctl) -> T + Send + 'static,
    done: impl FnOnce(&Rc<AppState>, &Router, Result<T, String>) + 'static,
) {
    if JOB_RUNNING.with(Cell::get) {
        return message(app, router, TITLE, "A scan or an import is still running. Wait for it to finish, then try again.");
    }
    let Some(dialog) = build_picker_dialog(app, router, title, &job_rows(initial, cancel_label)) else { return };
    let job = Rc::new(Background::spawn(work));
    JOB_RUNNING.with(|r| r.set(true));
    let cancelling = Rc::new(Cell::new(false));
    let cancel_row = Rc::new(Cell::new(job_rows(initial, cancel_label).len() - 1));
    let rows_text = Rc::new(RefCell::new(initial.to_string()));
    {
        let (job, cancelling, cancel_row, rows_text) = (job.clone(), cancelling.clone(), cancel_row.clone(), rows_text.clone());
        let weak = dialog.as_weak();
        dialog.on_item_selected(move |index| {
            if index as usize == cancel_row.get() && !cancelling.get() {
                job.cancel();
                cancelling.set(true);
                if let Some(d) = weak.upgrade() {
                    set_picker_items(&d, &job_rows(&rows_text.borrow(), "Cancelling (stops after the current step)..."));
                }
            }
        });
    }
    let weak = dialog.as_weak();
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));

    let done: RefCell<Option<Box<dyn FnOnce(&Rc<AppState>, &Router, Result<T, String>)>>> = RefCell::new(Some(Box::new(done)));
    let (app, router) = (app.clone(), router.clone());
    let closed = Cell::new(false);
    JOB_TIMER.with(|timer| {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
            if !closed.get() && !weak.upgrade().is_some_and(|d| d.window().is_visible()) {
                job.cancel();
                closed.set(true);
            }
            if let Some(result) = job.take() {
                stop_job_timer();
                if closed.get() && !deliver_after_close {
                    return;
                }
                if let Some(done) = done.borrow_mut().take() {
                    done(&app, &router, result);
                }
                return;
            }
            if closed.get() {
                return;
            }
            let text = job.progress();
            if text.is_empty() || text == *rows_text.borrow() {
                return;
            }
            *rows_text.borrow_mut() = text.clone();
            cancel_row.set(job_rows(&text, cancel_label).len() - 1);
            if let Some(d) = weak.upgrade() {
                let label = if cancelling.get() { "Cancelling (stops after the current step)..." } else { cancel_label };
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
    let ports = ports_folder(&app.paths.library_dir);
    let roots = folders.roots(&home, &ports);
    let catalog = app.catalog.borrow().clone();
    let (work_home, work_ports, deeper) = (home.clone(), ports.clone(), folders.look_deeper);
    let ctx = ctx.clone();
    run_job(
        app,
        router,
        "Looking for ports",
        "Looking in your scan folders...",
        "Cancel",
        false,
        move |ctl| {
            let shortcuts = read_shortcuts(&work_home, &settings);
            let input = ScanInput { roots, catalog, library_dir: work_ports, deeper, shortcuts };
            run_scan(&input, &|| ctl.cancelled(), &|t| ctl.set_progress(t))
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

type Picked = Rc<dyn Fn(&Rc<AppState>, &Router, Option<PathBuf>)>;

fn add_folder(app: &Rc<AppState>, router: &Router, ctx: &Ctx, state: Rc<FoldersState>) {
    let ctx = ctx.clone();
    let picked: Picked = Rc::new(move |app, router, choice| {
        if let Some(path) = choice {
            let home = home_dir().unwrap_or_default();
            let mut screen = state.screen.borrow_mut();
            let outcome = screen.folders.add_user(&path, &home);
            screen.note = match &outcome {
                AddOutcome::Added => save_folders(app, &screen.folders).or_else(|| Some(format!("Added {}", short_path(&path, &home)))),
                AddOutcome::DefaultTurnedOn(name) => save_folders(app, &screen.folders).or_else(|| Some(format!("~/{name} is back on")))
                ,
                AddOutcome::AlreadyListed => Some("That folder is already in the list".to_string()),
                AddOutcome::NotAFolder => Some("That is not a folder".to_string()),
            };
            if matches!(outcome, AddOutcome::Added | AddOutcome::DefaultTurnedOn(_)) {
                state.changed.set(true);
            }
        }
        show_folders(app, router, &ctx, state.clone());
    });
    pick_folder(app, router, picked);
}

// -- Folder picker (an adapter: see fork::import::picker) ---------------------------------------------------

fn pick_folder(app: &Rc<AppState>, router: &Router, picked: Picked) {
    let home = home_dir();
    let points = picker::device_start_points(home.as_deref());
    let mut labels: Vec<String> = points.iter().map(|(label, path)| format!("{label}  ({})", ellipsize(&path.to_string_lossy(), 46))).collect();
    labels.push("Browse from your home folder".into());
    labels.push("Browse from the top of the drive".into());
    labels.push("Cancel".into());
    open_picker_dialog(app, router, "Which folder holds ports?", labels, move |app, router, index| {
        if index == points.len() + 2 {
            return picked(app, router, None);
        }
        let dir = if let Some((_, path)) = points.get(index) {
            path.clone()
        } else if index == points.len() {
            home.clone().unwrap_or_else(|| PathBuf::from("/"))
        } else {
            PathBuf::from("/")
        };
        show_browser(app, router, Rc::new(RefCell::new(FolderBrowser::open(&dir))), picked.clone());
    });
}

fn show_browser(app: &Rc<AppState>, router: &Router, browser: Rc<RefCell<FolderBrowser>>, picked: Picked) {
    let labels = picker::browser_labels(&browser.borrow());
    let title = format!("Folder: {}", ellipsize(&browser.borrow().current.to_string_lossy(), 30));
    let Some(dialog) = build_picker_dialog(app, router, &title, &labels) else { return };
    {
        let (app, router) = (app.clone(), router.clone());
        dialog.on_item_selected(move |index| {
            let line = browser.borrow().lines().get(index as usize).copied();
            match line {
                Some(BrowseLine::UseThis) => {
                    let path = browser.borrow().current.clone();
                    return picked(&app, &router, Some(path));
                }
                Some(BrowseLine::Up) => browser.borrow_mut().up(),
                Some(BrowseLine::Folder(i)) => browser.borrow_mut().enter(i),
                Some(BrowseLine::Cancel) => return picked(&app, &router, None),
                Some(BrowseLine::More) | None => return,
            }
            show_browser(&app, &router, browser.clone(), picked.clone());
        });
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
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
    let (plans, free) = build_plans(&screen.borrow());
    let dry_run = engine_for_import().is_dry_run();
    let (home, ports) = (screen.borrow().home.clone(), screen.borrow().ports_folder.clone());
    let text = padded(&confirm_text(&plans, &ports, &home, free, dry_run));
    if plans.iter().all(|p| !p.can_run()) {
        return message(app, router, TITLE, &text);
    }
    let label = if dry_run { "Show preview" } else { "Import" };
    let ctx = ctx.clone();
    open_confirm_dialog(app, router, TITLE.into(), text.into(), label.into(), move |app, router| run_import(app, router, &ctx, plans.clone(), home.clone()));
}

fn run_import(app: &Rc<AppState>, router: &Router, ctx: &Ctx, plans: Vec<PortPlan>, home: PathBuf) {
    let engine = engine_for_import();
    let initial = plans.iter().map(|p| format!("{}: waiting", p.request.port_name)).collect::<Vec<_>>().join("\n");
    let ctx = ctx.clone();
    run_job(
        app,
        router,
        if engine.is_dry_run() { "Preparing the preview" } else { "Importing ports" },
        &initial,
        "Cancel (stops after the current port)",
        true,
        move |ctl| move_selected(engine.as_ref(), plans, &|| ctl.cancelled(), &|t| ctl.set_progress(t), &NoPrompts),
        move |app, router, result| match result {
            Ok(summary) => show_result(app, router, &ctx, Rc::new(summary), &home),
            Err(e) => {
                // The worker panicked: nothing more is known, so say that and leave the old folders alone.
                message(app, router, "Import problem", &format!("The import stopped: {e}\nThe old folders were not changed by the app."));
            }
        },
    );
}

fn show_result(app: &Rc<AppState>, router: &Router, ctx: &Ctx, summary: Rc<ImportSummary>, home: &std::path::Path) {
    app.refresh_current_view();
    let mut text = summary.text(home);
    let candidates = summary.link_candidates();
    if !candidates.is_empty() {
        text.push_str(&format!(
            "\n\n{} port{} did not fit. They can be linked where they are instead (nothing moves).",
            candidates.len(),
            if candidates.len() == 1 { "" } else { "s" }
        ));
        let requests: Vec<_> = candidates.iter().map(|&i| summary.items[i].plan.request.clone()).collect();
        let ctx = ctx.clone();
        return open_confirm_dialog(app, router, summary.title().into(), padded(&text).into(), "Link instead...".into(), move |app, router| {
            confirm_link(app, router, &ctx, requests.clone());
        });
    }
    if ctx.from_wizard {
        let ctx = ctx.clone();
        return open_confirm_dialog(app, router, summary.title().into(), padded(&text).into(), "Continue setup".into(), move |app, router| ctx.finish(app, router));
    }
    message(app, router, summary.title(), &text);
}

fn confirm_link(app: &Rc<AppState>, router: &Router, ctx: &Ctx, requests: Vec<crate::fork::import::plan::MoveRequest>) {
    let refs: Vec<_> = requests.iter().collect();
    let text = padded(&fallback::confirm_text(&refs));
    let ctx = ctx.clone();
    open_confirm_dialog(app, router, TITLE.into(), text.into(), "Link".into(), move |app, router| {
        let local_json = app.paths.config_dir.join("ports.local.json");
        let catalog = app.catalog.borrow().clone();
        let refs: Vec<_> = requests.iter().collect();
        let done = fallback::link_instead(&refs, &catalog, &app.paths.library_dir, &local_json);
        for port in done.linked {
            app.state.borrow_mut().mark_installed(port.key(), None);
            let mut catalog = app.catalog.borrow_mut();
            let old = std::mem::take(&mut *catalog);
            *catalog = crate::core::config::merge_local_catalog(old, vec![port]);
        }
        app.refresh_current_view();
        if ctx.from_wizard {
            let ctx = ctx.clone();
            return open_confirm_dialog(app, router, TITLE.into(), padded(&done.text).into(), "Continue setup".into(), move |app, router| ctx.finish(app, router));
        }
        message(app, router, TITLE, &done.text);
    });
}
