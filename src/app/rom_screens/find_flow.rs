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

use super::{choose_folder, load_store, message, open_list, run_job, store_path, Back, JobUi, Router, Then, JOB_RUNNING, TITLE};
use crate::app::dialogs::{build_picker_dialog, close_current_dialog, dialog_is_open, finish_dialog_open, open_confirm_dialog, DialogSlot};
use crate::app::page_dialog::{show_page, Handlers};
use crate::fork::page::Page;
use crate::fork::port_page::paths::{human_path, Places};
use crate::fork::rom_library::{Approval, SlotStatus};
use crate::fork::rom_screens::pages::{self, GfAct};
use crate::fork::rom_screens::util::file_name;
use crate::app::folder_picker_screen::open_folder_picker;
use crate::app::state::AppState;
use crate::core::models::Port;
use crate::core::path_safety::safe_join;
use crate::fork::rom_library::{load_local_entries, Mapping, RomLibraryStore};
use crate::fork::setup_wizard::files_state::FilesState;
use crate::fork::rom_screens::connect_screen::{ConnectScreen, Pick};
use crate::fork::rom_screens::file_pick::{FileBrowser, Line as FileLine};
use crate::fork::rom_screens::find::{
    find_game_files_for_port, find_game_files_for_ports, match_hand_picked, no_match_lines, use_anyway,
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
/// them), not connected. What the setup wizard asks about. A port that names only a game title counts as not having
/// them until a file in its folder carries the title or a connection is recorded (Home shows that one without red:
/// `port_files_state`).
pub(crate) fn port_missing_game_files(app: &AppState, port: &Port) -> bool {
    FilesState::counts_as_missing(port_files_state(app, port))
}

/// Home's view: `None` when the port is not installed or needs no game files, else the state (red only for `Missing`).
pub(crate) fn port_files_state(app: &AppState, port: &Port) -> Option<FilesState> {
    if !port_needs_game_files(app, port) {
        return None;
    }
    let Ok(dir) = crate::core::path_safety::safe_join(&app.paths.library_dir(), &port.folder) else { return Some(FilesState::Missing) };
    Some(crate::fork::setup_wizard::files_state::files_state(port, &dir, &app.paths.config_dir))
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
        let name = port.display_name().to_string();
        return gf_result(app, router, &name, "No game files needed", &format!("{name} does not name any game files it needs, so there is nothing to look for."), back);
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
        // Fork (port page): one port's search waits on a page (Cancel, or B), not a list.
        JobUi::Page { title: SEARCH_TITLE, cancellable: true },
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
    // Fork (port page): the result is a card page (`fork::rom_screens::pages`), every place named the human way.
    let places = Places::system();
    let place = |p: &Path| human_path(p, &places);
    let ctx = Rc::new(Ctx { req, back });
    match outcome {
        FindOutcome::NoFilesNeeded => (ctx.back)(app, router),
        FindOutcome::Found { row, .. } if row.connected => {
            let files = connected_files(&ctx.req, &place);
            gf_page(app, router, pages::connected_page(&name, &files), ctx, None);
        }
        FindOutcome::Found { row, .. } => {
            let files: Vec<(String, String)> = row
                .matched
                .slots
                .iter()
                .filter(|s| !s.optional)
                .filter_map(|s| if let SlotStatus::Found(f) = &s.status { Some((file_name(&f.path), f.path.parent().map(&place).unwrap_or_default())) } else { None })
                .collect();
            let page = pages::found_page(&name, &files, row.low_confidence);
            gf_page(app, router, page, ctx, Some(Rc::new(row)));
        }
        FindOutcome::Ambiguous { row, slots, .. } => pick_then_connect(app, router, row, slots, ctx.back.clone()),
        FindOutcome::NotFound { needs, looked_in, notes } => {
            let page = pages::not_found_page(&name, &needs, &place(&looked_in.folder), &notes, looked_in.problems);
            gf_page(app, router, page, ctx, None);
        }
        FindOutcome::NoRomFolder { needs } => gf_page(app, router, pages::no_folder_page(&name, &needs), ctx, None),
        FindOutcome::FolderUnreadable { needs, folder, error } => {
            let page = pages::unreadable_page(&name, &needs, &place(&folder), &ellipsize(&error, 80));
            gf_page(app, router, page, ctx, None);
        }
        FindOutcome::FolderLooksEmpty { needs, folder, saved } => {
            let why = format!("it held {} last time, but now it has none", crate::fork::rom_screens::find::count_files(saved));
            gf_page(app, router, pages::unreadable_page(&name, &needs, &place(&folder), &why), ctx, None);
        }
    }
}

// -- The result pages -------------------------------------------------------------------------------------

/// The one port the pages are about, and where they lead when the user is done (Later, Done, B).
struct Ctx {
    req: FindRequest,
    back: Back,
}

/// The connected files of the port: (file name, its place).
fn connected_files(req: &FindRequest, place: &dyn Fn(&Path) -> String) -> Vec<(String, String)> {
    req.mappings.iter().filter(|m| m.port_key == req.port.key()).map(|m| (file_name(&m.source), m.source.parent().map(place).unwrap_or_default())).collect()
}

/// Shows a result page; `row` is what Use it connects (the Found page).
fn gf_page(app: &Rc<AppState>, router: &Router, page: Page<GfAct>, ctx: Rc<Ctx>, row: Option<Rc<PortRow>>) {
    let again_page = page.clone();
    let (c_act, c_back, c_exit) = (ctx.clone(), ctx.clone(), ctx);
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| {
            let key = c_act.req.port.key().to_string();
            match act {
                GfAct::UseIt => match &row {
                    Some(row) => connect_row(app, router, row, c_act.back.clone()),
                    None => (c_act.back)(app, router),
                },
                GfAct::LookAgain => search_key(app, router, &key, c_act.back.clone()),
                GfAct::ChooseFolder => choose_folder(app, router, Then::Find { key, back: c_act.back.clone() }),
                GfAct::ChooseFile => {
                    let again: Back = {
                        let (page, ctx, row) = (again_page.clone(), c_act.clone(), row.clone());
                        Rc::new(move |app, router| gf_page(app, router, page.clone(), ctx.clone(), row.clone()))
                    };
                    connect_by_hand(app, router, c_act.req.clone(), c_act.back.clone(), again);
                }
                GfAct::Later | GfAct::Done | GfAct::Copy | GfAct::Pick(_) => (c_act.back)(app, router),
            }
        }),
        on_back: Rc::new(move |app, router| (c_back.back)(app, router)),
        on_exit: Rc::new(move |app, router| (c_exit.back)(app, router)),
        on_input: None,
    };
    show_page(app, router, page, handlers);
}

/// A page with only Done (a result), leading to `back`.
fn gf_result(app: &Rc<AppState>, router: &Router, name: &str, title: &str, text: &str, back: Back) {
    let (b1, b2, b3) = (back.clone(), back.clone(), back);
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, _: GfAct| b1(app, router)),
        on_back: Rc::new(move |app, router| b2(app, router)),
        on_exit: Rc::new(move |app, router| b3(app, router)),
        on_input: None,
    };
    show_page(app, router, pages::result_page(name, title, text), handlers);
}

/// "Use it": connect what was found, the way Settings > Game files does (its own rules: use in place, link, or a copy
/// only after a yes with its size shown), on pages.
fn connect_row(app: &Rc<AppState>, router: &Router, row: &PortRow, back: Back) {
    if !row.installed {
        let name = row.port.display_name().to_string();
        return gf_result(app, router, &name, "Install it first", &format!("Install {name} first, then connect its game files."), back);
    }
    let Some(store) = load_store(app, router) else { return };
    let Ok(port_dir) = safe_join(&app.paths.library_dir(), &row.port.folder) else { return };
    connect_screen_now(app, router, ConnectScreen::new(row, port_dir, store.mappings()), back);
}

fn connect_screen_now(app: &Rc<AppState>, router: &Router, screen: ConnectScreen, back: Back) {
    let name = screen.port_name.clone();
    let requests = match screen.requests() {
        Ok(r) => r,
        Err(text) => return gf_result(app, router, &name, "Nothing was connected", &text, back),
    };
    let unchecked = Rc::new(screen.unchecked_files());
    run_job(
        app,
        router,
        JobUi::Page { title: "Checking the files", cancellable: false },
        move |_| crate::fork::rom_screens::connect_screen::make_plan(requests),
        move |app, router, result| {
            let plan = match result.and_then(|r| r) {
                Ok(plan) => Rc::new(plan),
                Err(e) => return gf_result(app, router, &name, "Nothing was connected", &e, back.clone()),
            };
            if !plan.needs_copy_approval() {
                return apply_pages(app, router, &name, plan, Approval::None, unchecked.clone(), back.clone());
            }
            let (name2, plan2, unchecked2, back2, back3, back4) = (name.clone(), plan.clone(), unchecked.clone(), back.clone(), back.clone(), back.clone());
            let handlers = Handlers {
                on_action: Rc::new(move |app, router, act| match act {
                    GfAct::Copy => apply_pages(app, router, &name2, plan2.clone(), Approval::CopyAllowed, unchecked2.clone(), back2.clone()),
                    _ => back2(app, router),
                }),
                on_back: Rc::new(move |app, router| back3(app, router)),
                on_exit: Rc::new(move |app, router| back4(app, router)),
                on_input: None,
            };
            show_page(app, router, pages::copy_page(&name, &plan.confirm_text(&name)), handlers);
        },
    );
}

fn apply_pages(app: &Rc<AppState>, router: &Router, name: &str, plan: Rc<crate::fork::rom_screens::connect_screen::PlanOutput>, approval: Approval, unchecked: Rc<Vec<String>>, back: Back) {
    let title = if approval == Approval::CopyAllowed { "Copying game files" } else { "Connecting game files" };
    let (path, plans) = (store_path(app), plan.plans.clone());
    let name = name.to_string();
    run_job(
        app,
        router,
        JobUi::Page { title, cancellable: false },
        move |_| crate::fork::rom_screens::connect_screen::apply_plans(&path, &plans, approval),
        move |app, router, result| match result.and_then(|r| r) {
            Ok(mappings) => {
                let mut text = crate::fork::rom_screens::connect_screen::done_text(&name, &plan.plans, &mappings);
                for w in plan.warnings() {
                    text.push_str(&format!("\n\nNote: {w}"));
                }
                for file in unchecked.iter() {
                    text.push_str(&format!("\n\n{file} was matched by its name only. If the game does not start, choose the file yourself."));
                }
                gf_result(app, router, &name, "Game files connected", &text, back.clone());
            }
            Err(e) => gf_result(app, router, &name, "Nothing was connected", &format!("{e}\n\nNothing was changed."), back.clone()),
        },
    );
}

// -- Several ports at once --------------------------------------------------------------------------

/// A message; with a `then` it is a Continue dialog (also on B) that runs it.
fn notice_then(app: &Rc<AppState>, router: &Router, text: &str, then: &Option<Back>) {
    let Some(then) = then else { return message(app, router, TITLE, text) };
    let on_confirm = then.clone();
    open_confirm_dialog(app, router, TITLE.into(), format!("{text}\n").into(), "Continue".into(), move |app, router| on_confirm(app, router));
    if let DialogSlot::Confirm(d) = &*app.dialog_nav.dialogs.borrow() {
        let (app, router, then) = (app.clone(), router.clone(), then.clone());
        d.on_close_requested(move || if crate::fork::press_echo::key(crate::fork::press_echo::Press::Back) {
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
        d.on_close_requested(move || if crate::fork::press_echo::key(crate::fork::press_echo::Press::Back) {
            close_current_dialog(&app, &router);
            then(&app, &router);
        });
    }
}

// -- Connect ----------------------------------------------------------------------------------------

fn open_row(app: &Rc<AppState>, router: &Router, row: &PortRow, back: Back) {
    // Fork (port page): a file the user picked or said to use goes straight to connecting it, on pages.
    connect_row(app, router, row, back);
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

/// One page per thing with several matching files ("Which one is your copy?"), then connecting with the choices made.
fn pick_next(app: &Rc<AppState>, router: &Router, screen: Rc<RefCell<ConnectScreen>>, slots: Rc<Vec<(usize, String)>>, n: usize, back: Back) {
    let Some((slot, label)) = slots.get(n).cloned() else {
        let screen = screen.borrow().clone();
        return connect_screen_now(app, router, screen, back);
    };
    if !screen.borrow().is_editable(slot) {
        return pick_next(app, router, screen, slots, n + 1, back);
    }
    let (picks, pick_labels) = (screen.borrow().picks(slot), screen.borrow().pick_labels(slot));
    let files: Vec<(Pick, String)> =
        picks.into_iter().zip(pick_labels).filter(|(p, _)| matches!(p, Pick::File(_))).map(|(p, l)| (p, l.trim_start_matches("File: ").to_string())).collect();
    let name = screen.borrow().port_name.clone();
    let shown: Vec<(String, String)> = files.iter().map(|(_, l)| (l.clone(), String::new())).collect();
    let page = pages::pick_page(&name, &ellipsize(&label, 50), &shown);
    let (b1, b2) = (back.clone(), back.clone());
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| match act {
            GfAct::Pick(i) => {
                if let Some((pick, _)) = files.get(i) {
                    screen.borrow_mut().choose(slot, *pick);
                }
                pick_next(app, router, screen.clone(), slots.clone(), n + 1, back.clone());
            }
            _ => back(app, router),
        }),
        on_back: Rc::new(move |app, router| b1(app, router)),
        on_exit: Rc::new(move |app, router| b2(app, router)),
        on_input: None,
    };
    show_page(app, router, page, handlers);
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
        JobUi::Page { title: "Checking the file", cancellable: true },
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
