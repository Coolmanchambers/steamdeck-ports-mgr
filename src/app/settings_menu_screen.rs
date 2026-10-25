//! Settings as pages (`crate::fork::settings_menu` says what they show; the window is `page_dialog::show_page`).
//!
//! The first page has a card for each group with a line of where it stands; a group's page has its actions as cards,
//! and the two switches (Check for updates, Discord status) flip in place. B goes up one level: group page, Settings,
//! the main window. Every entry of the old list does what it did (`run_entry`), including the
//! save-restore refusal guard. Screens that used to lead back to the Settings list (Cloud backup, Artwork sources,
//! Restore saves, Ports folder) lead back to their group's page (`open_group`).
//!
//! The lines are read when a page is drawn and none of them waits on slow input or output: the mount table, the queue
//! file, one look at the backup folder.

use super::dialogs::{close_current_dialog, dialog_is_open, open_files_picker, open_language_picker, open_theme_picker, start_save_backup};
use super::page_dialog::{show_page, Handlers};
use super::state::AppState;
use crate::fork::folder_picker::mounts::{default_media_roots, removable_drives, system_mounts};
use crate::fork::settings_menu::{self as menu, Act, Cloud, Facts, Group, Labels};
use crate::fork::setup_wizard::places;
use crate::ui::gamepad_router::GamepadRouter;
use crate::Tr;
use slint::ComponentHandle;
use std::cell::RefCell;
use std::rc::Rc;

type Router = Rc<RefCell<GamepadRouter>>;

/// Settings, from the Home screen's Y, the title bar's button, the tray and `open_settings_dialog`.
pub(crate) fn open(app: &Rc<AppState>, router: &Router) {
    show_top(app, router);
}

/// A group's page, for the screens that lead back to Settings (their Back rows). Like Settings itself it gives way to the
/// setup wizard when that is waiting for the screen the user is leaving.
pub(crate) fn open_group(app: &Rc<AppState>, router: &Router, group: Group) {
    if dialog_is_open(app) {
        return;
    }
    if super::setup_wizard_screen::resume_instead_of_menu(app, router, crate::fork::setup_wizard::flow::Menu::Settings) {
        return;
    }
    show_group(app, router, group);
}

fn leave(app: &Rc<AppState>, router: &Router) {
    close_current_dialog(app, router);
}

fn show_top(app: &Rc<AppState>, router: &Router) {
    let page = menu::top_page(&labels(app), &facts(app));
    let handlers = Handlers {
        on_action: Rc::new(|app, router, act| match act {
            Act::Open(group) => show_group(app, router, group),
            Act::Back => leave(app, router),
            Act::Do(_) => {}
        }),
        on_back: Rc::new(leave),
        on_exit: Rc::new(leave),
        on_input: None,
    };
    show_page(app, router, page, handlers);
}

fn show_group(app: &Rc<AppState>, router: &Router, group: Group) {
    let page = menu::group_page(group, &labels(app), &facts(app));
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| match act {
            Act::Do(entry) => choose(app, router, group, entry),
            Act::Back => show_top(app, router),
            Act::Open(_) => {}
        }),
        on_back: Rc::new(show_top),
        on_exit: Rc::new(leave),
        on_input: None,
    };
    show_page(app, router, page, handlers);
}

/// A card of a group's page was chosen. A switch flips and the page is drawn again with the cursor where it was; the
/// entries that touch saves wait while a restore runs; the rest are what the old list did.
fn choose(app: &Rc<AppState>, router: &Router, group: Group, entry: menu::Entry) {
    if entry.waits_for_restore() && super::save_restore_screen::refuse_while_restoring(app, router) {
        return;
    }
    match entry {
        menu::Entry::CheckUpdates => {
            let value = !app.state.borrow().release_sync;
            app.state.borrow_mut().set_release_sync(value);
            show_group(app, router, group);
        }
        menu::Entry::DiscordRpc => {
            let value = !app.state.borrow().discord_rpc_enabled;
            app.state.borrow_mut().set_discord_rpc_enabled(value);
            show_group(app, router, group);
        }
        _ => run_entry(app, router, entry),
    }
}

/// What a Settings entry does when it is chosen: the entries of the old list, each as it was (the dialog is closed first,
/// then the same call). The two switches are flipped by `choose`, and its restore guard is asked before this.
pub(crate) fn run_entry(app: &Rc<AppState>, router: &Router, entry: menu::Entry) {
    use menu::Entry;
    close_current_dialog(app, router);
    match entry {
        Entry::SetupWizard => super::setup_wizard_screen::open_from_settings(app, router),
        Entry::Themes => open_theme_picker(app, router),
        Entry::Language => open_language_picker(app, router),
        Entry::Files => open_files_picker(app, router),
        Entry::OpenPortsFolder => crate::core::launch::open_folder(&app.paths.library_dir()),
        Entry::PortsFolder => super::ports_folder_screen::open_menu(app, router),
        Entry::AddToSteam => super::steam_screens::open_steam_screen(app, router),
        // Leaving Import ports (B, the end of an import, a notice) comes back to the Ports page.
        Entry::ImportPorts => super::import_screen::open_back_to(app, router, Rc::new(|app, router| open_group(app, router, Group::Ports))),
        Entry::GameFiles => super::rom_screens::open_game_files(app, router),
        Entry::ArtSources => super::art_screens::open_art_sources(app, router),
        Entry::BackupSaves => start_save_backup(app, router),
        Entry::RestoreSaves => super::save_restore_screen::open_from_settings(app, router),
        Entry::CloudBackup => super::cloud_screen::open_from_settings(app, router),
        Entry::Version => crate::core::launch::open_url(&format!("{}/releases", crate::core::version::PROJECT_URL)),
        Entry::ForceUpdate => super::sync::launch_self_update(app, router),
        Entry::AddPort => super::add_port_screen::open(app, router, true),
        Entry::CheckUpdates | Entry::DiscordRpc => {}
    }
}

/// The translated upstream labels the cards that keep upstream's words show.
fn labels(app: &AppState) -> Labels {
    let window = app.window();
    let tr = window.global::<Tr>();
    Labels {
        settings: tr.invoke_dialog_title_settings().to_string(),
        themes: tr.invoke_label_themes().to_string(),
        language: tr.invoke_label_language().to_string(),
        files: tr.invoke_label_files().to_string(),
        library: tr.invoke_label_library().to_string(),
        backup_saves: tr.invoke_label_backup_saves().to_string(),
        check_updates: menu::without_state(&tr.invoke_label_check_updates_on()),
        discord: menu::without_state(&tr.invoke_label_discord_rpc_on()),
        version: tr.invoke_version_tag(version().into()).to_string(),
    }
}

fn version() -> String {
    crate::core::version::APP_VERSION.replace('-', ".")
}

/// Where the ports folder is, in a word or two; `None` when it could not be read.
fn ports_place(app: &AppState) -> Option<String> {
    let current = app.paths.library_dir();
    let internal_default = crate::fork::ports_folder::default_dir(&app.paths.config_dir);
    let drives = removable_drives(&system_mounts(), &default_media_roots());
    menu::place_name(places::place_of(&current, &internal_default, &drives))
}

fn facts(app: &AppState) -> Facts {
    let now = chrono::Utc::now().timestamp();
    let backups = app.paths.saves_backup_dir.join(crate::fork::save_restore::GLOBAL_BACKUPS_DIR);
    let last_backup_days = menu::newest_zip(&backups)
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| menu::days_ago(d.as_secs() as i64, now));
    let state = app.state.borrow();
    Facts {
        ports_place: ports_place(app),
        installed: app.catalog.borrow().iter().filter(|p| !p.is_android_only() && app.is_installed(p)).count(),
        steam_waiting: crate::fork::steam_queue::load(&crate::fork::steam_queue::queue_path(&app.paths.config_dir)).queue.len(),
        last_backup_days,
        cloud: Cloud::from_mark(super::cloud_screen::home_status(app).0),
        check_updates: state.release_sync,
        discord: state.discord_rpc_enabled,
        version: version(),
    }
}
