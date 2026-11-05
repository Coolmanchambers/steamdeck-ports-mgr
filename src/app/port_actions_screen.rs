//! The Port page's questions on screen (`crate::fork::port_page::actions` says what they show): which version to install
//! ("Getting the versions", "Which version?", "Could not get the versions"), whether to install the extra files ("Install
//! the extra files?", "Getting the extras", "Extras installed") and whether to reset the play time. They replace upstream's
//! "Loading" progress dialog, "Choose a version" picker, "Error" message, "Install extras - X" confirm, "Extras" progress
//! and "Reset Game Time - X" confirm.
//!
//! The work behind them is upstream's, unchanged: the release list is fetched on a thread (`Forge::list_releases`) and
//! answers with `AppEvent::VersionsFetched` / `VersionsFetchError`; the extras install is `jobs::run_extra_install` and
//! answers with `AppEvent::ExtraInstallDone` (`events.rs` hands both to [`versions_fetched`] and [`extras_done`]). A
//! version that is chosen goes through the same game check (for a port that is not installed) and `start_install`, which
//! shows the install wizard's downloading page. Every page leads back through the `Back` its caller gave.

use super::dialogs::close_current_dialog;
use super::events::{lock, AppEvent};
use super::page_dialog::{show_page, show_page_handle, Handlers, PageHandle};
use super::playtime::is_port_running;
use super::state::AppState;
use crate::core::forge::Forge;
use crate::core::models::Port;
use crate::fork::port_page::actions::{self as pages, Step};
use crate::fork::port_page::{play_time_words, Card};
use crate::ui::gamepad_router::GamepadRouter;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

type Router = Rc<RefCell<GamepadRouter>>;
/// Where Back goes.
pub(crate) type Back = Rc<dyn Fn(&Rc<AppState>, &Router)>;

/// How many of the newest releases are listed.
const VERSION_CHOICES: usize = 3;

/// A job that was asked for and has not answered yet: whose it is and where its pages lead back to. `id` numbers the request
/// (its answer carries it back), `wait` is the waiting page, so an answer is shown only while that page is still on screen.
struct Pending {
    id: u64,
    port: Port,
    card: Card,
    back: Back,
    wait: Option<PageHandle<Step>>,
}

thread_local! {
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
    static VERSIONS: RefCell<HashMap<String, Pending>> = RefCell::new(HashMap::new());
    static EXTRAS: RefCell<HashMap<String, Pending>> = RefCell::new(HashMap::new());
}

fn handlers(on_action: impl Fn(&Rc<AppState>, &Router, Step) + 'static) -> Handlers<Step> {
    let on_action: Rc<dyn Fn(&Rc<AppState>, &Router, Step)> = Rc::new(on_action);
    let (b, x) = (on_action.clone(), on_action.clone());
    Handlers { on_action, on_back: Rc::new(move |app, router| b(app, router, Step::Back)), on_exit: Rc::new(move |app, router| x(app, router, Step::Back)), on_input: None }
}

/// A page nothing can be chosen on, and that B cannot leave (a short job that cannot be stopped).
fn still_handlers() -> Handlers<Step> {
    Handlers { on_action: Rc::new(|_, _, _| {}), on_back: Rc::new(|_, _| {}), on_exit: Rc::new(|_, _| {}), on_input: None }
}

/// Takes the port's one job slot (the same one an install, an update and a restore hold). When another job has it, the
/// page says so and Back goes where it should; a ports folder that is missing says so itself.
fn claim(app: &Rc<AppState>, router: &Router, port: &Port, card: Card, back: &Back) -> Option<String> {
    let key = port.key().to_string();
    if app.install_runtime.installing.borrow().contains(&key) || is_port_running(app, &key) {
        let back = back.clone();
        show_page(app, router, pages::busy_page(port.display_name(), card), handlers(move |app, router, _| back(app, router)));
        return None;
    }
    if super::ports_folder_screen::refuse_when_blocked(app) {
        return None;
    }
    app.install_runtime.installing.borrow_mut().insert(key.clone());
    Some(key)
}

fn free(app: &Rc<AppState>, key: &str) {
    app.install_runtime.installing.borrow_mut().remove(key);
}

// -- Select version ---------------------------------------------------------------------------------------------------

/// "Getting the versions", then the list. `card` is the Port page card the pages are named after; `back` runs on Back.
pub(crate) fn versions(app: &Rc<AppState>, router: &Router, port: Port, card: Card, back: Back) {
    let Some(key) = claim(app, router, &port, card, &back) else { return };
    let id = NEXT_ID.with(|n| n.replace(n.get() + 1));
    // Cancel (or B) gives up and frees the port at once; the answer, when it comes, carries a request number that no longer
    // matches anything and is ignored.
    let giving_up = key.clone();
    let on_back = back.clone();
    let wait = show_page_handle(
        app,
        router,
        pages::versions_wait_page(port.display_name()),
        handlers(move |app, router, _| {
            if VERSIONS.with(|v| v.borrow_mut().remove(&giving_up)).is_some() {
                free(app, &giving_up);
            }
            on_back(app, router);
        }),
    );
    VERSIONS.with(|v| v.borrow_mut().insert(key.clone(), Pending { id, port: port.clone(), card, back, wait }));
    let tagged = pages::tag_request(&key, id);
    let (github_token, gitlab_token) = app.api_tokens();
    let events = app.events.clone();
    let repo = port.repo.clone().unwrap_or_default();
    let forge = Forge::of(port.source_type);
    std::thread::spawn(move || {
        let result = match forge {
            Some(forge) => forge.list_releases(&repo, forge.token(github_token.as_deref(), gitlab_token.as_deref()), VERSION_CHOICES),
            None => Ok(Vec::new()),
        };
        let event = match result {
            Ok(releases) if !releases.is_empty() => AppEvent::VersionsFetched { key: tagged, releases },
            Ok(_) => AppEvent::VersionsFetchError { key: tagged, message: "This source has no version history.".to_string() },
            Err(e) => AppEvent::VersionsFetchError { key: tagged, message: e.message().to_string() },
        };
        lock(&events).push(event);
    });
}

/// The release list (or why not) is in. `tagged` is the port's key with the request number (`pages::tag_request`). An answer
/// to a request that was given up, or replaced by a newer one, is ignored; so is one that comes after the waiting page was
/// left for something else.
pub(crate) fn versions_fetched(app: &Rc<AppState>, router: &Router, tagged: &str, result: Result<Vec<Value>, String>) {
    let (key, id) = pages::split_request(tagged);
    let current = VERSIONS.with(|v| v.borrow().get(key).is_some_and(|p| Some(p.id) == id));
    if !current {
        return;
    }
    free(app, key);
    let Some(pending) = VERSIONS.with(|v| v.borrow_mut().remove(key)) else { return };
    if !pending.wait.as_ref().is_some_and(PageHandle::is_open) {
        return;
    }
    match result {
        Ok(releases) if !releases.is_empty() => show_versions(app, router, pending, releases),
        Ok(_) => show_versions_failed(app, router, pending, "This source has no version history."),
        Err(raw) => show_versions_failed(app, router, pending, &raw),
    }
}

fn show_versions(app: &Rc<AppState>, router: &Router, pending: Pending, releases: Vec<Value>) {
    let installed_tag = app.state.borrow().get(pending.port.key()).and_then(|i| i.installed_tag.clone());
    let list = pages::versions_of(&releases, installed_tag.as_deref());
    let page = pages::versions_page(pending.port.display_name(), &list);
    let Pending { port, back, .. } = pending;
    show_page(
        app,
        router,
        page,
        handlers(move |app, router, step| {
            let Step::Pick(i) = step else { return back(app, router) };
            let Some(release) = releases.get(i).cloned() else { return };
            // Installing a version of a port that is not installed asks the same game question first, as before.
            let go = {
                let port = port.clone();
                move |app: &Rc<AppState>, router: &Router| super::install_launch::start_install(app, router, port.clone(), None, Some(release.clone()))
            };
            if app.is_installed(&port) {
                go(app, router);
            } else {
                super::same_game_screen::confirm_then(app, router, &port, go);
            }
        }),
    );
}

fn show_versions_failed(app: &Rc<AppState>, router: &Router, pending: Pending, raw: &str) {
    eprintln!("versions: could not get them for {}: {raw}", pending.port.key());
    let page = pages::versions_failed_page(pending.port.display_name(), raw);
    let Pending { port, card, back, .. } = pending;
    show_page(
        app,
        router,
        page,
        handlers(move |app, router, step| match step {
            Step::TryAgain => versions(app, router, port.clone(), card, back.clone()),
            _ => back(app, router),
        }),
    );
}

// -- Install extras ---------------------------------------------------------------------------------------------------

/// "Install the extra files?", then "Getting the extras", then the result. No is the answer the cursor starts on.
pub(crate) fn extras(app: &Rc<AppState>, router: &Router, port: Port, card: Card, back: Back) {
    let page = pages::extras_ask_page(port.display_name());
    show_page(
        app,
        router,
        page,
        handlers(move |app, router, step| match step {
            Step::Yes => run_extras(app, router, port.clone(), card, back.clone()),
            _ => back(app, router),
        }),
    );
}

fn run_extras(app: &Rc<AppState>, router: &Router, port: Port, card: Card, back: Back) {
    let Some(key) = claim(app, router, &port, card, &back) else { return };
    EXTRAS.with(|e| e.borrow_mut().insert(key.clone(), Pending { id: 0, port: port.clone(), card, back, wait: None }));
    show_page(app, router, pages::extras_wait_page(port.display_name()), still_handlers());
    let library_dir = app.paths.library_dir();
    let saves_backup_dir = app.paths.saves_backup_dir.clone();
    let events = app.events.clone();
    std::thread::spawn(move || {
        // The install says when it starts putting files in place; a failure after that may have changed some of them.
        let partway = Cell::new(false);
        let mut on_progress = |line: &str| {
            if line == crate::core::installer::EXTRA_PLACING {
                partway.set(true);
            }
        };
        let result = crate::core::jobs::run_extra_install(&port, &library_dir, &saves_backup_dir, &mut on_progress);
        let result = result.map_err(|e| if partway.get() { format!("{}{e}", pages::PARTWAY) } else { e });
        lock(&events).push(AppEvent::ExtraInstallDone { key, result });
    });
}

/// The extras install ended: the page says how, and Back leads where the caller said.
pub(crate) fn extras_done(app: &Rc<AppState>, router: &Router, key: &str, result: Result<(), String>) {
    free(app, key);
    let pending = EXTRAS.with(|e| e.borrow_mut().remove(key));
    if let Err(raw) = &result {
        eprintln!("extras: the install for {key} failed: {}", raw.trim_start_matches(pages::PARTWAY));
    }
    let close: Back = Rc::new(|app, router| close_current_dialog(app, router));
    let (name, back) = match pending {
        Some(p) => (p.port.display_name().to_string(), p.back),
        None => match app.find_port(key) {
            Some(port) => (port.display_name().to_string(), close),
            None => return,
        },
    };
    let mut page = pages::extras_done_page(&name, result.as_ref().map(|_| ()).map_err(String::as_str));
    // What the extras set aside to keep the user's own files (`fork::update_merge`), in one plain line.
    let port_dir = app.find_port(key).and_then(|p| crate::core::installer::port_dir(&app.paths.library_dir(), &p.folder).ok());
    if let Some(line) = port_dir.and_then(|d| crate::fork::update_merge::take(&d)).and_then(|r| r.line("extras", &crate::fork::port_page::paths::human_path_here)) {
        page.fact(crate::fork::page::Mark::Done, line);
    }
    show_page(app, router, page, handlers(move |app, router, _| back(app, router)));
}

// -- Reset play time --------------------------------------------------------------------------------------------------

/// "Reset the play time for <name>?": No is the answer the cursor starts on. Yes resets it and says so.
pub(crate) fn reset_play_time(app: &Rc<AppState>, router: &Router, port: Port, back: Back) {
    let seconds = app.state.borrow().get(port.key()).map_or(0, |i| i.playtime_seconds);
    let played = play_time_words(seconds).unwrap_or_else(|| "none".to_string());
    let page = pages::reset_ask_page(port.display_name(), &played);
    show_page(
        app,
        router,
        page,
        handlers(move |app, router, step| match step {
            Step::Yes => {
                app.state.borrow_mut().reset_playtime(port.key());
                app.refresh_current_view();
                eprintln!("play time: reset for {}", port.display_name());
                let back = back.clone();
                show_page(app, router, pages::reset_done_page(port.display_name()), handlers(move |app, router, _| back(app, router)));
            }
            _ => back(app, router),
        }),
    );
}
