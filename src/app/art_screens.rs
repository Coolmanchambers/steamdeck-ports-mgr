//! The "Artwork sources" screen (Settings menu): paste the SteamGridDB key (and, optionally, a
//! TheGamesDB key and a ScreenScraper login), test each, see each source's status and last problem,
//! fetch artwork for the installed ports, and read the credits.
//!
//! Every screen is a list built from the picker dialog the app already has, so it works with the
//! controller, keyboard and mouse in a window or fullscreen. A key is typed or pasted in the search
//! dialog used as a one-line text box (with the Steam on-screen keyboard on the Deck: Steam + X, or
//! the keyboard button the dialog brings up). What the rows say and do is decided in
//! `crate::fork::art::screen` (tested without any UI); this file draws it and runs the network calls
//! on a worker thread. The text is English only; it is not part of upstream's translation tables.
//!
//! Keys are stored by `crate::fork::secrets` in `secrets.json` (mode 0600), shown masked, and never
//! written to a log or to `state.json`.

use super::dialogs::{
    build_picker_dialog, build_text_entry_dialog, close_current_dialog, finish_dialog_open, open_message_dialog, open_settings_dialog, set_picker_items,
    DialogSlot,
};
use super::state::AppState;
use crate::fork::art::net::Net;
use crate::fork::art::resolve::{ArtDirs, Mode, PortRef, Session};
use crate::fork::art::screen::{check_message, check_source, wrap, ArtScreen, Line, TITLE};
use crate::fork::art::source::SourceId;
use crate::fork::rom_screens::job::{Background, Ctl};
use crate::fork::secrets::{BuildCreds, Field, Secrets, FILE_NAME as SECRETS_FILE};
use crate::ui::gamepad_router::GamepadRouter;
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

type Router = Rc<RefCell<GamepadRouter>>;
type Screen = Rc<RefCell<ArtScreen>>;

/// Width, in characters, of the rows a message is broken into.
const WRAP: usize = 50;

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

// -- The menu --------------------------------------------------------------------------------------

/// Settings > Artwork sources...
pub(crate) fn open_art_sources(app: &Rc<AppState>, router: &Router) {
    // No "dialog already open" guard: the Back rows of the deeper lists come back here while their own
    // list is still up, and showing the menu replaces whatever is open.
    let screen = ArtScreen::load(&app.paths.config_dir, &app.paths.cache_dir, BuildCreds::from_build(), unix_now());
    show_menu(app, router, Rc::new(RefCell::new(screen)));
}

fn show_menu(app: &Rc<AppState>, router: &Router, screen: Screen) {
    let rows = screen.borrow().rows();
    let labels: Vec<String> = rows.iter().map(|(_, l)| l.clone()).collect();
    let Some(dialog) = build_picker_dialog(app, router, TITLE, &labels) else { return };
    {
        let (app, router) = (app.clone(), router.clone());
        dialog.on_item_selected(move |index| {
            let Some((line, _)) = rows.get(index as usize).cloned() else { return };
            match line {
                Line::Info => {}
                Line::Edit(SourceId::ScreenScraper) => edit_screenscraper(&app, &router, screen.clone(), None),
                Line::Edit(src) => edit_key(&app, &router, screen.clone(), src, None),
                Line::Test(src) => start_test(&app, &router, screen.clone(), src),
                Line::Fetch => start_fetch(&app, &router, screen.clone()),
                Line::Pending => {
                    let text = ArtScreen::pending_text(&app.paths.cache_dir);
                    show_text(&app, &router, "Look-alike matches", &text, screen.clone());
                }
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

/// A message shown as rows (long text wraps), with a Back row that returns to the menu.
fn show_text(app: &Rc<AppState>, router: &Router, title: &str, text: &str, screen: Screen) {
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
                show_menu(&app, &router, screen.clone());
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

/// Opens the text box; `done` runs once with what the user chose (Save, Remove or Cancel/B).
fn ask(app: &Rc<AppState>, router: &Router, title: &str, placeholder: &str, password: bool, done: impl Fn(&Rc<AppState>, &Router, Entry) + 'static) {
    let labels = vec!["Save".to_string(), "Remove what is saved".to_string(), "Cancel".to_string()];
    let Some(dialog) = build_text_entry_dialog(app, router, title, placeholder, &labels, password) else { return };
    let text = Rc::new(RefCell::new(String::new()));
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

fn edit_key(app: &Rc<AppState>, router: &Router, screen: Screen, src: SourceId, note: Option<String>) {
    let (field, title, default_placeholder) = field_and_texts(src);
    let placeholder = note.unwrap_or_else(|| default_placeholder.to_string());
    ask(app, router, title, &placeholder, false, move |app, router, entry| match entry {
        Entry::Save(text) => {
            let result = screen.borrow_mut().set(field, Some(&text));
            match result {
                Ok(()) => after_edit(app, router, &screen),
                Err(why) => edit_key(app, router, screen.clone(), src, Some(format!("Not saved: {why}"))),
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
fn edit_screenscraper(app: &Rc<AppState>, router: &Router, screen: Screen, note: Option<String>) {
    let placeholder = note.unwrap_or_else(|| "Your ScreenScraper user name, then choose Save".to_string());
    ask(app, router, "ScreenScraper login", &placeholder, false, move |app, router, entry| match entry {
        Entry::Save(user) => {
            let result = screen.borrow_mut().set(Field::ScreenScraperUser, Some(&user));
            match result {
                Ok(()) => edit_screenscraper_password(app, router, screen.clone(), None),
                Err(why) => edit_screenscraper(app, router, screen.clone(), Some(format!("Not saved: {why}"))),
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

fn edit_screenscraper_password(app: &Rc<AppState>, router: &Router, screen: Screen, note: Option<String>) {
    let placeholder = note.unwrap_or_else(|| "Your ScreenScraper password, then choose Save".to_string());
    ask(app, router, "ScreenScraper password", &placeholder, true, move |app, router, entry| match entry {
        Entry::Save(password) => {
            let result = screen.borrow_mut().set(Field::ScreenScraperPassword, Some(&password));
            match result {
                Ok(()) => after_edit(app, router, &screen),
                Err(why) => edit_screenscraper_password(app, router, screen.clone(), Some(format!("Not saved: {why}"))),
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

fn job_labels(progress: &str, cancelling: bool) -> Vec<String> {
    let (line1, line2) = progress.split_once('\n').unwrap_or((progress, ""));
    vec![
        if line1.is_empty() { "Working...".to_string() } else { line1.to_string() },
        if line2.is_empty() { " ".to_string() } else { line2.chars().take(60).collect() },
        if cancelling { "Stopping...".to_string() } else { "Stop (what is done is kept)".to_string() },
    ]
}

fn stop_job_timer() {
    JOB_TIMER.with(|t| t.stop());
    JOB_RUNNING.with(|r| r.set(false));
}

/// Runs `work` on a thread behind a list that shows its progress and a Stop row. Closing the list
/// (B) stops it too, and then `done` is not called. `done` gets the result on the UI thread.
fn run_job<T: Send + 'static>(
    app: &Rc<AppState>,
    router: &Router,
    title: &'static str,
    first_status: &str,
    work: impl FnOnce(&Ctl) -> T + Send + 'static,
    done: impl FnOnce(&Rc<AppState>, &Router, Result<T, String>) + 'static,
) {
    if JOB_RUNNING.with(Cell::get) {
        return open_message_dialog(app, router, TITLE, "Another artwork task is still running. Wait for it to finish, then try again.\n");
    }
    let job = Rc::new(Background::spawn(work));
    JOB_RUNNING.with(|r| r.set(true));
    let cancelling = Rc::new(Cell::new(false));
    let Some(dialog) = build_picker_dialog(app, router, title, &job_labels(&format!("{first_status}\nA long port name that keeps going and going and going"), false)) else {
        job.cancel();
        stop_job_timer();
        return;
    };
    {
        let (job, cancelling) = (job.clone(), cancelling.clone());
        let weak = dialog.as_weak();
        dialog.on_item_selected(move |index| {
            if index == 2 {
                job.cancel();
                cancelling.set(true);
                if let Some(d) = weak.upgrade() {
                    set_picker_items(&d, &job_labels(&job.progress(), true));
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
                set_picker_items(&d, &job_labels(&text, cancelling.get()));
            }
        });
    });
}

// -- Test key --------------------------------------------------------------------------------------

fn start_test(app: &Rc<AppState>, router: &Router, screen: Screen, src: SourceId) {
    let secrets_path = app.paths.config_dir.join(SECRETS_FILE);
    let build = BuildCreds::from_build();
    run_job(
        app,
        router,
        TITLE,
        &format!("Testing the {} key...", src.label()),
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

struct PortJob {
    key: String,
    folder: String,
    name: String,
    instructions: String,
    portrait: Option<PathBuf>,
    icon: Option<PathBuf>,
}

#[derive(Default)]
struct FetchSummary {
    looked_up: usize,
    total: usize,
    found: BTreeMap<&'static str, usize>,
    problems: Vec<String>,
    stopped: bool,
}

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
    let total = ports.len();
    run_job(
        app,
        router,
        TITLE,
        &format!("Fetching artwork for {total} ports..."),
        move |ctl| {
            let session = Session::open(ArtDirs { config_dir: &config_dir, cache_dir: &cache_dir }, Mode::Online);
            let mut summary = FetchSummary { total, ..FetchSummary::default() };
            for (i, port) in ports.iter().enumerate() {
                if ctl.cancelled() {
                    summary.stopped = true;
                    break;
                }
                ctl.set_progress(format!("Port {} of {total}\n{}", i + 1, port.name));
                let reference = PortRef { key: &port.key, folder: &port.folder, name: &port.name, instructions: Some(&port.instructions) };
                let art = session.resolve_port(&config_dir, reference, port.portrait.as_deref(), port.icon.as_deref(), None);
                summary.looked_up += 1;
                for s in &art.sources {
                    *summary.found.entry(s.slot.name()).or_default() += 1;
                }
                for (_, problem) in art.problems {
                    if !summary.problems.contains(&problem) && summary.problems.len() < 6 {
                        summary.problems.push(problem);
                    }
                }
            }
            session.save();
            summary
        },
        move |app, router, result| {
            let text = match result {
                Ok(s) => summary_text(&s),
                Err(why) => format!("Fetching stopped: {why}"),
            };
            // The statuses (last problem, back-off) may have changed.
            let fresh = ArtScreen::load(&app.paths.config_dir, &app.paths.cache_dir, BuildCreds::from_build(), unix_now());
            *screen.borrow_mut() = fresh;
            show_text(app, router, "Fetch artwork", &text, screen);
        },
    );
}

fn summary_text(s: &FetchSummary) -> String {
    let mut text = format!("Looked up {} of {} installed ports{}.", s.looked_up, s.total, if s.stopped { " (stopped early)" } else { "" });
    let found: Vec<String> = ["portrait", "wide", "hero", "logo", "icon"].iter().map(|slot| format!("{slot} {}", s.found.get(slot).copied().unwrap_or(0))).collect();
    text.push_str(&format!("\nArtwork ready to place, by slot: {}.", found.join(", ")));
    if !s.problems.is_empty() {
        text.push_str(&format!("\nProblems: {}", s.problems.join("; ")));
    }
    text.push_str("\n\nAdd to Steam places this artwork for the ports it adds. Art you chose yourself is never replaced.");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fetch_summary_counts_every_slot_and_lists_problems() {
        let mut s = FetchSummary { looked_up: 2, total: 3, stopped: true, ..FetchSummary::default() };
        s.found.insert("hero", 2);
        s.found.insert("logo", 1);
        s.problems.push("SteamGridDB: offline (could not connect)".into());
        let text = summary_text(&s);
        assert!(text.contains("Looked up 2 of 3 installed ports (stopped early)"), "{text}");
        assert!(text.contains("portrait 0, wide 0, hero 2, logo 1, icon 0"), "{text}");
        assert!(text.contains("Problems: SteamGridDB: offline"), "{text}");
    }

    #[test]
    fn job_rows_show_progress_and_a_stop_row() {
        assert_eq!(job_labels("Port 1 of 3\nShip of Harkinian", false), vec!["Port 1 of 3", "Ship of Harkinian", "Stop (what is done is kept)"]);
        assert_eq!(job_labels("", true), vec!["Working...", " ", "Stopping..."]);
    }
}
