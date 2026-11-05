//! Fork: Back up saves (Settings > Saves, and Fix problems) as a waiting page and a done page instead of upstream's
//! "Saves Backup" progress dialog and its counts of save folders. What the pages say is `crate::fork::backup_pages`; the
//! backup itself is upstream's `save_backup::run_global_backup`, unchanged in what it writes (one zip of every port's
//! saves in Saves Backup > Global Backups, named by the date). OK and B on the done page go up to the Saves page of
//! Settings (which hands back to Fix problems when that opened it).

use super::dialogs::close_current_dialog;
use super::page_dialog::{show_page, show_page_handle, Handlers};
use super::state::AppState;
use crate::fork::backup_pages::{done_page, waiting_page, BackupAct};
use crate::fork::port_page::paths::human_path_here;
use crate::fork::rom_screens::job::Background;
use crate::ui::gamepad_router::GamepadRouter;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

type Router = Rc<RefCell<GamepadRouter>>;

thread_local! {
    static TIMER: slint::Timer = slint::Timer::default();
    static RUNNING: Cell<bool> = const { Cell::new(false) };
}

/// Up to the Saves page of Settings (closing the page first: `open_group` gives way to an open dialog).
fn to_saves_page(app: &Rc<AppState>, router: &Router) {
    close_current_dialog(app, router);
    super::settings_menu_screen::open_group(app, router, crate::fork::settings_menu::Group::Saves);
}

fn nothing() -> Rc<dyn Fn(&Rc<AppState>, &Router)> {
    Rc::new(|_, _| {})
}

/// Review W6: which ports this run could not back up, for the checkup's Save backups line
/// (`crate::fork::backup_last_run`). A run stopped before any port (the folder could not be made) has nothing to say.
fn record_last_run(global_dir: &std::path::Path, ports: &[(String, crate::core::save_backup::PortBackup)], stopped: bool) {
    if stopped {
        return;
    }
    let missed = ports.iter().filter(|(_, o)| matches!(o, crate::core::save_backup::PortBackup::Failed(_))).map(|(n, _)| n.clone()).collect();
    let run = crate::fork::backup_last_run::LastRun { date: chrono::Local::now().format("%Y-%m-%d").to_string(), at: crate::fork::backup_last_run::now_secs(), missed };
    if let Err(e) = crate::fork::backup_last_run::write(global_dir, &run) {
        eprintln!("save backup: could not note which ports were missed ({e})");
    }
}

/// Starts the backup of every port's saves, behind its waiting page.
pub(crate) fn start(app: &Rc<AppState>, router: &Router) {
    if RUNNING.with(Cell::get) {
        return;
    }
    let place = human_path_here(&app.paths.saves_backup_dir);
    let handlers = Handlers { on_action: Rc::new(|_, _, _: BackupAct| {}), on_back: nothing(), on_exit: nothing(), on_input: None };
    let handle = show_page_handle(app, router, waiting_page(&place, ""), handlers);
    RUNNING.with(|r| r.set(true));
    let catalog = app.catalog.borrow().clone();
    // The backup reports the catalog's own names ("Space Cadet (SpaceCadetPinball)"); the pages use the shown name.
    let shown: std::collections::HashMap<String, String> = catalog.iter().map(|p| (p.name.clone(), p.display_name().to_string())).collect();
    let shown = Rc::new(shown);
    let shown_name = {
        let shown = shown.clone();
        move |name: &str| shown.get(name).cloned().unwrap_or_else(|| name.to_string())
    };
    let library_dir = app.paths.library_dir();
    let saves_backup_dir = app.paths.saves_backup_dir.clone();
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let global_dir = app.paths.saves_backup_dir.join(crate::fork::save_restore::GLOBAL_BACKUPS_DIR);
    let job = Background::spawn(move |ctl| crate::core::save_backup::run_global_backup(&catalog, &library_dir, &saves_backup_dir, &date, &mut |name: &str| ctl.set_progress(name.to_string())));
    let (app, router) = (app.clone(), router.clone());
    let last = RefCell::new(String::new());
    TIMER.with(|t| {
        t.start(slint::TimerMode::Repeated, Duration::from_millis(150), move || {
            let Some(result) = job.take() else {
                let now = job.progress();
                if now != *last.borrow() {
                    *last.borrow_mut() = now.clone();
                    if let Some(h) = &handle {
                        h.update(waiting_page(&place, &shown_name(&now)));
                    }
                }
                return;
            };
            TIMER.with(slint::Timer::stop);
            RUNNING.with(|r| r.set(false));
            let page = match result {
                Ok(summary) => {
                    eprintln!("save backup: {} save folder(s) copied, {} skipped, {} failed", summary.copied, summary.skipped, summary.failed);
                    let ports: Vec<_> = summary.ports.iter().map(|(name, o)| (shown_name(name), o.clone())).collect();
                    record_last_run(&global_dir, &ports, summary.problem.is_some());
                    done_page(&ports, summary.problem.as_deref(), &place)
                }
                Err(e) => done_page(&[], Some(&format!("the app ran into an error ({e})")), &place),
            };
            super::cloud_screen::trigger(&app, crate::fork::cloud::hooks::Why::SaveBackup); // copy it to the cloud
            // The done page takes the waiting page's window; something else that replaced it stays, and the done page waits
            // for nobody (it is shown over it only when the waiting page is still up).
            if handle.as_ref().is_some_and(super::page_dialog::PageHandle::is_open) || !super::dialogs::dialog_is_open(&app) {
                let handlers = Handlers { on_action: Rc::new(|app, router, _: BackupAct| to_saves_page(app, router)), on_back: Rc::new(to_saves_page), on_exit: Rc::new(close_current_dialog), on_input: None };
                show_page(&app, &router, page, handlers);
            }
        });
    });
}
