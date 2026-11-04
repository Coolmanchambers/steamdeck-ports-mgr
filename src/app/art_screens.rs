//! Settings > Steam > Artwork sources, as pages (`crate::fork::art::pages` says what each shows): the sources as
//! cards, a page per source, a text box page for a key or login (Steam + X brings up the keyboard in Game Mode), a
//! yes/no before a saved key is removed, the key test with a waiting page and a result page, the credits, the look-alike
//! matches as a choice page, and fetching artwork as a progress page with a Stop card and a done page.
//!
//! B goes back one level everywhere: a source's page to the sources, the sources to the Steam page of Settings, the
//! text box to the source's page, a result to the page it came from. On the fetching page B is Stop.
//!
//! What is saved, where, and what is downloaded is as before (`crate::fork::art::screen`): keys go to `secrets.json`
//! through `crate::fork::secrets` (never to a log or `state.json`, never shown in full), a test makes one call, a fetch
//! fills the art cache and the id table, a look-alike choice is confirmed in the id table.

use super::dialogs::close_current_dialog;
use super::page_dialog::{show_page, show_page_handle, Handlers, PageHandle};
use super::state::AppState;
use crate::fork::art::net::Net;
use crate::fork::art::pages::{self, ArtAct, KeyField, LookAlikes};
use crate::fork::art::resolve::{ArtDirs, Mode, Session};
use crate::fork::art::screen::{check_log_text, check_source, fetch_all, ArtScreen, FetchSummary, PortJob};
use crate::fork::art::source::{KeyCheck, SourceId};
use crate::fork::page::Page;
use crate::fork::rom_screens::job::Background;
use crate::fork::secrets::{BuildCreds, Field, Secrets, FILE_NAME as SECRETS_FILE};
use crate::ui::gamepad_router::GamepadRouter;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

type Router = Rc<RefCell<GamepadRouter>>;
type Screen = Rc<RefCell<ArtScreen>>;
type Then = Rc<dyn Fn(&Rc<AppState>, &Router)>;

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

thread_local! {
    static JOB_TIMER: slint::Timer = slint::Timer::default();
    static JOB_RUNNING: Cell<bool> = const { Cell::new(false) };
}

fn job_is_running() -> bool {
    JOB_RUNNING.with(Cell::get)
}

fn load_screen(app: &AppState) -> ArtScreen {
    ArtScreen::load(&app.paths.config_dir, &app.paths.cache_dir, BuildCreds::from_build(), unix_now())
}

fn reload(app: &AppState, screen: &Screen) {
    let fresh = load_screen(app);
    *screen.borrow_mut() = fresh;
}

/// A page whose every action (and B) does one thing, the title bar's X closes everything.
fn show_simple(app: &Rc<AppState>, router: &Router, page: Page<ArtAct>, then: Then) {
    let on_back = then.clone();
    let handlers = Handlers { on_action: Rc::new(move |app, router, _| then(app, router)), on_back: Rc::new(move |app, router| on_back(app, router)), on_exit: Rc::new(close_current_dialog), on_input: None };
    show_page(app, router, page, handlers);
}

// -- The sources -------------------------------------------------------------------------------------------------------

/// Settings > Steam > Artwork sources.
pub(crate) fn open_art_sources(app: &Rc<AppState>, router: &Router) {
    show_menu(app, router, Rc::new(RefCell::new(load_screen(app))));
}

/// Up to the Steam page of Settings (closing the page first: `open_group` gives way to an open dialog).
fn to_steam_page(app: &Rc<AppState>, router: &Router) {
    close_current_dialog(app, router);
    super::settings_menu_screen::open_group(app, router, crate::fork::settings_menu::Group::Steam);
}

fn show_menu(app: &Rc<AppState>, router: &Router, screen: Screen) {
    if let Some(problem) = &screen.borrow().secrets.problem {
        eprintln!("artwork sources: {problem}");
    }
    let page = pages::menu_page(&screen.borrow());
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| match act {
            ArtAct::Source(src) => show_source(app, router, screen.clone(), src),
            ArtAct::Fetch => start_fetch(app, router, screen.clone()),
            ArtAct::Pending => show_pending(app, router, screen.clone()),
            ArtAct::Credits => {
                let back = screen.clone();
                show_simple(app, router, pages::credits_page(&screen.borrow().cache_line()), Rc::new(move |app, router| show_menu(app, router, back.clone())));
            }
            ArtAct::Back => to_steam_page(app, router),
            _ => {}
        }),
        on_back: Rc::new(to_steam_page),
        on_exit: Rc::new(close_current_dialog),
        on_input: None,
    };
    show_page(app, router, page, handlers);
}

/// "An artwork task is still running" (one started earlier writes the same files), then back to `then`.
fn busy(app: &Rc<AppState>, router: &Router, then: Then) {
    let page = pages::notice_page("busy", "Still working on artwork", "An artwork task is still running. Wait for it to finish, then try again.");
    show_simple(app, router, page, then);
}

fn show_source(app: &Rc<AppState>, router: &Router, screen: Screen, src: SourceId) {
    let page = pages::source_page(&screen.borrow(), src);
    let back_to = screen.clone();
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| {
            let again: Then = {
                let screen = screen.clone();
                Rc::new(move |app, router| show_source(app, router, screen.clone(), src))
            };
            match act {
                ArtAct::EnterKey | ArtAct::Test if job_is_running() => busy(app, router, again),
                ArtAct::EnterKey if src == SourceId::ScreenScraper => edit(app, router, screen.clone(), KeyField::User, String::new(), None),
                ArtAct::EnterKey => edit(app, router, screen.clone(), KeyField::Key(src), String::new(), None),
                ArtAct::Test => start_test(app, router, screen.clone(), src),
                ArtAct::Back => show_menu(app, router, screen.clone()),
                _ => {}
            }
        }),
        on_back: Rc::new(move |app, router| show_menu(app, router, back_to.clone())),
        on_exit: Rc::new(close_current_dialog),
        on_input: None,
    };
    show_page(app, router, page, handlers);
}

// -- Entering a key ----------------------------------------------------------------------------------------------------

fn field_of(field: KeyField) -> Field {
    match field {
        KeyField::Key(SourceId::SteamGridDb) => Field::SteamGridDbKey,
        KeyField::Key(_) => Field::TheGamesDbKey,
        KeyField::User => Field::ScreenScraperUser,
        KeyField::Password => Field::ScreenScraperPassword,
    }
}

/// What is saved now for the field: `Some(Some(last 4))` for a long value, `Some(None)` for a short one, `None` for nothing.
fn saved_of(screen: &ArtScreen, field: KeyField) -> Option<Option<String>> {
    let value = match field {
        KeyField::Key(SourceId::SteamGridDb) => screen.secrets.sgdb_key().map(str::to_string),
        KeyField::Key(_) => screen.secrets.tgdb_key().map(str::to_string),
        // A user name is not a secret, but it is shown on the source page already; here only that one is saved.
        KeyField::User => screen.secrets.screenscraper_user().map(|_| String::new()),
        KeyField::Password => screen.secrets.has_screenscraper_password().then(String::new),
    }?;
    let chars: Vec<char> = value.chars().collect();
    Some((chars.len() > 8).then(|| chars[chars.len() - 4..].iter().collect()))
}

/// The text box page. `text` is what the box holds (kept after a refusal or a No on the remove question).
fn edit(app: &Rc<AppState>, router: &Router, screen: Screen, field: KeyField, text: String, problem: Option<String>) {
    let src = field.source();
    let page = pages::key_page(field, saved_of(&screen.borrow(), field), &text, problem.as_deref(), crate::fork::steam_queue::game_mode_from_env());
    let typed = Rc::new(RefCell::new(text));
    let to_source: Then = {
        let screen = screen.clone();
        Rc::new(move |app, router| show_source(app, router, screen.clone(), src))
    };
    let on_input = {
        let typed = typed.clone();
        Rc::new(move |_: &Rc<AppState>, _: &Router, now: &str| {
            *typed.borrow_mut() = now.to_string();
            None
        })
    };
    let back = to_source.clone();
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| match act {
            ArtAct::Save => {
                let value = typed.borrow().clone();
                let result = screen.borrow_mut().set(field_of(field), Some(&value));
                match result {
                    Ok(()) => {
                        screen.borrow().save_states();
                        if field == KeyField::User {
                            edit(app, router, screen.clone(), KeyField::Password, String::new(), None);
                        } else {
                            to_source(app, router);
                        }
                    }
                    Err(why) => {
                        eprintln!("artwork sources: a {} was not saved: {why}", if field == KeyField::Password { "password" } else { "key or login" });
                        edit(app, router, screen.clone(), field, value, Some(pages::refused_reason(&why)));
                    }
                }
            }
            ArtAct::Remove => ask_remove(app, router, screen.clone(), field, typed.borrow().clone()),
            ArtAct::Cancel => to_source(app, router),
            _ => {}
        }),
        on_back: Rc::new(move |app, router| back(app, router)),
        on_exit: Rc::new(close_current_dialog),
        on_input: Some(on_input),
    };
    show_page(app, router, page, handlers);
}

/// "Remove the saved key?" No goes back to the text box as it was; Yes removes it (for the login, the password too).
fn ask_remove(app: &Rc<AppState>, router: &Router, screen: Screen, field: KeyField, text: String) {
    let page = pages::remove_page(field);
    let no: Then = {
        let (screen, text) = (screen.clone(), text.clone());
        Rc::new(move |app, router| edit(app, router, screen.clone(), field, text.clone(), None))
    };
    let back = no.clone();
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| match act {
            ArtAct::RemoveYes => {
                let _ = screen.borrow_mut().set(field_of(field), None);
                if field == KeyField::User {
                    let _ = screen.borrow_mut().set(Field::ScreenScraperPassword, None);
                }
                screen.borrow().save_states();
                show_source(app, router, screen.clone(), field.source());
            }
            ArtAct::RemoveNo => no(app, router),
            _ => {}
        }),
        on_back: Rc::new(move |app, router| back(app, router)),
        on_exit: Rc::new(close_current_dialog),
        on_input: None,
    };
    show_page(app, router, page, handlers);
}

// -- Background jobs ---------------------------------------------------------------------------------------------------

fn stop_job_timer() {
    JOB_TIMER.with(slint::Timer::stop);
    JOB_RUNNING.with(|r| r.set(false));
}

/// Runs `work` on a thread while `handle`'s page is up. `on_progress` redraws the page when the progress text changes;
/// `done` gets the result on the UI thread, with whether the page is still the one on screen (B left it: what was
/// learned is still kept, but no page is shown over whatever is up now). A job counts as running until its thread has
/// really finished, whatever the user did.
fn run_job<T: Send + 'static>(
    handle: Option<Rc<PageHandle<ArtAct>>>,
    job: Rc<Background<T>>,
    on_progress: impl Fn(&PageHandle<ArtAct>, &str) + 'static,
    done: impl FnOnce(Result<T, String>, bool) + 'static,
) {
    JOB_RUNNING.with(|r| r.set(true));
    let done: RefCell<Option<Box<dyn FnOnce(Result<T, String>, bool)>>> = RefCell::new(Some(Box::new(done)));
    let last = RefCell::new(String::new());
    JOB_TIMER.with(|timer| {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
            let open = handle.as_ref().is_some_and(|h| h.is_open());
            if let Some(result) = job.take() {
                stop_job_timer();
                if let Some(done) = done.borrow_mut().take() {
                    done(result, open);
                }
                return;
            }
            let now = job.progress();
            if open && now != *last.borrow() {
                *last.borrow_mut() = now.clone();
                if let Some(h) = &handle {
                    on_progress(h, &now);
                }
            }
        });
    });
}

// -- Test a key --------------------------------------------------------------------------------------------------------

fn start_test(app: &Rc<AppState>, router: &Router, screen: Screen, src: SourceId) {
    let back: Then = {
        let screen = screen.clone();
        Rc::new(move |app, router| show_source(app, router, screen.clone(), src))
    };
    let on_back = back.clone();
    let handlers = Handlers { on_action: Rc::new(|_, _, _| {}), on_back: Rc::new(move |app, router| on_back(app, router)), on_exit: Rc::new(close_current_dialog), on_input: None };
    let handle = show_page_handle(app, router, pages::testing_page(src), handlers).map(Rc::new);
    let secrets_path = app.paths.config_dir.join(SECRETS_FILE);
    let build = BuildCreds::from_build();
    let job = Rc::new(Background::spawn(move |_| {
        let secrets = Secrets::load(&secrets_path);
        check_source(src, &secrets, &build, &Net::new(), &std::thread::sleep)
    }));
    let (app, router) = (app.clone(), router.clone());
    // B on the waiting page leaves it, but the answer is still recorded, so the card says how the test went.
    run_job(handle, job, |_, _| {}, move |result, open| {
        let check = match result {
            Ok(check) => {
                let secrets = screen.borrow().records_secrets_in();
                screen.borrow_mut().states.record_check(src, &check, unix_now(), &secrets);
                screen.borrow().save_states();
                eprintln!("artwork sources: test {}", check_log_text(src, &check, &secrets));
                check
            }
            Err(why) => {
                eprintln!("artwork sources: the test could not run: {why}");
                KeyCheck::Unknown(why)
            }
        };
        if open {
            show_simple(&app, &router, pages::test_result_page(src, &check), back);
        }
    });
}

// -- Look-alike matches ------------------------------------------------------------------------------------------------

fn show_pending(app: &Rc<AppState>, router: &Router, screen: Screen) {
    let groups = pages::group_lookalikes(&ArtScreen::pending_choices(&app.paths.cache_dir));
    match groups.len() {
        0 => {
            reload(app, &screen);
            let page = pages::notice_page("look-alikes", "Nothing to choose", "No look-alike matches are waiting.");
            show_simple(app, router, page, Rc::new(move |app, router| show_menu(app, router, screen.clone())));
        }
        1 => {
            let back: Then = {
                let screen = screen.clone();
                Rc::new(move |app, router| show_menu(app, router, screen.clone()))
            };
            show_lookalike(app, router, screen, groups[0].clone(), back);
        }
        _ => show_lookalike_list(app, router, screen, groups),
    }
}

fn show_lookalike_list(app: &Rc<AppState>, router: &Router, screen: Screen, groups: Vec<LookAlikes>) {
    let page = pages::lookalike_list_page(&groups);
    let back: Then = {
        let screen = screen.clone();
        Rc::new(move |app, router| show_menu(app, router, screen.clone()))
    };
    let on_back = back.clone();
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| match act {
            ArtAct::Port(i) => {
                if let Some(group) = groups.get(i) {
                    let up: Then = {
                        let screen = screen.clone();
                        Rc::new(move |app, router| show_pending(app, router, screen.clone()))
                    };
                    show_lookalike(app, router, screen.clone(), group.clone(), up);
                }
            }
            ArtAct::Back => back(app, router),
            _ => {}
        }),
        on_back: Rc::new(move |app, router| on_back(app, router)),
        on_exit: Rc::new(close_current_dialog),
        on_input: None,
    };
    show_page(app, router, page, handlers);
}

/// One port's look-alikes. A choice is confirmed in the id table (as before); "None of these" and B change nothing.
fn show_lookalike(app: &Rc<AppState>, router: &Router, screen: Screen, group: LookAlikes, up: Then) {
    let page = pages::lookalike_page(&group);
    let on_back = up.clone();
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| match act {
            ArtAct::Pick(i) => {
                let Some(choice) = group.choices.get(i) else { return };
                let problem = choice.confirm(&app.paths.cache_dir).err().map(|e| {
                    eprintln!("artwork sources: a look-alike choice was not saved: {e}");
                    "the app's record of matches could not be written".to_string()
                });
                reload(app, &screen);
                show_simple(app, router, pages::chosen_page(&choice.port_name, &choice.name, problem.as_deref()), up.clone());
            }
            ArtAct::NoneOfThese => up(app, router),
            _ => {}
        }),
        on_back: Rc::new(move |app, router| on_back(app, router)),
        on_exit: Rc::new(close_current_dialog),
        on_input: None,
    };
    show_page(app, router, page, handlers);
}

// -- Fetch artwork -----------------------------------------------------------------------------------------------------

fn start_fetch(app: &Rc<AppState>, router: &Router, screen: Screen) {
    let to_menu: Then = {
        let screen = screen.clone();
        Rc::new(move |app, router| show_menu(app, router, screen.clone()))
    };
    if job_is_running() {
        return busy(app, router, to_menu);
    }
    if !screen.borrow().any_ready() {
        let page = pages::notice_page("no key", "Set up a source first", "Fetching artwork needs a SteamGridDB key.\n\nChoose SteamGridDB, then Enter a key.");
        return show_simple(app, router, page, to_menu);
    }
    let ports: Vec<PortJob> = app
        .catalog
        .borrow()
        .iter()
        .filter(|p| !p.is_android_only() && app.is_installed(p))
        .map(|p| {
            let cache = &app.paths.cache_dir;
            let portrait = crate::fork::art::catalog::cached_portrait_path(cache, &p.folder);
            let icon = crate::core::image_cache::cached_image_path(cache, &format!("{}_icon", p.folder)).ok().filter(|f| f.is_file());
            PortJob { key: p.key().to_string(), folder: p.folder.clone(), name: p.display_name().to_string(), instructions: p.instructions.clone(), portrait, icon }
        })
        .collect();
    if ports.is_empty() {
        let page = pages::notice_page("no ports", "No ports to fetch for", "No ports are installed yet, so there is nothing to fetch artwork for.");
        return show_simple(app, router, page, to_menu);
    }
    let total = ports.len();
    let (config_dir, cache_dir) = (app.paths.config_dir.clone(), app.paths.cache_dir.clone());
    let job = Rc::new(Background::spawn(move |ctl| {
        let session = Session::open(ArtDirs { config_dir: &config_dir, cache_dir: &cache_dir }, Mode::Online);
        fetch_all(&session, &config_dir, &ports, ctl)
    }));
    let stopping = Rc::new(Cell::new(false));
    let progress = Rc::new(RefCell::new(String::new()));
    // The page's handle, once it is up (the Stop handlers are made before it is shown).
    let slot: Rc<RefCell<Option<Rc<PageHandle<ArtAct>>>>> = Rc::new(RefCell::new(None));
    // Stop and B both stop: the page says "Stopping..." until the port it is on is done, then the done page shows.
    let stop: Rc<dyn Fn()> = {
        let (job, stopping, progress, slot) = (job.clone(), stopping.clone(), progress.clone(), slot.clone());
        Rc::new(move || {
            job.cancel();
            stopping.set(true);
            if let Some(h) = slot.borrow().as_ref() {
                h.update(pages::fetching_page(&progress.borrow(), total, true));
            }
        })
    };
    let on_stop = stop.clone();
    let handlers = Handlers {
        on_action: Rc::new(move |_, _, act| {
            if act == ArtAct::Stop {
                on_stop();
            }
        }),
        on_back: Rc::new(move |_, _| stop()),
        on_exit: Rc::new(close_current_dialog),
        on_input: None,
    };
    let handle = show_page_handle(app, router, pages::fetching_page("", total, false), handlers).map(Rc::new);
    *slot.borrow_mut() = handle.clone();
    let redraw = move |h: &PageHandle<ArtAct>, now: &str| {
        *progress.borrow_mut() = now.to_string();
        h.update(pages::fetching_page(now, total, stopping.get()));
    };
    let (app, router) = (app.clone(), router.clone());
    run_job(handle, job, redraw, move |result: Result<FetchSummary, String>, open| {
        let summary = result.unwrap_or_else(|why| {
            eprintln!("artwork sources: fetching stopped: {why}");
            FetchSummary { total, problems: vec!["the app ran into an error and stopped".to_string()], stopped: true, ..FetchSummary::default() }
        });
        eprintln!("artwork sources: fetched {} of {} ports, {} problems", summary.looked_up, summary.total, summary.problems.len());
        // The statuses (last problem, a pause) and the look-alikes waiting may have changed.
        reload(&app, &screen);
        if open {
            show_simple(&app, &router, pages::fetched_page(&summary), to_menu);
        }
    });
}
