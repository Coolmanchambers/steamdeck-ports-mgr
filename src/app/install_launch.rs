
use super::dialogs::{dialog_is_open, open_error_dialog, open_info_dialog, open_message_dialog, open_picker_dialog, open_progress_dialog, open_update_toggle_dialog, tr};
use super::events::{lock, AppEvent};
use super::playtime::is_port_running;
use super::state::AppState;
use crate::core::executable_detect::ExecutableSelectionError;
use crate::core::installer::{InstallOverrides, InstallPaths};
use crate::core::jobs::InstallOutcome;
use crate::core::models::{Port, SourceType};
use crate::ui::gamepad_router::GamepadRouter;
use crate::Tr;
use serde_json::Value;
use slint::ComponentHandle;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

type Router = Rc<RefCell<GamepadRouter>>;

const VERSION_CHOICES: usize = 3;

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
        open_info_dialog(app, router, &port);
        return;
    }
    let Some(key) = try_claim_op_slot(app, &port) else { return };

    let window = app.window();
    let tr = window.global::<Tr>();
    open_progress_dialog(app, router, &tr.invoke_dialog_title_installing(), &tr.invoke_progress_installing(port.name.clone().into()));

    let pin_version = release_override.is_some();
    let (github_token, gitlab_token) = app.api_tokens();
    let library_dir = app.paths.library_dir();
    let saves_backup_dir = app.paths.saves_backup_dir.clone();
    let events = app.events.clone();
    let mut on_progress = progress_sender(&events);

    std::thread::spawn(move || {
        let overrides = InstallOverrides { asset: asset_override.as_ref(), release: release_override.as_ref() };
        let paths = InstallPaths { library_dir: &library_dir, saves_backup_dir: &saves_backup_dir };
        let outcome = crate::core::jobs::run_install(&port, paths, github_token.as_deref(), gitlab_token.as_deref(), overrides, &mut on_progress);
        let event = match outcome {
            InstallOutcome::Done { tag } => AppEvent::InstallDone { key, tag, pin_version },
            InstallOutcome::AssetAmbiguous { assets } => AppEvent::InstallAssetAmbiguous { key, assets, release_override },
            InstallOutcome::Error(message) => AppEvent::InstallError { key, message },
        };
        lock(&events).push(event);
    });
}

pub(crate) fn open_version_picker(app: &Rc<AppState>, router: &Router, port: Port) {
    let Some(key) = try_claim_op_slot(app, &port) else { return };
    let window = app.window();
    let tr = window.global::<Tr>();
    open_progress_dialog(app, router, &tr.invoke_dialog_title_loading(), &tr.invoke_progress_fetching_versions(port.name.clone().into()));

    let (github_token, gitlab_token) = app.api_tokens();
    let events = app.events.clone();
    let repo = port.repo.clone().unwrap_or_default();
    let source_type = port.source_type;
    std::thread::spawn(move || {
        let result = match source_type {
            SourceType::Github => crate::core::github_api::list_releases(&repo, github_token.as_deref(), VERSION_CHOICES),
            SourceType::Gitlab => crate::core::gitlab_api::list_releases(&repo, gitlab_token.as_deref(), VERSION_CHOICES),
            SourceType::DirectUrl | SourceType::None => Ok(Vec::new()),
        };
        let event = match result {
            Ok(releases) if !releases.is_empty() => AppEvent::VersionsFetched { key, releases },
            Ok(_) => AppEvent::VersionsFetchError { key, message: "This source has no version history.".to_string() },
            Err(e) => AppEvent::VersionsFetchError { key, message: e.message().to_string() },
        };
        lock(&events).push(event);
    });
}

pub(crate) fn start_extra_install(app: &Rc<AppState>, router: &Router, port: Port) {
    let Some(key) = try_claim_op_slot(app, &port) else { return };
    let window = app.window();
    let tr = window.global::<Tr>();
    open_progress_dialog(app, router, &tr.invoke_dialog_title_extras(), &tr.invoke_progress_installing_extras(port.name.clone().into()));

    let library_dir = app.paths.library_dir();
    let events = app.events.clone();
    let mut on_progress = progress_sender(&events);
    std::thread::spawn(move || {
        let result = crate::core::jobs::run_extra_install(&port, &library_dir, &mut on_progress);
        lock(&events).push(AppEvent::ExtraInstallDone { key, result });
    });
}

const CHOOSE_PROGRAM_TITLE: &str = "Choose the program to start (remembered; change it in Info)";

fn file_label(path: &Path) -> String {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string()
}

pub(crate) fn open_favorite_exe_picker(app: &Rc<AppState>, router: &Router, port: Port) {
    let Some(game_dir) = game_dir(app, &port).filter(|dir| dir.is_dir()) else { return };
    // Fork: the full list of programs, shallowest folder first (the launch list shows only the shallowest).
    let candidates = crate::core::executable_detect::list_candidates(&game_dir);
    if candidates.is_empty() {
        return open_info_dialog(app, router, &port);
    }
    let mut labels = vec![tr!(app).invoke_picker_ask_every_time().to_string()];
    labels.extend(candidates.iter().map(|p| option_label(&game_dir, p)));
    let key = port.key().to_string();
    open_picker_dialog(app, router, &tr!(app).invoke_dialog_title_choose_favorite_executable(), labels, move |app, _, idx| {
        let exe = idx.checked_sub(1).and_then(|i| candidates.get(i)).and_then(|p| crate::fork::program_detect::relative_pick(&game_dir, p));
        app.state.borrow_mut().set_favorite_exe(&key, exe);
    });
}

// Fork: a program below the top folder is named with its folder, so two `java` files can be told apart.
fn option_label(game_dir: &Path, path: &Path) -> String {
    match crate::fork::program_detect::relative_pick(game_dir, path) {
        Some(rel) if crate::fork::program_detect::depth_in(game_dir, path) > 0 => rel.replace('\\', "/"),
        _ => file_label(path),
    }
}

/// True when the program was started.
fn launch_executable(app: &Rc<AppState>, router: &Router, port: &Port, exe: &Path) -> bool {
    if is_port_running(app, port.key()) {
        return false;
    }
    if !exe.exists() {
        open_error_dialog(app, router, port.clone());
        return false;
    }
    let child = match crate::core::launch::launch(exe) {
        Ok(child) => child,
        Err(e) => {
            eprintln!("launch: could not start {}: {e}", exe.display());
            return false;
        }
    };
    let key = port.key().to_string();
    let runtime = &app.install_runtime;
    runtime.running_processes.borrow_mut().insert(key.clone(), child);
    runtime.launch_started_at.borrow_mut().insert(key.clone(), Instant::now());
    if app.state.borrow().discord_rpc_enabled {
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

pub(crate) fn launch_flow(app: &Rc<AppState>, router: &Router, port: &Port) {
    // Fork: the tray's recent games call this directly; never start a game while it installs, updates or
    // has its saves restored (the install slot is held for all three).
    if app.install_runtime.installing.borrow().contains(port.key()) {
        // Its progress dialog is usually up; leave that in place rather than replacing it with this note.
        if !dialog_is_open(app) {
            open_message_dialog(app, router, &port.name, "This port is being installed, updated or restored. Wait for that to finish, then start it.");
        }
        return;
    }
    let Some(game_dir) = game_dir(app, port) else {
        let window = app.window();
        let tr = window.global::<Tr>();
        return open_message_dialog(app, router, &tr.invoke_dialog_title_invalid_port(), &tr.invoke_message_invalid_folder_name());
    };
    if !game_dir.exists() {
        return open_error_dialog(app, router, port.clone());
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
            let labels = candidates.iter().map(|p| file_label(p)).collect();
            let port = port.clone();
            let game_dir = game_dir.clone();
            // Fork: the title says the choice is remembered (upstream's text is just "Choose an executable").
            open_picker_dialog(app, router, CHOOSE_PROGRAM_TITLE, labels, move |app, router, idx| {
                if let Some(exe) = candidates.get(idx) {
                    // Fork: a pick that starts is remembered, so the next start does not ask again (Info can set it
                    // back to "Ask every time"); one that fails to start is not.
                    if launch_executable(app, router, &port, exe) {
                        if let Some(rel) = crate::fork::program_detect::relative_pick(&game_dir, exe) {
                            app.state.borrow_mut().set_favorite_exe(port.key(), Some(rel));
                        }
                    }
                }
            });
        }
        Err(ExecutableSelectionError::NotFound) => open_info_dialog(app, router, port),
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
        with_indexed_port(app, index, |port| start_install(app, router, port, None, None));
    }
}

pub(crate) fn open_update_toggle_dialog_row(app: &Rc<AppState>, router: &Router, index: i32) {
    if !dialog_is_open(app) {
        with_indexed_port(app, index, |port| open_update_toggle_dialog(app, router, port));
    }
}

pub(crate) fn activate_port(app: &Rc<AppState>, router: &Router, port: &Port) {
    if dialog_is_open(app) || app.install_runtime.installing.borrow().contains(port.key()) {
        return;
    }
    if app.is_installed(port) {
        launch_with_update_check(app, router, port);
    } else {
        start_install(app, router, port.clone(), None, None);
    }
}

fn launch_with_update_check(app: &Rc<AppState>, router: &Router, port: &Port) {
    let check = {
        let state = app.state.borrow();
        let info = state.get(port.key());
        let installed_at = info.map(|i| i.installed_at.clone()).unwrap_or_default();
        // Fork: one rule for Play and the home screen (crate::fork::home::update_check_due).
        crate::fork::home::update_check_due(state.release_sync, &port.source_type, info.map(|i| i.update), &installed_at).then(|| (info.and_then(|i| i.installed_tag.clone()), installed_at))
    };
    let Some((tag, installed_at)) = check else {
        return launch_flow(app, router, port);
    };
    if try_claim_op_slot(app, port).is_none() {
        return;
    }
    let (github_token, gitlab_token) = app.api_tokens();
    let port = port.clone();
    let events = app.events.clone();
    std::thread::spawn(move || {
        let available = crate::core::jobs::run_update_check(&port, tag.as_deref(), &installed_at, github_token.as_deref(), gitlab_token.as_deref());
        lock(&events).push(AppEvent::PlayUpdateChecked { port: Box::new(port), available });
    });
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
