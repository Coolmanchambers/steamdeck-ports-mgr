//! The setup wizard's screen: one page per view (`ui/fork/setup-wizard.slint`: header with the step
//! count and a progress bar, a title, wrapped text, a summary card, a few real options and a footer of
//! buttons), drawn from `crate::fork::setup_wizard::text::Page` by `page_dialog::show_page`.
//!
//! It opens by itself at the first start (`start_if_first_run`, called once from `main.rs`) and any
//! time from Settings > Setup wizard... (`open_from_settings`). Focus moves only between options and
//! buttons (D-pad or arrows), A activates, B goes back one step (on the welcome page it asks whether
//! to skip setup), and the primary button has the initial focus.
//!
//! The wizard hands over to the app's own screens (ports folder, import, game files, Add to Steam,
//! Artwork sources). Those do not know about the wizard, so while one is open the wizard waits (`tick`)
//! and, once no dialog is open for a moment, comes back at the same step and reads what changed. Their
//! Back rows that lead to the Settings list come back to the wizard instead (`resume_instead_of_menu`).
//!
//! Integration points: `ports_folder_step` (the ports-folder screens: shared picker, checks, moving
//! installed ports) and `import_step` (the import screen, which tells the wizard when the user leaves it).

use super::dialogs::{close_current_dialog, dialog_is_open};
use super::page_dialog::{show_page, Handlers};
use super::state::AppState;
use crate::fork::folder_picker::mounts::{default_media_roots, removable_drives, system_mounts};
use crate::fork::rom_library::RomLibraryStore;
use crate::fork::settings::{ForkSettings, FILE_NAME};
use crate::fork::setup_wizard::flow::{self, Adapters, Menu, Mode, Wizard};
use crate::fork::setup_wizard::text::{self, Context};
use crate::fork::setup_wizard::{places, setup_state, SetupState};
use crate::ui::gamepad_router::GamepadRouter;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

type Router = Rc<RefCell<GamepadRouter>>;
type Session = Rc<RefCell<Wizard>>;

/// A build before the real ports folder setting recorded the wizard's choice under this key. Offered
/// once as a quick choice, then forgotten.
const KEY_EARLIER_CHOICE: &str = "setup_ports_folder";
/// How long (in ticks of `TICK`) no dialog must be open before the wizard comes back.
const QUIET_TICKS: u32 = 2;
const TICK: Duration = Duration::from_millis(250);
const START_DELAY: Duration = Duration::from_millis(500);
/// How long the wizard waits at start for another startup dialog (the missing ports folder message) to
/// be dealt with: about an hour, then it is offered again at the next start.
const MAX_START_WAITS: u32 = 7200;

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    static WATCH: slint::Timer = slint::Timer::default();
    static QUIET: Cell<u32> = const { Cell::new(0) };
    /// The folder an earlier build recorded, offered in this run only.
    static EARLIER: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    /// The ports folder when this wizard run started.
    static STARTED: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

fn settings_path(app: &AppState) -> PathBuf {
    app.paths.config_dir.join(FILE_NAME)
}

// -- Starting and ending ---------------------------------------------------------------------------

/// Called once at start: opens the wizard when setup was never finished (new installs and installs
/// from before the wizard existed). It waits while another startup dialog (the missing ports folder
/// message, which the Steam queue offer waits for too) is due or open.
pub(crate) fn start_if_first_run(app: &Rc<AppState>, router: &Router) {
    if !crate::fork::setup_wizard::opens_at_start(&settings_path(app)) {
        return;
    }
    start_when_quiet(app.clone(), router.clone(), 0);
}

fn start_when_quiet(app: Rc<AppState>, router: Router, waited: u32) {
    // After the first frame, so the dialog finds the main window already placed.
    slint::Timer::single_shot(START_DELAY, move || {
        // Setup was finished meanwhile (the wizard was run and finished from Settings), or a wizard is
        // already showing: nothing to open.
        if !crate::fork::setup_wizard::opens_at_start(&settings_path(&app)) || SESSION.with(|s| s.borrow().is_some()) {
            return;
        }
        if super::ports_folder_screen::startup_dialog_pending() || dialog_is_open(&app) {
            if waited < MAX_START_WAITS {
                start_when_quiet(app, router, waited + 1);
            }
            return;
        }
        begin(&app, &router, Mode::FirstRun);
    });
}

/// Settings > Setup wizard...
pub(crate) fn open_from_settings(app: &Rc<AppState>, router: &Router) {
    let mode = if setup_state(&settings_path(app)) == SetupState::Done { Mode::Rerun } else { Mode::FirstRun };
    begin(app, router, mode);
}

fn begin(app: &Rc<AppState>, router: &Router, mode: Mode) {
    let session: Session = Rc::new(RefCell::new(Wizard::new(mode)));
    EARLIER.with(|e| *e.borrow_mut() = take_earlier_choice(app));
    STARTED.with(|s| *s.borrow_mut() = Some(app.paths.library_dir()));
    SESSION.with(|s| *s.borrow_mut() = Some(session.clone()));
    QUIET.with(|q| q.set(0));
    start_watch(app, router);
    let mut ui = Ui { app: app.clone(), router: router.clone() };
    ui.show(&session.borrow());
}

fn end_session(app: &Rc<AppState>, router: &Router) {
    SESSION.with(|s| *s.borrow_mut() = None);
    WATCH.with(|t| t.stop());
    close_current_dialog(app, router);
    app.refresh_current_view();
}

/// Runs `f` on the running wizard. Does nothing when there is none, or when the wizard is already busy
/// (a callback fired from inside a step).
fn with_wizard(app: &Rc<AppState>, router: &Router, f: impl FnOnce(&mut Wizard, &mut Ui)) {
    let Some(session) = SESSION.with(|s| s.borrow().clone()) else { return };
    let Ok(mut wizard) = session.try_borrow_mut() else { return };
    let mut ui = Ui { app: app.clone(), router: router.clone() };
    f(&mut wizard, &mut ui);
}

/// A menu (Settings, Settings > Ports folder, Settings > Game files) is about to open. When the wizard is
/// waiting for the screen it opened to finish and this menu is where that screen leads when the user backs
/// out (`flow::menu_returns_to_wizard`), come back to the wizard instead. True = handled. A menu that a
/// screen opens for its own purpose (the ports-folder flow sending the user to Game files to fix a file)
/// is left alone.
pub(crate) fn resume_instead_of_menu(app: &Rc<AppState>, router: &Router, menu: Menu) -> bool {
    let waiting = SESSION.with(|s| {
        s.borrow().as_ref().is_some_and(|w| w.try_borrow().is_ok_and(|w| w.handoff.is_some_and(|h| flow::menu_returns_to_wizard(h, menu))))
    });
    if waiting {
        with_wizard(app, router, |w, ui| flow::resume(w, ui));
    }
    waiting
}

fn start_watch(app: &Rc<AppState>, router: &Router) {
    let (app, router) = (app.clone(), router.clone());
    WATCH.with(|t| t.start(slint::TimerMode::Repeated, TICK, move || tick(&app, &router)));
}

/// While an existing screen is open the wizard waits. When no dialog has been open for a moment, the
/// wizard comes back: after a hand-off it reads what the other screen did; with none pending (an
/// unrelated dialog took its place and was closed) it just shows the same page again. It ends only when
/// the user leaves it (the title bar's X or the window's close button), or finishes.
fn tick(app: &Rc<AppState>, router: &Router) {
    if SESSION.with(|s| s.borrow().is_none()) {
        return WATCH.with(|t| t.stop());
    }
    // A running import counts as "something is open": after B on its progress list nothing is on screen while the
    // worker stops, and its late result and questions must not meet a different wizard page.
    if dialog_is_open(app) || super::import_screen::import_running() {
        QUIET.with(|q| q.set(0));
        return;
    }
    let quiet = QUIET.with(|q| {
        q.set(q.get() + 1);
        q.get()
    });
    if quiet < QUIET_TICKS {
        return;
    }
    QUIET.with(|q| q.set(0));
    // `resume` shows the page again whether or not a hand-off is pending.
    with_wizard(app, router, |w, ui| flow::resume(w, ui));
}

// -- Drawing ---------------------------------------------------------------------------------------

fn context(app: &AppState) -> Context {
    let current = app.paths.library_dir();
    let internal_default = crate::fork::ports_folder::default_dir(&app.paths.config_dir);
    let drives = removable_drives(&system_mounts(), &default_media_roots());
    let earlier = EARLIER.with(|e| e.borrow().clone());
    let started_with = STARTED.with(|s| s.borrow().clone()).unwrap_or_else(|| current.clone());
    Context {
        quick: places::quick_choices(&current, &internal_default, &drives, earlier.as_deref()),
        current_is_default: current == internal_default,
        started_with,
        ports_folder: current,
        rom_folder: rom_folder(app),
    }
}

fn draw(app: &Rc<AppState>, router: &Router, w: &Wizard) {
    let ctx = context(app);
    let page = text::page_for(w, &ctx);
    let handlers = Handlers {
        on_action: Rc::new(|app, router, action| with_wizard(app, router, |w, ui| flow::press(w, action, ui))),
        // B and Escape: back one step (a question or a notice: its own way out).
        on_back: Rc::new(|app, router| {
            with_wizard(app, router, |w, ui| {
                let action = flow::back_action(w);
                flow::press(w, action, ui);
            });
        }),
        // The title bar's X, or the window system's own close: leave the wizard. Setup stays unfinished, so it
        // returns at the next start.
        on_exit: Rc::new(end_session),
        on_input: None,
    };
    show_page(app, router, page, handlers);
}

// -- The adapters ----------------------------------------------------------------------------------

struct Ui {
    app: Rc<AppState>,
    router: Router,
}

impl Adapters for Ui {
    fn show(&mut self, wizard: &Wizard) {
        draw(&self.app, &self.router, wizard);
    }

    fn ports_folder_step(&mut self, chosen: Option<PathBuf>) {
        ports_folder_step(&self.app, &self.router, chosen);
    }

    fn import_step(&mut self) {
        import_step(&self.app, &self.router);
    }

    fn rom_folder_step(&mut self) {
        close_current_dialog(&self.app, &self.router);
        super::rom_screens::open_folder_chooser(&self.app, &self.router);
    }

    fn steam_step(&mut self) {
        close_current_dialog(&self.app, &self.router);
        super::steam_screens::open_steam_screen(&self.app, &self.router);
    }

    fn art_sources_step(&mut self) {
        close_current_dialog(&self.app, &self.router);
        super::art_screens::open_art_sources(&self.app, &self.router);
    }

    fn installed_count(&mut self) -> usize {
        installed_count(&self.app)
    }

    fn mark_done(&mut self) -> Result<(), String> {
        crate::fork::setup_wizard::mark_done(&settings_path(&self.app)).map_err(|e| format!("{}: {e}", FILE_NAME))
    }

    fn close(&mut self) {
        end_session(&self.app, &self.router);
    }
}

fn installed_count(app: &AppState) -> usize {
    app.catalog.borrow().iter().filter(|p| app.is_installed(p)).count()
}

fn rom_folder(app: &AppState) -> Option<PathBuf> {
    RomLibraryStore::load(&app.paths.config_dir.join("rom_library.json")).ok().and_then(|s| s.rom_dir().map(Path::to_path_buf))
}

// -- The import step -----------------------------------------------------------------------------------

/// Opens the import screen (`import_screen`, as step c of the wizard). It runs `finish` once when the user
/// leaves it, whichever way (Continue setup, the end of an import with its old-copy questions and game
/// file search, B on its main list or its progress list, or a notice); quitting the app is the one exit
/// that does not. `finish` hands control back to the wizard, which counts the ports now installed against
/// the count taken when the step was opened, so the Done page reports what the import really moved.
fn import_step(app: &Rc<AppState>, router: &Router) {
    // Only the import hand-off is resumed: a late finish must not take another hand-off (the game files chooser, say).
    let finish: super::import_screen::Finish = Rc::new(|app, router| with_wizard(app, router, |w, ui| flow::resume_import(w, ui)));
    super::import_screen::open_with(app, router, true, Some(finish));
}

// -- The ports folder step -------------------------------------------------------------------------

/// The ports folder step hands over to the app's own ports-folder flow (`ports_folder_screen`): with a
/// quick choice the folder is checked, created after a confirm, and installed ports can be moved there
/// (old copies kept until the user removes them); with `None` the shared folder picker opens first.
/// The flow ends in its own dialogs, and the wizard comes back when they are closed.
fn ports_folder_step(app: &Rc<AppState>, router: &Router, chosen: Option<PathBuf>) {
    close_current_dialog(app, router);
    match chosen {
        Some(dir) => super::ports_folder_screen::consider(app, router, dir),
        None => super::ports_folder_screen::choose_folder(app, router),
    }
}

/// A folder an earlier build of the wizard recorded: read once and forgotten, and ignored when it is
/// already the ports folder.
fn take_earlier_choice(app: &AppState) -> Option<PathBuf> {
    let mut settings = ForkSettings::load(&settings_path(app));
    let earlier = settings.get(KEY_EARLIER_CHOICE).and_then(Value::as_str).map(PathBuf::from).filter(|p| p.is_absolute())?;
    let _ = settings.remove(KEY_EARLIER_CHOICE);
    (earlier != app.paths.library_dir()).then_some(earlier)
}
