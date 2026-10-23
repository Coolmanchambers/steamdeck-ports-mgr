//! The "Game files" screens (Settings menu) and the search for a port's game files after it installs.
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
    build_picker_dialog, close_current_dialog, finish_dialog_open, open_confirm_dialog, open_message_dialog, open_picker_dialog, open_progress_dialog,
    set_picker_items, update_progress_dialog, DialogSlot,
};
use super::state::AppState;
use crate::core::path_safety::safe_join;
use crate::fork::rom_library::{Approval, RomLibraryStore};
use crate::fork::folder_picker::{PickerRequest, Purpose};
use crate::fork::rom_screens::browse;
use super::folder_picker_screen::open_folder_picker;
use crate::fork::rom_screens::connect_screen::{self, ConnectScreen, Line as ConnectLine, PlanOutput};
use crate::fork::rom_screens::connections::connection_groups;
use crate::fork::rom_screens::job::{Background, Ctl};
use crate::fork::rom_screens::report::{scan_time, PortRow, Report, ReportInput};
use crate::fork::rom_screens::scan_work::{self, ScanOutput};
use crate::fork::rom_screens::util::ellipsize;
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

// The "find this port's game files" flow (after an install, Info, Settings, and for other screens).
mod find_flow;
#[allow(unused_imports)]
pub(crate) use find_flow::{after_install, after_ports_added_then, find_for_port, open_from_info, port_needs_game_files};

/// What to do once a folder is chosen (and where Back goes).
#[derive(Clone)]
enum Then {
    Results,
    /// Look for this port's game files (`Port::key()`); `back` is where its flow returns.
    Find { key: String, back: Back },
}

/// Leave the folder chooser without choosing.
fn leave(app: &Rc<AppState>, router: &Router, then: &Then) {
    match then {
        Then::Results => open_game_files(app, router),
        Then::Find { key, back } => find_flow::resume(app, router, key, back),
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

/// A list that stays open (cursor where it was) when an information row is chosen. Rows that lead
/// somewhere replace it by opening the next dialog, which closes this one.
/// Every action row must open a dialog (which closes this one) or close it explicitly.
fn open_list(app: &Rc<AppState>, router: &Router, title: &str, labels: Vec<String>, on_select: impl Fn(&Rc<AppState>, &Router, usize) + 'static) {
    let Some(dialog) = build_picker_dialog(app, router, title, &labels) else { return };
    {
        let (app, router) = (app.clone(), router.clone());
        dialog.on_item_selected(move |index| on_select(&app, &router, index as usize));
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
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

thread_local! {
    /// Run when the user closes the progress list of the next job with B (the job is cancelled and its result
    /// dropped): lets a flow that waits for that job (the import screen's, in the setup wizard) still go on.
    /// Set right before `run_job`, taken when that job ends either way.
    static ABANDON_HOOK: RefCell<Option<Back>> = const { RefCell::new(None) };
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
        return message(app, router, TITLE, "Another game files task is still running. Wait for it to finish, then try again.");
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
            if let Some(dialog) = build_picker_dialog(app, router, title, &cancel_labels("Checked 99999 files, 999.9 GB read\nA long game file name that keeps going and going.iso", false)) {
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
    let abandoned = Cell::new(false);
    JOB_TIMER.with(|timer| {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
            if let Some(weak) = &picker {
                // The user closed the list with B: stop the work and drop its result. The timer keeps
                // running until the worker has really stopped, so a new job cannot overlap it.
                if !abandoned.get() && !weak.upgrade().is_some_and(|d| d.window().is_visible()) {
                    job.cancel();
                    abandoned.set(true);
                }
            }
            if let Some(result) = job.take() {
                stop_job_timer();
                let hook = ABANDON_HOOK.with(|h| h.borrow_mut().take());
                if abandoned.get() {
                    if let Some(hook) = hook {
                        hook(&app, &router);
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
            } else if let Some(d) = picker.as_ref().and_then(slint::Weak::upgrade) {
                set_picker_items(&d, &cancel_labels(&text, cancelling.get()));
            }
        });
    });
}

// -- Menu -----------------------------------------------------------------------------------------

/// Settings > Game files...
pub(crate) fn open_game_files(app: &Rc<AppState>, router: &Router) {
    // No "dialog already open" guard: the Back and Cancel rows of the deeper lists call this while their
    // own list is still up, and showing the menu replaces whatever is open.
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
        "Find game files for a port...".to_string(),
    ];
    open_list(app, router, TITLE, labels, move |app, router, index| match index {
        1 => choose_folder(app, router, Then::Results),
        2 => match store.rom_dir() {
            Some(_) => start_scan(app, router, Then::Results),
            None => choose_folder(app, router, Then::Results),
        },
        3 => show_saved_results(app, router),
        4 => show_connections(app, router),
        5 => find_flow::show_ports_needing_files(app, router),
        _ => {}
    });
}

fn connection_count(store: &RomLibraryStore) -> usize {
    connection_groups(store.mappings(), &|_| None).len()
}

// -- Choosing the folder --------------------------------------------------------------------------

fn choose_folder(app: &Rc<AppState>, router: &Router, then: Then) {
    // The shared folder picker, with the usual ROM folders (SD card, EmuDeck) as extra quick places.
    let home = home_dir();
    let mut request = PickerRequest::new("Where are your game files?", home.as_deref().unwrap_or(Path::new("/")), Purpose::ScanFolder);
    request.extra_places = browse::start_points(home.as_deref(), &browse::system_mounts());
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
                Err(e) => message(app, router, TITLE, &format!("Could not use that folder: {e}")),
            }
        },
        // Cancel returns to where the picker was opened from: the Game files menu or the find prompt.
        move |app, router| leave(app, router, &on_cancel),
    );
}

// -- Scanning -------------------------------------------------------------------------------------

fn start_scan(app: &Rc<AppState>, router: &Router, then: Then) {
    if let Then::Find { key, back } = &then {
        return find_flow::resume(app, router, key, back);
    }
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
            Ok(Some(out)) => after_scan(app, router, out),
        },
    );
}

fn after_scan(app: &Rc<AppState>, router: &Router, out: ScanOutput) {
    if out.scan.files.is_empty() {
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
        return;
    }
    let mut report = out.report;
    report.note = out.cache_error.map(|err| format!("This scan could not be saved ({}), so it will run again next time.", ellipsize(&err, 50)));
    show_systems(app, router, Rc::new(report));
}

fn show_saved_results(app: &Rc<AppState>, router: &Router) {
    let Some(store) = load_store(app, router) else { return };
    let Some(root) = store.rom_dir().map(Path::to_path_buf) else { return choose_folder(app, router, Then::Results) };
    if !store.load_scan_cache().is_some_and(|c| c.root == root) {
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
    if let Some(note) = &report.note {
        labels.push(note.clone());
    }
    let first_group = labels.len();
    labels.extend(order.iter().map(|i| format!("{}: {}", report.groups[*i].system.label(), report.groups[*i].summary())));
    let (rescan, connected) = (labels.len(), labels.len() + 1);
    labels.push("Scan the folder again".into());
    labels.push("Connected ports".into());
    labels.push("Back".into());
    open_list(app, router, "What's in your game folders", labels, move |app, router, index| {
        if index >= first_group && index < rescan {
            show_system(app, router, report.clone(), order[index - first_group]);
        } else if index == rescan {
            start_scan(app, router, Then::Results);
        } else if index == connected {
            show_connections(app, router);
        } else if index == connected + 1 {
            open_game_files(app, router);
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
    open_list(app, router, g.system.label(), labels, move |app, router, index| {
        if index >= 1 && index < back_index {
            show_port(app, router, report.clone(), group, index - 1);
        } else if index == back_index {
            show_systems(app, router, report.clone());
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
    open_list(app, router, &title, labels, move |app, router, index| {
        if index == connect_index && can_connect {
            let back: Back = {
                let report = report.clone();
                Rc::new(move |app, router| show_port(app, router, report.clone(), group, port))
            };
            open_connect(app, router, &report.groups[group].ports[port], back);
        } else if index == back_index {
            show_system(app, router, report.clone(), group);
        }
    });
}

// -- Connect --------------------------------------------------------------------------------------

fn open_connect(app: &Rc<AppState>, router: &Router, row: &PortRow, back: Back) {
    let Some(store) = load_store(app, router) else { return };
    let Ok(port_dir) = safe_join(&app.paths.library_dir(), &row.port.folder) else { return };
    let screen = Rc::new(RefCell::new(ConnectScreen::new(row, port_dir, store.mappings())));
    show_connect(app, router, screen, back);
}

fn show_connect(app: &Rc<AppState>, router: &Router, screen: Rc<RefCell<ConnectScreen>>, back: Back) {
    let title = format!("Connect game files: {}", screen.borrow().port_name);
    let labels = screen.borrow().labels();
    let Some(dialog) = build_picker_dialog(app, router, &title, &labels) else { return };
    {
        let (app, router) = (app.clone(), router.clone());
        dialog.on_item_selected(move |index| {
            let line = screen.borrow().lines().get(index as usize).copied();
            match line {
                Some(ConnectLine::Slot(i)) if screen.borrow().is_editable(i) => {
                    let (mut labels, picks) = (screen.borrow().pick_labels(i), screen.borrow().picks(i));
                    labels.push("Keep as it is".into());
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
    let unchecked = Rc::new(screen.borrow().unchecked_files());
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
                let (name2, plan2, unchecked2) = (name.clone(), plan.clone(), unchecked.clone());
                open_confirm_dialog(app, router, "Copy game files".into(), padded(&plan.confirm_text(&name)).into(), "Copy".into(), move |app, router| {
                    apply_now(app, router, &name2, plan2.clone(), Approval::CopyAllowed, unchecked2.clone());
                });
            } else {
                apply_now(app, router, &name, plan, Approval::None, unchecked);
            }
        },
    );
}

/// `unchecked`: files matched by name only or used anyway; the message says they were not checksum-checked.
fn apply_now(app: &Rc<AppState>, router: &Router, name: &str, plan: Rc<PlanOutput>, approval: Approval, unchecked: Rc<Vec<String>>) {
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
                for file in unchecked.iter() {
                    text.push_str(&format!("\n\n{file} was not checked against the catalog's checksum."));
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
    open_list(app, router, "Connected ports", labels, move |app, router, index| {
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
        }
    });
}

