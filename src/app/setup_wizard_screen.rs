//! The setup wizard's screen: one page per view (`ui/fork/setup-wizard.slint`: header with the step
//! count and a progress bar, a title, a sentence or two, a summary card, up to three big choices and a footer of
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
    /// The old copies already listed when this wizard run started: only the ones added since are asked about.
    static OLD_AT_START: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}

fn settings_path(app: &AppState) -> PathBuf {
    app.paths.config_dir.join(FILE_NAME)
}

// -- Starting and ending ---------------------------------------------------------------------------

/// Called once at start: opens the wizard when setup was never finished (new installs and installs
/// from before the wizard existed). It waits while another startup dialog (the missing ports folder
/// message, which the Steam queue offer waits for too) is due or open.
/// A wizard run is open (the screens it hands over to keep their old copies for its last question).
pub(crate) fn active() -> bool {
    SESSION.with(|s| s.borrow().is_some())
}

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
    OLD_AT_START.with(|o| *o.borrow_mut() = old_copy_paths(app));
    SESSION.with(|s| *s.borrow_mut() = Some(session.clone()));
    QUIET.with(|q| q.set(0));
    start_watch(app, router);
    let mut ui = Ui { app: app.clone(), router: router.clone() };
    ui.show(&session.borrow());
}

/// The title bar X of a screen the wizard handed over to: leave the wizard like the X on its own pages (setup stays
/// unfinished and comes back at the next start).
pub(crate) fn exit_wizard(app: &Rc<AppState>, router: &Router) {
    end_session(app, router);
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
        s.borrow().as_ref().is_some_and(|w| w.try_borrow().is_ok_and(|w| w.handoff.as_ref().is_some_and(|h| flow::menu_returns_to_wizard(h, menu))))
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

fn context(app: &AppState, step: crate::fork::setup_wizard::flow::Step) -> Context {
    let current = app.paths.library_dir();
    let internal_default = crate::fork::ports_folder::default_dir(&app.paths.config_dir);
    let drives = removable_drives(&system_mounts(), &default_media_roots());
    let earlier = EARLIER.with(|e| e.borrow().clone());
    Context {
        quick: places::quick_choices(&current, &internal_default, &drives, earlier.as_deref()),
        current_is_default: current == internal_default,
        ports_folder: current,
        rom_folder_set: rom_folder(app).is_some(),
        steam: (step == crate::fork::setup_wizard::flow::Step::Steam).then(|| steam_info(app)),
    }
}

fn draw(app: &Rc<AppState>, router: &Router, w: &Wizard) {
    let ctx = context(app, w.step);
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

    fn ports_folder(&mut self) -> PathBuf {
        self.app.paths.library_dir()
    }

    fn import_step(&mut self) {
        import_step(&self.app, &self.router);
    }

    fn import_went_back(&mut self) -> bool {
        super::import_screen::take_went_back()
    }

    fn rom_folder_step(&mut self) {
        close_current_dialog(&self.app, &self.router);
        super::rom_screens::open_folder_chooser(&self.app, &self.router);
    }

    fn rom_folder_set(&mut self) -> bool {
        rom_folder(&self.app).is_some()
    }

    fn steam_accounts(&mut self) -> Vec<String> {
        steam_info(&self.app).accounts
    }

    fn steam_choose_account(&mut self, index: usize) -> Result<(), String> {
        let screen = steam_screen(&self.app);
        let account = screen.accounts.get(index).ok_or("That account is not there any more.")?;
        let mut settings = ForkSettings::load(&settings_path(&self.app));
        settings.set_steam_account(account.account_id).map_err(|e| e.to_string())
    }

    fn steam_add_all(&mut self) -> flow::SteamOutcome {
        let mut screen = steam_screen(&self.app);
        let settings = ForkSettings::load(&settings_path(&self.app));
        let queue = crate::fork::steam_queue::queue_path(&self.app.paths.config_dir);
        crate::fork::setup_wizard::steam::add_all(&mut screen, &crate::fork::store::SystemSteamCheck::new(), &settings, &queue, std::time::SystemTime::now(), &crate::fork::store::backup_stamp())
    }

    fn art_sources_step(&mut self) {
        close_current_dialog(&self.app, &self.router);
        super::art_screens::open_art_sources(&self.app, &self.router);
    }

    fn kept_old_copies(&mut self) -> usize {
        new_old_copies(&self.app).len()
    }

    fn old_copies_step(&mut self) {
        old_copies_step(&self.app, &self.router);
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

// -- Steam ---------------------------------------------------------------------------------------------

/// The Add to Steam screen's own load, for the wizard's one-button version.
fn steam_screen(app: &AppState) -> crate::fork::screens::SteamScreen {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let settings = ForkSettings::load(&settings_path(app));
    let catalog = app.catalog.borrow();
    let state = app.state.borrow();
    crate::fork::setup_wizard::steam::load_screen(
        &home,
        &app.paths.library_dir(),
        &catalog,
        &settings,
        &|key| state.get(key).and_then(|i| i.favorite_exe.clone()),
        &app.paths.cache_dir,
        &crate::fork::steam_queue::queue_path(&app.paths.config_dir),
        crate::fork::steam_queue::game_mode_from_env(),
    )
}

fn steam_info(app: &AppState) -> crate::fork::setup_wizard::steam::Info {
    // Covers are fetched in the background while the page is up, so "Add all" finds them cached.
    super::steam_screens::prefetch_art(app);
    crate::fork::setup_wizard::steam::info(&steam_screen(app), &crate::fork::store::SystemSteamCheck::new())
}

// -- Old copies ---------------------------------------------------------------------------------------

fn old_copy_paths(app: &AppState) -> Vec<PathBuf> {
    crate::fork::port_mover::old_copies::load(&app.paths.config_dir).unwrap_or_default().into_iter().map(|o| o.old_path).collect()
}

/// The old copies this wizard run kept: the ones listed now that were not listed when it started.
fn new_old_copies(app: &AppState) -> Vec<crate::fork::port_mover::old_copies::OldCopy> {
    let before = OLD_AT_START.with(|o| o.borrow().clone());
    crate::fork::port_mover::old_copies::load(&app.paths.config_dir).unwrap_or_default().into_iter().filter(|o| !before.contains(&o.old_path)).collect()
}

/// The one page about the old copies this run kept (the same page Import ports used to show after its move):
/// Keep them is the default and B. The wizard goes on whichever way it is left.
fn old_copies_step(app: &Rc<AppState>, router: &Router) {
    close_current_dialog(app, router);
    let copies: std::collections::VecDeque<_> = new_old_copies(app).into();
    super::ports_folder_screen::prompt_old_copies_batch_then(app, router, copies, String::new(), |app, router, _text| {
        let (app, router) = (app.clone(), router.clone());
        // After this dialog's own closing step has finished.
        slint::Timer::single_shot(Duration::from_millis(0), move || with_wizard(&app, &router, |w, ui| flow::resume_old_copies(w, ui)));
    });
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
