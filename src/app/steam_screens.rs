//! The "Add to Steam" screen (Settings menu); "Import ports" is in `import_screen.rs`.
//!
//! The main screen stays a list on purpose (the picker dialog upstream already has): choosing a row toggles it, and
//! the last rows review and add. Its questions and notices are pages (`crate::fork::steam_pages`): which Steam
//! account, the Steam changes waiting (drop one, clear all, each asked with No first), the review ("Add these to
//! Steam?"), letting changes wait, and the notices ("Steam changes waiting", "Added to Steam"). B on a page goes
//! back to the list. The decisions are made in `crate::fork::screens` (tested without any UI). The text is English
//! only; it is not part of upstream's translation tables.
//!
//! With Steam running (always, in Game Mode) the review row turns into "Let N changes wait": the
//! chosen changes go to `steam-queue.json` and `start_queue_watch` offers to apply them once Steam is
//! seen closed (at startup, then every 30 seconds). With Steam running in Desktop Mode, queuing, the
//! "apply now" row and the watch (once per run) offer "Close Steam and update shortcuts"
//! (`steam_close_screen`). The decisions live in `crate::fork::steam_queue` and `crate::fork::steam_close`.

use super::dialogs::{build_picker_dialog, close_current_dialog, dialog_is_open, finish_dialog_open, set_picker_items, DialogSlot};
use super::page_dialog::{show_page, Handlers};
use super::state::AppState;
use crate::fork::screens::{self, SteamLine, SteamScreen};
use crate::fork::steam_pages::{self, SteamAct, WAITING};
use crate::fork::settings::{ForkSettings, FILE_NAME};
use super::steam_close_screen;
use crate::fork::steam_queue::{self, WatchAction, WatchMemory};
use crate::fork::steam_user::SteamAccount;
use crate::fork::store::{backup_stamp, SteamProcessCheck, SystemSteamCheck};
use crate::ui::gamepad_router::GamepadRouter;
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

type Router = Rc<RefCell<GamepadRouter>>;

const STEAM_TITLE: &str = steam_pages::TITLE;

fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"].iter().find_map(|v| std::env::var_os(v)).map(PathBuf::from).filter(|p| p.is_absolute())
}

pub(crate) fn settings_path(app: &AppState) -> PathBuf {
    app.paths.config_dir.join(FILE_NAME)
}

fn queue_path(app: &AppState) -> PathBuf {
    steam_queue::queue_path(&app.paths.config_dir)
}

fn no_home(app: &Rc<AppState>, router: &Router, title: &str) {
    notice(app, router, title, "Your home folder was not found", "Could not find your home folder, so there is nowhere to look for Steam.", Rc::new(close_current_dialog));
}

// -- Add to Steam ---------------------------------------------------------------------------

/// Settings > Add to Steam: the full list. Fork (steam add page): the Port page and Fix problems no longer open it with
/// one port selected; they ask on `steam_add_screen`'s pages.
pub(crate) fn open_steam_screen(app: &Rc<AppState>, router: &Router) {
    if dialog_is_open(app) {
        return;
    }
    let Some(home) = home_dir() else { return no_home(app, router, STEAM_TITLE) };
    let settings = ForkSettings::load(&settings_path(app));
    let mut screen = {
        let catalog = app.catalog.borrow();
        let state = app.state.borrow();
        SteamScreen::load(&home, &app.paths.library_dir(), &catalog, &settings, &|key| state.get(key).and_then(|i| i.favorite_exe.clone()))
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
pub(crate) fn prefetch_art(app: &AppState) {
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
    screen.set_account(index, &app.paths.library_dir(), &catalog, &|key| state.get(key).and_then(|i| i.favorite_exe.clone()));
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
                    return notice(&app, &router, WAITING, WAITING, &note, back_to_list(&screen));
                }
                Some(SteamLine::ApplyQueue) => return offer_queue(&app, &router, &steam, back_to_list(&screen)),
                Some(SteamLine::ManageQueue) => return show_waiting(&app, &router),
                Some(SteamLine::Account) => {
                    if screen.borrow().accounts.len() > 1 {
                        return choose_account(&app, &router, screen.clone());
                    }
                }
                Some(SteamLine::FixIcons) => {
                    // Fork: the Steam icons page; leaving it brings this screen back.
                    super::steam_icons_screen::open(&app, &router, Rc::new(|app, router| open_steam_screen(app, router)));
                    return;
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

type Back = Rc<dyn Fn(&Rc<AppState>, &Router)>;

/// Back to the list as it was (the ports switched on or off stay so).
fn back_to_list(screen: &Rc<RefCell<SteamScreen>>) -> Back {
    let screen = screen.clone();
    Rc::new(move |app, router| {
        close_current_dialog(app, router);
        show_steam_screen(app, router, screen.clone());
    })
}

/// Back to a fresh list (the changes waiting were changed).
fn fresh_list() -> Back {
    Rc::new(|app, router| {
        close_current_dialog(app, router);
        open_steam_screen(app, router);
    })
}

/// A notice page: OK and B run `then`, the title bar's X closes everything.
fn notice(app: &Rc<AppState>, router: &Router, window: &str, title: &str, text: &str, then: Back) {
    let page = steam_pages::notice_page(window, title, text);
    let on_back = then.clone();
    let handlers = Handlers { on_action: Rc::new(move |app, router, _| then(app, router)), on_back: Rc::new(move |app, router| on_back(app, router)), on_exit: Rc::new(close_current_dialog), on_input: None };
    show_page(app, router, page, handlers);
}

/// A question page: Yes runs `yes`, No and B run `no`.
fn ask(app: &Rc<AppState>, router: &Router, page: crate::fork::page::Page<SteamAct>, yes: Back, no: Back) {
    let on_back = no.clone();
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| match act {
            SteamAct::Yes => yes(app, router),
            SteamAct::No => no(app, router),
            _ => {}
        }),
        on_back: Rc::new(move |app, router| on_back(app, router)),
        on_exit: Rc::new(close_current_dialog),
        on_input: None,
    };
    show_page(app, router, page, handlers);
}

/// Which Steam account: a choice page, the one in use marked. Choosing one remembers it and reloads the list.
fn choose_account(app: &Rc<AppState>, router: &Router, screen: Rc<RefCell<SteamScreen>>) {
    let (labels, current): (Vec<String>, Option<usize>) = {
        let s = screen.borrow();
        (s.accounts.iter().map(SteamAccount::label).collect(), s.account)
    };
    let back = back_to_list(&screen);
    let on_back = back.clone();
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| match act {
            SteamAct::Account(choice) => {
                let Some(account) = screen.borrow().accounts.get(choice).cloned() else { return };
                let mut settings = ForkSettings::load(&settings_path(app));
                if let Err(e) = settings.set_steam_account(account.account_id) {
                    eprintln!("add to steam: could not remember the account: {e}");
                    return notice(app, router, STEAM_TITLE, "Could not remember the choice", "The app's settings could not be saved, so the account in use stays as it was.", back.clone());
                }
                reload_after_account_change(app, &mut screen.borrow_mut(), choice);
                back(app, router);
            }
            SteamAct::Back => back(app, router),
            _ => {}
        }),
        on_back: Rc::new(move |app, router| on_back(app, router)),
        on_exit: Rc::new(close_current_dialog),
        on_input: None,
    };
    show_page(app, router, steam_pages::account_page(&labels, current), handlers);
}

fn review_steam_changes(app: &Rc<AppState>, router: &Router, screen: Rc<RefCell<SteamScreen>>) {
    let steam = SystemSteamCheck::new();
    let back = back_to_list(&screen);
    // Work out everything first, so no borrow of the screen is held while pages are swapped.
    let queued = {
        let screen = screen.borrow();
        screen.can_queue(&steam).then(|| {
            let changes = steam_queue::changes_from_screen(&screen, &backup_stamp());
            // The account is named only when the Deck has more than one.
            let account = (screen.accounts.len() > 1).then(|| screen.active_account().map(SteamAccount::label)).flatten();
            (changes, screen.game_mode, account)
        })
    };
    if let Some((changes, game_mode, account)) = queued {
        return confirm_queue(app, router, changes, game_mode, account, back);
    }
    // Steam is closed but changes still wait (a moved port's shortcut that still points at its old copy): writing
    // adds now could add a second shortcut for that port, so the waiting ones go first (the first row).
    if !screen.borrow().queue.is_empty() {
        return notice(app, router, STEAM_TITLE, "Some changes are still waiting", "Make the Steam changes that are waiting first (the first row of the list), then review the rest.", back);
    }
    let problem = screen.borrow().review_problem(&steam);
    if let Some(message) = problem {
        return notice(app, router, STEAM_TITLE, "Nothing was added yet", &message, back);
    }
    let (added, updated) = screen.borrow().review_items();
    let yes: Back = {
        let back = back.clone();
        Rc::new(move |app, router| {
            let settings = ForkSettings::load(&settings_path(app));
            let result = screens::apply_selected(&screen.borrow(), &SystemSteamCheck::new(), &settings, SystemTime::now());
            match result {
                Ok(done) => notice(app, router, STEAM_TITLE, "Added to Steam", &done.text, fresh_list()),
                Err(message) => notice(app, router, STEAM_TITLE, "Could not add to Steam", &message, back.clone()),
            }
        })
    };
    ask(app, router, steam_pages::review_page(&added, &updated), yes, back);
}

// -- Changes that wait while Steam runs -----------------------------------------------------

fn confirm_queue(app: &Rc<AppState>, router: &Router, changes: Vec<steam_queue::QueuedChange>, game_mode: bool, account: Option<String>, back: Back) {
    let page = steam_pages::wait_page(&changes, game_mode, account.as_deref());
    let yes: Back = Rc::new(move |app, router| match steam_queue::queue_changes(&queue_path(app), changes.clone(), game_mode) {
        // Desktop Mode: Steam can be closed from here, so the next question is whether to do that now.
        Ok(_) if !game_mode => {
            let n = changes.len();
            let lead = format!("{n} change{} now waiting.", if n == 1 { "" } else { "s" });
            close_steam_flow(app, router, steam_close_screen::Opts { ask_first: true, lead: Some(lead), ..Default::default() }, true);
        }
        Ok(text) => notice(app, router, WAITING, WAITING, &text, fresh_list()),
        Err(text) => notice(app, router, STEAM_TITLE, "Could not save the changes", &text, fresh_list()),
    });
    ask(app, router, page, yes, back);
}

/// A line for the question when covers for waiting adds are not downloaded yet.
fn missing_art_lead(app: &AppState, queue: &steam_queue::SteamQueue) -> Option<String> {
    let missing = steam_queue::missing_art(queue, &app.catalog.borrow(), &app.paths.cache_dir);
    (!missing.is_empty()).then(|| format!("Artwork for {} is not downloaded yet: choose Not now and wait a minute to add it with the artwork.", missing.join(", ")))
}

/// The shared "Close Steam and update shortcuts" flow (`steam_close_screen`); `back_to_screen` reopens Add to
/// Steam when it ends.
fn close_steam_flow(app: &Rc<AppState>, router: &Router, opts: steam_close_screen::Opts, back_to_screen: bool) {
    let then: steam_close_screen::Then = if back_to_screen { Rc::new(|app, router, _| open_steam_screen(app, router)) } else { Rc::new(|_, _, _| {}) };
    steam_close_screen::open(app, router, opts, then);
}

/// The screen's "make them now" row: asks first when Steam is closed (outside Game Mode); with Steam running it
/// offers to close Steam and make them (the shared flow).
fn offer_queue(app: &Rc<AppState>, router: &Router, steam: &dyn SteamProcessCheck, back: Back) {
    let loaded = steam_queue::load(&queue_path(app));
    if let Some(problem) = loaded.problem {
        return notice(app, router, WAITING, WAITING, &problem, back);
    }
    if loaded.queue.is_empty() {
        return notice(app, router, WAITING, WAITING, "Nothing is waiting.", back);
    }
    // Game Mode: the shared flow's page says to finish in Desktop Mode.
    if steam_queue::game_mode_from_env() {
        return close_steam_flow(app, router, steam_close_screen::Opts::default(), true);
    }
    match steam.running_steam_processes() {
        Ok(_) => close_steam_flow(app, router, steam_close_screen::Opts { ask_first: true, lead: missing_art_lead(app, &loaded.queue), ..Default::default() }, true),
        Err(_) => notice(app, router, WAITING, WAITING, "Could not tell whether Steam is running, so nothing will be changed.", back),
    }
}

/// The Deck's Steam accounts, each with the name the account page shows (the folder `SteamScreen::load` would use).
fn device_accounts() -> Vec<(u32, String)> {
    let Some(home) = home_dir() else { return Vec::new() };
    crate::fork::artwork::existing_steam_roots(&home)
        .iter()
        .filter_map(|root| crate::fork::steam_user::discover_accounts(root).ok())
        .find(|accounts| !accounts.is_empty())
        .map(|accounts| accounts.iter().map(|a| (a.account_id, a.label())).collect())
        .unwrap_or_default()
}

/// The Steam changes waiting, one card each: choosing one asks to drop it; the last card asks to clear them all.
fn show_waiting(app: &Rc<AppState>, router: &Router) {
    let loaded = steam_queue::load(&queue_path(app));
    if let Some(problem) = &loaded.problem {
        return notice(app, router, WAITING, WAITING, problem, fresh_list());
    }
    let queue = loaded.queue;
    let page = steam_pages::waiting_page(&queue, &device_accounts(), steam_queue::game_mode_from_env());
    let again: Back = Rc::new(show_waiting);
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| match act {
            SteamAct::Change(i) => {
                let Some(change) = queue.entries.get(i) else { return };
                let mut changed = queue.clone();
                changed.remove(i);
                let yes: Back = Rc::new(move |app, router| match steam_queue::save(&queue_path(app), &changed) {
                    Ok(()) => show_waiting(app, router),
                    Err(e) => notice(app, router, WAITING, "Could not drop the change", &format!("The list of Steam changes waiting could not be saved ({e})."), Rc::new(show_waiting)),
                });
                ask(app, router, steam_pages::drop_page(change), yes, again.clone());
            }
            SteamAct::ClearAll => {
                let yes: Back = Rc::new(|app, router| match steam_queue::save(&queue_path(app), &steam_queue::SteamQueue::default()) {
                    Ok(()) => fresh_list()(app, router),
                    Err(e) => notice(app, router, WAITING, "Could not clear the changes", &format!("The list of Steam changes waiting could not be saved ({e})."), Rc::new(show_waiting)),
                });
                ask(app, router, steam_pages::clear_page(queue.len()), yes, again.clone());
            }
            SteamAct::Back => fresh_list()(app, router),
            _ => {}
        }),
        on_back: fresh_list(),
        on_exit: Rc::new(close_current_dialog),
        on_input: None,
    };
    show_page(app, router, page, handlers);
}

#[derive(Default)]
struct QueueWatch {
    /// What the offers so far were (see `steam_queue::watch_step`).
    memory: Cell<WatchMemory>,
    /// Queue-file problems already shown, so one that moving the file cannot fix is shown once.
    shown_problems: RefCell<Vec<String>>,
}

/// Looks for queued Steam changes shortly after startup and then every 30 seconds, and offers to apply
/// them when Steam is closed outside Game Mode. Keep the returned timer alive for as long as the app runs.
pub(crate) fn start_queue_watch(app: &Rc<AppState>, router: &Router) -> slint::Timer {
    eprintln!("{}", steam_queue::game_mode_log_line());
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

/// The user is not looking at the app: hidden to the tray, minimized (X11 only: winit reports it as
/// unknown on Wayland), or put away for a game.
fn window_away(app: &AppState) -> bool {
    if app.install_runtime.minimized_for_game.get() {
        return true;
    }
    app.window.upgrade().is_none_or(|w| !w.window().is_visible() || w.window().is_minimized())
}

fn check_queue(app: &Rc<AppState>, router: &Router, watch: &QueueWatch) {
    // Never on top of another dialog, while a game started from here runs, or while the window is away;
    // `offered` is left alone so a later look still offers.
    // Fork (ports folder): the "ports folder not available" message comes first.
    // Not during the setup wizard either: it has its own Steam step, and the gaps between its pages are not
    // a moment to pop up a question.
    if super::ports_folder_screen::startup_dialog_pending() || dialog_is_open(app) || super::setup_wizard_screen::active() || !app.install_runtime.running_processes.borrow().is_empty() || window_away(app) {
        return;
    }
    // A versioned update started by Play left a note (fork::versioned): it comes before the offer.
    if super::versioned_update::show_saved_note(app, router, true) {
        return;
    }
    let path = queue_path(app);
    if !path.exists() {
        watch.memory.set(WatchMemory::default());
        return;
    }
    let loaded = steam_queue::load(&path);
    if let Some(problem) = loaded.problem {
        if !watch.shown_problems.borrow().contains(&problem) {
            eprintln!("steam queue: {}", problem.replace('\n', " "));
            watch.shown_problems.borrow_mut().push(problem.clone());
            notice(app, router, WAITING, WAITING, &problem, Rc::new(close_current_dialog));
        }
        return;
    }
    let mut memory = watch.memory.get();
    let steam = SystemSteamCheck::new().running_steam_processes();
    let action = steam_queue::watch_step(loaded.queue.len(), &steam, &mut memory, steam_queue::game_mode_from_env());
    watch.memory.set(memory);
    match action {
        WatchAction::Offer => {
            eprintln!("steam queue: offering {}", loaded.queue.len());
            prefetch_art(app);
            close_steam_flow(app, router, steam_close_screen::Opts { ask_first: true, stray_safe: true, lead: missing_art_lead(app, &loaded.queue), ..Default::default() }, false);
        }
        WatchAction::OfferClose => {
            eprintln!("steam queue: offering to close Steam for {}", loaded.queue.len());
            close_steam_flow(app, router, steam_close_screen::Opts { ask_first: true, stray_safe: true, ..Default::default() }, false);
        }
        WatchAction::Nothing => {}
    }
}
