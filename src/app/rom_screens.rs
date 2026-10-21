//! The "Game files" screens (Settings menu) and the offer after a port installs.
//!
//! Every screen is a list built from the picker dialog the app already has, so it works with the
//! controller, keyboard and mouse, in a window or fullscreen. What the lists say and do is decided in
//! `crate::fork::rom_screens` (tested without any UI); this file draws it, runs the slow parts on a
//! worker thread (a scan can read thousands of files, a copy gigabytes) and opens the next list. The
//! text is English only; it is not part of upstream's translation tables.
//!
//! Hard rules, kept: the game folder is only read, a copy is made only after the user confirms it
//! with its size shown, and forgetting a connection removes the record and no files.

use super::dialogs::{
    build_picker_dialog, close_current_dialog, dialog_is_open, finish_dialog_open, open_confirm_dialog, open_message_dialog, open_picker_dialog, open_progress_dialog,
    set_picker_items, update_progress_dialog, DialogSlot,
};
use super::state::AppState;
use crate::core::path_safety::safe_join;
use crate::fork::rom_library::{load_local_entries, Approval, RomLibraryStore, RomSpec};
use crate::fork::rom_screens::browse::{self, FolderBrowser};
use crate::fork::rom_screens::connect_screen::{self, ConnectScreen, Line as ConnectLine, PlanOutput};
use crate::fork::rom_screens::connections::connection_groups;
use crate::fork::rom_screens::job::{Background, Ctl};
use crate::fork::rom_screens::offer::{needs_game_files, offer_text};
use crate::fork::rom_screens::report::{scan_time, PortRow, Report, ReportInput};
use crate::fork::rom_screens::scan_work::{self, ScanOutput};
use crate::fork::rom_screens::util::ellipsize;
use crate::fork::settings::{ForkSettings, FILE_NAME};
use crate::ui::gamepad_router::GamepadRouter;
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

type Router = Rc<RefCell<GamepadRouter>>;
type Back = Rc<dyn Fn(&Rc<AppState>, &Router)>;

const TITLE: &str = "Game files";
const STORE_FILE: &str = "rom_library.json";
const NOTE_READ_ONLY: &str = "Your game files are only read, never moved or deleted.";

/// What to do once a scan has finished.
#[derive(Clone)]
enum Then {
    Results,
    /// Open the Connect step for this port (`Port::key()`).
    Connect(String),
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
    /// A list showing how far the job got, with a Cancel row. Closing it (B) cancels too.
    Cancellable { title: &'static str },
}

fn cancel_labels(progress: &str, cancelling: bool) -> Vec<String> {
    let (line1, line2) = progress.split_once('\n').unwrap_or((progress, ""));
    vec![
        if line1.is_empty() { "Starting...".to_string() } else { line1.to_string() },
        if line2.is_empty() { " ".to_string() } else { ellipsize(line2, 60) },
        if cancelling { "Cancelling...".to_string() } else { "Cancel (nothing is changed)".to_string() },
    ]
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
        return;
    }
    let job = Rc::new(Background::spawn(work));
    JOB_RUNNING.with(|r| r.set(true));
    let cancelling = Rc::new(Cell::new(false));
    let mut picker = None;
    let progress_mode = match &ui {
        JobUi::Progress { title, status } => {
            open_progress_dialog(app, router, title, status);
            true
        }
        JobUi::Cancellable { title } => {
            if let Some(dialog) = build_picker_dialog(app, router, title, &cancel_labels("", false)) {
                let (job, cancelling) = (job.clone(), cancelling.clone());
                let weak = dialog.as_weak();
                dialog.on_item_selected(move |index| {
                    if index == 2 {
                        job.cancel();
                        cancelling.set(true);
                        if let Some(d) = weak.upgrade() {
                            set_picker_items(&d, &cancel_labels(&job.progress(), true));
                        }
                    }
                });
                picker = Some(dialog.as_weak());
                let _ = dialog.show();
                finish_dialog_open(app, router, DialogSlot::Picker(dialog));
            }
            false
        }
    };

    let done: RefCell<Option<Box<dyn FnOnce(&Rc<AppState>, &Router, Result<T, String>)>>> = RefCell::new(Some(Box::new(done)));
    let (app, router) = (app.clone(), router.clone());
    let last = RefCell::new(String::new());
    JOB_TIMER.with(|timer| {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
            if let Some(result) = job.take() {
                stop_job_timer();
                close_current_dialog(&app, &router);
                if let Some(done) = done.borrow_mut().take() {
                    done(&app, &router, result);
                }
                return;
            }
            if let Some(weak) = &picker {
                // The user closed the list with B: stop the work and drop its result.
                if !weak.upgrade().is_some_and(|d| d.window().is_visible()) {
                    job.cancel();
                    stop_job_timer();
                    return;
                }
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
            } else if let Some(d) = picker.as_ref().and_then(slint::Weak::upgrade) {
                set_picker_items(&d, &cancel_labels(&text, cancelling.get()));
            }
        });
    });
}

// -- Menu -----------------------------------------------------------------------------------------

/// Settings > Game files...
pub(crate) fn open_game_files(app: &Rc<AppState>, router: &Router) {
    if dialog_is_open(app) {
        return;
    }
    let Some(store) = load_store(app, router) else { return };
    show_menu(app, router, store);
}

fn show_menu(app: &Rc<AppState>, router: &Router, store: RomLibraryStore) {
    let folder = match store.rom_dir() {
        Some(dir) => format!("Game files folder: {}", ellipsize(&dir.to_string_lossy(), 56)),
        None => "Game files folder: not set yet (choose one)".to_string(),
    };
    let scanned = scan_time(&store).map(|t| format!("scanned {}", local_date(t))).unwrap_or_else(|| "not scanned yet".into());
    let labels = vec![
        NOTE_READ_ONLY.to_string(),
        folder,
        "Scan the folder now".to_string(),
        format!("Results by system ({scanned})"),
        format!("Connected ports ({})", connection_count(&store)),
    ];
    open_picker_dialog(app, router, TITLE, labels, move |app, router, index| match index {
        1 => choose_folder(app, router, Then::Results),
        2 => match store.rom_dir() {
            Some(_) => start_scan(app, router, Then::Results),
            None => choose_folder(app, router, Then::Results),
        },
        3 => show_saved_results(app, router),
        4 => show_connections(app, router),
        _ => show_menu(app, router, store.clone()),
    });
}

fn connection_count(store: &RomLibraryStore) -> usize {
    connection_groups(store.mappings(), &|_| None).len()
}

// -- Choosing the folder --------------------------------------------------------------------------

fn choose_folder(app: &Rc<AppState>, router: &Router, then: Then) {
    let home = home_dir();
    let points = browse::start_points(home.as_deref(), &browse::system_mounts());
    let mut labels: Vec<String> = points.iter().map(|(label, path)| format!("{label}  ({})", ellipsize(&path.to_string_lossy(), 46))).collect();
    labels.push("Browse from your home folder".into());
    labels.push("Browse from the top of the drive".into());
    open_picker_dialog(app, router, "Where are your game files?", labels, move |app, router, index| {
        let dir = if let Some((_, path)) = points.get(index) {
            path.clone()
        } else if index == points.len() {
            home.clone().unwrap_or_else(|| PathBuf::from("/"))
        } else {
            PathBuf::from("/")
        };
        show_browser(app, router, Rc::new(RefCell::new(FolderBrowser::open(&dir))), then.clone());
    });
}

fn browser_title(browser: &FolderBrowser) -> String {
    format!("Folder: {}", ellipsize(&browser.current.to_string_lossy(), 48))
}

fn show_browser(app: &Rc<AppState>, router: &Router, browser: Rc<RefCell<FolderBrowser>>, then: Then) {
    let labels = browser.borrow().labels();
    let Some(dialog) = build_picker_dialog(app, router, &browser_title(&browser.borrow()), &labels) else { return };
    {
        let (app, router, weak) = (app.clone(), router.clone(), dialog.as_weak());
        dialog.on_item_selected(move |index| {
            let line = browser.borrow().lines().get(index as usize).copied();
            match line {
                Some(browse::Line::UseThis) => {
                    let path = browser.borrow().current.clone();
                    let Some(mut store) = load_store(&app, &router) else { return };
                    match store.set_rom_dir(Some(path)) {
                        Ok(()) => start_scan(&app, &router, then.clone()),
                        Err(e) => message(&app, &router, TITLE, &format!("Could not use that folder: {e}")),
                    }
                    return;
                }
                Some(browse::Line::Up) => browser.borrow_mut().up(),
                Some(browse::Line::Folder(i)) => browser.borrow_mut().enter(i),
                Some(browse::Line::More) | None => return,
            }
            if let Some(d) = weak.upgrade() {
                set_picker_items(&d, &browser.borrow().labels());
                d.set_dialog_title(browser_title(&browser.borrow()).into());
                app.dialog_nav.picker_index.set(0);
                d.set_selected_index(0);
                d.set_scroll_trigger(!d.get_scroll_trigger());
            }
        });
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
}

// -- Scanning -------------------------------------------------------------------------------------

fn start_scan(app: &Rc<AppState>, router: &Router, then: Then) {
    let Some(store) = load_store(app, router) else { return };
    let Some(root) = store.rom_dir().map(Path::to_path_buf) else { return choose_folder(app, router, then) };
    let input = report_input(app, &store);
    run_job(
        app,
        router,
        JobUi::Cancellable { title: "Scanning game files" },
        move |ctl| scan_work::run_scan(ctl, &store, &root, &input),
        move |app, router, result| match result.and_then(|r| r) {
            Err(e) => message(app, router, TITLE, &e),
            Ok(None) => open_game_files(app, router),
            Ok(Some(out)) => after_scan(app, router, out, then),
        },
    );
}

fn after_scan(app: &Rc<AppState>, router: &Router, out: ScanOutput, then: Then) {
    if let Some(err) = &out.cache_error {
        message(app, router, TITLE, &format!("The scan worked, but its result could not be saved ({err}), so it will run again next time."));
    }
    if out.scan.files.is_empty() {
        if out.cache_error.is_none() {
            message(
                app,
                router,
                TITLE,
                &format!(
                    "No game files were found in {}.\n\n{} other files were ignored. Game files are looked for up to 6 folders deep, with folder names like n64, snes or gc helping to tell the console.",
                    out.report.root.display(),
                    out.report.ignored_files
                ),
            );
        }
        return;
    }
    if out.cache_error.is_some() {
        return;
    }
    match then {
        Then::Results => show_systems(app, router, Rc::new(out.report)),
        Then::Connect(key) => connect_from_report(app, router, &Rc::new(out.report), &key),
    }
}

fn show_saved_results(app: &Rc<AppState>, router: &Router) {
    let Some(store) = load_store(app, router) else { return };
    let Some(root) = store.rom_dir().map(Path::to_path_buf) else { return choose_folder(app, router, Then::Results) };
    if !store.scan_cache_path().exists() {
        return start_scan(app, router, Then::Results);
    }
    let input = report_input(app, &store);
    run_job(
        app,
        router,
        JobUi::Progress { title: TITLE, status: "Matching saved scan to ports...".into() },
        move |_| scan_work::run_report(&store, &root, &input),
        move |app, router, result| match result.and_then(|r| r) {
            Ok(out) => show_systems(app, router, Rc::new(out.report)),
            Err(e) => message(app, router, TITLE, &e),
        },
    );
}

// -- Results --------------------------------------------------------------------------------------

fn show_systems(app: &Rc<AppState>, router: &Router, report: Rc<Report>) {
    // Systems with ports first, then the ones that only have files nobody uses.
    let mut order: Vec<usize> = (0..report.groups.len()).collect();
    order.sort_by_key(|i| report.groups[*i].ports.is_empty());
    let mut labels = vec![
        format!("{} game files in {}", report.total_files, ellipsize(&report.root.to_string_lossy(), 40)),
        format!("{} not used by any port, {} other files ignored", report.unused_files, report.ignored_files),
    ];
    if report.problems > 0 {
        labels.push(format!("{} folders could not be read and were skipped.", report.problems));
    }
    let first_group = labels.len();
    labels.extend(order.iter().map(|i| format!("{}: {}", report.groups[*i].system.label(), report.groups[*i].summary())));
    let (rescan, connected) = (labels.len(), labels.len() + 1);
    labels.push("Scan the folder again".into());
    labels.push("Connected ports".into());
    open_picker_dialog(app, router, "What's in your game folders", labels, move |app, router, index| {
        if index >= first_group && index < rescan {
            show_system(app, router, report.clone(), order[index - first_group]);
        } else if index == rescan {
            start_scan(app, router, Then::Results);
        } else if index == connected {
            show_connections(app, router);
        } else {
            show_systems(app, router, report.clone());
        }
    });
}

fn show_system(app: &Rc<AppState>, router: &Router, report: Rc<Report>, group: usize) {
    let g = &report.groups[group];
    let mut labels = vec![format!("{}: {}", g.system.label(), g.summary())];
    labels.extend(g.ports.iter().map(|r| {
        let installed = if r.installed { "" } else { " (not installed)" };
        format!("{} - {}{installed}", r.port.display_name(), r.chip())
    }));
    let back_index = labels.len();
    labels.push("Back".into());
    open_picker_dialog(app, router, g.system.label(), labels, move |app, router, index| {
        if index >= 1 && index < back_index {
            show_port(app, router, report.clone(), group, index - 1);
        } else if index == back_index {
            show_systems(app, router, report.clone());
        } else {
            show_system(app, router, report.clone(), group);
        }
    });
}

fn show_port(app: &Rc<AppState>, router: &Router, report: Rc<Report>, group: usize, port: usize) {
    let row = &report.groups[group].ports[port];
    let mut labels = vec![format!("{} - {}", row.port.display_name(), row.chip())];
    labels.extend(row.detail_lines());
    let connect_index = labels.len();
    labels.push(if row.can_connect() {
        "Connect game files...".to_string()
    } else if !row.installed {
        format!("Install {} first, then connect its game files", row.port.display_name())
    } else {
        "Nothing to connect yet (no matching file was found)".to_string()
    });
    let back_index = labels.len();
    labels.push("Back".into());
    let can_connect = row.can_connect();
    let title = row.port.display_name().to_string();
    open_picker_dialog(app, router, &title, labels, move |app, router, index| {
        if index == connect_index && can_connect {
            let back: Back = {
                let report = report.clone();
                Rc::new(move |app, router| show_port(app, router, report.clone(), group, port))
            };
            open_connect(app, router, &report.groups[group].ports[port], back);
        } else if index == back_index {
            show_system(app, router, report.clone(), group);
        } else {
            show_port(app, router, report.clone(), group, port);
        }
    });
}

// -- Connect --------------------------------------------------------------------------------------

fn connect_from_report(app: &Rc<AppState>, router: &Router, report: &Rc<Report>, key: &str) {
    let row = report.groups.iter().flat_map(|g| g.ports.iter()).find(|r| r.port.key() == key);
    match row {
        Some(row) if row.can_connect() => {
            let back: Back = Rc::new(|app, router| open_game_files(app, router));
            open_connect(app, router, row, back);
        }
        Some(row) => {
            let mut text = format!("None of the files {} needs were found in {}.\n", row.port.display_name(), report.root.display());
            for line in row.detail_lines().iter().take(4) {
                text.push_str(&format!("\n  {line}"));
            }
            text.push_str("\n\nAdd the files to your game folder, then use Settings > Game files > Scan the folder now.");
            message(app, router, &format!("Game files for {}", row.port.display_name()), &text);
        }
        None => message(app, router, TITLE, "That port does not need game files, or it is not installed."),
    }
}

fn open_connect(app: &Rc<AppState>, router: &Router, row: &PortRow, back: Back) {
    let Some(store) = load_store(app, router) else { return };
    let Ok(port_dir) = safe_join(&app.paths.library_dir, &row.port.folder) else { return };
    let screen = Rc::new(RefCell::new(ConnectScreen::new(row, port_dir, store.mappings())));
    show_connect(app, router, screen, back);
}

fn show_connect(app: &Rc<AppState>, router: &Router, screen: Rc<RefCell<ConnectScreen>>, back: Back) {
    let title = format!("Connect game files: {}", screen.borrow().port_name);
    let labels = screen.borrow().labels();
    let Some(dialog) = build_picker_dialog(app, router, &title, &labels) else { return };
    {
        let (app, router, weak) = (app.clone(), router.clone(), dialog.as_weak());
        dialog.on_item_selected(move |index| {
            let line = screen.borrow().lines().get(index as usize).copied();
            match line {
                Some(ConnectLine::Slot(i)) if screen.borrow().is_editable(i) => {
                    let (labels, picks) = (screen.borrow().pick_labels(i), screen.borrow().picks(i));
                    let (screen, back) = (screen.clone(), back.clone());
                    open_picker_dialog(&app, &router, "Change this file", labels, move |app, router, choice| {
                        if let Some(pick) = picks.get(choice) {
                            screen.borrow_mut().choose(i, *pick);
                        }
                        show_connect(app, router, screen.clone(), back.clone());
                    });
                    return;
                }
                Some(ConnectLine::Connect) => return connect_now(&app, &router, screen.clone()),
                Some(ConnectLine::Back) => return back(&app, &router),
                _ => {}
            }
            if let Some(d) = weak.upgrade() {
                set_picker_items(&d, &screen.borrow().labels());
            }
        });
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
}

fn connect_now(app: &Rc<AppState>, router: &Router, screen: Rc<RefCell<ConnectScreen>>) {
    let requests = match screen.borrow().requests() {
        Ok(r) => r,
        Err(text) => return message(app, router, TITLE, &text),
    };
    let name = screen.borrow().port_name.clone();
    run_job(
        app,
        router,
        JobUi::Progress { title: TITLE, status: "Checking the files...".into() },
        move |_| connect_screen::make_plan(requests),
        move |app, router, result| {
            let plan = match result.and_then(|r| r) {
                Ok(plan) => Rc::new(plan),
                Err(e) => return message(app, router, TITLE, &format!("Nothing was connected: {e}")),
            };
            if plan.needs_copy_approval() {
                let (name2, plan2) = (name.clone(), plan.clone());
                open_confirm_dialog(app, router, "Copy game files".into(), padded(&plan.confirm_text(&name)).into(), "Copy".into(), move |app, router| {
                    apply_now(app, router, &name2, plan2.clone(), Approval::CopyAllowed);
                });
            } else {
                apply_now(app, router, &name, plan, Approval::None);
            }
        },
    );
}

fn apply_now(app: &Rc<AppState>, router: &Router, name: &str, plan: Rc<PlanOutput>, approval: Approval) {
    let status = if approval == Approval::CopyAllowed { "Copying game files. This can take a while and cannot be cancelled." } else { "Connecting game files..." };
    let (path, plans) = (store_path(app), plan.plans.clone());
    let name = name.to_string();
    run_job(
        app,
        router,
        JobUi::Progress { title: TITLE, status: status.into() },
        move |_| connect_screen::apply_plans(&path, &plans, approval),
        move |app, router, result| match result.and_then(|r| r) {
            Ok(mappings) => {
                let mut text = connect_screen::done_text(&name, &plan.plans, &mappings);
                for w in plan.warnings() {
                    text.push_str(&format!("\n\nNote: {w}"));
                }
                message(app, router, "Game files connected", &text);
            }
            Err(e) => message(app, router, TITLE, &format!("Nothing was connected: {e}")),
        },
    );
}

// -- Connected ports ------------------------------------------------------------------------------

fn show_connections(app: &Rc<AppState>, router: &Router) {
    let Some(store) = load_store(app, router) else { return };
    let names = |key: &str| app.find_port(key).map(|p| p.display_name().to_string());
    let groups = connection_groups(store.mappings(), &names);
    let mut labels: Vec<String> = groups.iter().map(|g| g.label.clone()).collect();
    if groups.is_empty() {
        labels.push("No ports are connected yet.".into());
    }
    let back_index = labels.len();
    labels.push("Back".into());
    open_picker_dialog(app, router, "Connected ports", labels, move |app, router, index| {
        if let Some(group) = groups.get(index) {
            let (key, slot) = (group.port_key.clone(), group.slot.clone());
            let text = format!(
                "{}\n\nForget this connection? Only the record is removed. The game files, links and copies stay where they are; this app never deletes game files.",
                group.label
            );
            open_confirm_dialog(app, router, "Forget connection".into(), padded(&text).into(), "Forget".into(), move |app, router| {
                if let Some(mut store) = load_store(app, router) {
                    if let Err(e) = store.forget(&key, &slot) {
                        return message(app, router, TITLE, &format!("Could not update the record: {e}"));
                    }
                }
                show_connections(app, router);
            });
        } else if index == back_index {
            open_game_files(app, router);
        } else {
            show_connections(app, router);
        }
    });
}

// -- After a port installs -------------------------------------------------------------------------

/// Offered once per port, right after it installs, when the port names files it cannot run without.
/// Declining is final for that port; Settings > Game files stays available.
pub(crate) fn offer_after_install(app: &Rc<AppState>, router: &Router, key: &str) {
    if dialog_is_open(app) {
        return;
    }
    let Some(port) = app.find_port(key) else { return };
    if port.is_android_only() {
        return;
    }
    let settings_path = app.paths.config_dir.join(FILE_NAME);
    let mut settings = ForkSettings::load(&settings_path);
    if settings.rom_prompted(key) {
        return;
    }
    let local = load_local_entries(&app.paths.config_dir.join("ports.local.json"));
    let spec = RomSpec::for_port(&port, local.get(&port.folder));
    if !needs_game_files(&spec) {
        return;
    }
    let _ = settings.mark_rom_prompted(key);
    let key = key.to_string();
    open_confirm_dialog(
        app,
        router,
        format!("Game files for {}", port.display_name()).into(),
        padded(&offer_text(port.display_name(), &spec)).into(),
        "Connect".into(),
        move |app, router| connect_port(app, router, &key),
    );
}

fn connect_port(app: &Rc<AppState>, router: &Router, key: &str) {
    let Some(store) = load_store(app, router) else { return };
    match store.rom_dir() {
        None => choose_folder(app, router, Then::Connect(key.to_string())),
        Some(_) => start_scan(app, router, Then::Connect(key.to_string())),
    }
}
