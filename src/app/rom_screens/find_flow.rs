//! "Find this port's game files": the screens around `fork::rom_screens::find`.
//!
//! Entry points (all `pub(crate)`, re-exported from `app::rom_screens`):
//! - [`after_install`]: called by the app when an install finishes. Once per port, only for a port
//!   that names files it cannot run without and is not connected yet, it looks in the game files folder
//!   on a worker thread (with progress and Cancel) and shows what it found.
//! - [`search_ports_now`]: "Find their game files" on the import's card page. One walk of the folder for
//!   several ports, then a list with each port's result and Pick another folder...
//! - [`find_for_port`]: the same search for one port from anywhere (the setup wizard, the port's Info
//!   dialog, Settings > Game files). `back` runs when the user leaves (Later, Cancel, Back).
//! - [`show_ports_needing_files`]: Settings > Game files > Find game files for a port.
//!
//! What the user sees: the Connect step when the file is there (a pick list first when several files
//! match), or a prompt that names the port, the files it needs (with CRC when the catalog gives one) and
//! where it looked, with Search again, Connect by hand... and Later. Nothing is copied without the
//! user's yes (the Connect step asks), and game files are only read.

use super::{
    choose_folder, load_store, message, open_connect, open_list, run_job, show_connect, store_path, Back, JobUi, Router, Then, JOB_RUNNING, TITLE,
};
use crate::app::dialogs::{build_picker_dialog, close_current_dialog, dialog_is_open, finish_dialog_open, open_confirm_dialog, open_picker_dialog, DialogSlot};
use crate::app::folder_picker_screen::open_folder_picker;
use crate::app::state::AppState;
use crate::core::models::Port;
use crate::core::path_safety::safe_join;
use crate::fork::rom_library::{load_local_entries, Mapping, RomLibraryStore};
use crate::fork::rom_screens::connect_screen::{ConnectScreen, Pick};
use crate::fork::rom_screens::file_pick::{FileBrowser, Line as FileLine};
use crate::fork::rom_screens::find::{
    find_game_files_for_port, find_game_files_for_ports, match_hand_picked, looks_empty_lines, no_folder_lines, no_match_lines, not_found_lines, unreadable_lines,
    use_anyway,
    FindOutcome, FindRequest, HandPick, LATER_HINT,
};
use crate::fork::rom_screens::offer::{self, AutoSearch};
use crate::fork::rom_screens::report::PortRow;
use crate::fork::rom_screens::util::ellipsize;
use crate::fork::settings::{ForkSettings, FILE_NAME};
use slint::ComponentHandle;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::AtomicBool;

const SEARCH_TITLE: &str = "Looking for game files";
const ROW_WIDTH: usize = 110;

fn close_back() -> Back {
    Rc::new(|app, router| close_current_dialog(app, router))
}

fn title_for(name: &str) -> String {
    format!("Game files for {}", ellipsize(name, 50))
}

type LocalEntries = HashMap<String, Value>;

/// The raw `ports.local.json` entries (explicit `rom_files` live there), read once per screen.
fn local_entries(app: &AppState) -> LocalEntries {
    load_local_entries(&app.paths.config_dir.join("ports.local.json"))
}

fn request_for(app: &AppState, local: &LocalEntries, port: &Port, mappings: Vec<Mapping>) -> FindRequest {
    FindRequest::new(port.clone(), app.is_installed(port), local.get(&port.folder), mappings)
}

/// Installed, needs game files, and does not have them yet: not in its own folder (nor the archive it makes from
/// them), not connected. What the setup wizard asks about.
pub(crate) fn port_missing_game_files(app: &AppState, port: &Port) -> bool {
    if !port_needs_game_files(app, port) {
        return false;
    }
    let Ok(dir) = crate::core::path_safety::safe_join(&app.paths.library_dir(), &port.folder) else { return true };
    !crate::fork::setup_wizard::game_files::in_folder_keys(std::iter::once((port, dir)), &app.paths.config_dir).contains(port.key())
}

/// For the Info dialog: is the port installed and does it name files it needs.
pub(crate) fn port_needs_game_files(app: &AppState, port: &Port) -> bool {
    !port.is_android_only() && app.is_installed(port) && request_for(app, &local_entries(app), port, Vec::new()).needs_files()
}

/// The automatic search spends a port's one turn only once a result is really on screen: a search
/// cancelled or abandoned (B) while it runs leaves the port to be searched after its next install.
fn mark_searched(app: &AppState, keys: &[String]) {
    let mut settings = ForkSettings::load(&app.paths.config_dir.join(FILE_NAME));
    for key in keys {
        if let Err(e) = settings.mark_rom_prompted(key) {
            eprintln!("game files: could not remember that {key} was searched ({e})");
        }
    }
}

/// The ports folder is missing or being moved, or its startup message is still due: the automatic
/// search waits (the port is not marked, so it is searched after its next install) rather than open over it.
fn ports_folder_busy(app: &AppState) -> bool {
    crate::app::ports_folder_screen::startup_dialog_pending() || crate::app::ports_folder_screen::blocked_reason(app).is_some()
}

/// A list whose cursor starts on `start` (the first action row) instead of the first text row.
fn open_list_at(app: &Rc<AppState>, router: &Router, title: &str, labels: Vec<String>, start: usize, on_select: impl Fn(&Rc<AppState>, &Router, usize) + 'static) {
    let Some(dialog) = build_picker_dialog(app, router, title, &labels) else { return };
    let start = start.min(labels.len().saturating_sub(1)) as i32;
    dialog.set_selected_index(start);
    app.dialog_nav.picker_index.set(start);
    {
        let (app, router) = (app.clone(), router.clone());
        dialog.on_item_selected(move |index| on_select(&app, &router, index as usize));
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
}

// -- Entry points ---------------------------------------------------------------------------------

/// Right after a port finished installing. Once per port; quiet when there is nothing to do or a
/// dialog or another game files task is already up (the port stays reachable from Info and Settings).
pub(crate) fn after_install(app: &Rc<AppState>, router: &Router, key: &str) {
    if dialog_is_open(app) || JOB_RUNNING.with(Cell::get) || ports_folder_busy(app) {
        return;
    }
    let Some(port) = app.find_port(key) else { return };
    let Ok(store) = RomLibraryStore::load(&store_path(app)) else { return };
    let req = request_for(app, &local_entries(app), &port, store.mappings().to_vec());
    let settings = ForkSettings::load(&app.paths.config_dir.join(FILE_NAME));
    match offer::after_install(&req, port.is_android_only(), settings.rom_prompted(key)) {
        AutoSearch::Search => {}
        AutoSearch::Skip(_) => return,
    }
    search(app, router, req, store, close_back(), Some(key.to_string()));
}

/// Look for one port's game files and show the result. `back` runs when the user leaves.
pub(crate) fn find_for_port(app: &Rc<AppState>, router: &Router, key: &str, back: Back) {
    let Some(port) = app.find_port(key) else {
        return message(app, router, TITLE, "That port is no longer in the list.");
    };
    let Some(store) = load_store(app, router) else { return };
    let req = request_for(app, &local_entries(app), &port, store.mappings().to_vec());
    if !req.needs_files() {
        return message(app, router, &title_for(port.display_name()), &format!("{} does not name any game files it needs, so there is nothing to look for.", port.display_name()));
    }
    search(app, router, req, store, back, None);
}

/// The Info dialog's "Game files..." button.
pub(crate) fn open_from_info(app: &Rc<AppState>, router: &Router, port: Port) {
    find_for_port(app, router, port.key(), close_back());
}

/// "Find their game files" (the import's card page): every port among `keys` that needs a game file, whether or not
/// it was searched before. False only when it cannot start (another task runs).
pub(crate) fn search_ports_now(app: &Rc<AppState>, router: &Router, keys: &[String], then: Option<Back>) -> bool {
    search_ports(app, router, keys, "", then, true)
}

/// Searches several ports with one walk of the folder and lists the outcomes. `intro` lines go at the top of the list.
/// `then` runs when the user is done with what it shows: the Done row, B on the list, Continue on its notices, or B
/// on the search's progress list (not when it returns false: nothing was shown). `force`: search every port that
/// needs files; otherwise only the ones not searched after their install yet.
fn search_ports(app: &Rc<AppState>, router: &Router, keys: &[String], intro: &str, then: Option<Back>, force: bool) -> bool {
    if JOB_RUNNING.with(Cell::get) || ports_folder_busy(app) {
        return false;
    }
    let Ok(store) = RomLibraryStore::load(&store_path(app)) else { return false };
    let settings = ForkSettings::load(&app.paths.config_dir.join(FILE_NAME));
    let local = local_entries(app);
    let reqs: Vec<FindRequest> = keys
        .iter()
        .filter_map(|k| app.find_port(k))
        .map(|p| request_for(app, &local, &p, store.mappings().to_vec()))
        .filter(|r| if force { !r.port.is_android_only() && r.needs_files() } else { offer::after_install(r, r.port.is_android_only(), settings.rom_prompted(r.port.key())) == AutoSearch::Search })
        .collect();
    if reqs.is_empty() {
        return false;
    }
    let searched: Vec<String> = reqs.iter().map(|r| r.port.key().to_string()).collect();
    let intro: Rc<Vec<String>> = Rc::new(intro.lines().filter(|l| !l.trim().is_empty()).map(|l| ellipsize(l, ROW_WIDTH)).collect());
    if store.rom_dir().is_none() {
        let outcomes = find_game_files_for_ports(&store, &reqs, &AtomicBool::new(false), &mut |_| {}).unwrap_or_default();
        mark_searched(app, &searched);
        show_batch(app, router, intro, Rc::new(reqs.into_iter().zip(outcomes).collect()), then);
        return true;
    }
    let worker_reqs = reqs.clone();
    if let Some(then) = &then {
        super::ABANDON_HOOK.with(|h| *h.borrow_mut() = Some(then.clone()));
    }
    run_job(
        app,
        router,
        JobUi::Cancellable { title: SEARCH_TITLE },
        move |ctl| find_game_files_for_ports(&store, &worker_reqs, ctl.flag(), &mut |t| ctl.set_progress(t)),
        move |app, router, result| match result {
            Ok(Some(outcomes)) => {
                mark_searched(app, &searched);
                show_batch(app, router, intro, Rc::new(reqs.into_iter().zip(outcomes).collect()), then);
            }
            Ok(None) => notice_then(app, router, &format!("{}\n\nThe search for game files was cancelled. {LATER_HINT}", intro.join("\n")), &then),
            Err(e) => notice_then(app, router, &format!("{}\n\nCould not look for game files: {e}", intro.join("\n")), &then),
        },
    );
    true
}

/// Settings > Game files > Find game files for a port: every installed port that names files.
pub(crate) fn show_ports_needing_files(app: &Rc<AppState>, router: &Router) {
    let Some(store) = load_store(app, router) else { return };
    let catalog = app.catalog.borrow().clone();
    let local = local_entries(app);
    let mut rows: Vec<(bool, String, String)> = catalog
        .iter()
        .filter(|p| !p.is_android_only() && app.is_installed(p))
        .map(|p| request_for(app, &local, p, store.mappings().to_vec()))
        .filter(FindRequest::needs_files)
        .map(|r| {
            let connected = r.already_connected();
            let name = r.port.display_name().to_string();
            let label = format!("{} - {}", ellipsize(&name, 60), if connected { "connected" } else { "not connected yet" });
            (connected, label, r.port.key().to_string())
        })
        .collect();
    rows.sort_by(|a, b| (a.0, a.1.to_lowercase()).cmp(&(b.0, b.1.to_lowercase())));
    let mut labels: Vec<String> = rows.iter().map(|r| r.1.clone()).collect();
    if rows.is_empty() {
        labels.push("No installed port names game files it needs.".into());
    }
    let back_index = labels.len();
    labels.push("Back".into());
    open_list(app, router, "Find game files for a port", labels, move |app, router, index| {
        if let Some((_, _, key)) = rows.get(index) {
            let back: Back = Rc::new(|app, router| show_ports_needing_files(app, router));
            find_for_port(app, router, key, back);
        } else if index == back_index {
            super::open_game_files(app, router);
        }
    });
}

// -- The search -----------------------------------------------------------------------------------

/// `mark`: the port whose automatic search this is; it is marked as searched when the result shows.
fn search(app: &Rc<AppState>, router: &Router, req: FindRequest, store: RomLibraryStore, back: Back, mark: Option<String>) {
    if store.rom_dir().is_none() {
        // Nothing to walk: answer right away.
        if let Some(outcome) = find_game_files_for_port(&store, &req, &AtomicBool::new(false), &mut |_| {}) {
            show_outcome(app, router, req, outcome, back, mark);
        }
        return;
    }
    let worker_req = req.clone();
    // B on the progress list goes where Cancel does: `back`. (The automatic search after an install passes a
    // `mark` and a plain close as `back`; B just closes there, so it has no hook.)
    if mark.is_none() {
        super::ABANDON_HOOK.with(|h| *h.borrow_mut() = Some(back.clone()));
    }
    run_job(
        app,
        router,
        JobUi::Cancellable { title: SEARCH_TITLE },
        move |ctl| find_game_files_for_port(&store, &worker_req, ctl.flag(), &mut |t| ctl.set_progress(t)),
        move |app, router, result| match result {
            Ok(Some(outcome)) => show_outcome(app, router, req, outcome, back, mark),
            Ok(None) => back(app, router),
            Err(e) => message(app, router, TITLE, &e),
        },
    );
}

/// Search again for the same port, with the connections as they are now.
fn search_key(app: &Rc<AppState>, router: &Router, key: &str, back: Back) {
    let Some(port) = app.find_port(key) else { return back(app, router) };
    let Some(store) = load_store(app, router) else { return };
    let req = request_for(app, &local_entries(app), &port, store.mappings().to_vec());
    search(app, router, req, store, back, None);
}

/// Where "Choose ROM folder..." returns to: after a folder is chosen (search again there) and on Back.
pub(super) fn resume(app: &Rc<AppState>, router: &Router, key: &str, back: &Back) {
    search_key(app, router, key, back.clone());
}

fn show_outcome(app: &Rc<AppState>, router: &Router, req: FindRequest, outcome: FindOutcome, back: Back, mark: Option<String>) {
    let name = req.port.display_name().to_string();
    if let Some(key) = mark {
        mark_searched(app, &[key]);
    }
    match outcome {
        FindOutcome::NoFilesNeeded => back(app, router),
        FindOutcome::Found { row, .. } => open_row(app, router, &row, back),
        FindOutcome::Ambiguous { row, slots, .. } => pick_then_connect(app, router, row, slots, back),
        FindOutcome::NotFound { needs, looked_in, notes } => {
            let lines = not_found_lines(&name, &needs, &looked_in, &notes);
            prompt(app, router, Rc::new(Prompt { req, lines, acts: vec![Act::SearchAgain, Act::ByHand, Act::Later], back }));
        }
        FindOutcome::NoRomFolder { needs } => {
            let lines = no_folder_lines(&name, &needs);
            prompt(app, router, Rc::new(Prompt { req, lines, acts: vec![Act::ChooseFolder("Choose ROM folder..."), Act::ByHand, Act::Later], back }));
        }
        FindOutcome::FolderUnreadable { needs, folder, error } => {
            let lines = unreadable_lines(&name, &needs, &folder, &error);
            let acts = vec![Act::SearchAgain, Act::ChooseFolder("Choose another folder..."), Act::ByHand, Act::Later];
            prompt(app, router, Rc::new(Prompt { req, lines, acts, back }));
        }
        FindOutcome::FolderLooksEmpty { needs, folder, saved } => {
            let lines = looks_empty_lines(&name, &needs, &folder, saved);
            let acts = vec![Act::SearchAgain, Act::ChooseFolder("Choose another folder..."), Act::ByHand, Act::Later];
            prompt(app, router, Rc::new(Prompt { req, lines, acts, back }));
        }
    }
}

// -- The prompt (not found, no folder, unreadable) -------------------------------------------------

#[derive(Clone, Copy)]
enum Act {
    SearchAgain,
    ChooseFolder(&'static str),
    ByHand,
    Later,
}

impl Act {
    fn label(self) -> &'static str {
        match self {
            Act::SearchAgain => "Search again",
            Act::ChooseFolder(label) => label,
            Act::ByHand => "Connect by hand...",
            Act::Later => "Later",
        }
    }
}

struct Prompt {
    req: FindRequest,
    lines: Vec<String>,
    acts: Vec<Act>,
    back: Back,
}

fn prompt(app: &Rc<AppState>, router: &Router, p: Rc<Prompt>) {
    let mut labels: Vec<String> = p.lines.iter().map(|l| ellipsize(l, ROW_WIDTH)).collect();
    labels.push(LATER_HINT.to_string());
    let first = labels.len();
    labels.extend(p.acts.iter().map(|a| a.label().to_string()));
    let title = title_for(p.req.port.display_name());
    open_list_at(app, router, &title, labels, first, move |app, router, index| {
        let Some(act) = index.checked_sub(first).and_then(|i| p.acts.get(i)).copied() else { return };
        let key = p.req.port.key().to_string();
        match act {
            Act::SearchAgain => search_key(app, router, &key, p.back.clone()),
            Act::ChooseFolder(_) => choose_folder(app, router, Then::Find { key, back: p.back.clone() }),
            Act::ByHand => {
                let again: Back = {
                    let p = p.clone();
                    Rc::new(move |app, router| prompt(app, router, p.clone()))
                };
                connect_by_hand(app, router, p.req.clone(), p.back.clone(), again);
            }
            Act::Later => (p.back)(app, router),
        }
    });
}

// -- Several ports at once --------------------------------------------------------------------------

/// A message; with a `then` it is a Continue dialog (also on B) that runs it.
fn notice_then(app: &Rc<AppState>, router: &Router, text: &str, then: &Option<Back>) {
    let Some(then) = then else { return message(app, router, TITLE, text) };
    let on_confirm = then.clone();
    open_confirm_dialog(app, router, TITLE.into(), format!("{text}\n").into(), "Continue".into(), move |app, router| on_confirm(app, router));
    if let DialogSlot::Confirm(d) = &*app.dialog_nav.dialogs.borrow() {
        let (app, router, then) = (app.clone(), router.clone(), then.clone());
        d.on_close_requested(move || {
            close_current_dialog(&app, &router);
            then(&app, &router);
        });
    }
}

fn show_batch(app: &Rc<AppState>, router: &Router, intro: Rc<Vec<String>>, items: Rc<Vec<(FindRequest, FindOutcome)>>, then: Option<Back>) {
    let mut labels: Vec<String> = intro.as_ref().clone();
    let first = labels.len();
    let shown: Vec<usize> = (0..items.len()).filter(|i| !matches!(items[*i].1, FindOutcome::NoFilesNeeded)).collect();
    labels.extend(shown.iter().map(|i| format!("Game files for {}: {}", ellipsize(items[*i].0.port.display_name(), 50), items[*i].1.short())));
    // Another folder, then the same ports searched again.
    let pick = labels.len();
    let has_folder = RomLibraryStore::load(&store_path(app)).ok().is_some_and(|s| s.rom_dir().is_some());
    labels.push(if has_folder { "Pick another folder..." } else { "Pick a folder..." }.into());
    let done = labels.len();
    labels.push("Done".into());
    let keys: Vec<String> = items.iter().map(|(r, _)| r.port.key().to_string()).collect();
    let on_done = then.clone();
    let then_for_close = then.clone();
    open_list_at(app, router, TITLE, labels, first, move |app, router, index| {
        if index == pick {
            let (keys, then, again) = (keys.clone(), then.clone(), {
                let (intro, items, then) = (intro.clone(), items.clone(), then.clone());
                move |app: &Rc<AppState>, router: &Router| show_batch(app, router, intro.clone(), items.clone(), then.clone())
            });
            return open_folder_picker(
                app,
                router,
                super::game_files_picker(),
                move |app, router, path| {
                    let Some(mut store) = load_store(app, router) else { return };
                    match store.set_rom_dir(Some(path)) {
                        Ok(()) => {
                            if !search_ports_now(app, router, &keys, then.clone()) {
                                message(app, router, TITLE, "Another game files task is still running. Try again in a moment.");
                            }
                        }
                        Err(e) => message(app, router, TITLE, &format!("Could not use that folder: {e}")),
                    }
                },
                move |app, router| again(app, router),
            );
        }
        if index == done {
            close_current_dialog(app, router);
            if let Some(then) = &on_done {
                then(app, router);
            }
            return;
        }
        let Some(&i) = index.checked_sub(first).and_then(|n| shown.get(n)) else { return };
        let back: Back = {
            let (intro, items, then) = (intro.clone(), items.clone(), then.clone());
            Rc::new(move |app, router| show_batch(app, router, intro.clone(), items.clone(), then.clone()))
        };
        let (req, outcome) = items[i].clone();
        show_outcome(app, router, req, outcome, back, None);
    });
    // B on the list is Done too.
    if let (Some(then), DialogSlot::Picker(d)) = (then_for_close, &*app.dialog_nav.dialogs.borrow()) {
        let (app, router) = (app.clone(), router.clone());
        d.on_close_requested(move || {
            close_current_dialog(&app, &router);
            then(&app, &router);
        });
    }
}

// -- Connect ----------------------------------------------------------------------------------------

fn open_row(app: &Rc<AppState>, router: &Router, row: &PortRow, back: Back) {
    if !row.installed {
        let name = row.port.display_name();
        return message(app, router, &title_for(name), &format!("Install {name} first, then connect its game files."));
    }
    open_connect(app, router, row, back);
}

fn pick_then_connect(app: &Rc<AppState>, router: &Router, row: PortRow, slots: Vec<usize>, back: Back) {
    if !row.installed {
        return open_row(app, router, &row, back);
    }
    let Some(store) = load_store(app, router) else { return };
    let Ok(port_dir) = safe_join(&app.paths.library_dir(), &row.port.folder) else { return };
    let screen = Rc::new(RefCell::new(ConnectScreen::new(&row, port_dir, store.mappings())));
    let slots: Rc<Vec<(usize, String)>> = Rc::new(slots.into_iter().map(|i| (i, row.matched.slots[i].label.clone())).collect());
    pick_next(app, router, screen, slots, 0, back);
}

/// One pick list per slot with several matching files, then the Connect step with the choices made.
fn pick_next(app: &Rc<AppState>, router: &Router, screen: Rc<RefCell<ConnectScreen>>, slots: Rc<Vec<(usize, String)>>, n: usize, back: Back) {
    let Some((slot, label)) = slots.get(n).cloned() else { return show_connect(app, router, screen, back) };
    if !screen.borrow().is_editable(slot) {
        return pick_next(app, router, screen, slots, n + 1, back);
    }
    let (picks, pick_labels) = (screen.borrow().picks(slot), screen.borrow().pick_labels(slot));
    let files: Vec<(Pick, String)> =
        picks.into_iter().zip(pick_labels).filter(|(p, _)| matches!(p, Pick::File(_))).map(|(p, l)| (p, l.trim_start_matches("File: ").to_string())).collect();
    let mut labels: Vec<String> = files.iter().map(|f| f.1.clone()).collect();
    labels.push("Decide in the Connect step".into());
    let title = format!("Pick one: {}", ellipsize(&label, 30));
    open_picker_dialog(app, router, &title, labels, move |app, router, choice| {
        if let Some((pick, _)) = files.get(choice) {
            screen.borrow_mut().choose(slot, *pick);
        }
        pick_next(app, router, screen.clone(), slots.clone(), n + 1, back.clone());
    });
}

// -- Connect by hand --------------------------------------------------------------------------------

fn connect_by_hand(app: &Rc<AppState>, router: &Router, req: FindRequest, back: Back, cancel: Back) {
    let start = RomLibraryStore::load(&store_path(app)).ok().and_then(|s| s.rom_dir().map(Path::to_path_buf)).filter(|d| d.is_dir());
    let start = start.or_else(super::home_dir).unwrap_or_else(|| PathBuf::from("/"));
    show_file_browser(app, router, Rc::new(RefCell::new(FileBrowser::open(&start))), Rc::new(req), back, cancel);
}

fn show_file_browser(app: &Rc<AppState>, router: &Router, browser: Rc<RefCell<FileBrowser>>, req: Rc<FindRequest>, back: Back, cancel: Back) {
    let labels = browser.borrow().labels();
    let title = format!("Pick a file: {}", ellipsize(&browser.borrow().current.to_string_lossy(), 30));
    let Some(dialog) = build_picker_dialog(app, router, &title, &labels) else { return };
    {
        let (app, router) = (app.clone(), router.clone());
        dialog.on_item_selected(move |index| {
            let line = browser.borrow().lines().get(index as usize).copied();
            match line {
                Some(FileLine::Up) => browser.borrow_mut().up(),
                Some(FileLine::Folder(i)) => browser.borrow_mut().enter(i),
                Some(FileLine::File(i)) => {
                    let Some(path) = browser.borrow().file(i).map(Path::to_path_buf) else { return };
                    return hand_picked(&app, &router, browser.clone(), req.clone(), path, back.clone(), cancel.clone());
                }
                Some(FileLine::Cancel) => return cancel(&app, &router),
                Some(FileLine::Where | FileLine::More) | None => return,
            }
            show_file_browser(&app, &router, browser.clone(), req.clone(), back.clone(), cancel.clone());
        });
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
}

fn hand_picked(app: &Rc<AppState>, router: &Router, browser: Rc<RefCell<FileBrowser>>, req: Rc<FindRequest>, path: PathBuf, back: Back, cancel: Back) {
    let worker_req = (*req).clone();
    // B on the progress list goes where Cancel does: back to the same folder to pick again.
    {
        let (browser, req, back, cancel) = (browser.clone(), req.clone(), back.clone(), cancel.clone());
        let again: Back = Rc::new(move |app, router| show_file_browser(app, router, browser.clone(), req.clone(), back.clone(), cancel.clone()));
        super::ABANDON_HOOK.with(|h| *h.borrow_mut() = Some(again));
    }
    run_job(
        app,
        router,
        JobUi::Cancellable { title: "Checking the file" },
        move |ctl| match_hand_picked(&worker_req, &path, ctl.flag(), &mut |t| ctl.set_progress(t)),
        move |app, router, result| match result.and_then(|r| r) {
            Err(e) => message(app, router, TITLE, &e),
            // Cancelled: back to the same folder to pick again.
            Ok(None) => show_file_browser(app, router, browser.clone(), req.clone(), back.clone(), cancel.clone()),
            Ok(Some(HandPick::Matches { row })) => open_row(app, router, &row, back),
            Ok(Some(HandPick::NoMatch { row, file, slots, wrong_version })) => {
                let name = req.port.display_name().to_string();
                let mut labels: Vec<String> = no_match_lines(&name, &row, &file, wrong_version).iter().map(|l| ellipsize(l, ROW_WIDTH)).collect();
                let first = labels.len();
                labels.extend(slots.iter().map(|i| {
                    if slots.len() == 1 {
                        "Use it anyway".to_string()
                    } else {
                        format!("Use it anyway for: {}", ellipsize(&row.matched.slots[*i].label, 60))
                    }
                }));
                let other = labels.len();
                labels.push("Choose another file".into());
                labels.push("Cancel".into());
                open_list_at(app, router, &title_for(&name), labels, first, move |app, router, index| {
                    if let Some(&slot) = index.checked_sub(first).and_then(|n| slots.get(n)) {
                        open_row(app, router, &use_anyway(&row, slot, &file), back.clone());
                    } else if index == other {
                        show_file_browser(app, router, browser.clone(), req.clone(), back.clone(), cancel.clone());
                    } else if index == other + 1 {
                        cancel(app, router);
                    }
                });
            }
        },
    );
}
