
use super::dialogs::{open_message_dialog, tr};
use super::events::{lock, AppEvent};
use super::state::AppState;
use crate::core::catalog_sync::{self, CatalogUpdate};
use crate::core::state::StateManager;
use crate::core::version::{APP_VERSION, SELF_REPO};
use crate::fork::self_update;
use crate::fork::settings::ForkSettings;
use crate::ui::gamepad_router::GamepadRouter;
use crate::Tr;
use slint::ComponentHandle;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

#[cfg(target_os = "windows")]
const UPDATER_NAME: &str = "ports_launcher_updater.bat";
#[cfg(target_os = "linux")]
const UPDATER_NAME: &str = "ports_launcher_updater.sh";

pub(crate) fn launch_self_update(app: &Rc<AppState>, router: &Rc<RefCell<GamepadRouter>>) {
    match crate::core::launch::launch(&app.paths.config_dir.join(UPDATER_NAME)) {
        Ok(_) => {
            let mut state = app.state.borrow_mut();
            state.mark_launcher_update_check();
            state.set_launcher_update_available(false);
            let _ = slint::quit_event_loop();
        }
        Err(e) => open_message_dialog(app, router, &tr!(app).invoke_dialog_title_update_error(), &e.to_string()),
    }
}

/// Fork (upstream bug O4): run once at startup, before the footer shows the saved flag. A flag that no
/// longer applies to the build now running (it was updated by copying files over, or `state.json` came
/// from another copy) is cleared. Returns true when a check should run now instead of waiting a day.
pub(crate) fn settle_self_update_flag(state: &mut StateManager, config_dir: &Path) -> bool {
    if !state.launcher_update_available {
        return false;
    }
    let running = self_update::running(config_dir, APP_VERSION);
    let settings = ForkSettings::load(&config_dir.join(crate::fork::settings::FILE_NAME));
    if self_update::flag_still_applies(&running, self_update::saved_offer(&settings).as_ref()) {
        return false;
    }
    eprintln!("self-update: the saved Update flag does not apply to this build; checking again");
    state.set_launcher_update_available(false);
    state.release_sync
}

/// `force` skips the once-a-day throttle (used after `settle_self_update_flag` cleared a stale flag).
pub(crate) fn start_self_update_check(app: &AppState, force: bool) {
    let github_token = {
        let mut state = app.state.borrow_mut();
        if !(force && state.release_sync) && !state.should_check_launcher_update() {
            return;
        }
        state.mark_launcher_update_check();
        state.github_token.clone()
    };
    let running = self_update::running(&app.paths.config_dir, APP_VERSION);
    let events = app.events.clone();
    std::thread::spawn(move || {
        // Fork (upstream bug O5): compare commits (BUILD_INFO vs the release notes), dates only as a fallback.
        match crate::core::github_api::candidate_releases(SELF_REPO, github_token.as_deref()).and_then(self_update::offer_from_releases) {
            Ok(offer) if self_update::is_update(&running, &offer) => lock(&events).push(AppEvent::SelfUpdateAvailable(offer)),
            Ok(_) => {}
            Err(e) => eprintln!("self-update check failed: {}", e.message()),
        }
    });
}

pub(crate) fn start_catalog_sync(app: &AppState) {
    let known_etag = {
        let state = app.state.borrow();
        if !state.should_check_catalog() {
            return;
        }
        state.last_catalog_etag.clone()
    };
    let ports_json_path = app.paths.config_dir.join("ports.json");
    let ports_local_json_path = app.paths.config_dir.join("ports.local.json");
    let events = app.events.clone();
    std::thread::spawn(move || {
        let known_etag = catalog_sync::etag_for_saved_copy(&known_etag, &ports_json_path, catalog_sync::validate_ports);
        match catalog_sync::fetch_ports_if_changed(&known_etag) {
            Ok(CatalogUpdate::NotModified) => lock(&events).push(AppEvent::PortsCheckDone { etag: known_etag }),
            Ok(CatalogUpdate::Updated { text, etag }) => {
                let Ok(remote_ports) = crate::core::config::parse_catalog(&text) else { return };
                let ports = crate::core::config::merge_local_catalog(remote_ports, crate::core::config::load_local_config(&ports_local_json_path));
                let saved = crate::core::files::write_atomic(&ports_json_path, text.as_bytes()).is_ok();
                let mut events = lock(&events);
                events.push(AppEvent::PortsCheckDone { etag: catalog_sync::etag_to_record(saved, etag, known_etag) });
                events.push(AppEvent::RemoteCatalogFetched(ports));
            }
            // Fork (upstream bug O6): say why, and wait an hour (or what the server asked) before asking again.
            Err(e) => {
                let retry_in = catalog_sync::retry_delay(&e);
                eprintln!("catalog check failed: {} (next try in {} min)", e.message, retry_in.as_secs() / 60);
                lock(&events).push(AppEvent::PortsCheckFailed { retry_in });
            }
        }
    });
}

pub(crate) fn start_themes_sync(app: &AppState) {
    let known_etag = {
        let state = app.state.borrow();
        if !state.should_check_themes() {
            return;
        }
        state.last_themes_etag.clone()
    };
    let themes_path = app.paths.themes_path.clone();
    let events = app.events.clone();
    std::thread::spawn(move || {
        let known_etag = catalog_sync::etag_for_saved_copy(&known_etag, &themes_path, catalog_sync::validate_themes);
        match catalog_sync::fetch_themes_if_changed(&known_etag) {
            Ok(CatalogUpdate::NotModified) => lock(&events).push(AppEvent::ThemesCheckDone { etag: known_etag }),
            Ok(CatalogUpdate::Updated { text, etag }) => {
                let saved = crate::core::files::write_atomic(&themes_path, text.as_bytes()).is_ok();
                let mut events = lock(&events);
                events.push(AppEvent::ThemesCheckDone { etag: catalog_sync::etag_to_record(saved, etag, known_etag) });
                if saved {
                    events.push(AppEvent::RemoteThemesFetched);
                }
            }
            Err(e) => {
                let retry_in = catalog_sync::retry_delay(&e);
                eprintln!("themes check failed: {} (next try in {} min)", e.message, retry_in.as_secs() / 60);
                lock(&events).push(AppEvent::ThemesCheckFailed { retry_in });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUNNING: &str = "a2ec28a0f7f3c1a1b2c3d4e5f60718293a4b5c6d";
    const OTHER: &str = "21fc151d0e9f8a7b6c5d4e3f2a1b0c9d8e7f6a5b";

    /// A fake install folder: BUILD_INFO with RUNNING, state.json with the flag set, and optionally the
    /// saved offer. Returns the folder and the loaded state.
    fn install(name: &str, release_sync: bool, offer_commit: Option<&str>) -> (std::path::PathBuf, StateManager) {
        let dir = std::env::temp_dir().join(format!("dpm_settle_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(self_update::BUILD_INFO), format!("commit={RUNNING}\nbuild_date={APP_VERSION}\n")).unwrap();
        std::fs::write(dir.join("state.json"), format!(r#"{{"launcher_update_available": true, "release_sync": {release_sync}}}"#)).unwrap();
        if let Some(commit) = offer_commit {
            let settings = format!(r#"{{"self_update_offer": {{"commit": "{commit}", "date": "{APP_VERSION}T23:59:59Z"}}}}"#);
            std::fs::write(dir.join(crate::fork::settings::FILE_NAME), settings).unwrap();
        }
        let state = StateManager::load(&dir.join("state.json"));
        assert!(state.launcher_update_available);
        (dir, state)
    }

    #[test]
    fn the_offered_build_running_clears_the_flag_and_checks_again() {
        let (dir, mut state) = install("same", true, Some(RUNNING));
        assert!(settle_self_update_flag(&mut state, &dir));
        assert!(!state.launcher_update_available);
        assert!(!StateManager::load(&dir.join("state.json")).launcher_update_available, "saved");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_different_build_offered_the_same_day_keeps_the_flag() {
        let (dir, mut state) = install("other", true, Some(OTHER));
        assert!(!settle_self_update_flag(&mut state, &dir));
        assert!(state.launcher_update_available);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn with_release_sync_off_a_stale_flag_is_cleared_without_a_check() {
        let (dir, mut state) = install("nosync", false, Some(RUNNING));
        assert!(!settle_self_update_flag(&mut state, &dir));
        assert!(!state.launcher_update_available);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_flag_with_no_saved_offer_is_cleared() {
        let (dir, mut state) = install("nooffer", true, None);
        assert!(settle_self_update_flag(&mut state, &dir));
        assert!(!state.launcher_update_available);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_flag_means_nothing_to_do() {
        let (dir, mut state) = install("noflag", true, Some(OTHER));
        state.launcher_update_available = false;
        assert!(!settle_self_update_flag(&mut state, &dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
