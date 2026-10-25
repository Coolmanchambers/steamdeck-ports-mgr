//! The home screen: the first thing the app shows (`ui/fork/home.slint`; what it says is `crate::fork::home`).
//!
//! "Your ports" as cover tiles for the installed ports, each with one status badge read from the sources the app
//! already keeps: the update check (the same one Play runs, made once per start in the background), the game files
//! records, the Steam queue and the save finder. A Continue tile comes first for the port played last (the app's play
//! record or Steam's, whichever is later). With nothing installed, one card: Set up (the setup wizard) or Browse ports.
//!
//! The lists: L1/R1, Tab and the footer button go Home, All ports, Deck-ready, then Home again (`next_list`). All ports
//! is upstream's own list (or card grid in big mode), unchanged. Typing on Home goes to All ports with what was typed.
//! On Home, A plays (or opens the wizard on the Setup tile), X opens Info, Y opens Settings, B does nothing.

use super::dialogs::dialog_is_open;
use super::install_launch::activate_port;
use super::state::AppState;
use crate::core::models::Port;
use crate::fork::home::{self, Home, List, PortFacts, TileKind};
use crate::ui::gamepad_router::GamepadRouter;
use crate::{AppWindow, HomeTile};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Router = Rc<RefCell<GamepadRouter>>;

/// How long Steam's play times are reused before `shortcuts.vdf` is read again.
const STEAM_TTL: Duration = Duration::from_secs(30);
/// How long a port's "game files missing" answer is reused (it reads the port's folder and the game files records).
const FILES_TTL: Duration = Duration::from_secs(10);

/// THE SWITCH for the update check at start (Tim decides whether it stays): false turns it off; the badges then show
/// only what an earlier check found, and Play keeps its own check as before.
const CHECK_UPDATES_AT_START: bool = true;
/// `fork-settings.json` keys: when the last round ran, and what each port's check found and when.
const KEY_CHECKED_AT: &str = "home_update_check_at";
const KEY_RESULTS: &str = "home_update_checks";

#[derive(Default)]
pub(crate) struct HomeUi {
    active: Cell<bool>,
    selected: Cell<usize>,
    empty: Cell<bool>,
    tiles: RefCell<Vec<home::Tile>>,
    /// What the update checks found, by port key (loaded from `fork-settings.json`, added to by a round).
    updates: Arc<Mutex<HashMap<String, home::UpdateSeen>>>,
    /// The number of results a round has added so far, and whether it ended (the poll redraws and saves as they come).
    round_added: Arc<Mutex<usize>>,
    round_done: Arc<std::sync::atomic::AtomicBool>,
    updates_seen: Cell<usize>,
    steam: RefCell<Option<(Instant, HashMap<String, i64>)>>,
    files: RefCell<HashMap<String, (Instant, bool)>>,
}

impl HomeUi {
    pub(crate) fn is_active(&self) -> bool {
        self.active.get()
    }
}

thread_local! {
    static UPDATE_POLL: slint::Timer = slint::Timer::default();
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute())
}

pub(crate) fn wire(window: &AppWindow, app: &Rc<AppState>, router: &Router) {
    {
        let (app, router) = (app.clone(), router.clone());
        window.on_home_tile_activated(move |i| {
            select(&app, i.max(0) as usize);
            activate(&app, &router);
        });
    }
    {
        let app = app.clone();
        window.on_home_tile_hovered(move |i| select(&app, i.max(0) as usize));
    }
    {
        let app = app.clone();
        window.on_home_empty_hovered(move |i| select(&app, i.max(0) as usize));
    }
    {
        let (app, router) = (app.clone(), router.clone());
        window.on_home_setup_requested(move || open_setup(&app, &router));
    }
    {
        let (app, router) = (app.clone(), router.clone());
        window.on_home_browse_requested(move || browse(&app, &router));
    }
    start(app);
}

/// At start: the app opens on Home, and the update check runs once in the background.
fn start(app: &Rc<AppState>) {
    set_home(app, true);
    eprintln!("list: {}", List::Home.name());
    app.refresh_current_view();
    start_update_check(app);
}

fn set_home(app: &AppState, on: bool) {
    app.home.active.set(on);
    let window = app.window();
    window.set_home_mode(on);
    if on {
        window.invoke_clear_search();
        app.windowed_nav.search_query.borrow_mut().clear();
    }
    window.set_next_list_text(current_list(app).next_in(window.get_big_mode()).name().into());
}

/// The list on screen now.
pub(crate) fn current_list(app: &AppState) -> List {
    if app.home.is_active() {
        List::Home
    } else if app.deck_ready.is_active() {
        List::DeckReady
    } else {
        List::AllPorts
    }
}

/// L1/R1, Tab, the footer button and Browse ports: the next list (Home, All ports, Deck-ready, Home).
pub(crate) fn next_list(app: &Rc<AppState>, router: &Router) {
    if dialog_is_open(app) {
        return;
    }
    go_to(app, router, current_list(app).next_in(app.window().get_big_mode()));
}

/// "Browse ports" on the empty card: a list of ports to install (All ports windowed; in big mode, where All ports
/// shows only installed ports, the Deck-ready list).
fn browse(app: &Rc<AppState>, router: &Router) {
    if !dialog_is_open(app) {
        go_to(app, router, List::browse(app.window().get_big_mode()));
    }
}

fn go_to(app: &Rc<AppState>, router: &Router, next: List) {
    if current_list(app) == next {
        return;
    }
    match next {
        List::AllPorts => {
            if app.deck_ready.is_active() {
                super::deck_ready_screen::toggle(app, router);
            }
            set_home(app, false);
            app.refresh_current_view();
        }
        List::DeckReady => {
            if app.home.is_active() {
                set_home(app, false);
            }
            super::deck_ready_screen::toggle(app, router);
        }
        List::Home => {
            if app.deck_ready.is_active() {
                super::deck_ready_screen::toggle(app, router);
            }
            set_home(app, true);
            app.refresh_current_view();
        }
    }
    let window = app.window();
    window.set_next_list_text(current_list(app).next_in(window.get_big_mode()).name().into());
    eprintln!("list: {}", current_list(app).name());
}

/// Redraws Home. False when it gave way to All ports (something was typed): the caller draws that list instead.
pub(crate) fn rebuild(app: &AppState) -> bool {
    if !app.windowed_nav.search_query.borrow().trim().is_empty() {
        // Typing on Home searches All ports.
        set_home(app, false);
        eprintln!("list: {}", List::AllPorts.name());
        return false;
    }
    let installed: Vec<Port> = app.catalog.borrow().iter().filter(|p| !p.is_android_only() && app.is_installed(p)).cloned().collect();
    let facts = facts_of(app, &installed);
    let setup_done = crate::fork::setup_wizard::setup_state(&app.paths.config_dir.join(crate::fork::settings::FILE_NAME)) == crate::fork::setup_wizard::SetupState::Done;
    let window = app.window();
    match home::build(&facts, setup_done) {
        Home::Empty => {
            app.home.empty.set(true);
            app.home.tiles.borrow_mut().clear();
            app.home.selected.set(app.home.selected.get().min(1));
            window.set_home_empty(true);
            window.set_home_tiles(slint::ModelRc::new(slint::VecModel::from(Vec::<HomeTile>::new())));
        }
        Home::Tiles(tiles) => {
            app.home.empty.set(false);
            let by_key: HashMap<&str, &Port> = installed.iter().map(|p| (p.key(), p)).collect();
            let items: Vec<HomeTile> = tiles
                .iter()
                .map(|t| {
                    let image = by_key.get(t.key.as_str()).map(|p| super::cards::load_cached_card_image(&app.grid_nav.card_image_cache, &app.paths.cache_dir, &p.folder)).unwrap_or_default();
                    let badge = if t.kind == TileKind::Setup { String::new() } else { t.badge.label().to_string() };
                    HomeTile {
                        title: t.title.clone().into(),
                        image,
                        badge: badge.into(),
                        mark: t.badge.mark().code(),
                        kind: match t.kind {
                            TileKind::Port => 0,
                            TileKind::Continue => 1,
                            TileKind::Setup => 2,
                        },
                    }
                })
                .collect();
            app.home.selected.set(app.home.selected.get().min(items.len().saturating_sub(1)));
            *app.home.tiles.borrow_mut() = tiles;
            window.set_home_empty(false);
            window.set_home_tiles(slint::ModelRc::new(slint::VecModel::from(items)));
            app.ensure_card_images_cached(&installed);
        }
    }
    window.set_home_columns(columns(app) as i32);
    let (cloud, cloud_text) = super::cloud_screen::home_status(app);
    window.set_home_cloud(cloud);
    window.set_home_cloud_text(cloud_text.into());
    push_selected(app);
    true
}

/// How many tiles fit a row: about 210 px a tile in big mode, 160 px windowed. From the size the mode gives the window
/// (not its size this moment, which follows a mode change only a little later).
fn columns(app: &AppState) -> usize {
    let big = app.window().get_big_mode();
    let width = if big { app.window_geometry.fullscreen_mode.borrow().logical_width } else { app.window_geometry.normal_mode.borrow().logical_width };
    let tile = if big { 210.0 } else { 160.0 };
    ((width as f32 / tile).floor() as usize).clamp(2, 8)
}

fn focused_kind(app: &AppState) -> Option<TileKind> {
    if app.home.empty.get() {
        return None;
    }
    app.home.tiles.borrow().get(app.home.selected.get()).map(|t| t.kind)
}

fn push_selected(app: &AppState) {
    let window = app.window();
    window.set_home_selected(app.home.selected.get() as i32);
    window.set_home_hint(home::hints(focused_kind(app)).into());
    window.set_scroll_trigger(!window.get_scroll_trigger());
}

fn select(app: &AppState, i: usize) {
    let len = if app.home.empty.get() { 2 } else { app.home.tiles.borrow().len() };
    if i < len {
        app.home.selected.set(i);
        push_selected(app);
    }
}

pub(crate) fn move_selection(app: &AppState, dx: i32, dy: i32) {
    if app.home.empty.get() {
        if dx != 0 {
            select(app, if dx > 0 { 1 } else { 0 });
        }
        return;
    }
    let len = app.home.tiles.borrow().len();
    let cols = columns(app).max(1);
    let at = app.home.selected.get();
    if let Some((r, c)) = crate::core::grid::next_grid_position((at / cols, at % cols), dx, dy, cols, len) {
        select(app, r * cols + c);
    }
}

/// The port of the focused tile (Info, Open folder); `None` on the Setup tile and the empty card.
pub(crate) fn selected_port(app: &AppState) -> Option<Port> {
    if app.home.empty.get() {
        return None;
    }
    let key = app.home.tiles.borrow().get(app.home.selected.get()).filter(|t| t.kind != TileKind::Setup).map(|t| t.key.clone())?;
    app.catalog.borrow().iter().find(|p| p.key() == key).cloned()
}

/// A on Home: play the port (Continue too), set up on the Setup tile; on the empty card, Set up or Browse ports.
pub(crate) fn activate(app: &Rc<AppState>, router: &Router) {
    if app.home.empty.get() {
        return if app.home.selected.get() == 0 { open_setup(app, router) } else { next_list(app, router) };
    }
    if focused_kind(app) == Some(TileKind::Setup) {
        return open_setup(app, router);
    }
    if let Some(port) = selected_port(app) {
        activate_port(app, router, &port);
    }
}

fn open_setup(app: &Rc<AppState>, router: &Router) {
    if !dialog_is_open(app) {
        super::setup_wizard_screen::open_from_settings(app, router);
    }
}

// -- Facts ----------------------------------------------------------------------------------------------------

fn facts_of(app: &AppState, installed: &[Port]) -> Vec<PortFacts> {
    let queued: HashSet<String> = crate::fork::steam_queue::load(&crate::fork::steam_queue::queue_path(&app.paths.config_dir)).queue.entries.iter().map(|e| e.port_key.clone()).collect();
    let updates = app.home.updates.lock().map(|u| u.clone()).unwrap_or_default();
    let files = files_missing(app, installed);
    let steam = steam_played(app, installed);
    let state = app.state.borrow();
    installed
        .iter()
        .map(|p| {
            let key = p.key().to_string();
            let info = state.get(&key);
            let app_played = info.map_or(0, |i| home::app_played_at(&i.last_played_at));
            let installed_at = info.map_or(0, |i| home::app_played_at(&i.installed_at));
            PortFacts {
                name: p.display_name().to_string(),
                update_available: home::update_shown(updates.get(&key).copied(), installed_at),
                game_files_missing: files.get(&key).copied().unwrap_or(false),
                steam_waiting: queued.contains(&key),
                saves_not_found: super::save_finder_screen::saves_warning(app, p),
                last_played: app_played.max(steam.get(&key).copied().unwrap_or(0)),
                key,
            }
        })
        .collect()
}

/// "Game files missing" for each port, each answer reused for a few seconds (Home redraws often: covers arriving, the
/// update poll), since it reads the port's folder and the game files records.
fn files_missing(app: &AppState, installed: &[Port]) -> HashMap<String, bool> {
    let mut cache = app.home.files.borrow_mut();
    installed
        .iter()
        .map(|p| {
            let key = p.key().to_string();
            let fresh = cache.get(&key).filter(|(at, _)| at.elapsed() < FILES_TTL).map(|(_, v)| *v);
            let missing = fresh.unwrap_or_else(|| {
                let v = super::rom_screens::port_missing_game_files(app, p);
                cache.insert(key.clone(), (Instant::now(), v));
                v
            });
            (key, missing)
        })
        .collect()
}

/// When Steam last started each port's shortcut (its program inside the port's folder, the longest matching folder
/// when folders nest), from every account's `shortcuts.vdf`; reused for half a minute.
fn steam_played(app: &AppState, installed: &[Port]) -> HashMap<String, i64> {
    if let Some((at, known)) = app.home.steam.borrow().as_ref() {
        if at.elapsed() < STEAM_TTL {
            return known.clone();
        }
    }
    let mut out = HashMap::new();
    if let Some(home) = home_dir() {
        let library = app.paths.library_dir();
        // Each port's folder once, as the file system names it (a link to the SD card, say).
        let dirs: Vec<(String, PathBuf)> = installed
            .iter()
            .filter_map(|p| crate::core::path_safety::safe_join(&library, &p.folder).ok().map(|d| (p.key().to_string(), std::fs::canonicalize(&d).unwrap_or(d))))
            .collect();
        for account in crate::fork::port_mover::steam::device_accounts(&home) {
            let Ok(file) = crate::fork::store::ShortcutsFile::load(&account.shortcuts) else { continue };
            for s in crate::fork::shortcuts::list(&file.doc) {
                if s.last_play_time == 0 || s.exe_path().is_empty() {
                    continue;
                }
                if let Some(key) = owner_of(Path::new(s.exe_path()), &dirs) {
                    let t = out.entry(key.to_string()).or_insert(0i64);
                    *t = (*t).max(i64::from(s.last_play_time));
                }
            }
        }
    }
    *app.home.steam.borrow_mut() = Some((Instant::now(), out.clone()));
    out
}

/// The port whose folder holds `exe` (as written, or as the file system names it); the longest folder wins.
fn owner_of<'a>(exe: &Path, dirs: &'a [(String, PathBuf)]) -> Option<&'a str> {
    let canonical = std::fs::canonicalize(exe).ok();
    dirs.iter()
        .filter(|(_, d)| exe.starts_with(d) || canonical.as_ref().is_some_and(|c| c.starts_with(d)))
        .max_by_key(|(_, d)| d.as_os_str().len())
        .map(|(k, _)| k.as_str())
}

// -- The update check -----------------------------------------------------------------------------------------

fn settings_path(app: &AppState) -> PathBuf {
    app.paths.config_dir.join(crate::fork::settings::FILE_NAME)
}

fn unix_now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// At start: the badges show what earlier checks found (kept in `fork-settings.json`, so a restart asks nothing).
/// Then, when the switch is on, "check for updates" is on and the last round is 12 hours old or more, one round in the
/// background: Play's own check (`jobs::run_update_check_result`) for each installed port it is due for
/// (`home::update_check_due`, the same rule Play follows), one after another, stopping at the first rate-limit answer.
/// Each request has the API agent's 30 s limit, and the poll stops after the round or at a deadline.
fn start_update_check(app: &Rc<AppState>) {
    let mut settings = crate::fork::settings::ForkSettings::load(&settings_path(app));
    let known = home::updates_from_json(settings.get(KEY_RESULTS));
    if let Ok(mut u) = app.home.updates.lock() {
        *u = known;
    }
    if !CHECK_UPDATES_AT_START {
        return;
    }
    let last = settings.get(KEY_CHECKED_AT).and_then(serde_json::Value::as_i64);
    let now = unix_now();
    if !home::update_round_due(last, now) {
        return;
    }
    let (wanted, tokens) = {
        let state = app.state.borrow();
        let wanted: Vec<(Port, Option<String>, String)> = app
            .catalog
            .borrow()
            .iter()
            .filter(|p| app.is_installed(p))
            .filter_map(|p| {
                let info = state.get(p.key());
                let installed_at = info.map(|i| i.installed_at.clone()).unwrap_or_default();
                home::update_check_due(state.release_sync, &p.source_type, info.map(|i| i.update), &installed_at).then(|| (p.clone(), info.and_then(|i| i.installed_tag.clone()), installed_at))
            })
            .collect();
        (wanted, app.api_tokens())
    };
    if wanted.is_empty() {
        return;
    }
    // Stamped before it starts, so a crash or a quit half way does not make every start ask again.
    if let Err(e) = settings.set(KEY_CHECKED_AT, serde_json::json!(now)) {
        eprintln!("home: could not note the update check time ({e})");
    }
    eprintln!("home: checking {} installed ports for updates", wanted.len());
    let deadline = Instant::now() + Duration::from_secs(35 * wanted.len() as u64 + 10);
    let (results, added, done) = (app.home.updates.clone(), app.home.round_added.clone(), app.home.round_done.clone());
    std::thread::spawn(move || {
        for (port, tag, installed_at) in wanted {
            match crate::core::jobs::run_update_check_result(&port, tag.as_deref(), &installed_at, tokens.0.as_deref(), tokens.1.as_deref()) {
                Ok(available) => {
                    if let Ok(mut r) = results.lock() {
                        r.insert(port.key().to_string(), home::UpdateSeen { available, at: unix_now() });
                    }
                    if let Ok(mut n) = added.lock() {
                        *n += 1;
                    }
                }
                Err(e) if home::is_rate_limit(&e) => {
                    eprintln!("home: update check stopped: {e}");
                    break;
                }
                // No network, no release: the last known answer stays.
                Err(e) => eprintln!("home: no update answer for {}: {e}", port.key()),
            }
        }
        done.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    let app = app.clone();
    UPDATE_POLL.with(|t| {
        t.start(slint::TimerMode::Repeated, Duration::from_secs(2), move || {
            let n = app.home.round_added.lock().map(|n| *n).unwrap_or(0);
            let finished = app.home.round_done.load(std::sync::atomic::Ordering::Relaxed) || Instant::now() >= deadline;
            if n != app.home.updates_seen.get() {
                app.home.updates_seen.set(n);
                save_results(&app);
                if app.home.is_active() {
                    app.refresh_current_view();
                }
            }
            if finished {
                UPDATE_POLL.with(slint::Timer::stop);
            }
        });
    });
}

fn save_results(app: &AppState) {
    let Ok(map) = app.home.updates.lock().map(|m| m.clone()) else { return };
    let mut settings = crate::fork::settings::ForkSettings::load(&settings_path(app));
    if let Err(e) = settings.set(KEY_RESULTS, home::updates_to_json(&map)) {
        eprintln!("home: could not keep the update check results ({e})");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shortcut_belongs_to_the_port_with_the_longest_matching_folder() {
        let dirs = vec![("outer".to_string(), PathBuf::from("/lib/Games")), ("inner".to_string(), PathBuf::from("/lib/Games/Soh"))];
        assert_eq!(owner_of(Path::new("/lib/Games/Soh/soh.appimage"), &dirs), Some("inner"));
        assert_eq!(owner_of(Path::new("/lib/Games/other/x"), &dirs), Some("outer"));
        assert_eq!(owner_of(Path::new("/elsewhere/x"), &dirs), None);
    }
}
