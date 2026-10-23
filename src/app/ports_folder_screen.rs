//! Fork: Settings > Ports folder..., the "ports folder is not available" message at startup, the move
//! of installed ports to a new ports folder, and the old-copy prompts. The rules live in
//! `crate::fork::ports_folder` and the engine in `crate::fork::port_mover` (both tested without UI);
//! this file draws lists and runs the copies on a worker thread.
//!
//! For other screens:
//! - `refuse_when_blocked(app)`: true (and tells the user) while the ports folder is missing or a
//!   move is running; install paths call it.
//! - `prompt_old_copies(app, router, copies, summary)`: the "Remove the old copy now?" prompts that
//!   follow a move (stage 2's import calls it after its own moves).
//! - `open_old_copies(app, router)`: the list of old copies kept after moves.
//! - `choose_folder(app, router)` and `consider(app, router, folder)`: the whole ports folder change
//!   (picker, checks, create, move, switch, old-copy prompts), for the setup wizard.

use super::dialogs::{build_picker_dialog, close_current_dialog, finish_dialog_open, open_confirm_dialog, open_message_dialog, open_progress_dialog, set_picker_items, update_progress_dialog, DialogSlot};
use super::folder_picker_screen::open_folder_picker;
use super::state::AppState;
use crate::core::models::Port;
use crate::fork::folder_picker::mounts::{default_media_roots, system_mounts};
use crate::fork::folder_picker::{PickerRequest, Purpose};
use crate::fork::port_mover::game_files::{check_before, GameFiles, Issue};
use crate::fork::port_mover::old_copies::{self, OldCopy, RemovalCtx};
use crate::fork::port_mover::{self, MoveEnv, MoveError, MoveOptions, PortRef, Prepared, Stage};
use crate::fork::ports_folder::{self, CheckCtx, MovePlan, PortToMove, Startup, SystemProbe};
use crate::fork::rom_screens::job::Background;
use crate::fork::rom_screens::util::{ellipsize, free_bytes, human_bytes};
use crate::fork::settings::{ForkSettings, FILE_NAME};
use crate::fork::store::SystemSteamCheck;
use crate::ui::gamepad_router::GamepadRouter;
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

type Router = Rc<RefCell<GamepadRouter>>;

const TITLE: &str = "Ports folder";

thread_local! {
    // Set at startup, so install paths (which only have the app state) can show a message.
    static ROUTER: RefCell<Option<(std::rc::Weak<AppState>, Router)>> = const { RefCell::new(None) };
    static MOVING: Cell<bool> = const { Cell::new(false) };
    static JOB_TIMER: slint::Timer = slint::Timer::default();
}

fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"].iter().find_map(|v| std::env::var_os(v)).map(PathBuf::from).filter(|p| p.is_absolute())
}

fn settings_path(app: &AppState) -> PathBuf {
    app.paths.config_dir.join(FILE_NAME)
}

fn message(app: &Rc<AppState>, router: &Router, text: &str) {
    open_message_dialog(app, router, TITLE, &format!("{text}\n"));
}

/// A list whose rows act through `on_select`; information rows do nothing.
fn open_list(app: &Rc<AppState>, router: &Router, title: &str, labels: Vec<String>, on_select: impl Fn(&Rc<AppState>, &Router, usize) + 'static) -> Option<crate::ListPickerDialog> {
    let dialog = build_picker_dialog(app, router, title, &labels)?;
    {
        let (app, router) = (app.clone(), router.clone());
        dialog.on_item_selected(move |index| on_select(&app, &router, index as usize));
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog.clone_strong()));
    Some(dialog)
}

fn is_default(app: &AppState, dir: &Path) -> bool {
    dir == ports_folder::default_dir(&app.paths.config_dir)
}

/// Why the ports folder in use cannot be used right now, if it cannot.
fn unavailable(app: &AppState) -> Option<String> {
    let dir = app.paths.library_dir();
    if is_default(app, &dir) {
        // The default folder next to the app is simply made again, as upstream always did.
        let _ = std::fs::create_dir_all(&dir);
        return None;
    }
    ports_folder::availability(&dir, &system_mounts(), &default_media_roots()).map(|why| ports_folder::missing_text(&dir, &why))
}

/// Install paths call this: true (with a message) while the ports folder is missing or being moved.
pub(crate) fn refuse_when_blocked(app: &AppState) -> bool {
    let why = if MOVING.with(Cell::get) {
        Some("Installed ports are being moved to a new ports folder. Wait for that to finish, then try again.".to_string())
    } else {
        unavailable(app).map(|text| format!("{text}\nInstalls wait until it is back. To use another folder: Settings > Ports folder."))
    };
    let Some(why) = why else { return false };
    if let Some((app, router)) = ROUTER.with(|r| r.borrow().clone()) {
        // After the current event: the caller may be inside a dialog callback.
        slint::Timer::single_shot(Duration::ZERO, move || {
            if let Some(app) = app.upgrade() {
                message(&app, &router, &why);
            }
        });
    }
    true
}

/// Marks ports found in the ports folder as installed (as startup does) and redraws.
fn after_library_change(app: &AppState) {
    let dir = app.paths.library_dir();
    {
        let mut s = app.state.borrow_mut();
        for port in app.catalog.borrow().iter() {
            if s.get(port.key()).is_none() && crate::core::installer::is_installed(port, &dir) {
                s.mark_installed(port.key(), None);
            }
        }
    }
    app.refresh_current_view();
    app.refresh_tray_recent_games();
}

// -- Startup --------------------------------------------------------------------------------------

pub(crate) fn startup_check(app: &Rc<AppState>, router: &Router, startup: Startup) {
    ROUTER.with(|r| *r.borrow_mut() = Some((Rc::downgrade(app), router.clone())));
    eprintln!("ports folder: {} ({})", startup.dir.display(), if startup.available() { "available" } else { "NOT available" });
    if startup.available() {
        return;
    }
    let (app, router) = (app.clone(), router.clone());
    slint::Timer::single_shot(Duration::from_millis(800), move || show_missing(&app, &router, false));
}

fn show_missing(app: &Rc<AppState>, router: &Router, again: bool) {
    let dir = app.paths.library_dir();
    let Some(why) = ports_folder::availability(&dir, &system_mounts(), &default_media_roots()) else {
        after_library_change(app);
        if again {
            message(app, router, &format!("Found it: {} is back.", dir.display()));
        }
        return;
    };
    let question = match why {
        ports_folder::Missing::DriveNotMounted(_) => "Is the SD card in?",
        ports_folder::Missing::FolderGone => "Was it moved, renamed or deleted?",
        ports_folder::Missing::NotAFolder => "Something that is not a folder has its name.",
    };
    let mut labels = vec![
        if again { "Still not there. Your ports folder:".to_string() } else { "Your ports folder is not available:".to_string() },
        format!("  {}", ellipsize(&dir.to_string_lossy(), 70)),
        question.to_string(),
        "Try again".to_string(),
        "Choose another folder...".to_string(),
        "Continue without it (installs wait until it is back)".to_string(),
    ];
    labels.push("Quit".to_string());
    let Some(dialog) = open_list(app, router, "Ports folder not available", labels, |app, router, index| match index {
        3 => show_missing(app, router, true),
        4 => choose_folder(app, router),
        5 => close_current_dialog(app, router),
        6 => {
            close_current_dialog(app, router);
            let _ = slint::quit_event_loop();
        }
        _ => {}
    }) else {
        return;
    };
    dialog.set_selected_index(3);
    app.dialog_nav.picker_index.set(3);
    let (app, router) = (app.clone(), router.clone());
    dialog.on_close_requested(move || close_current_dialog(&app, &router));
}

// -- The menu -------------------------------------------------------------------------------------

/// Settings > Ports folder...
pub(crate) fn open_menu(app: &Rc<AppState>, router: &Router) {
    let dir = app.paths.library_dir();
    let state = match unavailable(app) {
        Some(_) => "Not available now (SD card out?)".to_string(),
        None => {
            let mounts = system_mounts();
            let mut parts = Vec::new();
            if let Some(free) = free_bytes(&dir) {
                parts.push(format!("{} free", human_bytes(free)));
            }
            if let Some(t) = crate::fork::folder_picker::mounts::fs_type_of(&dir, &mounts) {
                parts.push(t);
            }
            if let Some(d) = crate::fork::folder_picker::mounts::drive_of(&dir, &mounts, &default_media_roots()) {
                parts.push(d.title());
            }
            if parts.is_empty() { "Available".to_string() } else { parts.join(", ") }
        }
    };
    let old = old_copies::load(&app.paths.config_dir).unwrap_or_default();
    let mut labels = vec![
        format!("Ports folder: {}", ellipsize(&dir.to_string_lossy(), 64)),
        format!("  {state}{}", if is_default(app, &dir) { " (default, next to the app)" } else { "" }),
        "Choose another folder...".to_string(),
        "Open the ports folder".to_string(),
    ];
    let has_old = !old.is_empty();
    if has_old {
        let bytes: u64 = old.iter().map(|o| o.bytes).sum();
        labels.push(format!("Old copies kept after moves ({}, {})...", old.len(), human_bytes(bytes)));
    }
    let not_default = !is_default(app, &dir);
    if not_default {
        labels.push(format!("Use the default folder ({})", ellipsize(&ports_folder::default_dir(&app.paths.config_dir).to_string_lossy(), 48)));
    }
    labels.push("Back".to_string());
    let back = labels.len() - 1;
    let old_row = has_old.then_some(4);
    let default_row = not_default.then(|| if has_old { 5 } else { 4 });
    open_list(app, router, TITLE, labels, move |app, router, index| {
        if index == 2 {
            choose_folder(app, router);
        } else if index == 3 {
            crate::core::launch::open_folder(&app.paths.library_dir());
        } else if Some(index) == old_row {
            open_old_copies(app, router);
        } else if Some(index) == default_row {
            consider(app, router, ports_folder::default_dir(&app.paths.config_dir));
        } else if index == back {
            close_current_dialog(app, router);
            super::dialogs::open_settings_dialog(app, router);
        }
    });
}

/// The picker for a new ports folder, then the whole change (check, create, move, switch, old-copy prompts).
/// Cancel returns to the Ports folder list. The setup wizard can call this.
pub(crate) fn choose_folder(app: &Rc<AppState>, router: &Router) {
    let current = app.paths.library_dir();
    // The default folder sits inside the app folder: start at home instead of among the app's own files.
    let start = if current.is_dir() && !is_default(app, &current) {
        current.parent().map(Path::to_path_buf).unwrap_or(current)
    } else {
        home_dir().unwrap_or_else(|| PathBuf::from("/"))
    };
    open_folder_picker(app, router, PickerRequest::new("Choose the ports folder", &start, Purpose::PortsFolder), consider, open_menu);
}

/// The user picked `dir`: check it, create it if needed (asked first), then plan the switch. For callers
/// that already have a folder (the setup wizard): the same flow as Settings > Ports folder.
pub(crate) fn consider(app: &Rc<AppState>, router: &Router, dir: PathBuf) {
    let current = app.paths.library_dir();
    let (mounts, roots, home) = (system_mounts(), default_media_roots(), home_dir());
    let ctx = CheckCtx::system(&app.paths.config_dir, &current, home.as_deref(), &mounts, &roots);
    let check = ports_folder::check(&dir, &ctx, &SystemProbe);
    if check.same_as_current {
        return message(app, router, &format!("{} is already the ports folder.", dir.display()));
    }
    if !check.ok() {
        let text = format!("{}\n\n{}", dir.display(), check.problems.join("\n\n"));
        return open_confirm_dialog(app, router, "Can't use this folder".into(), format!("{text}\n").into(), "Choose another folder".into(), |app, router| choose_folder(app, router));
    }
    if !check.exists {
        let text = format!("{} does not exist yet.\n\nCreate it and use it as the ports folder? ({})\n", dir.display(), check.summary());
        return open_confirm_dialog(app, router, "Create the folder?".into(), text.into(), "Create it".into(), move |app, router| match ports_folder::create(&dir, &system_mounts(), &default_media_roots()) {
            Ok(()) => consider(app, router, dir.clone()),
            Err(e) => message(app, router, &e),
        });
    }
    plan_switch(app, router, dir, check.summary(), check.free);
}

// -- Switching ------------------------------------------------------------------------------------

fn plan_switch(app: &Rc<AppState>, router: &Router, new_dir: PathBuf, summary: String, free: Option<u64>) {
    let old_dir = app.paths.library_dir();
    let plan = if old_dir.is_dir() { ports_folder::plan_move(&old_dir, &new_dir, &app.catalog.borrow()) } else { MovePlan::default() };
    let n = plan.ports.len();
    let needed = plan.bytes.saturating_add(port_mover::SPACE_MARGIN);
    let fits = free.is_none_or(|f| f >= needed);
    let mut labels = vec![format!("New ports folder: {}", ellipsize(&new_dir.to_string_lossy(), 60)), format!("  {summary}")];
    if !plan.already_there.is_empty() {
        labels.push(format!("Already in the new folder (used from there): {}", plan.already_there.join(", ")));
    }
    if !plan.other_folders.is_empty() {
        labels.push(format!("Not ports, never moved: {}", ellipsize(&plan.other_folders.join(", "), 60)));
    }
    let first_action = labels.len();
    if n == 0 {
        labels.push("Switch to this folder".to_string());
    } else if fits {
        labels.push(format!("Move {n} port{} here ({}), then switch (the old copies are kept)", if n == 1 { "" } else { "s" }, human_bytes(plan.bytes)));
    } else {
        labels.push(format!("Not enough space to move the ports: needs {}, {} free", human_bytes(needed), human_bytes(free.unwrap_or(0))));
    }
    if n > 0 {
        labels.push(format!("Switch without moving ({n} port{} stay in the old folder and show as not installed)", if n == 1 { "" } else { "s" }));
    }
    labels.push("Cancel".to_string());
    let cancel = labels.len() - 1;
    let plan = Rc::new(plan);
    let Some(dialog) = open_list(app, router, "Switch the ports folder?", labels, move |app, router, index| {
        if index == cancel {
            open_menu(app, router);
        } else if index == first_action && n == 0 {
            switch_now(app, router, &new_dir, Vec::new(), String::new());
        } else if index == first_action && fits {
            check_game_files_then_move(app, router, new_dir.clone(), plan.clone());
        } else if index == first_action + 1 && n > 0 {
            switch_now(app, router, &new_dir, Vec::new(), format!("{n} port(s) stay in the old folder and show as not installed here."));
        }
    }) else {
        return;
    };
    dialog.set_selected_index(first_action as i32);
    app.dialog_nav.picker_index.set(first_action as i32);
}

fn find_port(app: &AppState, key: &str) -> Option<Port> {
    app.catalog.borrow().iter().find(|p| p.key() == key).cloned()
}

fn issue_lines(issues: &[(String, Issue)]) -> Vec<String> {
    // Short rows; choosing one shows the whole text.
    let mut lines: Vec<String> = issues.iter().take(8).map(|(name, i)| format!("{name}: {}: {}", i.file, i.problem.split(" (").next().unwrap_or(&i.problem))).collect();
    if issues.len() > 8 {
        lines.push(format!("... and {} more", issues.len() - 8));
    }
    lines
}

/// The game file check before anything is copied; problems are shown with Fix / Move anyway / Cancel.
fn check_game_files_then_move(app: &Rc<AppState>, router: &Router, new_dir: PathBuf, plan: Rc<MovePlan>) {
    let mut issues: Vec<(String, Issue)> = Vec::new();
    for p in &plan.ports {
        let files = find_port(app, &p.key).map(|port| GameFiles::for_port(&port, &app.paths.config_dir)).unwrap_or_default();
        issues.extend(check_before(&p.from, &files).into_iter().map(|i| (p.name.clone(), i)));
    }
    if issues.is_empty() {
        return start_move(app, router, new_dir, plan, false);
    }
    let mut labels = vec![format!("{} problem(s) with game files, found before copying:", issues.len())];
    labels.extend(issue_lines(&issues).into_iter().map(|l| format!("  {l}")));
    let fix = labels.len();
    let details: Vec<String> = issues.iter().map(|(name, i)| format!("{name}\n\n{}", i.line())).collect();
    labels.push("Fix in Game files (nothing is moved)".to_string());
    labels.push("Move anyway".to_string());
    labels.push("Cancel".to_string());
    let Some(dialog) = open_list(app, router, "Check the game files", labels, move |app, router, index| {
        if index > 0 && index < fix {
            if let Some(text) = details.get(index - 1) {
                message(app, router, text);
            }
        } else if index == fix {
            super::rom_screens::open_game_files(app, router);
        } else if index == fix + 1 {
            start_move(app, router, new_dir.clone(), plan.clone(), true);
        } else if index == fix + 2 {
            open_menu(app, router);
        }
    }) else {
        return;
    };
    dialog.set_selected_index(fix as i32);
    app.dialog_nav.picker_index.set(fix as i32);
}

/// What the worker brings back for one port.
enum PortResult {
    Moved(Prepared),
    Failed { name: String, text: String, after_copy_game_files: bool },
}

struct MoveRun {
    results: Vec<PortResult>,
    cancelled: bool,
}

fn move_env_parts(app: &AppState) -> (PathBuf, PathBuf) {
    (home_dir().unwrap_or_else(|| PathBuf::from("/")), app.paths.config_dir.clone())
}

fn running_in(dir: &Path) -> Vec<String> {
    crate::fork::save_restore::running::processes_under(&[dir.to_path_buf()]).iter().map(|f| f.describe()).collect()
}

fn free_space(dir: &Path) -> Option<u64> {
    free_bytes(dir)
}

fn start_move(app: &Rc<AppState>, router: &Router, new_dir: PathBuf, plan: Rc<MovePlan>, accept_issues: bool) {
    if MOVING.with(Cell::get) {
        return message(app, router, "A move is already running.");
    }
    if !app.install_runtime.installing.borrow().is_empty() {
        return message(app, router, "An install, update or save restore is running. Wait for it to finish, then try again.");
    }
    if super::playtime::any_process_running(app) {
        return message(app, router, "A game started from this app is running. Close it first, then try again.");
    }
    let keys: Vec<String> = plan.ports.iter().map(|p| p.key.clone()).collect();
    MOVING.with(|m| m.set(true));
    for k in &keys {
        app.install_runtime.installing.borrow_mut().insert(k.clone());
    }
    let jobs: Vec<(PortToMove, GameFiles)> =
        plan.ports.iter().map(|p| (p.clone(), find_port(app, &p.key).map(|port| GameFiles::for_port(&port, &app.paths.config_dir)).unwrap_or_default())).collect();
    let (home, config_dir) = move_env_parts(app);
    let total_ports = jobs.len();
    let job = Rc::new(Background::spawn(move |ctl| {
        let settings = ForkSettings::load(&config_dir.join(FILE_NAME));
        let steam = SystemSteamCheck::new();
        let env = MoveEnv { home: &home, config_dir: &config_dir, settings: &settings, steam: &steam, game_mode: false, running: &running_in, free_space: &free_space };
        let mut run = MoveRun { results: Vec::new(), cancelled: false };
        for (i, (p, files)) in jobs.into_iter().enumerate() {
            if ctl.cancelled() {
                run.cancelled = true;
                break;
            }
            let port = PortRef { key: p.key.clone(), name: p.name.clone() };
            let options = MoveOptions { game_files: files, accept_issues_before: accept_issues, ..Default::default() };
            let mut progress = |done: u64, total: u64, _: &Path| ctl.set_progress(format!("Copying {} of {total_ports}: {}\n{} of {}", i + 1, p.name, human_bytes(done), human_bytes(total)));
            ctl.set_progress(format!("Copying {} of {total_ports}: {}\nStarting...", i + 1, p.name));
            match port_mover::prepare(&port, &p.from, &p.to, &env, &options, &mut progress, ctl.flag()) {
                Ok(prepared) => run.results.push(PortResult::Moved(prepared)),
                Err(MoveError::Cancelled) => {
                    run.cancelled = true;
                    break;
                }
                Err(e) => {
                    let after = matches!(e, MoveError::GameFiles { stage: Stage::AfterCopy, .. });
                    run.results.push(PortResult::Failed { name: p.name.clone(), text: e.to_string(), after_copy_game_files: after });
                }
            }
        }
        run
    }));

    // A list showing progress with a Cancel row; B cancels too.
    let labels = |progress: &str, cancelling: bool| -> Vec<String> {
        let (a, b) = progress.split_once('\n').unwrap_or((progress, ""));
        vec![
            if a.is_empty() { "Starting...".into() } else { a.to_string() },
            if b.is_empty() { " ".into() } else { b.to_string() },
            if cancelling { "Cancelling...".into() } else { "Cancel (the copies made so far are removed)".into() },
        ]
    };
    let cancelling = Rc::new(Cell::new(false));
    let list = build_picker_dialog(app, router, "Moving ports", &labels("Copying 99 of 99: A port name that is quite long\n999.9 GB of 999.9 GB", false));
    let list_weak = list.as_ref().map(|d| d.as_weak());
    if let Some(dialog) = list {
        set_picker_items(&dialog, &labels("", false));
        dialog.set_selected_index(2);
        app.dialog_nav.picker_index.set(2);
        {
            let (job, cancelling, weak) = (job.clone(), cancelling.clone(), dialog.as_weak());
            dialog.on_item_selected(move |index| {
                if index == 2 {
                    job.cancel();
                    cancelling.set(true);
                    if let Some(d) = weak.upgrade() {
                        set_picker_items(&d, &labels(&job.progress(), true));
                    }
                }
            });
        }
        {
            let (job, cancelling) = (job.clone(), cancelling.clone());
            dialog.on_close_requested(move || {
                job.cancel();
                cancelling.set(true);
            });
        }
        let _ = dialog.show();
        finish_dialog_open(app, router, DialogSlot::Picker(dialog));
    }
    let (app, router) = (app.clone(), router.clone());
    let last = RefCell::new(String::new());
    JOB_TIMER.with(|timer| {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
            if let Some(result) = job.take() {
                JOB_TIMER.with(|t| t.stop());
                MOVING.with(|m| m.set(false));
                for k in &keys {
                    app.install_runtime.installing.borrow_mut().remove(k);
                }
                close_current_dialog(&app, &router);
                return match result {
                    Ok(run) => after_move(&app, &router, &new_dir, run),
                    Err(e) => message(&app, &router, &e),
                };
            }
            let text = job.progress();
            if text != *last.borrow() {
                *last.borrow_mut() = text.clone();
                if let Some(d) = list_weak.as_ref().and_then(slint::Weak::upgrade) {
                    set_picker_items(&d, &labels(&text, cancelling.get()));
                }
            }
        });
    });
}

fn discard_all(prepared: &[Prepared]) -> Vec<String> {
    prepared.iter().filter_map(|p| port_mover::discard(p).err().map(|e| format!("Could not remove the new copy {}: {e}", p.to.display()))).collect()
}

fn after_move(app: &Rc<AppState>, router: &Router, new_dir: &Path, run: MoveRun) {
    let mut moved: Vec<Prepared> = Vec::new();
    let mut failed: Vec<(String, String, bool)> = Vec::new();
    for r in run.results {
        match r {
            PortResult::Moved(p) => moved.push(p),
            PortResult::Failed { name, text, after_copy_game_files } => failed.push((name, text, after_copy_game_files)),
        }
    }
    if run.cancelled {
        let problems = discard_all(&moved);
        let mut text = format!("Move cancelled. Nothing changed: the ports folder is still {}, and the copies made so far were removed.", app.paths.library_dir().display());
        for p in problems {
            text.push_str(&format!("\n{p}"));
        }
        return message(app, router, &text);
    }
    if failed.is_empty() {
        return switch_now(app, router, new_dir, moved, String::new());
    }
    let mut labels = vec![format!("{} of {} ports could not be moved:", failed.len(), failed.len() + moved.len())];
    for (name, text, _) in &failed {
        labels.push(format!("  {name}: {}", ellipsize(&text.replace('\n', " "), 90)));
    }
    let game_files_problem = failed.iter().any(|(_, _, g)| *g);
    let first = labels.len();
    if game_files_problem {
        labels.push("Fix in Game files (nothing changes; the new copies are removed)".to_string());
    }
    if !moved.is_empty() {
        labels.push(format!("Switch anyway ({} moved; the others stay in the old folder and show as not installed)", moved.len()));
    }
    labels.push("Cancel (nothing changes; the new copies are removed)".to_string());
    let details: Vec<String> = failed.iter().map(|(n, t, _)| format!("{n}: {t}")).collect();
    let moved = Rc::new(moved);
    let new_dir = new_dir.to_path_buf();
    let n_rows = labels.len();
    let Some(dialog) = open_list(app, router, "Some ports were not moved", labels, move |app, router, index| {
        let fix_row = game_files_problem.then_some(first);
        let switch_row = (!moved.is_empty()).then(|| first + usize::from(game_files_problem));
        if index > 0 && index < first {
            // A failure row: show its whole text.
            if let Some(text) = details.get(index - 1) {
                message(app, router, text);
            }
        } else if Some(index) == fix_row {
            let problems = discard_all(&moved);
            super::rom_screens::open_game_files(app, router);
            if !problems.is_empty() {
                eprintln!("ports folder: {}", problems.join("; "));
            }
        } else if Some(index) == switch_row {
            switch_now(app, router, &new_dir, moved.as_ref().clone(), "Some ports were not moved and stay in the old folder.".into());
        } else if index == n_rows - 1 {
            let problems = discard_all(&moved);
            let mut text = "Nothing changed. The new copies were removed; the originals are untouched.".to_string();
            for p in problems {
                text.push_str(&format!("\n{p}"));
            }
            message(app, router, &text);
        }
    }) else {
        return;
    };
    dialog.set_selected_index(first as i32);
    app.dialog_nav.picker_index.set(first as i32);
}

/// Makes `new_dir` the ports folder, then repoints Steam and records for the moved ports, then asks
/// about each old copy.
fn switch_now(app: &Rc<AppState>, router: &Router, new_dir: &Path, moved: Vec<Prepared>, note: String) {
    let mut settings = ForkSettings::load(&settings_path(app));
    if let Err(e) = ports_folder::save_choice(&mut settings, &app.paths.config_dir, new_dir) {
        let problems = discard_all(&moved);
        let mut text = format!("The new ports folder could not be saved ({e}). Nothing changed; the new copies were removed.");
        for p in problems {
            text.push_str(&format!("\n{p}"));
        }
        return message(app, router, &text);
    }
    let old_dir = app.paths.library_dir();
    *app.paths.library.borrow_mut() = new_dir.to_path_buf();
    eprintln!("ports folder: switched from {} to {}", old_dir.display(), new_dir.display());
    after_library_change(app);

    let (home, config_dir) = move_env_parts(app);
    let steam = SystemSteamCheck::new();
    let env = MoveEnv {
        home: &home,
        config_dir: &config_dir,
        settings: &settings,
        steam: &steam,
        game_mode: crate::fork::steam_queue::game_mode_from_env(),
        running: &running_in,
        free_space: &free_space,
    };
    let mut summary = vec![format!("The ports folder is now {}.", new_dir.display())];
    if !moved.is_empty() {
        let bytes: u64 = moved.iter().map(|p| p.bytes).sum();
        summary.push(format!("Moved {} port(s), {} copied and checked.", moved.len(), human_bytes(bytes)));
    }
    let mut copies = VecDeque::new();
    for p in &moved {
        let report = port_mover::switch_over(p, &env);
        let text = report.text();
        if !text.is_empty() {
            summary.push(format!("{}:\n{text}", p.port.name));
        }
        // Information only: named by the port's entry, but neither connected nor in its folder (often needed
        // only once, at install time).
        if let Some(port) = find_port(app, &p.port.key) {
            let missing = crate::fork::port_mover::game_files::required_not_found(&p.to, &GameFiles::for_port(&port, &config_dir));
            if !missing.is_empty() {
                summary.push(format!(
                    "{}: for information, its entry names {} which is neither in its folder nor connected in Game files (fine if it was only needed when installing).",
                    p.port.name,
                    missing.join(", ")
                ));
            }
        }
        if !p.accepted_issues.is_empty() {
            summary.push(format!("{}: moved although the game file check found: {}.", p.port.name, p.accepted_issues.iter().map(|i| i.file.clone()).collect::<Vec<_>>().join(", ")));
        }
        for link in &p.rewritten_links {
            summary.push(format!("{}: the game file link {} now holds a full path, so it keeps working.", p.port.name, link.display()));
        }
        if let Some(entry) = old_copies::load(&config_dir).unwrap_or_default().into_iter().find(|o| o.old_path == p.from) {
            copies.push_back(entry);
        }
    }
    if !note.is_empty() {
        summary.push(note);
    }
    prompt_old_copies(app, router, copies, summary.join("\n\n"));
}

// -- Old copies -----------------------------------------------------------------------------------

fn removal_ctx_parts(app: &AppState) -> (PathBuf, PathBuf, PathBuf, Vec<PathBuf>) {
    let library = app.paths.library_dir();
    let active = app.catalog.borrow().iter().filter_map(|p| crate::core::path_safety::safe_join(&library, &p.folder).ok()).collect();
    (home_dir().unwrap_or_else(|| PathBuf::from("/")), app.paths.config_dir.clone(), library, active)
}

fn check_removal(app: &AppState, entry: &OldCopy) -> Result<u64, String> {
    let (home, config_dir, library, active) = removal_ctx_parts(app);
    old_copies::removal_check(entry, &RemovalCtx { home: &home, config_dir: &config_dir, library_dir: &library, active_paths: &active, running: &running_in })
}

/// "Remove the old copy now?" for each entry in turn, then `summary` (with what was removed or kept).
/// Nothing is removed without the user choosing "Remove old copy" for that entry.
pub(crate) fn prompt_old_copies(app: &Rc<AppState>, router: &Router, mut copies: VecDeque<OldCopy>, summary: String) {
    let Some(entry) = copies.pop_front() else {
        if !summary.is_empty() {
            message(app, router, &summary);
        }
        return;
    };
    let check = check_removal(app, &entry);
    let size = match &check {
        Ok(bytes) if entry.link_only => format!("A link to another folder ({} stays)", human_bytes(*bytes)),
        Ok(bytes) => human_bytes(*bytes),
        Err(_) => human_bytes(entry.bytes),
    };
    let mut labels = vec![
        entry.name.clone(),
        format!("Old copy: {}", ellipsize(&entry.old_path.to_string_lossy(), 70)),
        format!("Size: {size}"),
        format!("In use now: {}", ellipsize(&entry.new_path.to_string_lossy(), 66)),
    ];
    let can_remove = check.is_ok();
    match &check {
        Ok(_) => labels.push("Remove old copy (its saves are zipped first)".to_string()),
        Err(why) => labels.push(format!("Can't remove it now: {}", ellipsize(why, 90))),
    }
    labels.push("Keep for now".to_string());
    let why = check.err();
    let copies = Rc::new(RefCell::new(Some(copies)));
    let summary = Rc::new(summary);
    let Some(dialog) = open_list(app, router, "Remove the old copy now?", labels, move |app, router, index| {
        let rest = || copies.borrow_mut().take().unwrap_or_default();
        match index {
            4 if can_remove => remove_one(app, router, entry.clone(), rest(), summary.as_ref().clone()),
            4 => {
                if let Some(why) = &why {
                    message(app, router, why);
                }
            }
            5 => {
                let note = format!("{}\n\nKept the old copy of {} at {} (remove it later: Settings > Ports folder > Old copies).", summary, entry.name, entry.old_path.display());
                prompt_old_copies(app, router, rest(), note.trim_start().to_string());
            }
            _ => {}
        }
    }) else {
        return;
    };
    let select = if can_remove { 4 } else { 5 };
    dialog.set_selected_index(select);
    app.dialog_nav.picker_index.set(select);
}

fn remove_one(app: &Rc<AppState>, router: &Router, entry: OldCopy, rest: VecDeque<OldCopy>, summary: String) {
    let (home, config_dir, library, active) = removal_ctx_parts(app);
    let port = find_port(app, &entry.port_key);
    let saves_dir = app.paths.saves_backup_dir.clone();
    open_progress_dialog(app, router, "Removing the old copy", &format!("Removing {}...", entry.old_path.display()));
    let job = Rc::new(Background::spawn(move |_| {
        let backup = |e: &OldCopy| -> Result<Option<PathBuf>, String> {
            match &port {
                Some(port) => crate::core::save_backup::backup_before_update(port, &saves_dir, &e.old_path, &crate::fork::store::backup_stamp(), &mut |_| {}),
                None => Ok(None),
            }
        };
        let ctx = RemovalCtx { home: &home, config_dir: &config_dir, library_dir: &library, active_paths: &active, running: &running_in };
        old_copies::remove(&entry, &ctx, &backup).map(|r| (entry, r))
    }));
    let (app, router) = (app.clone(), router.clone());
    let rest = RefCell::new(Some(rest));
    JOB_TIMER.with(|timer| {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
            let Some(result) = job.take() else {
                if let DialogSlot::Progress(d) = &*app.dialog_nav.dialogs.borrow() {
                    update_progress_dialog(&app, d, "Removing the old copy...");
                }
                return;
            };
            JOB_TIMER.with(|t| t.stop());
            close_current_dialog(&app, &router);
            let line = match result {
                Ok(Ok((entry, removed))) => {
                    let zip = removed.saves_zip.map(|z| format!(" Its saves were zipped to {} first.", z.display())).unwrap_or_default();
                    format!("Removed the old copy of {} ({} freed).{zip}", entry.name, human_bytes(removed.bytes))
                }
                Ok(Err(e)) | Err(e) => e,
            };
            let summary = format!("{summary}\n\n{line}");
            prompt_old_copies(&app, &router, rest.borrow_mut().take().unwrap_or_default(), summary.trim_start().to_string());
        });
    });
}

/// The list of old copies kept after moves (Settings > Ports folder > Old copies).
pub(crate) fn open_old_copies(app: &Rc<AppState>, router: &Router) {
    let list = match old_copies::load(&app.paths.config_dir) {
        Ok(list) => list,
        Err(e) => return message(app, router, &e),
    };
    let mut labels: Vec<String> = list.iter().map(|o| format!("{}: {} ({})", o.name, ellipsize(&o.old_path.to_string_lossy(), 56), human_bytes(o.bytes))).collect();
    labels.push("Back".to_string());
    let list = Rc::new(list);
    open_list(app, router, "Old copies kept after moves", labels, move |app, router, index| match list.get(index) {
        Some(entry) => {
            // An old copy that is already gone: offer to forget the record.
            if std::fs::symlink_metadata(&entry.old_path).is_err() {
                let path = entry.old_path.clone();
                return open_confirm_dialog(
                    app,
                    router,
                    "Old copy already gone".into(),
                    format!("{} is not there any more.\n\nForget this record?\n", path.display()).into(),
                    "Forget it".into(),
                    move |app, router| {
                        let _ = old_copies::forget(&app.paths.config_dir, &path);
                        open_old_copies(app, router);
                    },
                );
            }
            prompt_old_copies(app, router, VecDeque::from([entry.clone()]), String::new());
        }
        None => open_menu(app, router),
    });
}
