//! "Find this port's game files": the screens around `fork::rom_screens::find`.
//!
//! Entry points (all `pub(crate)`, re-exported from `app::rom_screens`):
//! - [`after_install`]: called by the app when an install finishes. Once per port, only for a port
//!   that names files it cannot run without and is not connected yet, it looks in the game files folder
//!   on a worker thread (with progress and Cancel) and shows what it found.
//! - [`search_ports_now`]: "Find their game files" on the import's card page. One walk of the folder for
//!   several ports, then a page with one tile per port (its outcome as the tile's note) and "Look in another folder".
//! - [`find_for_port`]: the same search for one port from anywhere (the setup wizard, the Port page, the install
//!   wizard). `back` runs when the user leaves (Later, Cancel, Back). [`find_for_port_asking`] is the same from Settings
//!   > Ports > Game files, which also asks how the file reaches the port (point to it, copy it, read it where it is).
//! - [`show_ports_needing_files`]: Settings > Ports > Game files > Ports that need game files.
//!
//! What the user sees: the found page when the file is there (a "Which one is your copy?" page first when several
//! files match), or a page that names the port, the files it needs and where it looked, with Choose the file myself,
//! Look again and Add it later. Nothing is copied without the user's yes (the copy page asks, with the size), and game
//! files are only read.

use super::{choose_folder, load_store, message, run_job, store_path, Back, JobUi, Router, Then, JOB_RUNNING, TITLE};
use crate::app::dialogs::{close_current_dialog, dialog_is_open};
use crate::app::page_dialog::{show_page, Handlers};
use crate::fork::art::thumbs::ArtRef;
use crate::fork::page::Page;
use crate::fork::port_page::paths::{human_path, Places};
use crate::fork::rom_library::{Approval, SlotStatus};
use crate::fork::rom_screens::pages::{self, GfAct};
use crate::fork::rom_screens::settings_pages::{self, GsAct};
use crate::fork::rom_screens::util::file_name;
use crate::app::folder_picker_screen::open_folder_picker;
use crate::app::state::AppState;
use crate::core::models::Port;
use crate::core::path_safety::safe_join;
use crate::fork::rom_library::{load_local_entries, Mapping, RomLibraryStore};
use crate::fork::setup_wizard::files_state::FilesState;
use crate::fork::rom_screens::connect_screen::{ConnectScreen, Pick};
use crate::fork::rom_screens::file_pick::FileBrowser;
use crate::fork::rom_screens::find::{
    find_game_files_for_port, find_game_files_for_ports, match_hand_picked, use_anyway, FindOutcome, FindRequest, HandPick, Need,
    LATER_HINT,
};
use crate::fork::rom_screens::offer::{self, AutoSearch};
use crate::fork::rom_screens::report::PortRow;
use crate::fork::rom_screens::util::ellipsize;
use crate::fork::settings::{ForkSettings, FILE_NAME};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::AtomicBool;

const SEARCH_TITLE: &str = "Looking for game files";

fn close_back() -> Back {
    Rc::new(|app, router| close_current_dialog(app, router))
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

/// For the Port page: is the port installed and does it name files it needs.
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

// -- Entry points ---------------------------------------------------------------------------------

/// Right after a port finished installing. Once per port; quiet when there is nothing to do or a
/// dialog or another game files task is already up (the port stays reachable from its Port page and Settings).
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
    search(app, router, req, store, close_back(), Some(key.to_string()), false);
}

/// Look for one port's game files and show the result. `back` runs when the user leaves.
pub(crate) fn find_for_port(app: &Rc<AppState>, router: &Router, key: &str, back: Back) {
    find_one(app, router, key, back, false);
}

/// [`find_for_port`] from Settings > Ports > Game files: connecting also asks how the file reaches the port when the port
/// allows more than one way.
pub(crate) fn find_for_port_asking(app: &Rc<AppState>, router: &Router, key: &str, back: Back) {
    find_one(app, router, key, back, true);
}

fn find_one(app: &Rc<AppState>, router: &Router, key: &str, back: Back, ask_how: bool) {
    let Some(port) = app.find_port(key) else {
        return message(app, router, TITLE, "That port is no longer in the list.");
    };
    let Some(store) = load_store(app, router) else { return };
    let req = request_for(app, &local_entries(app), &port, store.mappings().to_vec());
    if !req.needs_files() {
        let name = port.display_name().to_string();
        return gf_result(app, router, &name, "No game files needed", &format!("{name} does not name any game files it needs, so there is nothing to look for."), back);
    }
    search(app, router, req, store, back, None, ask_how);
}

/// The Port page's "Game files" (kept under its old name for the callers).
pub(crate) fn open_from_info(app: &Rc<AppState>, router: &Router, port: Port) {
    find_for_port(app, router, port.key(), close_back());
}

/// "Find their game files" (the import's card page): every port among `keys` that needs a game file, whether or not
/// it was searched before. False only when it cannot start (another task runs).
pub(crate) fn search_ports_now(app: &Rc<AppState>, router: &Router, keys: &[String], then: Option<Back>) -> bool {
    search_ports(app, router, keys, then, true)
}

/// Searches several ports with one walk of the folder and shows the outcomes as tiles. `then` runs when the user is done
/// with what it shows: Done, B on the page, OK on its notices, or B on the search's waiting page (not when it returns
/// false: nothing was shown). `force`: search every port that needs files; otherwise only the ones not searched after
/// their install yet.
fn search_ports(app: &Rc<AppState>, router: &Router, keys: &[String], then: Option<Back>, force: bool) -> bool {
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
    if store.rom_dir().is_none() {
        let outcomes = find_game_files_for_ports(&store, &reqs, &AtomicBool::new(false), &mut |_| {}).unwrap_or_default();
        mark_searched(app, &searched);
        show_batch(app, router, Rc::new(reqs.into_iter().zip(outcomes).collect()), then);
        return true;
    }
    let worker_reqs = reqs.clone();
    if let Some(then) = &then {
        super::ABANDON_HOOK.with(|h| *h.borrow_mut() = Some(then.clone()));
    }
    run_job(
        app,
        router,
        // Fork (game files settings): a waiting page with a Cancel card, not a list.
        JobUi::Page { title: SEARCH_TITLE, cancellable: true },
        move |ctl| find_game_files_for_ports(&store, &worker_reqs, ctl.flag(), &mut |t| ctl.set_progress(t)),
        move |app, router, result| match result {
            Ok(Some(outcomes)) => {
                mark_searched(app, &searched);
                show_batch(app, router, Rc::new(reqs.into_iter().zip(outcomes).collect()), then);
            }
            Ok(None) => notice_then(app, router, "The search was stopped", &format!("Nothing was changed.\n\n{LATER_HINT}"), &then),
            Err(e) => notice_then(app, router, "Could not look for game files", &format!("{e}\n\nNothing was changed. {LATER_HINT}"), &then),
        },
    );
    true
}

/// The installed ports that name game files they need, the ones not connected yet first.
pub(super) struct NeedRow {
    pub name: String,
    pub key: String,
    pub connected: bool,
}

pub(super) fn need_rows(app: &AppState, store: &RomLibraryStore) -> Vec<NeedRow> {
    let catalog = app.catalog.borrow().clone();
    let local = local_entries(app);
    let mut rows: Vec<NeedRow> = catalog
        .iter()
        .filter(|p| !p.is_android_only() && app.is_installed(p))
        .map(|p| request_for(app, &local, p, store.mappings().to_vec()))
        .filter(FindRequest::needs_files)
        .map(|r| NeedRow { name: r.port.display_name().to_string(), key: r.port.key().to_string(), connected: r.already_connected() })
        .collect();
    rows.sort_by(|a, b| (a.connected, a.name.to_lowercase()).cmp(&(b.connected, b.name.to_lowercase())));
    rows
}

/// Settings > Ports > Game files > Ports that need game files: one card per installed port that names files; choosing
/// one looks for its files.
pub(crate) fn show_ports_needing_files(app: &Rc<AppState>, router: &Router) {
    let Some(store) = load_store(app, router) else { return };
    let rows = need_rows(app, &store);
    let shown: Vec<(String, bool)> = rows.iter().map(|r| (r.name.clone(), r.connected)).collect();
    let page = settings_pages::need_page(&shown);
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| match act {
            GsAct::Port(i) => {
                if let Some(row) = rows.get(i) {
                    find_for_port_asking(app, router, &row.key, Rc::new(show_ports_needing_files));
                }
            }
            _ => super::menu(app, router),
        }),
        on_back: Rc::new(super::menu),
        on_exit: Rc::new(|app, router| close_current_dialog(app, router)),
        on_input: None,
    };
    show_page(app, router, page, handlers);
}

// -- The search -----------------------------------------------------------------------------------

/// `mark`: the port whose automatic search this is; it is marked as searched when the result shows.
fn search(app: &Rc<AppState>, router: &Router, req: FindRequest, store: RomLibraryStore, back: Back, mark: Option<String>, ask_how: bool) {
    if store.rom_dir().is_none() {
        // Nothing to walk: answer right away.
        if let Some(outcome) = find_game_files_for_port(&store, &req, &AtomicBool::new(false), &mut |_| {}) {
            show_outcome(app, router, req, outcome, back, mark, ask_how);
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
            Ok(Some(outcome)) => show_outcome(app, router, req, outcome, back, mark, ask_how),
            Ok(None) => back(app, router),
            Err(e) => message(app, router, TITLE, &e),
        },
    );
}

/// Search again for the same port, with the connections as they are now.
fn search_key(app: &Rc<AppState>, router: &Router, key: &str, back: Back, ask_how: bool) {
    let Some(port) = app.find_port(key) else { return back(app, router) };
    let Some(store) = load_store(app, router) else { return };
    let req = request_for(app, &local_entries(app), &port, store.mappings().to_vec());
    search(app, router, req, store, back, None, ask_how);
}

/// Where "Choose ROM folder..." returns to: after a folder is chosen (search again there) and on Back.
pub(super) fn resume(app: &Rc<AppState>, router: &Router, key: &str, back: &Back, ask_how: bool) {
    search_key(app, router, key, back.clone(), ask_how);
}

fn show_outcome(app: &Rc<AppState>, router: &Router, req: FindRequest, outcome: FindOutcome, back: Back, mark: Option<String>, ask_how: bool) {
    let name = req.port.display_name().to_string();
    if let Some(key) = mark {
        mark_searched(app, &[key]);
    }
    // Fork (port page): the result is a card page (`fork::rom_screens::pages`), every place named the human way.
    let places = Places::system();
    let place = |p: &Path| human_path(p, &places);
    let ctx = Rc::new(Ctx { req, back, ask_how });
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
        FindOutcome::Ambiguous { row, slots, .. } => pick_then_connect(app, router, row, slots, ctx.back.clone(), ask_how),
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

/// The one port the pages are about, where they lead when the user is done (Later, Done, B), and whether connecting
/// asks how the file reaches the port.
struct Ctx {
    req: FindRequest,
    back: Back,
    ask_how: bool,
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
            let ask_how = c_act.ask_how;
            match act {
                GfAct::UseIt => match &row {
                    Some(row) => connect_row(app, router, row, c_act.back.clone(), ask_how),
                    None => (c_act.back)(app, router),
                },
                GfAct::LookAgain => search_key(app, router, &key, c_act.back.clone(), ask_how),
                GfAct::ChooseFolder => choose_folder(app, router, Then::Find { key, back: c_act.back.clone(), ask_how }),
                GfAct::ChooseFile => {
                    let again: Back = {
                        let (page, ctx, row) = (again_page.clone(), c_act.clone(), row.clone());
                        Rc::new(move |app, router| gf_page(app, router, page.clone(), ctx.clone(), row.clone()))
                    };
                    connect_by_hand(app, router, c_act.req.clone(), c_act.back.clone(), again, ask_how);
                }
                GfAct::Later | GfAct::Done | GfAct::Copy | GfAct::Pick(_) | GfAct::Up | GfAct::Folder(_) | GfAct::File(_) | GfAct::UseAnyway(_) | GfAct::OtherFile | GfAct::Way(_) => {
                    (c_act.back)(app, router)
                }
            }
        }),
        on_back: Rc::new(move |app, router| (c_back.back)(app, router)),
        on_exit: Rc::new(move |app, router| (c_exit.back)(app, router)),
        on_input: None,
    };
    show_page(app, router, page, handlers);
}

/// Handlers of a page whose every choice but `on` leads `back`.
fn simple_handlers(back: Back, on: impl Fn(&Rc<AppState>, &Router, GfAct) -> bool + 'static) -> Handlers<GfAct> {
    let (b1, b2, b3) = (back.clone(), back.clone(), back);
    Handlers {
        on_action: Rc::new(move |app, router, act| {
            if !on(app, router, act) {
                b1(app, router)
            }
        }),
        on_back: Rc::new(move |app, router| b2(app, router)),
        on_exit: Rc::new(move |app, router| b3(app, router)),
        on_input: None,
    }
}

/// A page with only Done (a result), leading to `back`.
fn gf_result(app: &Rc<AppState>, router: &Router, name: &str, title: &str, text: &str, back: Back) {
    show_page(app, router, pages::result_page(name, title, text), simple_handlers(back, |_, _, _| false));
}

/// "Use it": connect what was found, the way the port allows (read in place, pointed to, or a copy only after a yes with
/// its size shown), on pages. With `ask_how` the user chooses the way first.
fn connect_row(app: &Rc<AppState>, router: &Router, row: &PortRow, back: Back, ask_how: bool) {
    if !row.installed {
        let name = row.port.display_name().to_string();
        return gf_result(app, router, &name, "Install it first", &format!("Install {name} first, then connect its game files."), back);
    }
    let Some(store) = load_store(app, router) else { return };
    let Ok(port_dir) = safe_join(&app.paths.library_dir(), &row.port.folder) else { return };
    connect_screen_now(app, router, ConnectScreen::new(row, port_dir, store.mappings()), back, ask_how);
}

/// Fork (game files settings): "How should the port get it?" when asked for and the port allows more than one way, then
/// connecting with the way chosen.
fn connect_screen_now(app: &Rc<AppState>, router: &Router, screen: ConnectScreen, back: Back, ask_how: bool) {
    let ways = screen.ways();
    if !ask_how || ways.len() < 2 || screen.chosen_files().is_empty() {
        return connect_planned(app, router, screen, back);
    }
    let page = pages::how_page(&screen.port_name, &screen.chosen_files(), &ways, screen.suggested_way(), screen.bytes_if_copied());
    let go_back = back.clone();
    let handlers = simple_handlers(back, move |app, router, act| {
        let GfAct::Way(way) = act else { return false };
        let mut chosen = screen.clone();
        chosen.set_way(way);
        connect_planned(app, router, chosen, go_back.clone());
        true
    });
    show_page(app, router, page, handlers);
}

fn connect_planned(app: &Rc<AppState>, router: &Router, screen: ConnectScreen, back: Back) {
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
            let (name2, plan2, unchecked2, back2) = (name.clone(), plan.clone(), unchecked.clone(), back.clone());
            let handlers = simple_handlers(back.clone(), move |app, router, act| {
                if act != GfAct::Copy {
                    return false;
                }
                apply_pages(app, router, &name2, plan2.clone(), Approval::CopyAllowed, unchecked2.clone(), back2.clone());
                true
            });
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
                    text.push_str(&format!("\n\n{file}: not checked, the name matches. If the game does not start, choose the file yourself."));
                }
                gf_result(app, router, &name, "Game files connected", &text, back.clone());
            }
            Err(e) => gf_result(app, router, &name, "Nothing was connected", &format!("{e}\n\nNothing was changed."), back.clone()),
        },
    );
}

// -- Several ports at once --------------------------------------------------------------------------

/// Fork (game files settings): the search stopped or failed: a page with one OK, which runs `then` (as B does). Without
/// a `then` it closes.
fn notice_then(app: &Rc<AppState>, router: &Router, title: &str, text: &str, then: &Option<Back>) {
    let then = then.clone().unwrap_or_else(close_back);
    show_page(app, router, pages::batch_notice_page(title, text), simple_handlers(then, |_, _, _| false));
}

/// Fork (game files settings): one tile per port with what was found as its note (was a list of "Game files for X: found"
/// rows). A tile opens that port's pages, which come back here; Look in another folder searches the same ports again
/// there; Done and B run `then`.
fn show_batch(app: &Rc<AppState>, router: &Router, items: Rc<Vec<(FindRequest, FindOutcome)>>, then: Option<Back>) {
    let shown: Vec<usize> = (0..items.len()).filter(|i| !matches!(items[*i].1, FindOutcome::NoFilesNeeded)).collect();
    let tiles: Vec<(String, Option<ArtRef>, String)> = shown
        .iter()
        .map(|i| {
            let (req, outcome) = &items[*i];
            (req.port.display_name().to_string(), Some(ArtRef::of_port(&req.port)), pages::outcome_note(outcome).to_string())
        })
        .collect();
    let has_folder = RomLibraryStore::load(&store_path(app)).ok().is_some_and(|s| s.rom_dir().is_some());
    let page = pages::batch_page(&tiles, has_folder);
    let keys: Vec<String> = items.iter().map(|(r, _)| r.port.key().to_string()).collect();
    let done: Back = then.clone().unwrap_or_else(close_back);
    let again: Back = {
        let (items, then) = (items.clone(), then.clone());
        Rc::new(move |app, router| show_batch(app, router, items.clone(), then.clone()))
    };
    let handlers = simple_handlers(done, move |app, router, act| {
        match act {
            GfAct::Pick(n) => {
                let Some(&i) = shown.get(n) else { return true };
                let (req, outcome) = items[i].clone();
                show_outcome(app, router, req, outcome, again.clone(), None, false);
            }
            GfAct::ChooseFolder => {
                let (keys, then, again) = (keys.clone(), then.clone(), again.clone());
                open_folder_picker(
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
            _ => return false,
        }
        true
    });
    show_page(app, router, page, handlers);
}

// -- Connect ----------------------------------------------------------------------------------------

fn open_row(app: &Rc<AppState>, router: &Router, row: &PortRow, back: Back, ask_how: bool) {
    // Fork (port page): a file the user picked or said to use goes straight to connecting it, on pages.
    connect_row(app, router, row, back, ask_how);
}

fn pick_then_connect(app: &Rc<AppState>, router: &Router, row: PortRow, slots: Vec<usize>, back: Back, ask_how: bool) {
    if !row.installed {
        return open_row(app, router, &row, back, ask_how);
    }
    let Some(store) = load_store(app, router) else { return };
    let Ok(port_dir) = safe_join(&app.paths.library_dir(), &row.port.folder) else { return };
    let screen = Rc::new(RefCell::new(ConnectScreen::new(&row, port_dir, store.mappings())));
    let slots: Rc<Vec<(usize, String)>> = Rc::new(slots.into_iter().map(|i| (i, row.matched.slots[i].label.clone())).collect());
    pick_next(app, router, screen, slots, 0, back, ask_how);
}

/// One page per thing with several matching files ("Which one is your copy?"), then connecting with the choices made.
fn pick_next(app: &Rc<AppState>, router: &Router, screen: Rc<RefCell<ConnectScreen>>, slots: Rc<Vec<(usize, String)>>, n: usize, back: Back, ask_how: bool) {
    let Some((slot, label)) = slots.get(n).cloned() else {
        let screen = screen.borrow().clone();
        return connect_screen_now(app, router, screen, back, ask_how);
    };
    if !screen.borrow().is_editable(slot) {
        return pick_next(app, router, screen, slots, n + 1, back, ask_how);
    }
    // Fork (game files settings): each file with whether it was checked, its size and its place the human way (was
    // "(chosen) oot.z64 (8 MB)  in n64").
    let files: Vec<Pick> = screen.borrow().picks(slot).into_iter().filter(|p| matches!(p, Pick::File(_))).collect();
    let candidates = screen.borrow().candidates(slot);
    let places = Places::system();
    let shown: Vec<(String, String)> = files
        .iter()
        .filter_map(|p| if let Pick::File(c) = p { candidates.get(*c) } else { None })
        .map(|c| (c.name.clone(), pages::file_detail(c.checked, c.size, &human_path(&c.folder, &places))))
        .collect();
    let name = screen.borrow().port_name.clone();
    let page = pages::pick_page(&name, &ellipsize(&label, 50), &shown);
    let (b1, b2) = (back.clone(), back.clone());
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| match act {
            GfAct::Pick(i) => {
                if let Some(pick) = files.get(i) {
                    screen.borrow_mut().choose(slot, *pick);
                }
                pick_next(app, router, screen.clone(), slots.clone(), n + 1, back.clone(), ask_how);
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

fn connect_by_hand(app: &Rc<AppState>, router: &Router, req: FindRequest, back: Back, cancel: Back, ask_how: bool) {
    let start = RomLibraryStore::load(&store_path(app)).ok().and_then(|s| s.rom_dir().map(Path::to_path_buf)).filter(|d| d.is_dir());
    let start = start.or_else(super::home_dir).unwrap_or_else(|| PathBuf::from("/"));
    // Fork (install wizard): Up one level stops at Internal storage or the drive, never a raw "/home" or "/run/media".
    let browser = FileBrowser::open(&start).with_tops(Places::system().tops());
    show_file_browser(app, router, Rc::new(RefCell::new(browser)), Rc::new(req), back, cancel, ask_how);
}

/// Fork (install wizard): "Choose the file myself" as card pages (one folder a page, the place the human way), not a
/// picker list showing the raw path.
fn show_file_browser(app: &Rc<AppState>, router: &Router, browser: Rc<RefCell<FileBrowser>>, req: Rc<FindRequest>, back: Back, cancel: Back, ask_how: bool) {
    let folder = {
        let b = browser.borrow();
        pages::Folder { place: human_path(&b.current, &Places::system()), can_go_up: b.can_go_up(), folders: b.folder_names(), files: b.file_entries(), hidden: b.hidden_count, error: b.error.clone() }
    };
    let page = pages::browse_page(req.port.display_name(), &folder);
    let (c1, c2) = (cancel.clone(), cancel.clone());
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| {
            match act {
                GfAct::Up => browser.borrow_mut().up(),
                GfAct::Folder(i) => browser.borrow_mut().enter(i),
                GfAct::File(i) => {
                    let Some(path) = browser.borrow().file(i).map(Path::to_path_buf) else { return };
                    return hand_picked(app, router, browser.clone(), req.clone(), path, back.clone(), cancel.clone(), ask_how);
                }
                _ => return cancel(app, router),
            }
            show_file_browser(app, router, browser.clone(), req.clone(), back.clone(), cancel.clone(), ask_how);
        }),
        on_back: Rc::new(move |app, router| c1(app, router)),
        on_exit: Rc::new(move |app, router| c2(app, router)),
        on_input: None,
    };
    show_page(app, router, page, handlers);
}

#[allow(clippy::too_many_arguments)]
fn hand_picked(app: &Rc<AppState>, router: &Router, browser: Rc<RefCell<FileBrowser>>, req: Rc<FindRequest>, path: PathBuf, back: Back, cancel: Back, ask_how: bool) {
    let worker_req = (*req).clone();
    // B on the progress list goes where Cancel does: back to the same folder to pick again.
    {
        let (browser, req, back, cancel) = (browser.clone(), req.clone(), back.clone(), cancel.clone());
        let again: Back = Rc::new(move |app, router| show_file_browser(app, router, browser.clone(), req.clone(), back.clone(), cancel.clone(), ask_how));
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
            Ok(None) => show_file_browser(app, router, browser.clone(), req.clone(), back.clone(), cancel.clone(), ask_how),
            Ok(Some(HandPick::Matches { row })) => open_row(app, router, &row, back, ask_how),
            // Fork (install wizard): the answer as a card page (plain words, no checksums), not a picker list.
            Ok(Some(HandPick::NoMatch { row, file, slots, wrong_version })) => {
                let name = req.port.display_name().to_string();
                let needs: Vec<Need> = row.matched.slots.iter().filter(|s| !s.optional).map(Need::from_slot).collect();
                let uses: Vec<String> = slots.iter().map(|i| row.matched.slots[*i].label.clone()).collect();
                let page = pages::no_match_page(&name, &file_name(&file.path), wrong_version, &needs, &uses);
                let (c1, c2) = (cancel.clone(), cancel.clone());
                let handlers = Handlers {
                    on_action: Rc::new(move |app, router, act| match act {
                        GfAct::UseAnyway(n) => {
                            if let Some(&slot) = slots.get(n) {
                                open_row(app, router, &use_anyway(&row, slot, &file), back.clone(), ask_how);
                            }
                        }
                        GfAct::OtherFile => show_file_browser(app, router, browser.clone(), req.clone(), back.clone(), cancel.clone(), ask_how),
                        _ => cancel(app, router),
                    }),
                    on_back: Rc::new(move |app, router| c1(app, router)),
                    on_exit: Rc::new(move |app, router| c2(app, router)),
                    on_input: None,
                };
                show_page(app, router, page, handlers);
            }
        },
    );
}
