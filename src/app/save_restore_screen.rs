//! The "Restore saves" pages: Settings > Saves > Restore saves (which port, then which backup), the Port page's Saves >
//! Restore and the cloud backup's hand-over (which backup), then "Put back the saves from ...?", a waiting page and a
//! done page, all in the guided page window (`page_dialog::show_page`). What the pages say is
//! `crate::fork::save_restore::pages` and the restore itself is `crate::fork::save_restore::restore` (both tested
//! without any UI); this file shows them and runs the restore on a worker thread. The restore is unchanged: the saves
//! now are backed up before anything is replaced. English only, like the fork's other screens.

use super::dialogs::{close_current_dialog, dialog_is_open, open_info_dialog, open_message_dialog};
use super::page_dialog::{show_page, show_page_handle, Handlers, PageHandle};
use super::playtime::is_port_running;
use super::state::AppState;
use crate::core::models::Port;
use crate::core::path_safety::safe_join;
use crate::fork::port_page::paths::{human_path, plain_paths_here, Places};
use crate::fork::rom_screens::job::Background;
use crate::fork::save_restore::pages::{self, PortChoice, RsAct};
use crate::fork::save_restore::{self, Backup, RestoreError, RestoreInput, SaveTarget, Target};
use crate::ui::gamepad_router::GamepadRouter;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

type Router = Rc<RefCell<GamepadRouter>>;
type Back = Rc<dyn Fn(&Rc<AppState>, &Router)>;

const TITLE: &str = "Restore saves";

fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"].iter().find_map(|v| std::env::var_os(v)).map(PathBuf::from).filter(|p| p.is_absolute())
}

fn targets_for(app: &AppState, port: &Port) -> Option<Vec<SaveTarget>> {
    let game_dir = safe_join(&app.paths.library_dir(), &port.folder).ok()?;
    Some(save_restore::save_targets(port, &game_dir, &app.paths.saves_backup_dir, home_dir().as_deref()))
}

/// Handlers for a page here: B runs `back`, the title bar X leaves.
fn handlers(on_action: impl Fn(&Rc<AppState>, &Router, RsAct) + 'static, back: Back) -> Handlers<RsAct> {
    Handlers { on_action: Rc::new(on_action), on_back: back, on_exit: Rc::new(close_current_dialog), on_input: None }
}

/// Something to say on a page with OK, then `then` (OK and B alike).
fn notice(app: &Rc<AppState>, router: &Router, header: &str, title: &str, text: &str, then: Back) {
    let page = pages::notice_page(header, title, &plain_paths_here(text));
    let after = then.clone();
    if !show_page(app, router, page, handlers(move |app, router, _| after(app, router), then.clone())) {
        then(app, router);
    }
}

// -- Entry points -----------------------------------------------------------------------------------

/// The Info dialog's "Restore saves...". Back returns to the Info dialog.
pub(crate) fn open_from_info(app: &Rc<AppState>, router: &Router, port: Port) {
    if super::ports_folder_screen::refuse_when_blocked(app) {
        return;
    }
    let back: Back = {
        let port = port.clone();
        Rc::new(move |app, router| open_info_dialog(app, router, &port))
    };
    show_port(app, router, &port, back);
}

/// Up from the Settings page: the Saves page of Settings.
fn to_saves_page(app: &Rc<AppState>, router: &Router) {
    close_current_dialog(app, router);
    super::settings_menu_screen::open_group(app, router, crate::fork::settings_menu::Group::Saves);
}

/// Settings > Saves > Restore saves: every port that has at least one backup, a card each.
pub(crate) fn open_from_settings(app: &Rc<AppState>, router: &Router) {
    if super::ports_folder_screen::refuse_when_blocked(app) {
        return;
    }
    let all = save_restore::list_all_backups(&app.paths.saves_backup_dir);
    let rows: Vec<(PortChoice, Option<Port>)> = {
        let catalog = app.catalog.borrow();
        all.iter()
            .map(|(folder, backups)| {
                let port = catalog.iter().find(|p| &p.folder == folder && !p.is_android_only()).cloned();
                let name = port.as_ref().map_or_else(|| folder.clone(), |p| p.display_name().to_string());
                let newest = backups.first().map(pages::when_short).unwrap_or_default();
                (PortChoice { name, backups: backups.len(), newest, known: port.is_some() }, port)
            })
            .collect()
    };
    let page = pages::ports_page(&rows.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>());
    let place = crate::fork::port_page::paths::human_path_here(&app.paths.saves_backup_dir);
    let up: Back = Rc::new(to_saves_page);
    let here: Back = Rc::new(open_from_settings);
    show_page(
        app,
        router,
        page,
        handlers(
            move |app, router, act| match act {
                RsAct::Port(i) => match rows.get(i) {
                    Some((_, Some(port))) => show_port(app, router, port, here.clone()),
                    Some((choice, None)) => notice(
                        app,
                        router,
                        &choice.name,
                        "Not in your ports list",
                        &format!("{} is not in your ports list any more, so the app does not know where its saves go and cannot put them back.\n\nIts backups stay in {place}.", choice.name),
                        here.clone(),
                    ),
                    None => {}
                },
                _ => to_saves_page(app, router),
            },
            up,
        ),
    );
}

// -- One port ---------------------------------------------------------------------------------------

/// A port's backups, newest first (and any restore that stopped part way). `back` is where B goes.
pub(crate) fn show_port(app: &Rc<AppState>, router: &Router, port: &Port, back: Back) {
    let name = port.display_name().to_string();
    let places = Places::system();
    let Some(targets) = targets_for(app, port) else {
        return notice(app, router, &name, "Cannot put saves back", "This port's folder name cannot be used, so its saves cannot be put back.", back);
    };
    if !targets.iter().any(|t| matches!(t.target, Target::Folder(_))) {
        return notice(app, router, &name, "Nothing to put back", &pages::nothing_restorable_text(&name, &targets, &places), back);
    }
    let backups = save_restore::list_backups(&app.paths.saves_backup_dir, &port.folder);
    // Folders an interrupted restore left beside a save folder come first; they are never deleted here.
    let interrupted = save_restore::interrupted_restores(&targets);
    let saves: Vec<String> = targets.iter().filter_map(|t| if let Target::Folder(p) = &t.target { Some(human_path(p, &places)) } else { None }).collect();
    let page = pages::backups_page(&name, &backups, &saves, interrupted.len());
    let again: Back = {
        let (port, back) = (port.clone(), back.clone());
        Rc::new(move |app, router| show_port(app, router, &port, back.clone()))
    };
    let port = port.clone();
    let on_back = back.clone();
    show_page(
        app,
        router,
        page,
        handlers(
            move |app, router, act| match act {
                RsAct::Interrupted(i) => {
                    if let Some(item) = interrupted.get(i) {
                        notice(app, router, &name, "A restore stopped part way", &pages::interrupted_text(item, &places), again.clone());
                    }
                }
                RsAct::Backup(i) => {
                    if let Some(backup) = backups.get(i) {
                        choose_backup(app, router, &port, backup.clone(), again.clone());
                    }
                }
                _ => back(app, router),
            },
            on_back,
        ),
    );
}

fn busy_reason(app: &AppState, port: &Port) -> Option<String> {
    // Fork (ports folder): a restore writes into the port folder, never while the ports folder is missing or moving.
    if let Some(why) = super::ports_folder_screen::blocked_reason(app) {
        return Some(why);
    }
    if is_port_running(app, port.key()) {
        return Some(RestoreError::GameRunning("The game (started from this app)".into()).to_string());
    }
    if let (Ok(game_dir), Some(targets)) = (safe_join(&app.paths.library_dir(), &port.folder), targets_for(app, port)) {
        if let Some(what) = save_restore::running_from(&game_dir, &targets) {
            return Some(RestoreError::GameRunning(what).to_string());
        }
    }
    if app.install_runtime.installing.borrow().contains(port.key()) {
        return Some("This port is being installed or updated. Wait for that to finish, then put the saves back; nothing was changed.".into());
    }
    if RESTORE_RUNNING.with(Cell::get) {
        return Some("Another restore is still running. Wait for it to finish, then try again.".into());
    }
    None
}

/// A backup was chosen: "Put back the saves from ...?" (or why it cannot be put back). `again` is the backups page.
fn choose_backup(app: &Rc<AppState>, router: &Router, port: &Port, backup: Backup, again: Back) {
    let name = port.display_name().to_string();
    if let Some(problem) = &backup.problem {
        return notice(app, router, &name, "This backup cannot be used", &RestoreError::UnusableBackup(problem.clone()).to_string(), again);
    }
    if let Some(reason) = busy_reason(app, port) {
        return notice(app, router, &name, "Not now", &reason, again);
    }
    let Some(targets) = targets_for(app, port) else { return again(app, router) };
    let places = Places::system();
    let plan = save_restore::plan_restore(&targets, &backup);
    if plan.replace.is_empty() {
        let mut text = "This backup holds no saves the app can put back here.".to_string();
        for (field, why) in &plan.skipped {
            text.push_str(&format!("\n\n{}: {why}.", field.label()));
        }
        return notice(app, router, &name, "Nothing to put back", &text, again);
    }
    let page = pages::ask_page(&pages::ask_facts(&name, &backup, &plan, &places));
    let port = port.clone();
    let back = again.clone();
    show_page(
        app,
        router,
        page,
        handlers(
            move |app, router, act| match act {
                RsAct::Restore => start_restore(app, router, port.clone(), backup.clone(), again.clone()),
                _ => again(app, router),
            },
            back,
        ),
    );
}

// -- Running the restore ----------------------------------------------------------------------------

thread_local! {
    static RESTORE_TIMER: slint::Timer = slint::Timer::default();
    static RESTORE_RUNNING: Cell<bool> = const { Cell::new(false) };
    /// Close or Quit was asked for while a restore ran: the app quits once it has finished.
    static QUIT_PENDING: Cell<bool> = const { Cell::new(false) };
    /// The waiting page of the restore running now, and a line to add to it (a refusal while it runs).
    static WORKING: RefCell<Option<Rc<PageHandle<RsAct>>>> = const { RefCell::new(None) };
    static NOTE: RefCell<Option<String>> = const { RefCell::new(None) };
    static WORKING_NAME: RefCell<String> = const { RefCell::new(String::new()) };
    static WORKING_STEP: RefCell<String> = const { RefCell::new(String::new()) };
}

const QUIT_NOTE: &str = "The app closes when the restore is done. Do not turn the Deck off.";

/// Redraws the waiting page with the step and the note.
fn redraw_working() {
    let Some(handle) = WORKING.with(|w| w.borrow().clone()) else { return };
    let name = WORKING_NAME.with(|n| n.borrow().clone());
    let mut text = WORKING_STEP.with(|s| s.borrow().clone());
    if let Some(note) = NOTE.with(|n| n.borrow().clone()) {
        text = format!("{text}\n{note}");
    }
    handle.update(pages::working_page(&name, &text));
}

/// Says `text` on the restore's waiting page when it is up, else in a message.
fn tell(app: &Rc<AppState>, router: &Router, text: &str) {
    if WORKING.with(|w| w.borrow().as_ref().is_some_and(|h| h.is_open())) {
        NOTE.with(|n| *n.borrow_mut() = Some(text.to_string()));
        return redraw_working();
    }
    open_message_dialog(app, router, TITLE, text);
}

/// Window Close (when it would quit), tray Quit and self-update ask this first. While a restore runs,
/// quitting would kill it between two folder swaps, so the app waits: it quits once the restore ends.
pub(crate) fn quit_waits_for_restore(app: &Rc<AppState>, router: &Router) -> bool {
    if !RESTORE_RUNNING.with(Cell::get) {
        return false;
    }
    QUIT_PENDING.with(|q| q.set(true));
    tell(app, router, &format!("Finishing the restore first. {QUIT_NOTE}"));
    true
}

/// Fork (checkup): a restore is running (the checkup does not look meanwhile).
pub(crate) fn restore_running() -> bool {
    RESTORE_RUNNING.with(Cell::get)
}

/// Settings entries that read or write saves (Back up saves, Restore saves, Force update) ask this first.
pub(crate) fn refuse_while_restoring(app: &Rc<AppState>, router: &Router) -> bool {
    if !RESTORE_RUNNING.with(Cell::get) {
        return false;
    }
    tell(app, router, "A restore is running. Wait for it to finish, then try again.");
    true
}

/// What the user is told when the restore thread panicked: unlike other jobs, it may have stopped part way.
fn panic_text(app: &AppState, port: &Port, detail: &str) -> String {
    let detail = detail.strip_suffix("; nothing else was changed").unwrap_or(detail);
    let kept = crate::fork::port_page::paths::human_path_here(&app.paths.saves_backup_dir.join(save_restore::RESTORE_BACKUPS_DIR).join(&port.folder));
    format!(
        "The restore stopped because of an error inside the app ({detail}).\n\nCheck your saves. Your saves from before the restore may be in a backup in {kept}.\n\nIf your saves look wrong, open Restore saves for this port again: it shows what was kept."
    )
}

fn start_restore(app: &Rc<AppState>, router: &Router, port: Port, backup: Backup, again: Back) {
    let name = port.display_name().to_string();
    // Checked again here: the game may have been started while the question was up.
    if let Some(reason) = busy_reason(app, &port) {
        return notice(app, router, &name, "Not now", &reason, again);
    }
    let Ok(game_dir) = safe_join(&app.paths.library_dir(), &port.folder) else { return again(app, router) };
    let key = port.key().to_string();
    // Holding the port's install slot keeps an install, update or launch of it from starting meanwhile.
    app.install_runtime.installing.borrow_mut().insert(key.clone());
    RESTORE_RUNNING.with(|r| r.set(true));
    NOTE.with(|n| *n.borrow_mut() = None);
    WORKING_NAME.with(|n| *n.borrow_mut() = name.clone());
    WORKING_STEP.with(|s| *s.borrow_mut() = String::new());
    // Nothing on the waiting page can be chosen, and B does nothing: the restore cannot be stopped part way.
    let nothing: Back = Rc::new(|_, _| {});
    let waiting = Handlers { on_action: Rc::new(|_, _, _: RsAct| {}), on_back: nothing.clone(), on_exit: nothing, on_input: None };
    let handle = show_page_handle(app, router, pages::working_page(&name, ""), waiting).map(Rc::new);
    WORKING.with(|w| *w.borrow_mut() = handle.clone());

    let saves_backup_dir = app.paths.saves_backup_dir.clone();
    let home = home_dir();
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let worker_port = port.clone();
    let worker_backup = backup.clone();
    let job = Background::spawn(move |ctl| {
        let input = RestoreInput {
            port: &worker_port,
            game_dir: &game_dir,
            saves_backup_dir: &saves_backup_dir,
            home: home.as_deref(),
            backup: &worker_backup,
            game_running: false,
            stamp: &stamp,
        };
        save_restore::restore(&input, &mut |step| ctl.set_progress(step.to_string()))
    });

    let (app, router) = (app.clone(), router.clone());
    let when = pages::when_short(&backup);
    RESTORE_TIMER.with(|timer| {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
            if let Some(result) = job.take() {
                RESTORE_TIMER.with(slint::Timer::stop);
                RESTORE_RUNNING.with(|r| r.set(false));
                WORKING.with(|w| *w.borrow_mut() = None);
                app.install_runtime.installing.borrow_mut().remove(&key);
                super::cloud_screen::trigger(&app, crate::fork::cloud::hooks::Why::RestoreBackup); // Fork: the safety backup made before the restore
                // A restore that could not be fully undone must be seen, even after Close or Quit.
                let must_tell = matches!(&result, Ok(Err(RestoreError::RollbackFailed(_))) | Err(_));
                if QUIT_PENDING.with(|q| q.replace(false)) && !must_tell {
                    let _ = slint::quit_event_loop();
                    return;
                }
                let ours = handle.as_ref().is_some_and(|h| h.is_open());
                if !ours && !must_tell && dialog_is_open(&app) {
                    // Something else is on screen now; the next visit to Restore saves shows what was put back.
                    eprintln!("restore saves: {} finished while another dialog was open", port.display_name());
                    return;
                }
                let places = Places::system();
                let page = match result {
                    Ok(Ok(done)) => pages::done_page(&pages::done_facts(port.display_name(), &when, &done, &places)),
                    Ok(Err(e)) => {
                        eprintln!("restore saves: {}: {e}", port.display_name());
                        pages::failed_page(port.display_name(), &e, &places)
                    }
                    Err(panic) => {
                        eprintln!("restore saves: {}: the worker stopped: {panic}", port.display_name());
                        let mut p = pages::notice_page(port.display_name(), "The restore stopped", &plain_paths_here(&panic_text(&app, &port, &panic)));
                        p.window_title = "Restore saves: not done".to_string();
                        p
                    }
                };
                let after = again.clone();
                show_page(&app, &router, page, handlers(move |app, router, _| after(app, router), again.clone()));
                return;
            }
            let step = job.progress();
            if step.is_empty() || WORKING_STEP.with(|s| *s.borrow() == step) {
                return;
            }
            WORKING_STEP.with(|s| *s.borrow_mut() = step);
            if QUIT_PENDING.with(Cell::get) {
                NOTE.with(|n| *n.borrow_mut() = Some(QUIT_NOTE.to_string()));
            }
            redraw_working();
        });
    });
}
