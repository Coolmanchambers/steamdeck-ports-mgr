//! The "Restore saves" screens: Info dialog > "Restore saves..." and Settings > "Restore saves...".
//!
//! Every screen is a list built from the picker dialog the app already has, so it works with the
//! controller (D-pad, A, B), keyboard and mouse, in a window or fullscreen. What the screens say is
//! built in `crate::fork::save_restore::view` and the restore itself is
//! `crate::fork::save_restore::restore` (both tested without any UI); this file draws the lists and
//! runs the restore on a worker thread behind a progress dialog. English only, like the fork's other
//! screens.

use super::dialogs::{
    build_picker_dialog, close_current_dialog, finish_dialog_open, open_confirm_dialog, open_info_dialog, open_message_dialog, open_progress_dialog, open_settings_dialog,
    update_progress_dialog, DialogSlot,
};
use super::playtime::is_port_running;
use super::state::AppState;
use crate::core::models::Port;
use crate::core::path_safety::safe_join;
use crate::fork::rom_screens::job::Background;
use crate::fork::save_restore::{self, view, Backup, RestoreError, RestoreInput, SaveTarget};
use crate::ui::gamepad_router::GamepadRouter;
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

type Router = Rc<RefCell<GamepadRouter>>;
type Back = Rc<dyn Fn(&Rc<AppState>, &Router)>;

const TITLE: &str = "Restore saves";

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

fn port_title(port: &Port) -> String {
    format!("Restore saves: {}", port.display_name())
}

/// A list whose information rows do nothing when chosen; action rows open the next dialog.
fn open_list(app: &Rc<AppState>, router: &Router, title: &str, labels: Vec<String>, on_select: impl Fn(&Rc<AppState>, &Router, usize) + 'static) {
    let Some(dialog) = build_picker_dialog(app, router, title, &labels) else { return };
    {
        let (app, router) = (app.clone(), router.clone());
        dialog.on_item_selected(move |index| on_select(&app, &router, index as usize));
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
}

fn targets_for(app: &AppState, port: &Port) -> Option<Vec<SaveTarget>> {
    let game_dir = safe_join(&app.paths.library_dir, &port.folder).ok()?;
    Some(save_restore::save_targets(port, &game_dir, &app.paths.saves_backup_dir, home_dir().as_deref()))
}

// -- Entry points -----------------------------------------------------------------------------------

/// Info dialog > "Restore saves...". Back returns to the Info dialog.
pub(crate) fn open_from_info(app: &Rc<AppState>, router: &Router, port: Port) {
    let back: Back = {
        let port = port.clone();
        Rc::new(move |app, router| open_info_dialog(app, router, &port))
    };
    show_port(app, router, &port, back);
}

/// Settings > "Restore saves...": every port that has at least one backup.
pub(crate) fn open_from_settings(app: &Rc<AppState>, router: &Router) {
    let all = save_restore::list_all_backups(&app.paths.saves_backup_dir);
    let rows: Vec<(String, Option<Port>, usize)> = {
        let catalog = app.catalog.borrow();
        all.iter()
            .map(|(folder, backups)| {
                let port = catalog.iter().find(|p| &p.folder == folder && !p.is_android_only()).cloned();
                let label = match &port {
                    Some(p) => view::port_row(p.display_name(), backups),
                    None => format!("{} (not in the catalog)", view::port_row(folder, backups)),
                };
                (label, port, backups.len())
            })
            .collect()
    };
    let mut labels: Vec<String> = rows.iter().map(|(label, ..)| label.clone()).collect();
    if rows.is_empty() {
        labels.push("No save backups yet. One is made before every update, and by Settings > Backup Saves.".into());
    }
    let back_index = labels.len();
    labels.push("Back".into());
    let backups_dir = app.paths.saves_backup_dir.clone();
    open_list(app, router, TITLE, labels, move |app, router, index| {
        if index == back_index {
            close_current_dialog(app, router);
            return open_settings_dialog(app, router);
        }
        match rows.get(index) {
            Some((_, Some(port), _)) => {
                let back: Back = Rc::new(|app, router| open_from_settings(app, router));
                show_port(app, router, port, back);
            }
            Some((label, None, _)) => message(
                app,
                router,
                TITLE,
                &format!(
                    "{label}\n\nThat port is not in the catalog any more, so the app does not know where its saves go and cannot restore them. The backups stay in {}.",
                    backups_dir.display()
                ),
            ),
            None => {}
        }
    });
}

// -- One port ---------------------------------------------------------------------------------------

fn show_port(app: &Rc<AppState>, router: &Router, port: &Port, back: Back) {
    let Some(targets) = targets_for(app, port) else {
        return message(app, router, &port_title(port), "This port's folder name is not usable, so its saves cannot be restored.");
    };
    if !view::any_restorable(&targets) {
        return message(app, router, &port_title(port), &view::nothing_restorable_text(port.display_name(), &targets));
    }
    let backups = save_restore::list_backups(&app.paths.saves_backup_dir, &port.folder);
    let mut labels: Vec<String> = targets.iter().map(view::target_row).collect();
    let first_backup = labels.len();
    labels.extend(backups.iter().map(view::backup_row));
    if backups.is_empty() {
        labels.push("No backups of these saves yet. One is made before every update, and by Settings > Backup Saves.".into());
    }
    let back_index = labels.len();
    labels.push("Back".into());
    let port = port.clone();
    open_list(app, router, &port_title(&port), labels, move |app, router, index| {
        if index == back_index {
            return back(app, router);
        }
        if let Some(backup) = index.checked_sub(first_backup).and_then(|i| backups.get(i)) {
            choose_backup(app, router, &port, backup.clone());
        }
    });
}

fn busy_reason(app: &AppState, port: &Port) -> Option<String> {
    if is_port_running(app, port.key()) {
        return Some(RestoreError::GameRunning.to_string());
    }
    if app.install_runtime.installing.borrow().contains(port.key()) {
        return Some("This port is being installed or updated. Wait for that to finish, then restore; nothing was changed.".into());
    }
    if RESTORE_RUNNING.with(Cell::get) {
        return Some("Another restore is still running. Wait for it to finish, then try again.".into());
    }
    None
}

fn choose_backup(app: &Rc<AppState>, router: &Router, port: &Port, backup: Backup) {
    let title = port_title(port);
    if let Some(problem) = &backup.problem {
        return message(app, router, &title, &RestoreError::UnusableBackup(problem.clone()).to_string());
    }
    if let Some(reason) = busy_reason(app, port) {
        return message(app, router, &title, &reason);
    }
    let Some(targets) = targets_for(app, port) else { return };
    let plan = save_restore::plan_restore(&targets, &backup);
    if plan.replace.is_empty() {
        let mut text = "This backup holds no save folder the app can restore here:".to_string();
        for (field, why) in &plan.skipped {
            text.push_str(&format!("\n  {}: {why}", field.label()));
        }
        return message(app, router, &title, &text);
    }
    let text = view::confirm_text(port.display_name(), &backup, &plan, &app.paths.saves_backup_dir, &port.folder);
    let port = port.clone();
    open_confirm_dialog(app, router, title.into(), padded(&text).into(), "Restore".into(), move |app, router| start_restore(app, router, port.clone(), backup.clone()));
}

// -- Running the restore ----------------------------------------------------------------------------

thread_local! {
    static RESTORE_TIMER: slint::Timer = slint::Timer::default();
    static RESTORE_RUNNING: Cell<bool> = const { Cell::new(false) };
}

fn start_restore(app: &Rc<AppState>, router: &Router, port: Port, backup: Backup) {
    let title = port_title(&port);
    // Checked again here: the game may have been started while the confirm dialog was up.
    if let Some(reason) = busy_reason(app, &port) {
        return message(app, router, &title, &reason);
    }
    let Ok(game_dir) = safe_join(&app.paths.library_dir, &port.folder) else { return };
    let key = port.key().to_string();
    // Holding the port's install slot keeps an install, update or launch of it from starting meanwhile.
    app.install_runtime.installing.borrow_mut().insert(key.clone());
    RESTORE_RUNNING.with(|r| r.set(true));
    open_progress_dialog(app, router, &title, "Backing up your current saves...");

    let saves_backup_dir = app.paths.saves_backup_dir.clone();
    let home = home_dir();
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let worker_port = port.clone();
    let job = Background::spawn(move |ctl| {
        let input = RestoreInput {
            port: &worker_port,
            game_dir: &game_dir,
            saves_backup_dir: &saves_backup_dir,
            home: home.as_deref(),
            backup: &backup,
            game_running: false,
            stamp: &stamp,
        };
        save_restore::restore(&input, &mut |step| ctl.set_progress(step.to_string()))
    });

    let (app, router) = (app.clone(), router.clone());
    let last = RefCell::new(String::new());
    RESTORE_TIMER.with(|timer| {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
            if let Some(result) = job.take() {
                RESTORE_TIMER.with(slint::Timer::stop);
                RESTORE_RUNNING.with(|r| r.set(false));
                app.install_runtime.installing.borrow_mut().remove(&key);
                if matches!(&*app.dialog_nav.dialogs.borrow(), DialogSlot::Progress(_)) {
                    close_current_dialog(&app, &router);
                }
                match result {
                    Ok(Ok(done)) => message(&app, &router, "Saves restored", &view::done_text(port.display_name(), &done)),
                    Ok(Err(e)) => message(&app, &router, &title, &e.to_string()),
                    Err(panic) => message(&app, &router, &title, &panic),
                }
                return;
            }
            let text = job.progress();
            if text.is_empty() || text == *last.borrow() {
                return;
            }
            *last.borrow_mut() = text.clone();
            if let DialogSlot::Progress(d) = &*app.dialog_nav.dialogs.borrow() {
                update_progress_dialog(&app, d, &text);
            }
        });
    });
}
