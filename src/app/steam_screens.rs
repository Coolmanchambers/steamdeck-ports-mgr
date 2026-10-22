//! The "Add to Steam" and "Adopt installed ports" screens (Settings menu).
//!
//! Both are lists built from the picker dialog upstream already has, so they work with the
//! controller, keyboard and mouse like every other dialog: choosing a row toggles it, and the last
//! rows review and apply. The decisions are made in `crate::fork::screens` (tested without any UI);
//! this file only draws their text. The text is English only; it is not part of upstream's
//! translation tables.
//!
//! With Steam running (always, in Game Mode) the review row turns into "Queue for Desktop Mode": the
//! chosen changes go to `steam-queue.json` and `start_queue_watch` offers to apply them once Steam is
//! seen closed (at startup, then every 30 seconds). The decisions live in `crate::fork::steam_queue`.

use super::dialogs::{build_picker_dialog, dialog_is_open, finish_dialog_open, open_confirm_dialog, open_message_dialog, open_picker_dialog, set_picker_items, DialogSlot};
use super::state::AppState;
use crate::core::launch::open_file;
use crate::fork::screens::{self, AdoptLine, AdoptScreen, SteamLine, SteamScreen};
use crate::fork::settings::{ForkSettings, FILE_NAME};
use crate::fork::steam_queue::{self, ApplyInputs, ManageLine, WatchAction};
use crate::fork::steam_user::SteamAccount;
use crate::fork::store::{backup_stamp, SteamProcessCheck, SystemSteamCheck};
use crate::ui::gamepad_router::GamepadRouter;
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

type Router = Rc<RefCell<GamepadRouter>>;

const STEAM_TITLE: &str = "Add to Steam";
const ADOPT_TITLE: &str = "Adopt installed ports";
const QUEUE_TITLE: &str = "Queued Steam changes";

fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"].iter().find_map(|v| std::env::var_os(v)).map(PathBuf::from).filter(|p| p.is_absolute())
}

fn settings_path(app: &AppState) -> PathBuf {
    app.paths.config_dir.join(FILE_NAME)
}

fn queue_path(app: &AppState) -> PathBuf {
    steam_queue::queue_path(&app.paths.config_dir)
}

/// The dialogs size themselves from a text estimate that comes out a line short for wrapped text, which
/// clips the last line; one spare line keeps it readable.
fn padded(text: &str) -> String {
    format!("{text}\n")
}

fn no_home(app: &Rc<AppState>, router: &Router, title: &str) {
    open_message_dialog(app, router, title, "Could not find your home folder, so there is nowhere to look.");
}

// -- Add to Steam ---------------------------------------------------------------------------

pub(crate) fn open_steam_screen(app: &Rc<AppState>, router: &Router) {
    if dialog_is_open(app) {
        return;
    }
    let Some(home) = home_dir() else { return no_home(app, router, STEAM_TITLE) };
    let settings = ForkSettings::load(&settings_path(app));
    let mut screen = {
        let catalog = app.catalog.borrow();
        let state = app.state.borrow();
        SteamScreen::load(&home, &app.paths.library_dir, &catalog, &settings, &|key| state.get(key).and_then(|i| i.favorite_exe.clone()))
    };
    screen.cache_dir = Some(app.paths.cache_dir.clone());
    let loaded = steam_queue::load(&queue_path(app));
    screen.queue = loaded.queue;
    screen.queue_note = loaded.problem;
    screen.game_mode = steam_queue::game_mode_from_env();
    prefetch_art(app);
    show_steam_screen(app, router, Rc::new(RefCell::new(screen)));
}

/// Fetches the catalog portrait and icon of every installed port in the background, so Apply finds
/// them in the cache (`<folder>.png`, `<folder>_icon.png`). Ones already cached are not fetched again,
/// and a failed fetch is silent: Apply says "no cached artwork yet" for that port.
fn prefetch_art(app: &AppState) {
    let wanted: Vec<(String, Option<String>, Option<String>)> = app
        .catalog
        .borrow()
        .iter()
        .filter(|p| !p.is_android_only() && app.is_installed(p))
        .map(|p| (p.folder.clone(), p.image.clone(), p.icon.clone()))
        .collect();
    let cache_dir = app.paths.cache_dir.clone();
    std::thread::spawn(move || {
        for (folder, image, icon) in wanted {
            if let Some(url) = image {
                crate::core::image_cache::cache_image(&url, &cache_dir, &folder);
            }
            if let Some(url) = icon {
                crate::core::image_cache::cache_image(&url, &cache_dir, &format!("{folder}_icon"));
            }
        }
    });
}

fn reload_after_account_change(app: &AppState, screen: &mut SteamScreen, index: usize) {
    let catalog = app.catalog.borrow();
    let state = app.state.borrow();
    screen.set_account(index, &app.paths.library_dir, &catalog, &|key| state.get(key).and_then(|i| i.favorite_exe.clone()));
}

fn show_steam_screen(app: &Rc<AppState>, router: &Router, screen: Rc<RefCell<SteamScreen>>) {
    let steam = SystemSteamCheck::new();
    let labels = screen.borrow().labels(&steam);
    let Some(dialog) = build_picker_dialog(app, router, STEAM_TITLE, &labels) else { return };
    {
        let (app, router, weak) = (app.clone(), router.clone(), dialog.as_weak());
        dialog.on_item_selected(move |index| {
            let line = screen.borrow().lines().get(index as usize).copied();
            let steam = SystemSteamCheck::new();
            match line {
                Some(SteamLine::Port(i)) => screen.borrow_mut().toggle(i),
                Some(SteamLine::Status) | None => {}
                Some(SteamLine::QueueNote) => {
                    let note = screen.borrow().queue_note.clone().unwrap_or_default();
                    return open_message_dialog(&app, &router, QUEUE_TITLE, &padded(&note));
                }
                Some(SteamLine::ApplyQueue) => return offer_queue(&app, &router, &steam),
                Some(SteamLine::ManageQueue) => return show_manage_queue(&app, &router),
                Some(SteamLine::Account) => {
                    let accounts: Vec<SteamAccount> = screen.borrow().accounts.clone();
                    if accounts.len() > 1 {
                        let labels = accounts.iter().map(SteamAccount::label).collect();
                        let screen = screen.clone();
                        open_picker_dialog(&app, &router, "Steam account", labels, move |app, router, choice| {
                            if let Some(account) = accounts.get(choice) {
                                let mut settings = ForkSettings::load(&settings_path(app));
                                if let Err(e) = settings.set_steam_account(account.account_id) {
                                    return open_message_dialog(app, router, STEAM_TITLE, &format!("Could not remember the choice: {e}"));
                                }
                                reload_after_account_change(app, &mut screen.borrow_mut(), choice);
                            }
                            show_steam_screen(app, router, screen.clone());
                        });
                        return;
                    }
                }
                Some(SteamLine::Review) => {
                    review_steam_changes(&app, &router, screen.clone());
                    return;
                }
            }
            if let Some(d) = weak.upgrade() {
                set_picker_items(&d, &screen.borrow().labels(&steam));
            }
        });
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
}

fn review_steam_changes(app: &Rc<AppState>, router: &Router, screen: Rc<RefCell<SteamScreen>>) {
    let steam = SystemSteamCheck::new();
    if screen.borrow().can_queue(&steam) {
        return confirm_queue(app, router, &screen.borrow());
    }
    let problem = screen.borrow().review_problem(&steam);
    if let Some(message) = problem {
        return open_message_dialog(app, router, STEAM_TITLE, &padded(&message));
    }
    let text = padded(&screen.borrow().review_text());
    open_confirm_dialog(app, router, "Review Steam changes".into(), text.into(), "Apply".into(), move |app, router| {
        let settings = ForkSettings::load(&settings_path(app));
        match screens::apply_selected(&screen.borrow(), &SystemSteamCheck::new(), &settings, SystemTime::now()) {
            Ok(done) => open_message_dialog(app, router, "Added to Steam", &padded(&done.text)),
            Err(message) => open_message_dialog(app, router, STEAM_TITLE, &padded(&message)),
        }
    });
}

// -- Queue for Desktop Mode -----------------------------------------------------------------

fn confirm_queue(app: &Rc<AppState>, router: &Router, screen: &SteamScreen) {
    let changes = steam_queue::changes_from_screen(screen, &backup_stamp());
    let game_mode = steam_queue::game_mode_from_env();
    let text = padded(&steam_queue::queue_confirm_text(&changes, game_mode));
    open_confirm_dialog(app, router, "Queue for Desktop Mode".into(), text.into(), "Queue".into(), move |app, router| {
        match steam_queue::queue_changes(&queue_path(app), changes.clone(), game_mode) {
            Ok(text) => open_message_dialog(app, router, QUEUE_TITLE, &padded(&text)),
            Err(text) => open_message_dialog(app, router, STEAM_TITLE, &padded(&text)),
        }
    });
}

/// The screen's "apply now" row: asks first when Steam is closed, otherwise says why it has to wait.
fn offer_queue(app: &Rc<AppState>, router: &Router, steam: &dyn SteamProcessCheck) {
    let loaded = steam_queue::load(&queue_path(app));
    if let Some(problem) = loaded.problem {
        return open_message_dialog(app, router, QUEUE_TITLE, &padded(&problem));
    }
    if loaded.queue.is_empty() {
        return open_message_dialog(app, router, QUEUE_TITLE, "Nothing is queued.");
    }
    match steam.running_steam_processes() {
        Ok(p) if p.is_empty() => open_apply_offer(app, router, &loaded.queue),
        Ok(p) => {
            let when = steam_queue::when_applied_text(steam_queue::game_mode_from_env(), loaded.queue.len());
            let text = format!("{when}\n\nStill running: {}", p.join(", "));
            open_message_dialog(app, router, QUEUE_TITLE, &padded(&text));
        }
        Err(_) => open_message_dialog(app, router, QUEUE_TITLE, "Could not tell whether Steam is running, so nothing will be written."),
    }
}

fn open_apply_offer(app: &Rc<AppState>, router: &Router, queue: &steam_queue::SteamQueue) {
    let text = padded(&steam_queue::offer_text(queue));
    open_confirm_dialog(app, router, QUEUE_TITLE.into(), text.into(), "Apply".into(), |app, router| apply_queue_now(app, router));
}

fn apply_queue_now(app: &Rc<AppState>, router: &Router) {
    let Some(home) = home_dir() else { return no_home(app, router, QUEUE_TITLE) };
    let settings = ForkSettings::load(&settings_path(app));
    let outcome = {
        let catalog = app.catalog.borrow();
        let state = app.state.borrow();
        let favorite = |key: &str| state.get(key).and_then(|i| i.favorite_exe.clone());
        let inputs = ApplyInputs {
            home: &home,
            library_dir: &app.paths.library_dir,
            catalog: &catalog,
            favorite_exe: &favorite,
            cache_dir: Some(app.paths.cache_dir.clone()),
            settings: &settings,
        };
        steam_queue::apply_queue_file(&queue_path(app), &inputs, &SystemSteamCheck::new(), SystemTime::now())
    };
    open_message_dialog(app, router, QUEUE_TITLE, &padded(&outcome.text));
}

fn show_manage_queue(app: &Rc<AppState>, router: &Router) {
    let loaded = steam_queue::load(&queue_path(app));
    if let Some(problem) = &loaded.problem {
        return open_message_dialog(app, router, QUEUE_TITLE, &padded(problem));
    }
    let queue = Rc::new(RefCell::new(loaded.queue));
    let labels = steam_queue::manage_labels(&queue.borrow());
    let Some(dialog) = build_picker_dialog(app, router, QUEUE_TITLE, &labels) else { return };
    {
        let (app, router, weak) = (app.clone(), router.clone(), dialog.as_weak());
        dialog.on_item_selected(move |index| {
            let line = steam_queue::manage_lines(&queue.borrow()).get(index as usize).copied();
            match line {
                Some(ManageLine::Entry(i)) => {
                    let mut changed = queue.borrow().clone();
                    changed.remove(i);
                    if let Err(e) = steam_queue::save(&queue_path(&app), &changed) {
                        return open_message_dialog(&app, &router, QUEUE_TITLE, &format!("Could not save the queue: {e}"));
                    }
                    *queue.borrow_mut() = changed;
                }
                Some(ManageLine::Clear) => {
                    let n = queue.borrow().len();
                    let text = format!("Remove all {n} queued Steam change{}? shortcuts.vdf is not touched.", if n == 1 { "" } else { "s" });
                    open_confirm_dialog(&app, &router, QUEUE_TITLE.into(), padded(&text).into(), "Clear".into(), |app, router| {
                        match steam_queue::save(&queue_path(app), &steam_queue::SteamQueue::default()) {
                            Ok(()) => open_message_dialog(app, router, QUEUE_TITLE, "The queue is empty."),
                            Err(e) => open_message_dialog(app, router, QUEUE_TITLE, &format!("Could not clear the queue: {e}")),
                        }
                    });
                    return;
                }
                Some(ManageLine::Back) => {
                    super::dialogs::close_current_dialog(&app, &router);
                    return open_steam_screen(&app, &router);
                }
                None => return,
            }
            if let Some(d) = weak.upgrade() {
                set_picker_items(&d, &steam_queue::manage_labels(&queue.borrow()));
            }
        });
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
}

#[derive(Default)]
struct QueueWatch {
    /// The offer was made during this stretch of Steam being closed (see `steam_queue::watch_step`).
    offered: Cell<bool>,
    /// Queue-file problems already shown, so one that moving the file cannot fix is shown once.
    shown_problems: RefCell<Vec<String>>,
    prefetched: Cell<bool>,
}

/// Looks for queued Steam changes shortly after startup and then every 30 seconds, and offers to apply
/// them when Steam is closed. Keep the returned timer alive for as long as the app runs.
pub(crate) fn start_queue_watch(app: &Rc<AppState>, router: &Router) -> slint::Timer {
    let watch = Rc::new(QueueWatch::default());
    {
        let (app, router, watch) = (app.clone(), router.clone(), watch.clone());
        slint::Timer::single_shot(Duration::from_secs(2), move || check_queue(&app, &router, &watch));
    }
    let timer = slint::Timer::default();
    let (app, router) = (app.clone(), router.clone());
    timer.start(slint::TimerMode::Repeated, Duration::from_secs(steam_queue::RECHECK_SECONDS), move || check_queue(&app, &router, &watch));
    timer
}

fn check_queue(app: &Rc<AppState>, router: &Router, watch: &QueueWatch) {
    // Never on top of another dialog or while a game started from here runs; a later look offers.
    if dialog_is_open(app) || !app.install_runtime.running_processes.borrow().is_empty() {
        return;
    }
    let path = queue_path(app);
    if !path.exists() {
        watch.offered.set(false);
        return;
    }
    let loaded = steam_queue::load(&path);
    if let Some(problem) = loaded.problem {
        if !watch.shown_problems.borrow().contains(&problem) {
            watch.shown_problems.borrow_mut().push(problem.clone());
            open_message_dialog(app, router, QUEUE_TITLE, &padded(&problem));
        }
        return;
    }
    if !loaded.queue.is_empty() && !watch.prefetched.replace(true) {
        prefetch_art(app);
    }
    let mut offered = watch.offered.get();
    let action = steam_queue::watch_step(loaded.queue.len(), &SystemSteamCheck::new().running_steam_processes(), &mut offered);
    watch.offered.set(offered);
    if action == WatchAction::Offer {
        open_apply_offer(app, router, &loaded.queue);
    }
}

// -- Adopt installed ports ------------------------------------------------------------------

pub(crate) fn open_adopt_screen(app: &Rc<AppState>, router: &Router) {
    if dialog_is_open(app) {
        return;
    }
    let Some(home) = home_dir() else { return no_home(app, router, ADOPT_TITLE) };
    let settings = ForkSettings::load(&settings_path(app));
    let screen = AdoptScreen::load(&home, &app.paths.library_dir, &app.catalog.borrow(), &settings);
    show_adopt_screen(app, router, Rc::new(RefCell::new(screen)));
}

fn show_adopt_screen(app: &Rc<AppState>, router: &Router, screen: Rc<RefCell<AdoptScreen>>) {
    let labels = screen.borrow().labels();
    let Some(dialog) = build_picker_dialog(app, router, ADOPT_TITLE, &labels) else { return };
    {
        let (app, router, weak) = (app.clone(), router.clone(), dialog.as_weak());
        dialog.on_item_selected(move |index| {
            let line = screen.borrow().lines().get(index as usize).copied();
            match line {
                Some(AdoptLine::Candidate(i)) => screen.borrow_mut().toggle(i),
                Some(AdoptLine::Info(_)) | None => {}
                Some(AdoptLine::Adopt) => {
                    if screen.borrow().selected_count() > 0 {
                        confirm_adopt(&app, &router, screen.clone());
                        return;
                    }
                }
                Some(AdoptLine::EditFolders) => {
                    let mut settings = ForkSettings::load(&settings_path(&app));
                    super::dialogs::close_current_dialog(&app, &router);
                    match settings.ensure_file() {
                        Ok(()) => open_file(settings.path()),
                        Err(e) => open_message_dialog(&app, &router, ADOPT_TITLE, &format!("Could not create {FILE_NAME}: {e}")),
                    }
                    return;
                }
            }
            if let Some(d) = weak.upgrade() {
                set_picker_items(&d, &screen.borrow().labels());
            }
        });
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
}

fn confirm_adopt(app: &Rc<AppState>, router: &Router, screen: Rc<RefCell<AdoptScreen>>) {
    let text = padded(&screen.borrow().confirm_text());
    open_confirm_dialog(app, router, ADOPT_TITLE.into(), text.into(), "Adopt".into(), move |app, router| {
        let local_json = app.paths.config_dir.join("ports.local.json");
        let catalog = app.catalog.borrow().clone();
        let done = screens::adopt_selected(&screen.borrow(), &catalog, &app.paths.library_dir, &local_json);
        for port in done.adopted {
            app.state.borrow_mut().mark_installed(port.key(), None);
            let mut catalog = app.catalog.borrow_mut();
            let old = std::mem::take(&mut *catalog);
            *catalog = crate::core::config::merge_local_catalog(old, vec![port]);
        }
        app.refresh_current_view();
        open_message_dialog(app, router, ADOPT_TITLE, &padded(&done.text));
    });
}
