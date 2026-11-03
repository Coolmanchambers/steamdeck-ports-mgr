use super::dialogs::{dialog_is_open, open_info_dialog, open_message_dialog};
use super::events::{lock, AppEvent};
use super::playtime::is_port_running;
use super::state::AppState;
use crate::core::executable_detect::ExecutableSelectionError;
use crate::core::installer::{InstallOverrides, InstallPaths};
use crate::core::jobs::InstallOutcome;
use crate::core::models::{Port, SourceType};
use crate::fork::play_pages::Problem;
use crate::ui::gamepad_router::GamepadRouter;
use serde_json::Value;
use slint::ComponentHandle;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

type Router = Rc<RefCell<GamepadRouter>>;

fn game_dir(app: &AppState, port: &Port) -> Option<PathBuf> {
    crate::core::path_safety::safe_join(&app.paths.library_dir(), &port.folder).ok()
}

fn try_claim_op_slot(app: &AppState, port: &Port) -> Option<String> {
    let key = port.key().to_string();
    let busy = app.install_runtime.installing.borrow().contains(&key);
    // Fork: no installs into a ports folder that is missing (SD card out) or being moved.
    if busy || is_port_running(app, &key) || super::ports_folder_screen::refuse_when_blocked(app) {
        return None;
    }
    app.install_runtime.installing.borrow_mut().insert(key.clone());
    Some(key)
}

fn progress_sender(events: &std::sync::Arc<std::sync::Mutex<Vec<AppEvent>>>) -> impl FnMut(&str) {
    let events = events.clone();
    move |message: &str| lock(&events).push(AppEvent::InstallProgress { message: message.to_string() })
}

pub(crate) fn start_install(app: &Rc<AppState>, router: &Router, port: Port, asset_override: Option<Value>, release_override: Option<Value>) {
    if super::ports_folder_screen::refuse_when_blocked(app) {
        return; // Fork: ports folder missing or being moved
    }
    if port.source_type == SourceType::None {
        if let Some(dir) = game_dir(app, &port) {
            if std::fs::create_dir_all(&dir).is_ok() {
                app.state.borrow_mut().mark_installed(port.key(), None);
                app.refresh_current_view();
            }
        }
        // Fork: a port with nothing to download is shown the way "All details" shows it (its instructions are the point).
        super::port_page_screen::open_details(app, router, &port, Rc::new(|app, router| super::dialogs::close_current_dialog(app, router)));
        return;
    }
    let Some(key) = try_claim_op_slot(app, &port) else { return };

    // Fork (install wizard): the install's own page (progress bar, MB of MB, speed, keep going or stop) instead of
    // upstream's progress dialog; the meter carries the byte count and the Stop flag.
    let (ctl, meter) = super::install_wizard_screen::downloading(app, router, &port);

    let pin_version = release_override.is_some();
    let (github_token, gitlab_token) = app.api_tokens();
    let library_dir = app.paths.library_dir();
    let saves_backup_dir = app.paths.saves_backup_dir.clone();
    let events = app.events.clone();
    let mut send = progress_sender(&events);
    let mut on_progress = move |message: &str| {
        meter.note(message);
        send(message);
    };

    std::thread::spawn(move || {
        let overrides = InstallOverrides { asset: asset_override.as_ref(), release: release_override.as_ref() };
        let paths = InstallPaths { library_dir: &library_dir, saves_backup_dir: &saves_backup_dir };
        let outcome = crate::core::jobs::run_install(&port, paths, github_token.as_deref(), gitlab_token.as_deref(), overrides, &mut on_progress, &ctl);
        let event = match outcome {
            InstallOutcome::Done { tag } => AppEvent::InstallDone { key, tag, pin_version },
            InstallOutcome::AssetAmbiguous { assets } => AppEvent::InstallAssetAmbiguous { key, assets, release_override },
            InstallOutcome::Error(message) => AppEvent::InstallError { key, message },
        };
        lock(&events).push(event);
    });
}

// Fork: the version list is a page ("Getting the versions", then "Which version?"; `port_actions_screen`). Info's Select
// version (kept for upstream's sake, no longer shown) and anything else that asks lands there, closing on Back.
pub(crate) fn open_version_picker(app: &Rc<AppState>, router: &Router, port: Port) {
    super::port_actions_screen::versions(app, router, port, crate::fork::port_page::Card::Updates, Rc::new(|app, router| super::dialogs::close_current_dialog(app, router)));
}

fn file_label(path: &Path) -> String {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string()
}

// Fork: "Which program starts <name>?" (`play_screen`), one option per program with its folder and "Ask every time" last;
// the pick is remembered as before. A port with no program says so on "Could not start".
pub(crate) fn open_favorite_exe_picker(app: &Rc<AppState>, router: &Router, port: Port) {
    let Some(game_dir) = game_dir(app, &port) else { return super::play_screen::could_not_start(app, router, &port, Problem::BadFolderName) };
    if !game_dir.is_dir() {
        return super::play_screen::could_not_start(app, router, &port, Problem::FolderGone);
    }
    // Fork: the full list of programs, shallowest folder first (the launch list shows only the shallowest).
    let candidates = crate::core::executable_detect::list_candidates(&game_dir);
    if candidates.is_empty() {
        return super::play_screen::could_not_start(app, router, &port, Problem::NoProgram);
    }
    let key = port.key().to_string();
    let dir = game_dir.clone();
    let picked: super::play_screen::Picked = Rc::new(move |app, _, exe| {
        let exe = exe.and_then(|p| crate::fork::program_detect::relative_pick(&dir, &p));
        app.state.borrow_mut().set_favorite_exe(&key, exe);
    });
    super::play_screen::choose_program(app, router, &port, &game_dir, candidates, true, picked);
}

/// True when the program was started.
fn launch_executable(app: &Rc<AppState>, router: &Router, port: &Port, exe: &Path) -> bool {
    if is_port_running(app, port.key()) {
        return false;
    }
    // Fork: "Could not start <name>" (a page) instead of upstream's "Error - X" dialog, and for a program the system would
    // not start (which only reached the log before).
    if !exe.exists() {
        super::play_screen::could_not_start(app, router, port, Problem::ProgramGone(file_label(exe)));
        return false;
    }
    let child = match crate::core::launch::launch(exe) {
        Ok(child) => child,
        Err(e) => {
            eprintln!("launch: could not start {}: {e}", exe.display());
            let why = crate::fork::play_pages::wont_start_reason(e.kind(), e.raw_os_error());
            super::play_screen::could_not_start(app, router, port, Problem::WontStart { file: file_label(exe), why });
            return false;
        }
    };
    let key = port.key().to_string();
    let runtime = &app.install_runtime;
    runtime.running_processes.borrow_mut().insert(key.clone(), child);
    runtime.launch_started_at.borrow_mut().insert(key.clone(), Instant::now());
    let show_game_presence = {
        let state = app.state.borrow();
        state.discord_rpc_enabled && !state.custom_presence_active()
    };
    if show_game_presence {
        let large_image = port.icon.clone().or_else(|| port.image.clone());
        let handle = crate::core::discord_presence::start(port.display_name().to_string(), port.folder.clone(), large_image);
        runtime.discord_presence.borrow_mut().insert(key, handle);
    }
    let window = app.window();
    if window.get_big_mode() {
        window.window().set_minimized(true);
        runtime.minimized_for_game.set(true);
    }
    true
}

// Starts the installed program with no update check (upstream's name since 2026-10-30; the fork's `launch_flow`).
// `pub(crate)`: events.rs launches straight after a failed or cancelled Play update.
pub(crate) fn launch_installed(app: &Rc<AppState>, router: &Router, port: &Port) {
    // Fork: the tray's recent games end up here; never start a game while it installs, updates or
    // has its saves restored (the install slot is held for all three).
    if app.install_runtime.installing.borrow().contains(port.key()) {
        // Its progress dialog is usually up; leave that in place rather than replacing it with this note.
        if !dialog_is_open(app) {
            open_message_dialog(app, router, &port.name, "This port is being installed, updated or restored. Wait for that to finish, then start it.");
        }
        return;
    }
    let Some(game_dir) = game_dir(app, port) else {
        return super::play_screen::could_not_start(app, router, port, Problem::BadFolderName);
    };
    if !game_dir.exists() {
        return super::play_screen::could_not_start(app, router, port, Problem::FolderGone);
    }
    let favorite = app.state.borrow().get(port.key()).and_then(|i| i.favorite_exe.clone());
    let favorite_path = favorite.and_then(|f| crate::core::path_safety::safe_join(&game_dir, &f).ok()).filter(|p| p.exists());
    if let Some(path) = favorite_path {
        launch_executable(app, router, port, &path);
        return;
    }
    match crate::core::executable_detect::resolve_executable(port.executable.as_ref(), &game_dir) {
        Ok(exe) => {
            launch_executable(app, router, port, &exe);
        }
        Err(ExecutableSelectionError::Ambiguous(candidates)) => {
            let (port2, dir) = (port.clone(), game_dir.clone());
            // Fork: "Which program starts <name>?" (a page: each program with its folder, the likely one recommended).
            let picked: super::play_screen::Picked = Rc::new(move |app, router, exe| {
                let Some(exe) = exe else { return };
                // Fork: a pick that starts is remembered, so the next start does not ask again (the Port page's Program to
                // start can set it back to "Ask every time"); one that fails to start is not.
                if launch_executable(app, router, &port2, &exe) {
                    if let Some(rel) = crate::fork::program_detect::relative_pick(&dir, &exe) {
                        app.state.borrow_mut().set_favorite_exe(port2.key(), Some(rel));
                    }
                }
            });
            super::play_screen::choose_program(app, router, port, &game_dir, candidates, false, picked);
        }
        Err(ExecutableSelectionError::NotFound) => super::play_screen::could_not_start(app, router, port, Problem::NoProgram),
    }
}

pub(crate) fn with_indexed_port(app: &AppState, index: i32, action: impl FnOnce(Port)) {
    let port = app.windowed_nav.displayed_windowed.borrow().get(index as usize).cloned();
    if let Some(port) = port {
        action(port);
    }
}

pub(crate) fn install_row(app: &Rc<AppState>, router: &Router, index: i32) {
    if !dialog_is_open(app) {
        // Fork (install wizard): "Install <name>?" first (with the same-game note), then its pages.
        with_indexed_port(app, index, |port| super::install_wizard_screen::open_from_list(app, router, port));
    }
}

// Fork: the orange "Updates off" button of a list row opens the port's Updates page, where "Check for updates" is the one
// switch (it was upstream's "Update - X" Enable/Disable picker).
pub(crate) fn open_updates_row(app: &Rc<AppState>, router: &Router, index: i32) {
    if !dialog_is_open(app) {
        with_indexed_port(app, index, |port| super::port_page_screen::open_updates(app, router, &port));
    }
}

pub(crate) fn activate_port(app: &Rc<AppState>, router: &Router, port: &Port) {
    // Fork (install wizard): A on a port that is installing brings its downloading page back.
    if dialog_is_open(app) || super::install_wizard_screen::reopen(app, router, port.key()) || app.install_runtime.installing.borrow().contains(port.key()) {
        return;
    }
    if app.is_installed(port) {
        play_port(app, router, port);
    } else {
        // Fork (install wizard): "Install <name>?" first (with the same-game note), then its pages.
        super::install_wizard_screen::open_from_list(app, router, port.clone());
    }
}

pub(crate) fn play_port(app: &Rc<AppState>, router: &Router, port: &Port) {
    if is_port_running(app, port.key()) {
        return;
    }
    let check = {
        let state = app.state.borrow();
        let info = state.get(port.key());
        let last_check = info.map(|i| i.last_update_check.clone()).unwrap_or_default();
        // Fork: one rule for Play and the home screen (crate::fork::home::update_check_due); since the upstream
        // catch-up it is "12 h since the last check" (`last_update_check`), not "24 h since the install".
        crate::fork::home::update_check_due(state.release_sync, &port.source_type, info.map(|i| i.update), &last_check)
            .then(|| (info.and_then(|i| i.installed_tag.clone()), info.map(|i| i.installed_at.clone()).unwrap_or_default()))
    };
    let Some((tag, installed_at)) = check else {
        return launch_installed(app, router, port);
    };
    // Fork (home screen): the home screen's own round may have answered already. A tile that says "Update ready"
    // leads to the update (never the old version); a fresh "nothing new" spares the request.
    if let Some(available) = super::home_screen::play_answer(app, port.key()) {
        if try_claim_op_slot(app, port).is_none() {
            return;
        }
        return finish_play_update_check(app, router, port.clone(), available, true);
    }
    if try_claim_op_slot(app, port).is_none() {
        return;
    }
    let (github_token, gitlab_token) = app.api_tokens();
    let port = port.clone();
    let events = app.events.clone();
    std::thread::spawn(move || {
        // Fork: a check that got no answer (offline, rate limit) is not stamped, so the next Play asks again.
        let answer = crate::core::jobs::run_update_check_result(&port, tag.as_deref(), &installed_at, github_token.as_deref(), gitlab_token.as_deref());
        if let Err(e) = &answer {
            eprintln!("play: no update answer for {}: {e}", port.key());
        }
        let (available, answered) = (answer.as_ref().copied().unwrap_or(false), answer.is_ok());
        lock(&events).push(AppEvent::PlayUpdateChecked { port: Box::new(port), available, answered });
    });
}

pub(crate) fn finish_play_update_check(app: &Rc<AppState>, router: &Router, port: Port, available: bool, answered: bool) {
    app.install_runtime.installing.borrow_mut().remove(port.key());
    if answered {
        app.state.borrow_mut().mark_update_checked(port.key());
    }
    if available {
        app.install_runtime.pending_launch_after_install.borrow_mut().insert(port.key().to_string());
        start_install(app, router, port, None, None);
    } else {
        launch_installed(app, router, &port);
    }
}

pub(crate) fn activate_selection(app: &Rc<AppState>, router: &Router) {
    // Fork (port page): A on the lists' "< Home" button.
    if super::home_screen::list_button_activate(app, router) {
        return;
    }
    if app.deck_ready.is_active() {
        return super::deck_ready_screen::activate(app, router);
    }
    if app.home.is_active() {
        return super::home_screen::activate(app, router);
    }
    if let Some(port) = app.current_selected_port() {
        activate_port(app, router, &port);
    }
}

pub(crate) fn show_info_for_current_selection(app: &Rc<AppState>, router: &Router) {
    if app.deck_ready.is_active() {
        return super::deck_ready_screen::show_info(app, router);
    }
    // Fork (port page): X on Home opens the focused port's Port page (Info is its "All details" card).
    if app.home.is_active() {
        return super::home_screen::open_options(app, router);
    }
    if let Some(port) = app.current_selected_port() {
        open_info_dialog(app, router, &port);
    }
}

pub(crate) fn reveal_selected_folder(app: &AppState) {
    if let Some(dir) = app.current_selected_port().and_then(|port| game_dir(app, &port)) {
        crate::core::launch::open_folder(&dir);
    }
}

// Fork: the uninstall runs on a worker from the Port page (`port_page_screen::uninstall`; game files are moved to Kept
// files first, which can take a while across drives). Upstream's guard and bookkeeping are split out so the page can
// wait between them; every uninstall entry (All ports row, Info, Port page) goes through that page.
pub(crate) fn may_uninstall(app: &AppState, port: &Port) -> bool {
    let installing = app.install_runtime.installing.borrow().contains(port.key());
    !(installing || port.user_managed || is_port_running(app, port.key()))
}

pub(crate) fn mark_uninstalled(app: &Rc<AppState>, port: &Port) {
    app.state.borrow_mut().mark_removed(port.key());
    app.refresh_current_view();
    app.refresh_tray_recent_games();
}

pub(crate) fn open_port_folder(app: &AppState, port: &Port) {
    if let Some(dir) = game_dir(app, port) {
        crate::core::launch::open_folder(&dir);
    }
}
