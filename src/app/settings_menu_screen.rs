//! Settings as pages (`crate::fork::settings_menu` says what they show; the window is `page_dialog::show_page`).
//!
//! The first page has a card for each group with a line of where it stands; a group's page has its actions as cards,
//! and the two switches (Check for updates, Discord status) flip in place. B goes up one level: group page, Settings,
//! the main window. Every entry of the old list does what it did (`dialogs::run_settings_entry`), including the
//! save-restore refusal guard. Screens that used to lead back to the Settings list (Cloud backup, Artwork sources,
//! Restore saves, Ports folder) lead back to their group's page (`open_group`).
//!
//! The lines are read when a page is drawn and none of them waits on slow input or output: the mount table, the queue
//! file, one look at the backup folder.

use super::dialogs::{close_current_dialog, dialog_is_open, run_settings_entry};
use super::page_dialog::{show_page, Handlers};
use super::state::AppState;
use crate::fork::folder_picker::mounts::{default_media_roots, removable_drives, system_mounts};
use crate::fork::settings_menu::{self as menu, Act, Cloud, Facts, Group, Labels};
use crate::fork::setup_wizard::places::{self, Kind};
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
    let page = menu::top_page(&facts(app));
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
        _ => run_settings_entry(app, router, entry),
    }
}

/// The translated upstream labels the cards that keep upstream's words show.
fn labels(app: &AppState) -> Labels {
    let window = app.window();
    let tr = window.global::<Tr>();
    Labels {
        themes: tr.invoke_label_themes().to_string(),
        language: tr.invoke_label_language().to_string(),
        files: tr.invoke_label_files().to_string(),
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
    Some(
        match places::place_of(&current, &internal_default, &drives) {
            Some(Kind::SdCard) => "SD card",
            Some(Kind::OtherDrive) => "Another drive",
            Some(Kind::Internal) => "Internal storage",
            _ => "Another folder",
        }
        .to_string(),
    )
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
