//! The setup wizard's screen: one page per view (`ui/fork/setup-wizard.slint`: header with the step
//! count and a progress bar, a title, wrapped text, a summary card, a few real options and a footer of
//! buttons), drawn from `crate::fork::setup_wizard::text::Page`.
//!
//! It opens by itself at the first start (`start_if_first_run`, called once from `main.rs`) and any
//! time from Settings > Setup wizard... (`open_from_settings`). Focus moves only between options and
//! buttons (D-pad or arrows), A activates, B goes back one step (on the welcome page it asks whether
//! to skip setup), and the primary button has the initial focus.
//!
//! The wizard hands over to existing screens (import, game files, Add to Steam, Artwork sources).
//! Those do not know about the wizard, so while one is open the wizard waits (`tick`) and, once no
//! dialog is open for a moment, comes back at the same step and reads what changed. Their Back rows
//! that lead to the Settings list come back to the wizard instead (`resume_instead_of_settings`).
//!
//! Integration points are `ports_folder_step` and `import_step` below. Today the first records the
//! choice and uses the existing folder browser; the second opens Settings > Adopt installed ports. Both
//! are meant to be switched to the reusable folder picker and the new import screen.

use super::dialogs::{build_picker_dialog, build_setup_wizard_dialog, close_current_dialog, dialog_is_open, finish_dialog_open};
use super::state::AppState;
use crate::fork::rom_library::RomLibraryStore;
use crate::fork::rom_screens::browse::{self, FolderBrowser, Line as BrowseLine};
use crate::fork::rom_screens::util::{ellipsize, file_name};
use crate::fork::settings::{ForkSettings, FILE_NAME};
use crate::fork::setup_wizard::flow::{self, Action, Adapters, FolderPlan, Mode, Wizard};
use crate::fork::setup_wizard::text::{self, Context, Page};
use crate::fork::setup_wizard::{ports_folder, setup_state, SetupState};
use crate::ui::{chrome, dialog_geometry};
use crate::ui::gamepad_router::GamepadRouter;
use crate::{ListPickerDialog, SetupWizardDialog, WizardOption};
use serde_json::Value;
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

type Router = Rc<RefCell<GamepadRouter>>;
type Session = Rc<RefCell<Wizard>>;

/// Until the ports-folder feature owns the setting, the choice is recorded here.
const KEY_PORTS_FOLDER_CHOICE: &str = "setup_ports_folder";
/// How long (in ticks of `TICK`) no dialog must be open before the wizard comes back.
const QUIET_TICKS: u32 = 2;
const TICK: Duration = Duration::from_millis(250);
const START_DELAY: Duration = Duration::from_millis(500);

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    static WATCH: slint::Timer = slint::Timer::default();
    static QUIET: Cell<u32> = const { Cell::new(0) };
    /// The focused item of the page on screen (an index into `text::focusables`).
    static FOCUS: Cell<usize> = const { Cell::new(0) };
}

fn settings_path(app: &AppState) -> PathBuf {
    app.paths.config_dir.join(FILE_NAME)
}

fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"].iter().find_map(|v| std::env::var_os(v)).map(PathBuf::from).filter(|p| p.is_absolute())
}

// -- Starting and ending ---------------------------------------------------------------------------

/// Called once at start: opens the wizard when setup was never finished (new installs and installs
/// from before the wizard existed).
pub(crate) fn start_if_first_run(app: &Rc<AppState>, router: &Router) {
    if !crate::fork::setup_wizard::opens_at_start(&settings_path(app)) {
        return;
    }
    let (app, router) = (app.clone(), router.clone());
    // After the first frame, so the dialog finds the main window already placed.
    slint::Timer::single_shot(START_DELAY, move || {
        if !dialog_is_open(&app) {
            begin(&app, &router, Mode::FirstRun);
        }
    });
}

/// Settings > Setup wizard...
pub(crate) fn open_from_settings(app: &Rc<AppState>, router: &Router) {
    let mode = if setup_state(&settings_path(app)) == SetupState::Done { Mode::Rerun } else { Mode::FirstRun };
    begin(app, router, mode);
}

fn begin(app: &Rc<AppState>, router: &Router, mode: Mode) {
    let session: Session = Rc::new(RefCell::new(Wizard::new(mode)));
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

/// Settings' list is about to open. When the wizard is waiting for an existing screen to finish (that
/// screen's Back row leads to Settings), come back to the wizard instead. True = handled.
pub(crate) fn resume_instead_of_settings(app: &Rc<AppState>, router: &Router) -> bool {
    let waiting = SESSION.with(|s| s.borrow().as_ref().is_some_and(|w| w.try_borrow().is_ok_and(|w| w.handoff.is_some())));
    if waiting {
        with_wizard(app, router, |w, ui| flow::resume(w, ui));
    }
    waiting
}

fn start_watch(app: &Rc<AppState>, router: &Router) {
    let (app, router) = (app.clone(), router.clone());
    WATCH.with(|t| t.start(slint::TimerMode::Repeated, TICK, move || tick(&app, &router)));
}

/// While an existing screen is open the wizard waits. When no dialog has been open for a moment: after a
/// hand-off, the user closed that screen, so the wizard returns; otherwise the user closed the wizard
/// itself (the window's close button), so it is over for this run and is offered again at the next start.
fn tick(app: &Rc<AppState>, router: &Router) {
    let Some(session) = SESSION.with(|s| s.borrow().clone()) else { return WATCH.with(|t| t.stop()) };
    if dialog_is_open(app) {
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
    let waiting = session.try_borrow().is_ok_and(|w| w.handoff.is_some());
    if waiting {
        with_wizard(app, router, |w, ui| flow::resume(w, ui));
    } else {
        end_session(app, router);
    }
}

// -- Drawing ---------------------------------------------------------------------------------------

fn context(app: &AppState, w: &Wizard) -> Context {
    let current = w.summary.ports_folder.clone().unwrap_or_else(|| current_ports_folder(app));
    let internal_default = app.paths.config_dir.join("Library");
    Context {
        quick: ports_folder::quick_choices(&current, &internal_default, &browse::system_mounts()),
        ports_folder: current,
        rom_folder: rom_folder(app),
    }
}

fn show_list(
    app: &Rc<AppState>,
    router: &Router,
    title: &str,
    labels: Vec<String>,
    selected: usize,
    on_select: impl Fn(&Rc<AppState>, &Router, usize) + 'static,
    on_back: impl Fn(&Rc<AppState>, &Router) + 'static,
) {
    let Some(dialog) = build_picker_dialog(app, router, title, &labels) else { return };
    fit_height(app, &dialog, labels.len());
    dialog.set_selected_index(selected as i32);
    app.dialog_nav.picker_index.set(selected as i32);
    {
        let (app, router) = (app.clone(), router.clone());
        dialog.on_item_selected(move |index| on_select(&app, &router, index as usize));
    }
    {
        let (app, router) = (app.clone(), router.clone());
        dialog.on_close_requested(move || on_back(&app, &router));
    }
    let _ = dialog.show();
    let weak = dialog.as_weak();
    finish_dialog_open(app, router, super::dialogs::DialogSlot::Picker(dialog));
    // Scroll the cursor row into view (a step can have more rows than fit).
    slint::Timer::single_shot(Duration::from_millis(80), move || {
        if let Some(d) = weak.upgrade() {
            d.set_scroll_trigger(!d.get_scroll_trigger());
        }
    });
}

/// The picker caps its height at 65% of the screen; a wizard step reads better with all its rows in
/// view, so allow up to 92% and re-centre over the main window.
fn fit_height(app: &AppState, dialog: &ListPickerDialog, rows: usize) {
    const GUTTER: i32 = 12; // the picker's own gutter
    let window = app.window();
    let big = window.get_big_mode();
    let fonts = if big { app.window_geometry.fullscreen_mode.borrow().fonts } else { app.window_geometry.normal_mode.borrow().fonts };
    let (wx, wy, ww, wh) = chrome::work_area_under_cursor();
    let scale = app.window_geometry.scale.get();
    let item_h = dialog_geometry::list_picker_item_height(wh, big);
    let border = app.window_geometry.border_width.get();
    let n = rows.max(1) as i32;
    let wanted = fonts.title_bar_height_px + GUTTER * 2 + n * item_h + (n - 1) * (GUTTER / 2) + border * 2;
    let height = wanted.min((wh as f64 * 0.92) as i32);
    if height as f32 / scale <= dialog.get_initial_height() {
        return;
    }
    dialog.set_initial_height(height as f32 / scale);
    let width = (dialog.get_initial_width() * scale) as i32;
    let (pos, size) = (window.window().position(), window.window().size());
    let (px, py, pw, ph) = if size.width > 0 { (pos.x, pos.y, size.width as i32, size.height as i32) } else { (wx, wy, ww, wh) };
    let (x, y) = dialog_geometry::center_over_parent(px, py, pw, ph, width, height);
    // A main window smaller than the dialog must not push it off the screen.
    let (x, y) = (x.clamp(wx, (wx + ww - width).max(wx)), y.clamp(wy, (wy + wh - height).max(wy)));
    dialog.window().set_position(slint::WindowPosition::Physical(slint::PhysicalPosition { x, y }));
}

fn model<T: Clone + 'static>(items: Vec<T>) -> slint::ModelRc<T> {
    slint::ModelRc::new(slint::VecModel::from(items))
}

fn fill(d: &SetupWizardDialog, w: &Wizard, page: &Page) {
    d.set_dialog_title(text::title(w).into());
    d.set_header_text(text::APP_NAME.into());
    d.set_step_text(text::step_text(page).into());
    d.set_step_count(page.step_count as i32);
    d.set_step_index(page.step_number as i32 - 1);
    d.set_page_title(page.title.clone().into());
    d.set_paragraphs(model(page.paragraphs.iter().map(|p| p.clone().into()).collect::<Vec<slint::SharedString>>()));
    d.set_facts(model(page.facts.iter().map(|f| f.clone().into()).collect::<Vec<slint::SharedString>>()));
    d.set_options(model(
        page.options
            .iter()
            .map(|o| WizardOption { title: o.title.clone().into(), detail: o.detail.clone().into(), kind: i32::from(o.choice.is_some()), chosen: o.choice == Some(true) })
            .collect::<Vec<_>>(),
    ));
    d.set_footnote(page.footnote.clone().into());
    let label = |slot: usize| page.buttons[slot].as_ref().map(|b| b.label.clone()).unwrap_or_default();
    d.set_left_text(label(0).into());
    d.set_middle_text(label(1).into());
    d.set_right_text(label(2).into());
    d.set_primary_slot(page.primary.map_or(-1, |s| s as i32));
    d.set_focus_left(text::button_number(page, 0));
    d.set_focus_middle(text::button_number(page, 1));
    d.set_focus_right(text::button_number(page, 2));
}

fn draw(app: &Rc<AppState>, router: &Router, w: &Wizard) {
    let ctx = context(app, w);
    let page = Rc::new(text::page_for(w, &ctx));
    let Some(dialog) = build_setup_wizard_dialog(app, router) else { return };
    fill(&dialog, w, &page);
    let start = text::initial_focus(&page);
    dialog.set_selected_index(start as i32);
    FOCUS.with(|f| f.set(start));

    let press = |app: &Rc<AppState>, router: &Router, action: Action| with_wizard(app, router, |w, ui| flow::press(w, action, ui));
    {
        let (app, router, page) = (app.clone(), router.clone(), page.clone());
        dialog.on_option_requested(move |i| {
            if let Some(option) = page.options.get(i as usize) {
                press(&app, &router, option.action.clone());
            }
        });
    }
    {
        let (app, router, page) = (app.clone(), router.clone(), page.clone());
        dialog.on_button_requested(move |slot| {
            if let Some(button) = page.buttons.get(slot as usize).and_then(Option::as_ref) {
                press(&app, &router, button.action.clone());
            }
        });
    }
    {
        let weak = dialog.as_weak();
        dialog.on_nav_hovered(move |n| {
            FOCUS.with(|f| f.set(n.max(0) as usize));
            if let Some(d) = weak.upgrade() {
                d.set_selected_index(n);
            }
        });
    }
    {
        let (weak, page) = (dialog.as_weak(), page.clone());
        dialog.on_move_requested(move |dx, dy| {
            let next = text::move_focus(&page, FOCUS.with(Cell::get), dx, dy);
            FOCUS.with(|f| f.set(next));
            if let Some(d) = weak.upgrade() {
                d.set_selected_index(next as i32);
            }
        });
    }
    {
        let (app, router, page) = (app.clone(), router.clone(), page.clone());
        dialog.on_activate_requested(move || {
            if let Some(action) = text::action_at(&page, FOCUS.with(Cell::get)) {
                press(&app, &router, action);
            }
        });
    }
    {
        // B and Escape: back one step (a question or a notice: its own way out).
        let (app, router) = (app.clone(), router.clone());
        dialog.on_close_requested(move || {
            with_wizard(&app, &router, |w, ui| {
                let action = flow::back_action(w);
                flow::press(w, action, ui);
            });
        });
    }
    {
        // The title bar's X: leave the wizard. Setup stays unfinished, so it returns at the next start.
        let (app, router) = (app.clone(), router.clone());
        dialog.on_exit_requested(move || end_session(&app, &router));
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, super::dialogs::DialogSlot::SetupWizard(dialog));
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

    fn plan_ports_folder(&mut self, path: &Path) -> FolderPlan {
        ports_folder::plan(path)
    }

    fn create_ports_folder(&mut self, path: &Path) -> Result<(), String> {
        ports_folder::create(path)
    }

    fn apply_ports_folder(&mut self, path: &Path) -> Result<(), String> {
        record_ports_folder(&self.app, path)
    }

    fn browse_ports_folder(&mut self) {
        ports_folder_step(&self.app, &self.router);
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

/// Integration point: the ports folder in use. Interim: the choice recorded by this wizard, else the
/// library folder the app runs with.
fn current_ports_folder(app: &AppState) -> PathBuf {
    ForkSettings::load(&settings_path(app))
        .get(KEY_PORTS_FOLDER_CHOICE)
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| app.paths.library_dir.clone())
}

/// Integration point: remember the chosen ports folder. Interim: written to `fork-settings.json` only;
/// nothing is moved and the app keeps using its current library folder until the ports-folder feature
/// takes this over.
fn record_ports_folder(app: &AppState, path: &Path) -> Result<(), String> {
    ForkSettings::load(&settings_path(app)).set(KEY_PORTS_FOLDER_CHOICE, Value::from(path.to_string_lossy().into_owned())).map_err(|e| format!("{FILE_NAME}: {e}"))
}

// -- Integration point: the import step ------------------------------------------------------------

/// Opens the screen that finds ports installed elsewhere. Today that is Settings > Adopt installed
/// ports; it will be switched to the new import screen. The wizard counts the ports it added by
/// comparing installed ports before and after, so the screen needs no callback.
fn import_step(app: &Rc<AppState>, router: &Router) {
    close_current_dialog(app, router);
    super::steam_screens::open_adopt_screen(app, router);
}

// -- Integration point: the ports folder step ------------------------------------------------------

fn finish_browse(app: &Rc<AppState>, router: &Router, picked: Option<PathBuf>) {
    with_wizard(app, router, |w, ui| flow::browse_done(w, picked, ui));
}

/// Opens the folder chooser for the ports folder; the answer goes back through `flow::browse_done`
/// (`Some(folder)`, or `None` when the user backed out). Today it uses the game files folder browser;
/// it will be switched to the reusable folder picker.
fn ports_folder_step(app: &Rc<AppState>, router: &Router) {
    let mut places: Vec<(String, PathBuf)> =
        browse::system_mounts().into_iter().map(|m| (format!("SD card or drive: {}", ellipsize(&file_name(&m), 30)), m)).collect();
    if let Some(home) = home_dir() {
        places.push(("Home folder".into(), home));
    }
    places.push(("Top of the drive".into(), PathBuf::from("/")));
    let mut labels: Vec<String> = places.iter().map(|(label, _)| label.clone()).collect();
    labels.push("Back".into());
    let back_row = labels.len() - 1;
    show_list(
        app,
        router,
        "Choose the ports folder: start in...",
        labels,
        0,
        move |app, router, index| match places.get(index) {
            Some((_, dir)) => show_browser(app, router, Rc::new(RefCell::new(FolderBrowser::open(dir)))),
            None if index == back_row => finish_browse(app, router, None),
            None => {}
        },
        |app, router| finish_browse(app, router, None),
    );
}

fn show_browser(app: &Rc<AppState>, router: &Router, browser: Rc<RefCell<FolderBrowser>>) {
    let lines = browser.borrow().lines();
    let mut labels = browser.borrow().labels();
    for (i, line) in lines.iter().enumerate() {
        if *line == BrowseLine::Cancel {
            labels[i] = "Back (do not choose a folder)".into();
        }
    }
    let title = format!("Ports folder: {}", ellipsize(&browser.borrow().current.to_string_lossy(), 44));
    show_list(
        app,
        router,
        &title,
        labels,
        0,
        move |app, router, index| match lines.get(index) {
            Some(BrowseLine::UseThis) => finish_browse(app, router, Some(browser.borrow().current.clone())),
            Some(BrowseLine::Cancel) => finish_browse(app, router, None),
            Some(BrowseLine::Up) => {
                browser.borrow_mut().up();
                show_browser(app, router, browser.clone());
            }
            Some(BrowseLine::Folder(i)) => {
                browser.borrow_mut().enter(*i);
                show_browser(app, router, browser.clone());
            }
            Some(BrowseLine::More) | None => {}
        },
        |app, router| finish_browse(app, router, None),
    );
}
