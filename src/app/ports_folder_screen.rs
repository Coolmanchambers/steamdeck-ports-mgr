//! Fork: Settings > Ports folder..., the "ports folder is not available" message at startup, the move
//! of installed ports to a new ports folder, and the old-copy prompts. The rules live in
//! `crate::fork::ports_folder`, the engine in `crate::fork::port_mover` and the lists in
//! `crate::fork::ports_folder_view` (all tested without UI); this file draws them and runs the copies
//! on a worker thread.
//!
//! For other screens:
//! - `refuse_when_blocked(app)`: true (and tells the user) while the ports folder is missing or a
//!   move is running; installs, adopting and restores call it. `blocked_reason(app)` without the message.
//! - `startup_dialog_pending()`: true while the "not available" message is due or showing; other
//!   startup dialogs wait for it.
//! - `quit_waits_for_move(app, router)`: Close and Quit ask it; a running move is stopped first.
//! - `prompt_old_copies_batch_then(app, router, copies, summary, then)`: the one page about old copies
//!   (`batch.rs`): after an import, after a move here, after "Finish switching", and for one copy chosen in
//!   Old copies. Each port has a status line; only Ready ones can be removed.
//! - `show_old_copies_then_summary`: that page followed by the closing message.
//! - `open_old_copies(app, router)`: the list of old copies kept after moves.
//! - `choose_folder(app, router)` and `consider(app, router, folder)`: the whole ports folder change
//!   (picker, checks, create, move, switch, old-copy prompts).

use super::dialogs::{build_picker_dialog, close_current_dialog, finish_dialog_open, open_confirm_dialog, open_message_dialog, set_picker_items, DialogSlot};
use super::folder_picker_screen::open_folder_picker;
use super::page_dialog::{show_page, show_page_handle, Handlers};
use super::state::AppState;
use crate::fork::setup_wizard::ports_pages::{self, FolderAction, MoveFacts};
use crate::core::models::Port;
use crate::core::path_safety::is_within;
use crate::fork::folder_picker::mounts::{default_media_roots, system_mounts};
use crate::fork::folder_picker::{PickerRequest, Purpose};
use crate::fork::port_mover::game_files::{check_before, GameFiles, Issue};
use crate::fork::port_mover::old_copies::{self, OldCopy, RemovalCtx};
use crate::fork::port_mover::{self, BatchItem, BatchOutcome, MoveEnv, MoveError, MoveOptions, PortRef, Prepared, Stage};
use crate::fork::ports_folder::{self, CheckCtx, MovePlan, PortToMove, Startup, SystemProbe};
use crate::fork::ports_folder_view::{self as view, Act, ListModel};
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
    /// A move or an old-copy removal runs on the worker thread.
    static JOB_BUSY: Cell<bool> = const { Cell::new(false) };
    static QUIT_AFTER_JOB: Cell<bool> = const { Cell::new(false) };
    static CANCEL_JOB: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
    static STARTUP_PENDING: Cell<bool> = const { Cell::new(false) };
    static JOB_TIMER: slint::Timer = slint::Timer::default();
    static CHECK_TIMER: slint::Timer = slint::Timer::default();
    static CHECK_RUNNING: Cell<bool> = const { Cell::new(false) };
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

/// A list drawn from a `ListModel`; `on_act` gets the chosen row's action and index.
fn open_model(app: &Rc<AppState>, router: &Router, title: &str, model: ListModel, on_act: impl Fn(&Rc<AppState>, &Router, Act) + 'static) -> Option<crate::ListPickerDialog> {
    let dialog = build_picker_dialog(app, router, title, &model.labels)?;
    let select = model.select as i32;
    {
        let (app, router) = (app.clone(), router.clone());
        dialog.on_item_selected(move |index| on_act(&app, &router, model.act(index as usize)));
    }
    dialog.set_selected_index(select);
    app.dialog_nav.picker_index.set(select);
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog.clone_strong()));
    Some(dialog)
}

/// A confirm dialog whose Cancel (and B) runs `on_cancel` instead of only closing.
fn confirm_or(app: &Rc<AppState>, router: &Router, title: &str, text: String, confirm: &str, on_confirm: impl Fn(&Rc<AppState>, &Router) + 'static, on_cancel: fn(&Rc<AppState>, &Router)) {
    open_confirm_dialog(app, router, title.into(), format!("{text}\n").into(), confirm.into(), on_confirm);
    if let DialogSlot::Confirm(d) = &*app.dialog_nav.dialogs.borrow() {
        let (app, router) = (app.clone(), router.clone());
        d.on_close_requested(move || if crate::fork::press_echo::key(crate::fork::press_echo::Press::Back) {
            close_current_dialog(&app, &router);
            on_cancel(&app, &router);
        });
    }
}

fn is_default(app: &AppState, dir: &Path) -> bool {
    dir == ports_folder::default_dir(&app.paths.config_dir)
}

/// Inside the setup wizard the first question has pages of its own where Settings > Ports folder has lists.
fn in_wizard() -> bool {
    super::setup_wizard_screen::active()
}

/// A wizard page of this flow. B goes back to the wizard (through the Ports folder menu, which the wizard takes
/// over while it waits for this screen), the title bar X ends the wizard.
fn wizard_page(app: &Rc<AppState>, router: &Router, page: crate::fork::page::Page<FolderAction>, on_action: impl Fn(&Rc<AppState>, &Router, FolderAction) + 'static) {
    let handlers = Handlers {
        on_action: Rc::new(on_action),
        on_back: Rc::new(|app, router| open_menu(app, router)),
        on_exit: Rc::new(|app, router| super::setup_wizard_screen::exit_wizard(app, router)),
        on_input: None,
    };
    if !show_page(app, router, page, handlers) {
        open_menu(app, router);
    }
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

/// Why writing into the ports folder must wait now (missing, or a move running), if it must.
pub(crate) fn blocked_reason(app: &AppState) -> Option<String> {
    view::blocked_reason(MOVING.with(Cell::get), unavailable(app))
}

/// Installs, adopting and restores call this: true (with a message) while the ports folder is missing
/// or being moved, so nothing is ever created at an unmounted ports folder.
pub(crate) fn refuse_when_blocked(app: &AppState) -> bool {
    let Some(why) = blocked_reason(app) else { return false };
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

/// Close and Quit ask this: a running move is cancelled (its copies removed) and an old-copy removal
/// finishes, then the app quits.
pub(crate) fn quit_waits_for_move(app: &Rc<AppState>, router: &Router) -> bool {
    if !JOB_BUSY.with(Cell::get) {
        return false;
    }
    QUIT_AFTER_JOB.with(|q| q.set(true));
    if let Some(cancel) = CANCEL_JOB.with(|c| c.borrow().clone()) {
        cancel();
        message(app, router, "Stopping the move first (the copies made so far are removed). The app closes when that is done.");
    } else {
        message(app, router, "Finishing the removal of the old copy first. The app closes when that is done.");
    }
    true
}

fn job_finished() -> bool {
    JOB_BUSY.with(|b| b.set(false));
    CANCEL_JOB.with(|c| *c.borrow_mut() = None);
    if QUIT_AFTER_JOB.with(Cell::get) {
        let _ = slint::quit_event_loop();
        return true;
    }
    false
}

/// The other startup dialogs wait while this is true.
pub(crate) fn startup_dialog_pending() -> bool {
    STARTUP_PENDING.with(Cell::get)
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

fn game_mode() -> bool {
    crate::fork::steam_queue::game_mode_from_env()
}

// -- Startup --------------------------------------------------------------------------------------

pub(crate) fn startup_check(app: &Rc<AppState>, router: &Router, startup: Startup) {
    ROUTER.with(|r| *r.borrow_mut() = Some((Rc::downgrade(app), router.clone())));
    let roots: Vec<String> = default_media_roots().iter().map(|r| r.display().to_string()).collect();
    eprintln!(
        "ports folder: {} ({}); drive folders looked at: {}",
        startup.dir.display(),
        if startup.available() { "available" } else { "NOT available" },
        roots.join(", ")
    );
    if startup.available() {
        return;
    }
    // First of the startup dialogs: the Steam queue offer skips while a dialog is open, other startup dialogs
    // wait for `startup_dialog_pending`.
    STARTUP_PENDING.with(|p| p.set(true));
    let (app, router) = (app.clone(), router.clone());
    slint::Timer::single_shot(Duration::from_millis(300), move || show_missing(&app, &router, false));
}

fn show_missing(app: &Rc<AppState>, router: &Router, again: bool) {
    let dir = app.paths.library_dir();
    let Some(why) = ports_folder::availability(&dir, &system_mounts(), &default_media_roots()) else {
        STARTUP_PENDING.with(|p| p.set(false));
        after_library_change(app);
        if again {
            message(app, router, &format!("Found it: {} is back.", dir.display()));
        }
        return;
    };
    let done = |app: &Rc<AppState>, router: &Router| {
        STARTUP_PENDING.with(|p| p.set(false));
        close_current_dialog(app, router);
    };
    let Some(dialog) = open_model(app, router, "Ports folder not available", view::missing(&dir, &why, again), move |app, router, act| match act {
        Act::TryAgain => show_missing(app, router, true),
        Act::ChooseAnother => {
            STARTUP_PENDING.with(|p| p.set(false));
            choose_folder(app, router);
        }
        Act::ContinueWithout => done(app, router),
        Act::Quit => {
            done(app, router);
            let _ = slint::quit_event_loop();
        }
        _ => {}
    }) else {
        STARTUP_PENDING.with(|p| p.set(false));
        return;
    };
    let (app, router) = (app.clone(), router.clone());
    dialog.on_close_requested(move || if crate::fork::press_echo::key(crate::fork::press_echo::Press::Back) { done(&app, &router) });
}

// -- The menu -------------------------------------------------------------------------------------

/// Settings > Ports folder...
pub(crate) fn open_menu(app: &Rc<AppState>, router: &Router) {
    // Fork (setup wizard): backing out of a screen the wizard opened lands here; go back to the wizard.
    if super::setup_wizard_screen::resume_instead_of_menu(app, router, crate::fork::setup_wizard::flow::Menu::PortsFolder) {
        return;
    }
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
                parts.push(crate::fork::folder_picker::mounts::fs_label(&t));
            }
            if let Some(d) = crate::fork::folder_picker::mounts::drive_of(&dir, &mounts, &default_media_roots()) {
                parts.push(d.title());
            }
            if parts.is_empty() { "Available".to_string() } else { parts.join(", ") }
        }
    };
    let old = old_copies::load(&app.paths.config_dir).unwrap_or_default();
    let old_summary = (old.len(), old.iter().map(|o| o.bytes).sum());
    // Switches cut short (the app closed between the setting and Steam), whose new copy is in use.
    let active = RemovalParts::of(app).active;
    let unfinished: Vec<OldCopy> = old.into_iter().filter(|o| !o.switched && active.contains(&o.new_path)).collect();
    let names: Vec<String> = unfinished.iter().map(|o| o.name.clone()).collect();
    let model = view::menu(&dir, &state, is_default(app, &dir), old_summary, &ports_folder::default_dir(&app.paths.config_dir), game_mode(), &names);
    open_model(app, router, TITLE, model, move |app, router, act| match act {
        Act::FinishSwitch(i) => {
            if let Some(record) = unfinished.get(i) {
                finish_switch(app, router, record);
            }
        }
        Act::ChooseAnother => choose_folder(app, router),
        Act::OpenFolder => crate::core::launch::open_folder(&app.paths.library_dir()),
        Act::OldCopies => open_old_copies(app, router),
        Act::UseDefault => consider(app, router, ports_folder::default_dir(&app.paths.config_dir)),
        Act::Back => {
            close_current_dialog(app, router);
            super::dialogs::open_settings_dialog(app, router);
        }
        _ => {}
    });
}

/// The picker for a new ports folder, then the whole change (check, create, move, switch, old-copy prompts).
/// Cancel returns to the Ports folder list.
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
/// that already have a folder (other screens): the same flow as Settings > Ports folder.
pub(crate) fn consider(app: &Rc<AppState>, router: &Router, dir: PathBuf) {
    let current = app.paths.library_dir();
    let (mounts, roots, home) = (system_mounts(), default_media_roots(), home_dir());
    let ctx = CheckCtx::system(&app.paths.config_dir, &current, home.as_deref(), &mounts, &roots);
    let check = ports_folder::check(&dir, &ctx, &SystemProbe);
    if check.same_as_current {
        return message(app, router, &format!("{} is already the ports folder.", dir.display()));
    }
    if !check.ok() && in_wizard() {
        return wizard_page(app, router, ports_pages::cannot_use_page(&check.problems), |app, router, action| match action {
            FolderAction::PickAnother => choose_folder(app, router),
            _ => open_menu(app, router),
        });
    }
    if !check.ok() {
        let text = format!("{}\n\n{}", dir.display(), check.problems.join("\n\n"));
        return confirm_or(app, router, "Can't use this folder", text, "Choose another folder", |app, router| choose_folder(app, router), open_menu);
    }
    if !check.exists && in_wizard() {
        return wizard_page(app, router, ports_pages::create_page(), move |app, router, action| match action {
            FolderAction::CreateIt => match ports_folder::create(&dir, &system_mounts(), &default_media_roots()) {
                Ok(()) => consider(app, router, dir.clone()),
                Err(e) => message(app, router, &e),
            },
            FolderAction::PickAnother => choose_folder(app, router),
            _ => open_menu(app, router),
        });
    }
    if !check.exists {
        let text = format!("{} does not exist yet.\n\nCreate it and use it as the ports folder? ({})", dir.display(), check.summary());
        return confirm_or(
            app,
            router,
            "Create the folder?",
            text,
            "Create it",
            move |app, router| match ports_folder::create(&dir, &system_mounts(), &default_media_roots()) {
                Ok(()) => consider(app, router, dir.clone()),
                Err(e) => message(app, router, &e),
            },
            open_menu,
        );
    }
    plan_switch(app, router, dir, check.summary(), check.free);
}

// -- Switching ------------------------------------------------------------------------------------

fn kept_old_copy_paths(app: &AppState) -> Vec<PathBuf> {
    old_copies::load(&app.paths.config_dir).unwrap_or_default().into_iter().map(|o| o.old_path).collect()
}

/// A game started from this app, or any program running from a port folder in the old ports folder.
fn running_now(app: &AppState, plan: &MovePlan) -> Option<String> {
    if super::playtime::any_process_running(app) {
        return Some("A game started from this app is running. Close it first, then try again.".into());
    }
    for p in plan.ports.iter().chain(&plan.stale) {
        let names = running_in(&p.from);
        if !names.is_empty() {
            return Some(format!("{} is running from {}. Close it first, then try again.", names.join(", "), p.name));
        }
    }
    None
}

fn plan_switch(app: &Rc<AppState>, router: &Router, new_dir: PathBuf, summary: String, free: Option<u64>) {
    let old_dir = app.paths.library_dir();
    let plan = if old_dir.is_dir() { ports_folder::plan_move(&old_dir, &new_dir, &app.catalog.borrow(), &kept_old_copy_paths(app)) } else { MovePlan::default() };
    let needed = plan.bytes.saturating_add(port_mover::SPACE_MARGIN);
    if in_wizard() {
        return wizard_switch(app, router, new_dir, plan, free, needed);
    }
    let model = view::switch_screen(&new_dir, &summary, &plan, free, needed);
    let plan = Rc::new(plan);
    open_model(app, router, "Switch the ports folder?", model, move |app, router, act| match act {
        Act::Cancel => open_menu(app, router),
        Act::SwitchEmpty | Act::SwitchWithoutMoving => {
            if let Some(why) = running_now(app, &plan) {
                return message(app, router, &why);
            }
            let note = if act == Act::SwitchWithoutMoving { format!("{} port(s) stay in the old folder and show as not installed here.", plan.ports.len()) } else { String::new() };
            switch_now(app, router, &new_dir, Vec::new(), note);
        }
        Act::Move => check_game_files_then_move(app, router, new_dir.clone(), plan.clone()),
        Act::ReplaceStaleAndMove => {
            let kept: Vec<String> = plan.stale.iter().map(|s| format!("  {}: {}", s.name, s.to.display())).collect();
            let text = format!(
                "The old {} you kept {} removed first (saves zipped to Saves Backup/Update Backups), then the copy in use is moved there:
{}

If the move then fails, the kept copy is already gone; the copy in use is not touched.",
                if kept.len() == 1 { "copy" } else { "copies" },
                if kept.len() == 1 { "is" } else { "are" },
                kept.join("
")
            );
            let (new_dir, plan) = (new_dir.clone(), plan.clone());
            confirm_or(app, router, "Replace the old copy?", text, "Replace and move", move |app, router| check_game_files_then_move(app, router, new_dir.clone(), plan.clone()), open_menu);
        }
        Act::NotEnoughSpace => message(app, router, "There is not enough free space in the new folder for the ports. Free some space or pick another folder."),
        _ => {}
    });
}

/// The wizard's version of "Switch the ports folder?": nothing to move goes straight on, else one question.
fn wizard_switch(app: &Rc<AppState>, router: &Router, new_dir: PathBuf, plan: MovePlan, free: Option<u64>, needed: u64) {
    let n = plan.ports.len() + plan.stale.len();
    let plan = Rc::new(plan);
    if n == 0 && !plan.already_there.is_empty() {
        // Nothing to copy, but ports are in that place already: say what the switch does.
        let there = plan.already_there.len();
        return wizard_page(app, router, ports_pages::already_there_page(there), move |app, router, action| match action {
            FolderAction::UseIt => switch_now(app, router, &new_dir, Vec::new(), String::new()),
            FolderAction::PickAnother => choose_folder(app, router),
            _ => open_menu(app, router),
        });
    }
    if n == 0 {
        if let Some(why) = running_now(app, &plan) {
            return message(app, router, &why);
        }
        return switch_now(app, router, &new_dir, Vec::new(), String::new());
    }
    let facts = MoveFacts
 { ports: n, bytes: plan.bytes, free, needed, stale: !plan.stale.is_empty() };
    wizard_page(app, router, ports_pages::move_page(&facts), move |app, router, action| match action {
        FolderAction::MoveThem | FolderAction::ReplaceAndMove => check_game_files_then_move(app, router, new_dir.clone(), plan.clone()),
        FolderAction::LeaveThem => {
            if let Some(why) = running_now(app, &plan) {
                return message(app, router, &why);
            }
            switch_now(app, router, &new_dir, Vec::new(), format!("{} port(s) stay in the old folder and show as not installed here.", plan.ports.len()));
        }
        FolderAction::PickAnother => choose_folder(app, router),
        _ => open_menu(app, router),
    });
}

fn find_port(app: &AppState, key: &str) -> Option<Port> {
    app.catalog.borrow().iter().find(|p| p.key() == key).cloned()
}

/// The game file check before anything is copied; problems are shown with Fix / Move anyway / Cancel.
fn check_game_files_then_move(app: &Rc<AppState>, router: &Router, new_dir: PathBuf, plan: Rc<MovePlan>) {
    let mut issues: Vec<(String, Issue)> = Vec::new();
    for p in plan.ports.iter().chain(&plan.stale) {
        let files = find_port(app, &p.key).map(|port| GameFiles::for_port(&port, &app.paths.config_dir)).unwrap_or_default();
        issues.extend(check_before(&p.from, &files).into_iter().map(|i| (p.name.clone(), i)));
    }
    if issues.is_empty() {
        return start_move(app, router, new_dir, plan, false, Vec::new());
    }
    if in_wizard() {
        return wizard_game_files(app, router, new_dir, plan, issues, Vec::new(), 0);
    }
    let details: Vec<String> = issues.iter().map(|(name, i)| format!("{name}\n\n{}", i.line())).collect();
    open_model(app, router, "Check the game files", view::game_file_issues(&issues), move |app, router, act| match act {
        Act::Detail(i) => {
            if let Some(text) = details.get(i) {
                message(app, router, text);
            }
        }
        Act::FixInGameFiles => super::rom_screens::open_game_files(app, router),
        Act::MoveAnyway => start_move(app, router, new_dir.clone(), plan.clone(), true, Vec::new()),
        Act::Cancel => open_menu(app, router),
        _ => {}
    });
}

/// The wizard asks about the ports with a game file problem one at a time: skip the port, or move it anyway.
fn wizard_game_files(app: &Rc<AppState>, router: &Router, new_dir: PathBuf, plan: Rc<MovePlan>, mut issues: Vec<(String, Issue)>, accept: Vec<String>, skipped: usize) {
    if issues.is_empty() {
        let left = plan.ports.len() + plan.stale.len();
        if left == 0 {
            // Every port was skipped: nothing moves, and the skipped ones stay where they are.
            return wizard_page(app, router, ports_pages::nothing_moved_page(skipped), move |app, router, action| match action {
                FolderAction::UseIt => switch_now(app, router, &new_dir, Vec::new(), String::new()),
                FolderAction::PickAnother => choose_folder(app, router),
                _ => open_menu(app, router),
            });
        }

        return start_move(app, router, new_dir, plan, false, accept);
    }
    // Every problem of the first port is one decision about that port.
    let name = issues[0].0.clone();
    let same: Vec<usize> = (0..issues.len()).filter(|i| issues[*i].0 == name).collect();
    let first = issues[same[0]].1.clone();
    let file = Path::new(&first.file).file_name().map_or(first.file.clone(), |n| n.to_string_lossy().into_owned());
    let problem = format!("{file}: {}.", first.problem.trim_end_matches('.'));
    let others: std::collections::BTreeSet<String> = issues.iter().map(|(n, _)| n.clone()).filter(|n| *n != name).collect();
    let key = plan.ports.iter().chain(&plan.stale).find(|p| p.name == name).map(|p| p.key.clone());
    for i in same.iter().rev() {
        issues.remove(*i);
    }
    let page = ports_pages::game_file_page(&name, &problem, others.len());
    wizard_page(app, router, page, move |app, router, action| match action {
        FolderAction::SkipPort => {
            let mut smaller = (*plan).clone();
            if let Some(key) = &key {
                smaller.ports.retain(|p| &p.key != key);
                smaller.stale.retain(|p| &p.key != key);
            }
            wizard_game_files(app, router, new_dir.clone(), Rc::new(smaller), issues.clone(), accept.clone(), skipped + 1);
        }
        FolderAction::MoveAnyway => {
            let mut accept = accept.clone();
            accept.extend(key.clone());
            wizard_game_files(app, router, new_dir.clone(), plan.clone(), issues.clone(), accept, skipped);
        }
        _ => open_menu(app, router),
    });
}

/// What the worker brings back.
struct MoveRun {
    moved: Vec<Prepared>,
    failed: Vec<(String, String, bool)>,
    cancelled: bool,
    drive_gone: Option<String>,
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

/// Replaces an old copy the user kept that sits where a port is moving to: its saves are zipped, then
/// it is removed and its record dropped. Runs on the worker thread.
fn replace_stale(stale: &PortToMove, record: Option<&OldCopy>, port: Option<&Port>, saves_dir: &Path, parts: &RemovalParts) -> Result<(), String> {
    let config_dir = &parts.config_dir;
    let Some(port) = port else { return Err("it is not in the catalog any more, so its saves cannot be found; remove that old copy by hand".into()) };
    let Some(record) = record else { return Err(format!("{} is no longer listed as an old copy, so it is left alone", stale.to.display())) };
    // The same guards as removing an old copy: protected folders, a game running from it, newer saves.
    let saves = |e: &OldCopy, dir: &Path| parts.save_folders(e, dir);
    let ctx = RemovalCtx { home: &parts.home, config_dir: &parts.config_dir, library_dir: &parts.library, active_paths: &parts.active, running: &running_in, save_folders: &saves };
    old_copies::replace_check(record, &stale.from, &ctx)?;
    crate::core::save_backup::backup_before_update(port, saves_dir, &stale.to, &crate::fork::store::backup_stamp(), &mut |_| {})
        .map_err(|e| format!("the saves in the old copy at {} could not be zipped, so it was left alone: {e}", stale.to.display()))?;
    port_mover::copy::remove_copy(&stale.to).map_err(|e| format!("could not remove the old copy at {}: {e}", stale.to.display()))?;
    let _ = old_copies::forget(config_dir, &stale.to);
    Ok(())
}

fn start_move(app: &Rc<AppState>, router: &Router, new_dir: PathBuf, plan: Rc<MovePlan>, accept_issues: bool, accept_keys: Vec<String>) {
    if JOB_BUSY.with(Cell::get) {
        return message(app, router, "A move is already running.");
    }
    if !app.install_runtime.installing.borrow().is_empty() {
        return message(app, router, "An install, update or save restore is running. Wait for it to finish, then try again.");
    }
    if let Some(why) = running_now(app, &plan) {
        return message(app, router, &why);
    }
    let keys: Vec<String> = plan.ports.iter().chain(&plan.stale).map(|p| p.key.clone()).collect();
    MOVING.with(|m| m.set(true));
    JOB_BUSY.with(|b| b.set(true));
    for k in &keys {
        app.install_runtime.installing.borrow_mut().insert(k.clone());
    }
    let catalog = app.catalog.borrow().clone();
    let port_of = move |key: &str| catalog.iter().find(|p| p.key() == key).cloned();
    let config_dir = app.paths.config_dir.clone();
    let files: Vec<GameFiles> = plan.ports.iter().chain(&plan.stale).map(|p| port_of(&p.key).map(|port| GameFiles::for_port(&port, &config_dir)).unwrap_or_default()).collect();
    let records = old_copies::load(&config_dir).unwrap_or_default();
    let stale: Vec<(PortToMove, Option<OldCopy>, Option<Port>)> = plan.stale.iter().map(|s| (s.clone(), records.iter().find(|r| r.old_path == s.to).cloned(), port_of(&s.key))).collect();
    let parts = RemovalParts::of(app);
    let items: Vec<BatchItem> = plan.ports.iter().chain(&plan.stale).map(|p| BatchItem { port: PortRef { key: p.key.clone(), name: p.name.clone() }, from: p.from.clone(), to: p.to.clone() }).collect();
    let (home, _) = move_env_parts(app);
    let saves_dir = app.paths.saves_backup_dir.clone();
    let total = items.len();
    let shown_names: Vec<String> = items.iter().map(|i| i.port.name.clone()).collect();
    let replaces = !plan.stale.is_empty();
    let was_wizard = in_wizard();
    let job = Rc::new(Background::spawn(move |ctl| {
        let settings = ForkSettings::load(&config_dir.join(FILE_NAME));
        let steam = SystemSteamCheck::new();
        let env = MoveEnv { home: &home, config_dir: &config_dir, settings: &settings, steam: &steam, game_mode: false, running: &running_in, free_space: &free_space };
        let mut run = MoveRun { moved: Vec::new(), failed: Vec::new(), cancelled: false, drive_gone: None };
        // Kept old copies in the way go first (checked, saves zipped, then removed), so their ports can move.
        let mut blocked: Vec<String> = Vec::new();
        for (s, record, port) in &stale {
            ctl.set_progress(format!("Checking and replacing the old copy of {}\n{}", s.name, s.to.display()));
            if let Err(e) = replace_stale(s, record.as_ref(), port.as_ref(), &saves_dir, &parts) {
                run.failed.push((s.name.clone(), e, false));
                blocked.push(s.key.clone());
            }
        }
        let todo: Vec<(BatchItem, GameFiles)> = items.into_iter().zip(files).filter(|(i, _)| !blocked.contains(&i.port.key)).collect();
        let (batch_items, batch_files): (Vec<BatchItem>, Vec<GameFiles>) = todo.into_iter().unzip();
        let names: Vec<String> = batch_items.iter().map(|i| i.port.name.clone()).collect();
        let options = |i: usize| MoveOptions { game_files: batch_files[i].clone(), accept_issues_before: accept_issues || accept_keys.contains(&batch_items[i].port.key), ..Default::default() };
        let count = names.len().max(1) as f32;
        let mut progress = |i: usize, done: u64, size: u64| {
            ctl.set_progress(format!("Copying {} of {total}: {}\n{} of {}", i + 1, names[i], human_bytes(done), human_bytes(size)));
            ctl.set_fraction((i as f32 + if size == 0 { 1.0 } else { done as f32 / size as f32 }) / count);
        };
        let result = port_mover::run_batch(&batch_items, &env, &options, &mut progress, ctl.flag());
        for outcome in result.outcomes {
            match outcome {
                BatchOutcome::Moved(p) => run.moved.push(p),
                BatchOutcome::Failed { port, error } => {
                    let after = matches!(error, MoveError::GameFiles { stage: Stage::AfterCopy, .. });
                    run.failed.push((port.name, error.to_string(), after));
                }
            }
        }
        run.cancelled = result.cancelled;
        run.drive_gone = result.drive_gone;
        run
    }));
    {
        let job = job.clone();
        CANCEL_JOB.with(|c| *c.borrow_mut() = Some(Rc::new(move || job.cancel())));
    }

    // A list showing progress with a Cancel row; B cancels too.
    let cancelling = Rc::new(Cell::new(false));
    // In the setup wizard the progress is a page: one line per port, and Cancel.
    let page_handle = if in_wizard() {
        let cancel_now = {
            let (job, cancelling) = (job.clone(), cancelling.clone());
            Rc::new(move |_: &Rc<AppState>, _: &Router| {
                job.cancel();
                cancelling.set(true);
            })
        };
        let (on_action, on_back) = (cancel_now.clone(), cancel_now);
        let exit_job = job.clone();
        let handlers = Handlers {
            on_action: Rc::new(move |app, router, _action: FolderAction| on_action(app, router)),
            on_back: Rc::new(move |app, router| on_back(app, router)),
            // Leaving the wizard stops the copy too; what it had copied is removed when the worker has stopped.
            on_exit: Rc::new(move |app, router| {
                exit_job.cancel();
                super::setup_wizard_screen::exit_wizard(app, router);
            }),
            on_input: None,
        };
        show_page_handle(app, router, ports_pages::moving_page(&shown_names, "", false, replaces).with_progress(None), handlers)

    } else {
        None
    };
    let list = if page_handle.is_some() { None } else { build_picker_dialog(app, router, "Moving ports", &view::progress_labels("Copying 99 of 99: A port name that is quite long\n999.9 GB of 999.9 GB", false)) };
    let list_weak = list.as_ref().map(|d| d.as_weak());
    if let Some(dialog) = list {
        set_picker_items(&dialog, &view::progress_labels("", false));
        dialog.set_selected_index(2);
        app.dialog_nav.picker_index.set(2);
        {
            let (job, cancelling, weak) = (job.clone(), cancelling.clone(), dialog.as_weak());
            dialog.on_item_selected(move |index| {
                if index == 2 {
                    job.cancel();
                    cancelling.set(true);
                    if let Some(d) = weak.upgrade() {
                        set_picker_items(&d, &view::progress_labels(&job.progress(), true));
                    }
                }
            });
        }
        {
            let (job, cancelling) = (job.clone(), cancelling.clone());
            dialog.on_close_requested(move || if crate::fork::press_echo::key(crate::fork::press_echo::Press::Back) {
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
                let quitting = QUIT_AFTER_JOB.with(Cell::get);
                match result {
                    Ok(mut run) => {
                        if quitting {
                            run.cancelled = true;
                        }
                        if was_wizard && !in_wizard() {
                            // The wizard was left during the move: no pages over the main window, no switch of
                            // the ports folder; what was copied is taken back.
                            let problems = discard_all(&run.moved);
                            eprintln!("ports folder: the wizard was left during the move; the new copies were removed {}", problems.join("; "));
                        } else {
                            after_move(&app, &router, &new_dir, run);
                        }

                    }
                    Err(e) => message(&app, &router, &e),
                }
                job_finished();
                return;
            }
            let text = job.progress();
            if text != *last.borrow() || cancelling.get() {
                *last.borrow_mut() = text.clone();
                if let Some(handle) = &page_handle {
                    handle.update(ports_pages::moving_page(&shown_names, &text, cancelling.get(), replaces).with_progress(job.fraction()));
                }
                if let Some(d) = list_weak.as_ref().and_then(slint::Weak::upgrade) {
                    set_picker_items(&d, &view::progress_labels(&text, cancelling.get()));
                }
            }
        });
    });
}

fn discard_all(prepared: &[Prepared]) -> Vec<String> {
    prepared.iter().filter_map(|p| port_mover::discard(p).err().map(|e| format!("Could not remove the new copy {}: {e}", p.to.display()))).collect()
}

fn with_problems(mut text: String, problems: Vec<String>) -> String {
    for p in problems {
        text.push_str(&format!("\n{p}"));
    }
    text
}

fn after_move(app: &Rc<AppState>, router: &Router, new_dir: &Path, run: MoveRun) {
    let MoveRun { moved, failed, cancelled, drive_gone } = run;
    if let Some(why) = drive_gone {
        return message(app, router, &with_problems(why, discard_all(&moved)));
    }
    if cancelled {
        let text = format!("Move cancelled. Nothing changed: the ports folder is still {}, and the copies made so far were removed.", app.paths.library_dir().display());
        return message(app, router, &with_problems(text, discard_all(&moved)));
    }
    if failed.is_empty() {
        return switch_now(app, router, new_dir, moved, String::new());
    }
    if in_wizard() {
        let shown: Vec<(String, String)> = failed.iter().map(|(n, t, _)| (n.clone(), ellipsize(t.lines().next().unwrap_or_default(), 80))).collect();
        let count = moved.len();
        let moved = Rc::new(moved);
        let new_dir = new_dir.to_path_buf();
        // B and the title bar X take the new copies back like Undo does: none are left untracked in the new folder.
        let undo = {
            let moved = moved.clone();
            move || {
                let problems = discard_all(&moved);
                if !problems.is_empty() {
                    eprintln!("ports folder: {}", problems.join("; "));
                }
            }
        };
        let (undo_action, undo_back, undo_exit, undo_fail) = (undo.clone(), undo.clone(), undo.clone(), undo);
        let handlers = Handlers {
            on_action: Rc::new(move |app, router, action| match action {
                FolderAction::UseMoved => switch_now(app, router, &new_dir, moved.as_ref().clone(), "Some ports were not moved and stay in the old folder.".into()),
                _ => {
                    undo_action();
                    open_menu(app, router)
                }
            }),
            on_back: Rc::new(move |app, router| {
                undo_back();
                open_menu(app, router)
            }),
            on_exit: Rc::new(move |app, router| {
                undo_exit();
                super::setup_wizard_screen::exit_wizard(app, router)
            }),
            on_input: None,
        };
        if !show_page(app, router, ports_pages::some_failed_page(&shown, count), handlers) {
            // The page could not be built: the new copies are taken back, as Undo would.
            undo_fail();
            open_menu(app, router);
        }
        return;

    }
    let details: Vec<String> = failed.iter().map(|(n, t, _)| format!("{n}: {t}")).collect();
    let model = view::after_move(&failed, moved.len());
    let moved = Rc::new(moved);
    let new_dir = new_dir.to_path_buf();
    open_model(app, router, "Some ports were not moved", model, move |app, router, act| match act {
        Act::Detail(i) => {
            if let Some(text) = details.get(i) {
                message(app, router, text);
            }
        }
        Act::FixInGameFiles => {
            let problems = discard_all(&moved);
            super::rom_screens::open_game_files(app, router);
            if !problems.is_empty() {
                eprintln!("ports folder: {}", problems.join("; "));
            }
        }
        Act::SwitchAnyway => switch_now(app, router, &new_dir, moved.as_ref().clone(), "Some ports were not moved and stay in the old folder.".into()),
        Act::Cancel => message(app, router, &with_problems("Nothing changed. The new copies were removed; the originals are untouched.".into(), discard_all(&moved))),
        _ => {}
    });
}

/// Makes `new_dir` the ports folder, then repoints Steam and records for the moved ports, then asks
/// about each old copy.
fn switch_now(app: &Rc<AppState>, router: &Router, new_dir: &Path, moved: Vec<Prepared>, note: String) {
    let (home, config_dir) = move_env_parts(app);
    // The old copies are listed before the setting changes, so a crash in between still keeps them listed.
    if let Err(e) = port_mover::record_pending(&config_dir, &moved) {
        let text = format!("The list of old copies could not be written ({e}). Nothing changed; the new copies were removed.");
        return message(app, router, &with_problems(text, discard_all(&moved)));
    }
    let mut settings = ForkSettings::load(&settings_path(app));
    if let Err(e) = ports_folder::save_choice(&mut settings, &app.paths.config_dir, new_dir) {
        for p in &moved {
            let _ = old_copies::forget(&config_dir, &p.from);
        }
        let text = format!("The new ports folder could not be saved ({e}). Nothing changed; the new copies were removed.");
        return message(app, router, &with_problems(text, discard_all(&moved)));
    }
    let old_dir = app.paths.library_dir();
    *app.paths.library.borrow_mut() = new_dir.to_path_buf();
    eprintln!("ports folder: switched from {} to {}", old_dir.display(), new_dir.display());
    after_library_change(app);

    let steam = SystemSteamCheck::new();
    let env = MoveEnv { home: &home, config_dir: &config_dir, settings: &settings, steam: &steam, game_mode: game_mode(), running: &running_in, free_space: &free_space };
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
            let missing = port_mover::game_files::required_not_found(&p.to, &GameFiles::for_port(&port, &config_dir));
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
            summary.push(format!("{}: the link {} was adjusted so it keeps working at the new place.", p.port.name, link.display()));
        }
        if let Some(entry) = old_copies::load(&config_dir).unwrap_or_default().into_iter().find(|o| o.old_path == p.from) {
            copies.push_back(entry);
        }
    }
    if !note.is_empty() {
        summary.push(note);
    }
    // In the setup wizard the old copies of a move are kept and asked about once, on its own page near the end.
    if in_wizard() {
        // The wizard's pages say what matters; the details go to the log. The old copies are kept, and the
        // wizard asks about them once, near the end.
        eprintln!("ports folder: {}", summary.join(" | ").replace('\n', " "));
        return;
    }
    show_old_copies_then_summary(app, router, copies, summary.join("\n\n"));
}

/// The old copies page for `copies` (nothing is removed unless chosen there), then `summary` with what happened.
fn show_old_copies_then_summary(app: &Rc<AppState>, router: &Router, copies: VecDeque<OldCopy>, summary: String) {
    prompt_old_copies_batch_then(app, router, copies, summary, Origin::PortsFolder, |app, router, text| {
        if let Some(text) = text.filter(|t| !t.trim().is_empty()) {
            message(app, router, &text);
        }
    });
}

// -- Old copies -----------------------------------------------------------------------------------

mod batch;
pub(crate) use batch::{note_launch_end, prompt_old_copies_batch_then, result_page, Origin, CHECK_FAILED};

/// Everything the removal check needs, owned, so it can also run on the worker thread.
struct RemovalParts {
    home: PathBuf,
    config_dir: PathBuf,
    library: PathBuf,
    active: Vec<PathBuf>,
    catalog: Vec<Port>,
}

impl RemovalParts {
    fn of(app: &AppState) -> RemovalParts {
        let library = app.paths.library_dir();
        let catalog = app.catalog.borrow().clone();
        let active = catalog.iter().filter_map(|p| crate::core::path_safety::safe_join(&library, &p.folder).ok()).collect();
        RemovalParts { home: home_dir().unwrap_or_else(|| PathBuf::from("/")), config_dir: app.paths.config_dir.clone(), library, active, catalog }
    }

    /// The port's save folders that resolve inside `dir`; `None` when the port left the catalog.
    fn save_folders(&self, entry: &OldCopy, dir: &Path) -> Option<Vec<PathBuf>> {
        let port = self.catalog.iter().find(|p| p.key() == entry.port_key)?;
        Some(
            [port.save.as_ref(), port.save2.as_ref()]
                .into_iter()
                .flatten()
                .filter_map(|value| crate::core::platform_resolve::resolve_save_folder(value, dir))
                .filter(|folder| is_within(folder, dir))
                .collect(),
        )
    }

    /// The removal checks alone (the old copies page shows readiness on its own line).
    fn removal_only(&self, entry: &OldCopy) -> Result<u64, String> {
        let saves = |e: &OldCopy, dir: &Path| self.save_folders(e, dir);
        old_copies::removal_check(
            entry,
            &RemovalCtx { home: &self.home, config_dir: &self.config_dir, library_dir: &self.library, active_paths: &self.active, running: &running_in, save_folders: &saves },
        )
    }

    /// Read from the record as it is now (a yes or a long enough run may have been recorded since `entry` was read).
    /// A list that cannot be read reads as not ready.
    fn readiness(&self, entry: &OldCopy) -> old_copies::Readiness {
        old_copies::readiness_now(entry, &self.home, &self.config_dir).unwrap_or(old_copies::Readiness::StartOnce)
    }
}

/// The list of old copies kept after moves (Settings > Ports folder > Old copies).
pub(crate) fn open_old_copies(app: &Rc<AppState>, router: &Router) {
    let list = match old_copies::load(&app.paths.config_dir) {
        Ok(list) => list,
        Err(e) => return message(app, router, &e),
    };
    let parts = RemovalParts::of(app);
    let mut labels: Vec<String> = list.iter().map(|o| format!("{}: {} ({})", o.name, parts.readiness(o).line(), human_bytes(o.bytes))).collect();
    labels.push("Back".to_string());
    let model = ListModel { acts: (0..list.len()).map(Act::Detail).chain([Act::Back]).collect(), labels, select: 0 };
    let list = Rc::new(list);
    open_model(app, router, "Old copies kept after moves", model, move |app, router, act| match act {
        Act::Detail(i) => {
            let Some(entry) = list.get(i) else { return };
            // An old copy that is already gone: offer to forget the record.
            if std::fs::symlink_metadata(&entry.old_path).is_err() {
                let path = entry.old_path.clone();
                return confirm_or(
                    app,
                    router,
                    "Old copy already gone",
                    format!("{} is not there any more.\n\nForget this record?", path.display()),
                    "Forget it",
                    move |app, router| {
                        let _ = old_copies::forget(&app.paths.config_dir, &path);
                        open_old_copies(app, router);
                    },
                    open_old_copies,
                );
            }
            // The old copies page for this one: its status, "It starts fine..." and, when ready, Remove.
            prompt_old_copies_batch_then(app, router, VecDeque::from([entry.clone()]), String::new(), Origin::PortsFolder, |app, router, text| {
                match text.filter(|t| !t.trim().is_empty()) {
                    // What happened, on a page with one OK, then the list again.
                    Some(text) => result_page(app, router, &text, Origin::PortsFolder, Rc::new(|app, router| open_old_copies(app, router))),
                    None => open_old_copies(app, router),
                }
            });
        }
        Act::Back => open_menu(app, router),
        _ => {}
    });
}

/// Settings > Ports folder > "Finish switching <port>": Steam and the game files records follow the
/// copy in use, and the record is marked finished (so its old copy can be removed later).
fn finish_switch(app: &Rc<AppState>, router: &Router, record: &OldCopy) {
    let (home, config_dir) = move_env_parts(app);
    let settings = ForkSettings::load(&settings_path(app));
    let steam = SystemSteamCheck::new();
    let env = MoveEnv { home: &home, config_dir: &config_dir, settings: &settings, steam: &steam, game_mode: game_mode(), running: &running_in, free_space: &free_space };
    let parts = RemovalParts::of(app);
    let report = match port_mover::finish_switch(record, &env, &parts.library, &parts.active) {
        Ok(report) => report,
        Err(why) => return message(app, router, &why),
    };
    let text = report.text();
    let summary = if text.is_empty() { format!("Finished switching {}.", record.name) } else { format!("Finished switching {}.\n\n{text}", record.name) };
    let entry = old_copies::load(&config_dir).unwrap_or_default().into_iter().find(|o| o.old_path == record.old_path);
    show_old_copies_then_summary(app, router, entry.into_iter().collect(), summary);
}
