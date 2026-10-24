//! The save finder: the "Saves not found" badge on a port, and the short wizard behind it (Info > "Find saves...",
//! or the badge itself on a touch screen or with a mouse). What each page says is `crate::fork::save_finder::view`;
//! what is found and stored is `crate::fork::save_finder` (both tested without any UI). This file draws the pages
//! through `page_dialog::show_page`, runs the search and the backup on worker threads and keeps the catalog in
//! step with the stored answer.
//!
//! Pages: where does the game keep its saves (Look for them / Pick the folder / No saves), the search (Cancel
//! works), the folders found as cards (or "I could not find them"), then "Saved" with Back up now and Undo. With an
//! answer already stored, Info's button opens a page to look again, pick another folder or go back to the
//! catalog's own answer. Nothing is moved or written into a save folder; "Back up now" copies the saves into a zip
//! in Saves Backup, the same Update Backup that Restore saves lists.

use super::dialogs::{close_current_dialog, open_info_dialog};
use super::folder_picker_screen::open_folder_picker;
use super::page_dialog::{show_page, show_page_handle, Handlers, PageHandle};
use super::state::AppState;
use crate::core::models::Port;
use crate::core::path_safety::safe_join;
use crate::fork::folder_picker::{PickerRequest, Purpose};
use crate::fork::page::Page;
use crate::fork::rom_screens::job::Background;
use crate::fork::save_finder::search::{self, Candidate};
use crate::fork::save_finder::view::{self, Act};
use crate::fork::save_finder::{self as finder, Override, SaveState};
use crate::fork::screens::short_path;
use crate::fork::settings::{ForkSettings, FILE_NAME};
use crate::ui::gamepad_router::GamepadRouter;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

type Router = Rc<RefCell<GamepadRouter>>;
type Back = Rc<dyn Fn(&Rc<AppState>, &Router)>;

fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"].iter().find_map(|v| std::env::var_os(v)).map(PathBuf::from).filter(|p| p.is_absolute())
}

fn settings_path(app: &AppState) -> PathBuf {
    app.paths.config_dir.join(FILE_NAME)
}

// -- The badge ------------------------------------------------------------------------------------------

/// How long a port's state is remembered; the badge is asked for on every redraw of the list.
const STATE_TTL: Duration = Duration::from_secs(8);

thread_local! {
    static STATES: RefCell<HashMap<String, (Instant, bool)>> = RefCell::new(HashMap::new());
    static OVERRIDES: RefCell<Option<(Instant, BTreeMap<String, Override>)>> = const { RefCell::new(None) };
    static FIND_TIMER: slint::Timer = slint::Timer::default();
    /// The backup has its own timer: leaving the search must not stop the one waiting for a backup's result.
    static BACKUP_TIMER: slint::Timer = slint::Timer::default();
}

/// Forget what was worked out, so the next redraw looks again (an answer was stored or cleared).
pub(crate) fn forget_states() {
    STATES.with(|s| s.borrow_mut().clear());
    OVERRIDES.with(|o| *o.borrow_mut() = None);
}

/// The stored answer for a port, from the settings file read at most once per `STATE_TTL` (a redraw asks for every
/// installed port).
fn stored_choice(app: &AppState, folder: &str) -> Option<Override> {
    OVERRIDES.with(|o| {
        let mut o = o.borrow_mut();
        if o.as_ref().is_none_or(|(at, _)| at.elapsed() >= STATE_TTL) {
            *o = Some((Instant::now(), finder::overrides(&ForkSettings::load(&settings_path(app)))));
        }
        o.as_ref().and_then(|(_, map)| map.get(folder).cloned())
    })
}

fn played(app: &AppState, port: &Port) -> bool {
    app.state.borrow().get(port.key()).is_some_and(|i| i.playtime_seconds > 0 || !i.last_played_at.is_empty())
}

fn state_for(app: &AppState, port: &Port) -> Option<(SaveState, Option<Override>)> {
    let game_dir = safe_join(&app.paths.library_dir(), &port.folder).ok().filter(|d| d.is_dir())?;
    let choice = stored_choice(app, &port.folder);
    Some((finder::state_of(port, &game_dir, choice.as_ref()), choice))
}

/// Whether the port shows the "Saves not found" badge: installed, and (see `SaveState::warns`) its saves are
/// unknown, or its known folder is empty or missing for a game that was played.
pub(crate) fn saves_warning(app: &AppState, port: &Port) -> bool {
    let key = port.key().to_string();
    if let Some(known) = STATES.with(|s| s.borrow().get(&key).copied()).filter(|(at, _)| at.elapsed() < STATE_TTL) {
        return known.1;
    }
    let warns = state_for(app, port).is_some_and(|(state, _)| state.warns(played(app, port)));
    STATES.with(|s| s.borrow_mut().insert(key, (Instant::now(), warns)));
    warns
}

/// For the Info dialog, in one look: the line about the port's saves (when there is one) and whether "Find saves..."
/// is there (the saves are not found, or an answer is stored).
pub(crate) fn info_parts(app: &AppState, port: &Port) -> (Option<String>, bool) {
    let Some((state, choice)) = state_for(app, port) else { return (None, false) };
    let played = played(app, port);
    (finder::info_line(state, choice.as_ref(), played), choice.is_some() || state.warns(played))
}

/// Puts the stored answers into a catalog just loaded or changed.
pub(crate) fn reapply(app: &AppState) {
    finder::apply_from_file(&mut app.catalog.borrow_mut(), &settings_path(app));
    forget_states();
}

/// After an answer is stored or cleared: the catalog entry is read again from the files (so a cleared answer
/// brings back the catalog's own) and the lists are redrawn. Returns the entry as it is now.
fn refresh_port(app: &Rc<AppState>, port: &Port) -> Port {
    forget_states();
    // When the catalog files cannot be read (a damaged ports.json), the answer is put into the entry in memory, so
    // the badge and "Back up now" are right without a restart.
    let fresh = finder::fresh_entry(&app.paths.config_dir, &port.folder).unwrap_or_else(|| {
        let mut one = [port.clone()];
        finder::apply_from_file(&mut one, &settings_path(app));
        let [entry] = one;
        entry
    });
    {
        let mut catalog = app.catalog.borrow_mut();
        if let Some(slot) = catalog.iter_mut().find(|p| p.folder == port.folder) {
            *slot = fresh.clone();
        }
    }
    app.refresh_current_view();
    fresh
}

// -- The wizard -----------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    First,
    Searching,
    Results,
    Saved,
    /// A message; its one button goes back to the first page.
    Notice,
    /// A message that ends the finder.
    Last,
    /// The backup runs; B and the X wait until it reports.
    Backing,
}

struct Session {
    port: RefCell<Port>,
    game_dir: PathBuf,
    /// What was stored when the finder opened (what Undo goes back to).
    previous: Option<Override>,
    stage: Cell<Stage>,
    found: RefCell<Vec<Candidate>>,
    job: RefCell<Option<Rc<Background<search::Found>>>>,
    back: Back,
}

fn name_of(s: &Session) -> String {
    s.port.borrow().display_name().to_string()
}

/// Info > "Find saves...", or the badge. `back` runs when the finder ends (Info reopens; the list needs nothing).
pub(crate) fn open(app: &Rc<AppState>, router: &Router, port: Port, back: Back) {
    if super::ports_folder_screen::refuse_when_blocked(app) {
        return;
    }
    let Some(game_dir) = safe_join(&app.paths.library_dir(), &port.folder).ok().filter(|d| d.is_dir()) else { return };
    let previous = finder::overrides(&ForkSettings::load(&settings_path(app))).remove(&port.folder);
    let s = Rc::new(Session { port: RefCell::new(port), game_dir, previous, stage: Cell::new(Stage::First), found: RefCell::new(Vec::new()), job: RefCell::new(None), back });
    show_first(app, router, &s);
}

/// The badge: the finder, and the list again when it ends.
pub(crate) fn open_from_badge(app: &Rc<AppState>, router: &Router, port: Port) {
    if super::dialogs::dialog_is_open(app) {
        return;
    }
    open(app, router, port, Rc::new(|app, router| close_current_dialog(app, router)));
}

/// Info's button: the finder, and Info again when it ends.
pub(crate) fn open_from_info(app: &Rc<AppState>, router: &Router, port: Port) {
    let key = port.key().to_string();
    let back: Back = Rc::new(move |app, router| {
        let port = app.catalog.borrow().iter().find(|p| p.key() == key).cloned();
        match port {
            Some(port) => open_info_dialog(app, router, &port),
            None => close_current_dialog(app, router),
        }
    });
    open(app, router, port, back);
}

fn handlers(s: &Rc<Session>) -> Handlers<Act> {
    let (s_act, s_back, s_exit) = (s.clone(), s.clone(), s.clone());
    Handlers {
        on_action: Rc::new(move |app, router, act| act_on(app, router, &s_act, act)),
        on_back: Rc::new(move |app, router| act_on(app, router, &s_back, Act::Back)),
        on_exit: Rc::new(move |app, router| leave(app, router, &s_exit)),
        on_input: None,
    }
}

fn show(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, stage: Stage, page: Page<Act>) {
    s.stage.set(stage);
    show_page(app, router, page, handlers(s));
}

fn leave(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    if s.stage.get() == Stage::Backing {
        return;
    }
    if let Some(job) = s.job.borrow_mut().take() {
        job.cancel();
    }
    FIND_TIMER.with(slint::Timer::stop);
    let back = s.back.clone();
    back(app, router);
}

fn show_first(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    let name = name_of(s);
    let stored = finder::overrides(&ForkSettings::load(&settings_path(app))).remove(&s.port.borrow().folder);
    let page = match stored {
        Some(choice) => view::current_page(&name, &describe(&choice, &s.game_dir)),
        None => view::ask_page(&name),
    };
    show(app, router, s, Stage::First, page);
}

fn describe(choice: &Override, game_dir: &std::path::Path) -> String {
    match choice {
        Override::NoSaves => "You said this game has no saves.".to_string(),
        Override::Folder(_) => match finder::folder_of(choice, game_dir) {
            Some(folder) => format!("Using the folder: {}", short_path(&folder, &home_dir().unwrap_or_default())),
            None => "Using a folder you chose.".to_string(),
        },
    }
}

fn notice(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, title: &str, paragraphs: Vec<String>, last: bool) {
    let label = if last { "Done" } else { "Back" };
    show(app, router, s, if last { Stage::Last } else { Stage::Notice }, view::message_page(&name_of(s), title, paragraphs, label));
}

fn act_on(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, act: Act) {
    if s.stage.get() == Stage::Backing {
        return;
    }
    match act {
        Act::Search => start_search(app, router, s),
        Act::Pick | Act::NoneOfThese => pick_folder(app, router, s),
        Act::NoSaves => store(app, router, s, Override::NoSaves),
        Act::Use(i) => {
            let chosen = s.found.borrow().get(i).map(|c| c.path.clone());
            if let Some(path) = chosen {
                store_folder(app, router, s, &path);
            }
        }
        Act::Cancel => match s.stage.get() {
            Stage::Searching => {
                if let Some(job) = s.job.borrow_mut().take() {
                    job.cancel();
                }
                FIND_TIMER.with(slint::Timer::stop);
                show_first(app, router, s);
            }
            _ => leave(app, router, s),
        },
        Act::Back => match s.stage.get() {
            Stage::First | Stage::Saved | Stage::Last | Stage::Backing => leave(app, router, s),
            Stage::Searching => act_on(app, router, s, Act::Cancel),
            Stage::Results | Stage::Notice => show_first(app, router, s),
        },
        Act::NotNow => leave(app, router, s),
        Act::BackUpNow => back_up(app, router, s),
        Act::Undo => undo(app, router, s),
        Act::UseCatalog => {
            let folder = s.port.borrow().folder.clone();
            let mut settings = ForkSettings::load(&settings_path(app));
            match finder::clear_override(&mut settings, &folder) {
                Ok(()) => {
                    let fresh = refresh_port(app, &s.port.borrow());
                    *s.port.borrow_mut() = fresh;
                    notice(app, router, s, "Done", vec!["The app uses what the catalog says again. Nothing was moved or changed.".into()], true);
                }
                Err(e) => notice(app, router, s, "Could not save that", vec![format!("{e}"), "Nothing was changed.".into()], false),
            }
        }
    }
}

// -- Picking a folder ---------------------------------------------------------------------------------------

fn pick_folder(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    let home = home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let start = s.previous.as_ref().and_then(|c| finder::folder_of(c, &s.game_dir)).filter(|p| p.is_dir()).unwrap_or_else(|| {
        let share = home.join(".local/share");
        if share.is_dir() { share } else { home.clone() }
    });
    let (chosen, cancelled) = (s.clone(), s.clone());
    open_folder_picker(
        app,
        router,
        PickerRequest { cursor_off_use_this: true, ..PickerRequest::new("Choose the folder with the game's saves", &start, Purpose::ScanFolder) },
        move |app, router, path| store_folder(app, router, &chosen, &path),
        move |app, router| show_first(app, router, &cancelled),
    );
}

fn store_folder(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, path: &std::path::Path) {
    let port = s.port.borrow().clone();
    match finder::check_choice(&port, path, &s.game_dir, &app.paths.saves_backup_dir, home_dir().as_deref()) {
        Ok(choice) => store(app, router, s, choice),
        Err(why) => notice(app, router, s, "That folder will not work", vec![why, "Nothing was changed.".into()], false),
    }
}

fn store(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, choice: Override) {
    let folder = s.port.borrow().folder.clone();
    let mut settings = ForkSettings::load(&settings_path(app));
    if let Err(e) = finder::set_override(&mut settings, &folder, &choice) {
        return notice(app, router, s, "Could not save that", vec![format!("{e}"), "Nothing was changed.".into()], false);
    }
    let fresh = refresh_port(app, &s.port.borrow());
    *s.port.borrow_mut() = fresh;
    let what = describe(&choice, &s.game_dir);
    let folder = finder::folder_of(&choice, &s.game_dir);
    let can_back_up = folder.as_deref().is_some_and(finder::has_files);
    let size = folder.as_deref().filter(|_| can_back_up).map(|f| finder::folder_size(f).0);
    show(app, router, s, Stage::Saved, view::saved_page(&name_of(s), &what, size, can_back_up));
}

fn undo(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    let folder = s.port.borrow().folder.clone();
    let mut settings = ForkSettings::load(&settings_path(app));
    let result = match &s.previous {
        Some(choice) => finder::set_override(&mut settings, &folder, choice),
        None => finder::clear_override(&mut settings, &folder),
    };
    if let Err(e) = result {
        return notice(app, router, s, "Could not undo that", vec![format!("{e}")], false);
    }
    let fresh = refresh_port(app, &s.port.borrow());
    *s.port.borrow_mut() = fresh;
    show_first(app, router, s);
}

// -- The search -----------------------------------------------------------------------------------------------

fn sd_steam_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for base in crate::fork::folder_picker::mounts::default_media_roots() {
        for entry in std::fs::read_dir(&base).into_iter().flatten().flatten() {
            if entry.path().join("steamapps").is_dir() {
                out.push(entry.path());
            }
        }
    }
    out
}

fn start_search(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    let port = s.port.borrow().clone();
    let catalog = app.catalog.borrow().clone();
    let (game_dir, library, backups) = (s.game_dir.clone(), app.paths.library_dir(), app.paths.saves_backup_dir.clone());
    let home = home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let job = Rc::new(Background::spawn(move |ctl| {
        let steam_roots = sd_steam_roots();
        let input = search::Input { port: &port, catalog: &catalog, game_dir: &game_dir, library_dir: &library, saves_backup_dir: &backups, home: &home, steam_roots: &steam_roots };
        search::find(&input, &|| ctl.cancelled(), &mut |text| ctl.set_progress(text.to_string()))
    }));
    *s.job.borrow_mut() = Some(job.clone());
    s.stage.set(Stage::Searching);
    let name = name_of(s);
    let Some(handle) = show_page_handle(app, router, view::searching_page(&name, ""), handlers(s)) else { return };
    watch_search(app.clone(), router.clone(), s.clone(), job, handle);
}

fn watch_search(app: Rc<AppState>, router: Router, s: Rc<Session>, job: Rc<Background<search::Found>>, handle: PageHandle<Act>) {
    let last = RefCell::new(String::new());
    FIND_TIMER.with(|timer| {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
            if s.stage.get() != Stage::Searching || !s.job.borrow().as_ref().is_some_and(|j| Rc::ptr_eq(j, &job)) {
                return FIND_TIMER.with(slint::Timer::stop);
            }
            if let Some(result) = job.take() {
                FIND_TIMER.with(slint::Timer::stop);
                s.job.borrow_mut().take();
                match result {
                    Ok(found) => show_results(&app, &router, &s, found),
                    Err(e) => notice(&app, &router, &s, "The search stopped", vec![e, "Nothing was changed.".into()], false),
                }
                return;
            }
            let text = job.progress();
            if text != *last.borrow() {
                *last.borrow_mut() = text.clone();
                if !handle.update(view::searching_page(&name_of(&s), &text)) {
                    FIND_TIMER.with(slint::Timer::stop);
                }
            }
        });
    });
}

fn show_results(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, found: search::Found) {
    let page = view::results_page(&name_of(s), &found.candidates, found.stopped_early, SystemTime::now());
    *s.found.borrow_mut() = found.candidates;
    show(app, router, s, Stage::Results, page);
}

// -- Back up now ----------------------------------------------------------------------------------------------

fn back_up(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    let port = s.port.borrow().clone();
    let (backups, game_dir) = (app.paths.saves_backup_dir.clone(), s.game_dir.clone());
    let stamp = crate::fork::store::backup_stamp();
    let folder = port.folder.clone();
    let job = Rc::new(Background::spawn(move |ctl| crate::core::save_backup::backup_before_update(&port, &backups, &game_dir, &stamp, &mut |t| ctl.set_progress(t.to_string()))));
    s.stage.set(Stage::Notice);
    show_page(app, router, view::backing_up_page(&name_of(s)), handlers(s));
    // From here B and the X wait: the result must be seen (`Stage::Backing`).
    s.stage.set(Stage::Backing);
    let (app, router, s) = (app.clone(), router.clone(), s.clone());
    BACKUP_TIMER.with(|timer| {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
            let Some(result) = job.take() else { return };
            BACKUP_TIMER.with(slint::Timer::stop);
            s.stage.set(Stage::Notice);
            let (title, text) = match &result {
                Ok(Ok(Some(_))) => ("Backup made", format!("Backed up to Saves Backup / Update Backups / {folder}. Restore saves lists it as \"Before update\".")),
                Ok(Ok(None)) => ("Nothing to back up", "The folder holds no saves yet, so there was nothing to copy.".to_string()),
                Ok(Err(detail)) | Err(detail) => {
                    eprintln!("save finder backup: {detail}");
                    let why = if detail.starts_with("not enough free space") { format!("There is {detail}.") } else { String::new() };
                    ("The backup did not work", format!("{why} Nothing was changed. Your saves are as they were.").trim().to_string())
                }
            };
            notice(&app, &router, &s, title, vec![text], true);
        });
    });
}
