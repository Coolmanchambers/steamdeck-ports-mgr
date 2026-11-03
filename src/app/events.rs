use super::dialogs::{apply_theme, close_current_dialog, open_message_dialog, update_progress_dialog, DialogSlot};
use super::install_launch::{finish_play_update_check, play_port};
use super::playtime::{any_process_running, checkpoint_playtime, refresh_live_playtime_display};
use super::state::AppState;
use crate::core::models::Port;
use crate::ui::chrome;
use crate::ui::gamepad_router::GamepadRouter;
use crate::Tr;
use serde_json::Value;
use slint::ComponentHandle;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) enum AppEvent {
    InstallProgress { message: String },
    InstallDone { key: String, tag: Option<String>, pin_version: bool },
    InstallAssetAmbiguous { key: String, assets: Vec<Value>, release_override: Option<Value> },
    InstallError { key: String, message: String },
    ExtraInstallDone { key: String, result: Result<(), String> },
    // Fork: `answered` is false when the check got no answer (offline, rate limit); then nothing is stamped.
    PlayUpdateChecked { port: Box<Port>, available: bool, answered: bool },
    // Fork: `Some(offer)` carries what raised the flag, so a later start can tell whether it still applies (O4);
    // `None` means the check found no update and clears the flag (upstream 2026-10-30).
    SelfUpdateChecked { offer: Option<crate::fork::self_update::Offer> },
    VersionsFetched { key: String, releases: Vec<Value> },
    VersionsFetchError { key: String, message: String },
    ImageCached { folder: String },
    BringToForeground,
    RemoteCatalogFetched(Vec<Port>),
    PortsCheckDone { etag: String },
    RemoteThemesFetched,
    ThemesCheckDone { etag: String },
    // Fork (upstream bug O6): a failed catalog or themes check, retried after `retry_in`.
    PortsCheckFailed { retry_in: std::time::Duration },
    ThemesCheckFailed { retry_in: std::time::Duration },
    SaveBackupProgress { name: String },
    SaveBackupDone { copied: usize, skipped: usize, failed: usize, no_linux_folder: Vec<String> },
    // Fork: the Deck-ready list finished a background check.
    DeckReadyFetched(Box<crate::fork::deck_ready::sync::Outcome>),
}

// Fork (install wizard): true when the install had been sent to the background (whatever is on screen now is left alone).
fn finish_operation(app: &Rc<AppState>, router: &Rc<RefCell<GamepadRouter>>, key: &str) -> bool {
    app.install_runtime.installing.borrow_mut().remove(key);
    let background = super::install_wizard_screen::end_run(key);
    if !background {
        close_current_dialog(app, router);
    }
    background
}

fn show_progress(app: &AppState, message: impl FnOnce(&Tr<'_>) -> slint::SharedString) {
    if let DialogSlot::Progress(d) = &*app.dialog_nav.dialogs.borrow() {
        let text = message(&d.global::<Tr>());
        update_progress_dialog(app, d, &text);
    }
}

pub(crate) fn poll_app_events(app: &Rc<AppState>, router: &Rc<RefCell<GamepadRouter>>) {
    app.refresh_geometry_if_scale_changed();
    checkpoint_playtime(app);
    refresh_live_playtime_display(app);
    let drained: Vec<AppEvent> = std::mem::take(&mut *lock(&app.events));
    let last_install_progress = drained.iter().rposition(|e| matches!(e, AppEvent::InstallProgress { .. }));
    let last_backup_progress = drained.iter().rposition(|e| matches!(e, AppEvent::SaveBackupProgress { .. }));
    for (i, event) in drained.into_iter().enumerate() {
        match event {
            AppEvent::InstallProgress { .. } if Some(i) != last_install_progress => {}
            AppEvent::SaveBackupProgress { .. } if Some(i) != last_backup_progress => {}
            AppEvent::InstallProgress { message } => show_progress(app, |_| message.into()),
            AppEvent::SaveBackupProgress { name } => show_progress(app, |tr| tr.invoke_progress_backing_up_named(name.into())),
            AppEvent::InstallDone { key, tag, pin_version } => {
                let background = finish_operation(app, router, &key);
                super::cloud_screen::trigger(app, crate::fork::cloud::hooks::Why::UpdateBackup); // Fork: the update's save backup
                let port = app.find_port(&key);
                if let Some(port) = &port {
                    app.grid_nav.card_image_cache.borrow_mut().remove(&port.folder);
                }
                {
                    let mut state = app.state.borrow_mut();
                    state.mark_installed(&key, tag.clone());
                    if pin_version {
                        state.set_port_update(&key, false);
                    }
                }
                // Fork: a release named with its version replaced the old file; launch target, favorite and
                // Steam follow the new one and the old one goes to .previous/ (fork::versioned).
                let launching = app.install_runtime.pending_launch_after_install.borrow().contains(&key);
                let versioned = super::versioned_update::after_install(app, &key, tag.as_deref(), launching);
                app.refresh_current_view();
                if app.install_runtime.pending_launch_after_install.borrow_mut().remove(&key) {
                    // Fork (install wizard): Play's update sent to the background never starts the game wherever the
                    // user is now; it gets the finished note instead.
                    if background {
                        super::install_wizard_screen::finished_in_background(app, router, &key, versioned);
                    } else if let Some(port) = port {
                        play_port(app, router, &port);
                    }
                } else if super::install_wizard_screen::after_install(app, router, &key, versioned.clone()) {
                    // Fork (install wizard): its next pages (kept files, game files, Steam, done) or its note.
                } else if let (Some(text), Some(port)) = (versioned, &port) {
                    open_message_dialog(app, router, port.display_name(), &text);
                } else {
                    // Fork: look for the port's game files in the ROM folder (once per port).
                    super::rom_screens::after_install(app, router, &key);
                }
            }
            AppEvent::InstallAssetAmbiguous { key, assets, release_override } => {
                finish_operation(app, router, &key);
                let was_play = app.install_runtime.pending_launch_after_install.borrow_mut().remove(&key);
                // Fork (install wizard): the choice is a card page in the wizard's style (was upstream's "Choose file"
                // picker); Not now drops the flow, and for Play (O2) starts the installed game, like a failed update.
                super::install_wizard_screen::choose_asset(app, router, &key, assets, release_override, was_play);
            }
            // Fork (port dialogs): the version list, its error and the extras' result are pages (`port_actions_screen`), not
            // upstream's "Choose a version" picker and "Error" message.
            AppEvent::VersionsFetched { key, releases } => super::port_actions_screen::versions_fetched(app, router, &key, Ok(releases)),
            AppEvent::VersionsFetchError { key, message } => super::port_actions_screen::versions_fetched(app, router, &key, Err(message)),
            AppEvent::InstallError { key, message } => {
                let background = finish_operation(app, router, &key);
                super::cloud_screen::trigger(app, crate::fork::cloud::hooks::Why::UpdateBackup); // Fork: a backup may have been made before it failed
                let launch = app.install_runtime.pending_launch_after_install.borrow_mut().remove(&key);
                // Fork (install wizard): a stop has nothing to report; any other failure is the wizard's "Could not
                // install" page (was upstream's "Installation error" message), whose Back returns where the install
                // started. A failed Play update still starts the installed game (upstream), unless it was in the
                // background.
                super::install_wizard_screen::install_failed(app, router, &key, &message, background, launch);
            }
            AppEvent::ExtraInstallDone { key, result } => super::port_actions_screen::extras_done(app, router, &key, result),
            AppEvent::PlayUpdateChecked { port, available, answered } => finish_play_update_check(app, router, *port, available, answered),
            AppEvent::SelfUpdateChecked { offer } => {
                if let Some(offer) = &offer {
                    let mut settings = crate::fork::settings::ForkSettings::load(&app.paths.config_dir.join(crate::fork::settings::FILE_NAME));
                    if let Err(e) = crate::fork::self_update::save_offer(&mut settings, offer) {
                        eprintln!("self-update: could not remember the offer ({e})");
                    }
                }
                let mut state = app.state.borrow_mut();
                state.set_launcher_update_available(offer.is_some());
                app.window().set_self_update_available(state.show_launcher_update());
            }
            AppEvent::ImageCached { folder } => {
                app.grid_nav.pending_image_fetches.borrow_mut().remove(&folder);
                let cached = crate::core::image_cache::cached_image_path(&app.paths.cache_dir, &folder).is_ok_and(|p| p.exists());
                if cached {
                    app.grid_nav.card_image_cache.borrow_mut().remove(&folder);
                    app.refresh_current_view();
                }
            }
            AppEvent::RemoteCatalogFetched(ports) => {
                *app.catalog.borrow_mut() = ports;
                app.refresh_current_view();
            }
            AppEvent::PortsCheckDone { etag } => app.state.borrow_mut().mark_catalog_check(etag),
            AppEvent::ThemesCheckDone { etag } => app.state.borrow_mut().mark_themes_check(etag),
            AppEvent::PortsCheckFailed { retry_in } => app.state.borrow_mut().defer_catalog_check(retry_in),
            AppEvent::ThemesCheckFailed { retry_in } => app.state.borrow_mut().defer_themes_check(retry_in),
            AppEvent::RemoteThemesFetched => {
                let active = app.state.borrow().active_theme.clone();
                crate::ui::theme::load(&app.paths.themes_path, &mut app.theme.theme_config.borrow_mut(), &active);
                apply_theme(&app.window(), &app.theme.theme_config.borrow(), app.window_geometry.border_width.get());
            }
            AppEvent::SaveBackupDone { copied, skipped, failed, no_linux_folder } => {
                close_current_dialog(app, router);
                let window = app.window();
                let tr = window.global::<Tr>();
                let summary = tr.invoke_message_backup_summary(copied as i32, skipped as i32, failed as i32);
                // Fork: name the installed ports whose catalog entry gives no Linux save folder.
                let summary = format!("{summary}{}", crate::fork::save_notes::no_linux_folder_note(&no_linux_folder));
                open_message_dialog(app, router, &tr.invoke_dialog_title_saves_backup(), &summary);
                super::cloud_screen::trigger(app, crate::fork::cloud::hooks::Why::SaveBackup); // Fork: copy it to the cloud
            }
            AppEvent::BringToForeground => chrome::restore_window(app.window().window()),
            AppEvent::DeckReadyFetched(outcome) => super::deck_ready_screen::on_fetched(app, *outcome),
        }
    }

    let still_running = any_process_running(app);
    let runtime = &app.install_runtime;
    if runtime.minimized_for_game.get() && !still_running {
        runtime.minimized_for_game.set(false);
        let window = app.window();
        if window.get_big_mode() {
            chrome::restore_window(window.window());
        }
    }
}
