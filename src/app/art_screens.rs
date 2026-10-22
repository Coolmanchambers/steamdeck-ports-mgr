//! The "Artwork sources" screen (Settings menu): paste the SteamGridDB key (and, optionally, a
//! TheGamesDB key and a ScreenScraper login), test each, see each source's status and last problem,
//! fetch artwork for the installed ports, choose between look-alike matches, and read the credits.
//!
//! Every screen is a list built from the picker dialog the app already has, so it works with the
//! controller, keyboard and mouse in a window or fullscreen. A key is typed or pasted in the search
//! dialog used as a one-line text box (with the Steam on-screen keyboard on the Deck: Steam + X, or
//! the keyboard button the dialog brings up). What the rows say and do is decided in
//! `crate::fork::art::screen` (tested without any UI); this file draws it and runs the network calls
//! on a worker thread. The text is English only; it is not part of upstream's translation tables.
//!
//! Keys are stored by `crate::fork::secrets` in `secrets.json` (mode 0600 where the drive allows),
//! shown masked, and never written to a log or to `state.json`.

use super::dialogs::{
    build_picker_dialog, build_text_entry_dialog, close_current_dialog, finish_dialog_open, open_message_dialog, open_settings_dialog, set_picker_items, DialogSlot,
};
use super::state::AppState;
use crate::fork::art::net::Net;
use crate::fork::art::resolve::{ArtDirs, Mode, Session};
use crate::fork::art::screen::{
    check_message, check_source, fetch_all, fetch_sizing_labels, job_labels, summary_text, test_progress, wrap, ArtScreen, FetchSummary, Line, PendingChoice, PortJob, TITLE,
};
use crate::fork::art::source::SourceId;
use crate::fork::rom_screens::job::{Background, Ctl};
use crate::fork::secrets::{BuildCreds, Field, Secrets, FILE_NAME as SECRETS_FILE};
use crate::ui::gamepad_router::GamepadRouter;
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

type Router = Rc<RefCell<GamepadRouter>>;
type Screen = Rc<RefCell<ArtScreen>>;
type Then = Rc<dyn Fn(&Rc<AppState>, &Router)>;

/// Width, in characters, of the rows a message is broken into.
const WRAP: usize = 50;

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn job_is_running() -> bool {
    JOB_RUNNING.with(Cell::get)
}

fn load_screen(app: &AppState) -> ArtScreen {
    ArtScreen::load(&app.paths.config_dir, &app.paths.cache_dir, BuildCreds::from_build(), unix_now())
}

// -- The menu --------------------------------------------------------------------------------------

/// Settings > Artwork sources...
pub(crate) fn open_art_sources(app: &Rc<AppState>, router: &Router) {
    // No "dialog already open" guard: the Back rows of the deeper lists come back here while their own
    // list is still up, and showing the menu replaces whatever is open.
    show_menu(app, router, Rc::new(RefCell::new(load_screen(app))));
}

fn show_menu(app: &Rc<AppState>, router: &Router, screen: Screen) {
    let rows = screen.borrow().rows();
    let labels: Vec<String> = rows.iter().map(|(_, l)| l.clone()).collect();
    let Some(dialog) = build_picker_dialog(app, router, TITLE, &labels) else { return };
    {
        let (app, router) = (app.clone(), router.clone());
        dialog.on_item_selected(move |index| {
            let Some((line, _)) = rows.get(index as usize).cloned() else { return };
            // A task started earlier may still be running (its list was closed with B). It writes the
            // same files as the rows below, so they wait for it.
            if job_is_running() && matches!(line, Line::Edit(_) | Line::Test(_) | Line::Fetch) {
                return show_text(&app, &router, TITLE, "An artwork task is still running in the background. Wait for it to finish, then try again.", screen.clone());
            }
            match line {
                Line::Info => {}
                Line::Edit(SourceId::ScreenScraper) => edit_screenscraper(&app, &router, screen.clone(), String::new()),
                Line::Edit(src) => edit_key(&app, &router, screen.clone(), src, String::new()),
                Line::Test(src) => start_test(&app, &router, screen.clone(), src),
                Line::Fetch => start_fetch(&app, &router, screen.clone()),
                Line::Pending => show_pending(&app, &router, screen.clone()),
                Line::Credits => {
                    let text = screen.borrow().credits_text();
                    show_text(&app, &router, "Credits", &text, screen.clone());
                }
                Line::Back => {
                    close_current_dialog(&app, &router);
                    open_settings_dialog(&app, &router);
                }
            }
        });
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
}

/// A message shown as rows (long text wraps), with a Back row that runs `then`.
fn show_text_then(app: &Rc<AppState>, router: &Router, title: &str, text: &str, then: Then) {
    let mut labels = wrap(text, WRAP);
    labels.push("Back".to_string());
    let back = labels.len() - 1;
    let Some(dialog) = build_picker_dialog(app, router, title, &labels) else { return };
    dialog.set_selected_index(back as i32);
    app.dialog_nav.picker_index.set(back as i32);
    {
        let (app, router) = (app.clone(), router.clone());
        dialog.on_item_selected(move |index| {
            if index as usize == back {
                then(&app, &router);
            }
        });
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
}

/// [`show_text_then`] whose Back row returns to the menu.
fn show_text(app: &Rc<AppState>, router: &Router, title: &str, text: &str, screen: Screen) {
    show_text_then(app, router, title, text, Rc::new(move |app, router| show_menu(app, router, screen.clone())));
}

// -- Look-alike matches -----------------------------------------------------------------------------

/// One row per waiting candidate ("Use "Doom II" (42) for Doom"); choosing one confirms it. Under them,
/// how to choose by hand in `ports.local.json`.
fn show_pending(app: &Rc<AppState>, router: &Router, screen: Screen) {
    let choices = ArtScreen::pending_choices(&app.paths.cache_dir);
    if choices.is_empty() {
        let fresh = load_screen(app);
        *screen.borrow_mut() = fresh;
        return show_text(app, router, "Look-alike matches", "No look-alike matches are waiting.", screen);
    }
    let mut labels: Vec<String> = choices.iter().map(PendingChoice::label).collect();
    labels.push(" ".to_string());
    labels.extend(wrap(&choices[0].hint(), WRAP));
    labels.push("Back".to_string());
    let back = labels.len() - 1;
    let Some(dialog) = build_picker_dialog(app, router, "Look-alike matches", &labels) else { return };
    {
        let (app, router) = (app.clone(), router.clone());
        dialog.on_item_selected(move |index| {
            let index = index as usize;
            if index == back {
                return show_menu(&app, &router, screen.clone());
            }
            if let Some(choice) = choices.get(index) {
                let text = match choice.confirm(&app.paths.cache_dir) {
                    Ok(()) => format!("\"{}\" will be used for {}. Fetch artwork to get its pictures.", choice.name, choice.port_name),
                    Err(e) => format!("Could not save the choice: {e}"),
                };
                let fresh = load_screen(&app);
                *screen.borrow_mut() = fresh;
                show_text(&app, &router, "Look-alike matches", &text, screen.clone());
            }
        });
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
}

// -- Entering a key --------------------------------------------------------------------------------

enum Entry {
    Save(String),
    Remove,
    Cancel,
}

/// Opens the text box (showing `initial`); `done` runs once with what the user chose (Save, Remove, or
/// Cancel, which is also what closing the box with B reports).
fn ask(app: &Rc<AppState>, router: &Router, title: &str, placeholder: &str, initial: &str, password: bool, done: impl Fn(&Rc<AppState>, &Router, Entry) + 'static) {
    let labels = vec!["Save".to_string(), "Remove what is saved".to_string(), "Cancel".to_string()];
    let Some(dialog) = build_text_entry_dialog(app, router, title, placeholder, initial, &labels, password) else { return };
    let text = Rc::new(RefCell::new(initial.to_string()));
    let done = Rc::new(done);
    {
        let text = text.clone();
        dialog.on_search_changed(move |query| *text.borrow_mut() = query.to_string());
    }
    {
        let (app, router, done) = (app.clone(), router.clone(), done.clone());
        dialog.on_item_selected(move |index| {
            let entry = match index {
                0 => Entry::Save(text.borrow().clone()),
                1 => Entry::Remove,
                _ => Entry::Cancel,
            };
            done(&app, &router, entry);
        });
    }
    {
        let (app, router, done) = (app.clone(), router.clone(), done);
        dialog.on_close_requested(move || done(&app, &router, Entry::Cancel));
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::SearchList(dialog));
}

fn field_and_texts(src: SourceId) -> (Field, &'static str, &'static str) {
    match src {
        SourceId::SteamGridDb => (Field::SteamGridDbKey, "SteamGridDB API key", "Paste your SteamGridDB API key, then choose Save"),
        _ => (Field::TheGamesDbKey, "TheGamesDB API key", "Paste your TheGamesDB API key, then choose Save"),
    }
}

fn after_edit(app: &Rc<AppState>, router: &Router, screen: &Screen) {
    screen.borrow().save_states();
    show_menu(app, router, screen.clone());
}

/// A value the store refused: say why in a message, then bring the box back with the text as it was, so a
/// long paste is not lost over a stray character.
fn refused(app: &Rc<AppState>, router: &Router, title: &str, why: &str, again: Then) {
    show_text_then(app, router, title, &format!("Not saved: {why}\n\nYour text is still in the box; fix it and choose Save again."), again);
}

fn edit_key(app: &Rc<AppState>, router: &Router, screen: Screen, src: SourceId, initial: String) {
    let (field, title, placeholder) = field_and_texts(src);
    ask(app, router, title, placeholder, &initial, false, move |app, router, entry| match entry {
        Entry::Save(text) => {
            let result = screen.borrow_mut().set(field, Some(&text));
            match result {
                Ok(()) => after_edit(app, router, &screen),
                Err(why) => {
                    let screen = screen.clone();
                    refused(app, router, title, &why, Rc::new(move |app, router| edit_key(app, router, screen.clone(), src, text.clone())));
                }
            }
        }
        Entry::Remove => {
            let _ = screen.borrow_mut().set(field, None);
            after_edit(app, router, &screen);
        }
        Entry::Cancel => show_menu(app, router, screen.clone()),
    });
}

/// ScreenScraper takes a user name, then a password (shown as dots).
fn edit_screenscraper(app: &Rc<AppState>, router: &Router, screen: Screen, initial: String) {
    ask(app, router, "ScreenScraper login", "Your ScreenScraper user name, then choose Save", &initial, false, move |app, router, entry| match entry {
        Entry::Save(user) => {
            let result = screen.borrow_mut().set(Field::ScreenScraperUser, Some(&user));
            match result {
                Ok(()) => edit_screenscraper_password(app, router, screen.clone(), String::new()),
                Err(why) => {
                    let screen = screen.clone();
                    refused(app, router, "ScreenScraper login", &why, Rc::new(move |app, router| edit_screenscraper(app, router, screen.clone(), user.clone())));
                }
            }
        }
        Entry::Remove => {
            let _ = screen.borrow_mut().set(Field::ScreenScraperUser, None);
            let _ = screen.borrow_mut().set(Field::ScreenScraperPassword, None);
            after_edit(app, router, &screen);
        }
        Entry::Cancel => show_menu(app, router, screen.clone()),
    });
}

fn edit_screenscraper_password(app: &Rc<AppState>, router: &Router, screen: Screen, initial: String) {
    ask(app, router, "ScreenScraper password", "Your ScreenScraper password, then choose Save", &initial, true, move |app, router, entry| match entry {
        Entry::Save(password) => {
            let result = screen.borrow_mut().set(Field::ScreenScraperPassword, Some(&password));
            match result {
                Ok(()) => after_edit(app, router, &screen),
                Err(why) => {
                    let screen = screen.clone();
                    refused(app, router, "ScreenScraper password", &why, Rc::new(move |app, router| edit_screenscraper_password(app, router, screen.clone(), password.clone())));
                }
            }
        }
        Entry::Remove => {
            let _ = screen.borrow_mut().set(Field::ScreenScraperPassword, None);
            after_edit(app, router, &screen);
        }
        Entry::Cancel => show_menu(app, router, screen.clone()),
    });
}

// -- Background jobs -------------------------------------------------------------------------------

thread_local! {
    static JOB_TIMER: slint::Timer = slint::Timer::default();
    static JOB_RUNNING: Cell<bool> = const { Cell::new(false) };
}

fn stop_job_timer() {
    JOB_TIMER.with(|t| t.stop());
    JOB_RUNNING.with(|r| r.set(false));
}

/// Runs `work` on a thread behind a list that shows its progress (and, when `stoppable`, a Stop row).
/// Closing the list with B stops the work too, and then `done` is not called. `done` gets the result on
/// the UI thread. The list is sized from `sizing` (the widest text it will show) and starts out showing
/// `first`. A job is counted as running until its thread has really finished, whatever the user closed.
fn run_job<T: Send + 'static>(
    app: &Rc<AppState>,
    router: &Router,
    sizing: Vec<String>,
    first: String,
    stoppable: bool,
    work: impl FnOnce(&Ctl) -> T + Send + 'static,
    done: impl FnOnce(&Rc<AppState>, &Router, Result<T, String>) + 'static,
) {
    if job_is_running() {
        return open_message_dialog(app, router, TITLE, "Another artwork task is still running. Wait for it to finish, then try again.\n");
    }
    // The list first: if it cannot open, nothing has been started.
    let Some(dialog) = build_picker_dialog(app, router, TITLE, &sizing) else { return };
    set_picker_items(&dialog, &job_labels(&first, stoppable, false));
    let job = Rc::new(Background::spawn(work));
    JOB_RUNNING.with(|r| r.set(true));
    let cancelling = Rc::new(Cell::new(false));
    if stoppable {
        let (job, cancelling) = (job.clone(), cancelling.clone());
        let weak = dialog.as_weak();
        dialog.on_item_selected(move |index| {
            if index == 2 {
                job.cancel();
                cancelling.set(true);
                if let Some(d) = weak.upgrade() {
                    set_picker_items(&d, &job_labels(&job.progress(), true, true));
                }
            }
        });
    }
    let picker = dialog.as_weak();
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));

    let done: RefCell<Option<Box<dyn FnOnce(&Rc<AppState>, &Router, Result<T, String>)>>> = RefCell::new(Some(Box::new(done)));
    let (app, router) = (app.clone(), router.clone());
    let last = RefCell::new(String::new());
    let abandoned = Cell::new(false);
    JOB_TIMER.with(|timer| {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(250), move || {
            // The user closed the list with B: stop the work and drop its result. The timer runs until
            // the worker has really stopped, so a second job cannot overlap it.
            if !abandoned.get() && !picker.upgrade().is_some_and(|d| d.window().is_visible()) {
                job.cancel();
                abandoned.set(true);
            }
            if let Some(result) = job.take() {
                stop_job_timer();
                if abandoned.get() {
                    return;
                }
                close_current_dialog(&app, &router);
                if let Some(done) = done.borrow_mut().take() {
                    done(&app, &router, result);
                }
                return;
            }
            if abandoned.get() {
                return;
            }
            let text = job.progress();
            if text.is_empty() || text == *last.borrow() {
                return;
            }
            *last.borrow_mut() = text.clone();
            if let Some(d) = picker.upgrade() {
                set_picker_items(&d, &job_labels(&text, stoppable, cancelling.get()));
            }
        });
    });
}

// -- Test key --------------------------------------------------------------------------------------

fn start_test(app: &Rc<AppState>, router: &Router, screen: Screen, src: SourceId) {
    let secrets_path = app.paths.config_dir.join(SECRETS_FILE);
    let build = BuildCreds::from_build();
    let progress = test_progress(src);
    run_job(
        app,
        router,
        job_labels(&progress, false, false),
        progress,
        false,
        move |_| {
            let secrets = Secrets::load(&secrets_path);
            check_source(src, &secrets, &build, &Net::new(), &std::thread::sleep)
        },
        move |app, router, result| {
            let text = match result {
                Ok(check) => {
                    let secrets = screen.borrow().records_secrets_in();
                    screen.borrow_mut().states.record_check(src, &check, unix_now(), &secrets);
                    screen.borrow().save_states();
                    check_message(src, &check, &secrets)
                }
                Err(why) => format!("The test could not run: {why}"),
            };
            show_text(app, router, "Test result", &text, screen);
        },
    );
}

// -- Fetch artwork ---------------------------------------------------------------------------------

fn start_fetch(app: &Rc<AppState>, router: &Router, screen: Screen) {
    if !screen.borrow().any_ready() {
        return show_text(app, router, "Fetch artwork", "Set a SteamGridDB key first (choose its row, paste the key, Save).", screen);
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
        return show_text(app, router, "Fetch artwork", "No ports are installed yet, so there is nothing to fetch art for.", screen);
    }
    let (config_dir, cache_dir) = (app.paths.config_dir.clone(), app.paths.cache_dir.clone());
    let sizing = fetch_sizing_labels(&ports);
    let first = format!("Fetching artwork for {} ports...", ports.len());
    run_job(
        app,
        router,
        sizing,
        first,
        true,
        move |ctl| {
            let session = Session::open(ArtDirs { config_dir: &config_dir, cache_dir: &cache_dir }, Mode::Online);
            fetch_all(&session, &config_dir, &ports, ctl)
        },
        move |app, router, result: Result<FetchSummary, String>| {
            let text = match result {
                Ok(s) => summary_text(&s),
                Err(why) => format!("Fetching stopped: {why}"),
            };
            // The statuses (last problem, back-off) and the waiting matches may have changed.
            let fresh = load_screen(app);
            *screen.borrow_mut() = fresh;
            show_text(app, router, "Fetch artwork", &text, screen);
        },
    );
}
