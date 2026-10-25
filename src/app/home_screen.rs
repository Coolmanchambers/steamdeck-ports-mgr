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
use crate::core::models::{Port, SourceType};
use crate::fork::home::{self, Home, List, PortFacts, TileKind};
use crate::ui::gamepad_router::GamepadRouter;
use crate::{AppWindow, HomeTile};
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Router = Rc<RefCell<GamepadRouter>>;

/// How long Steam's play times are reused before `shortcuts.vdf` is read again.
const STEAM_TTL: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(crate) struct HomeUi {
    active: Cell<bool>,
    selected: Cell<usize>,
    empty: Cell<bool>,
    tiles: RefCell<Vec<home::Tile>>,
    /// What the background update check found, by port key.
    updates: Arc<Mutex<HashMap<String, bool>>>,
    /// The number of results already drawn (the poll redraws when more arrive).
    updates_seen: Cell<usize>,
    steam: RefCell<Option<(Instant, HashMap<String, i64>)>>,
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
        window.on_home_browse_requested(move || next_list(&app, &router));
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
    window.set_next_list_text(current_list(app).next().name().into());
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
    let next = current_list(app).next();
    match next {
        List::AllPorts => {
            set_home(app, false);
            app.refresh_current_view();
        }
        List::DeckReady => super::deck_ready_screen::toggle(app, router),
        List::Home => {
            if app.deck_ready.is_active() {
                super::deck_ready_screen::toggle(app, router);
            }
            set_home(app, true);
            app.refresh_current_view();
        }
    }
    app.window().set_next_list_text(current_list(app).next().name().into());
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
    let steam = steam_played(app, installed);
    let state = app.state.borrow();
    installed
        .iter()
        .map(|p| {
            let key = p.key().to_string();
            let app_played = state.get(&key).map_or(0, |i| home::app_played_at(&i.last_played_at));
            PortFacts {
                name: p.display_name().to_string(),
                update_available: updates.get(&key).copied().unwrap_or(false),
                game_files_missing: super::rom_screens::port_missing_game_files(app, p),
                steam_waiting: queued.contains(&key),
                saves_not_found: super::save_finder_screen::saves_warning(app, p),
                last_played: app_played.max(steam.get(&key).copied().unwrap_or(0)),
                key,
            }
        })
        .collect()
}

/// When Steam last started each port's shortcut (its program inside the port's folder), from every account's
/// `shortcuts.vdf`; reused for half a minute.
fn steam_played(app: &AppState, installed: &[Port]) -> HashMap<String, i64> {
    if let Some((at, known)) = app.home.steam.borrow().as_ref() {
        if at.elapsed() < STEAM_TTL {
            return known.clone();
        }
    }
    let mut out = HashMap::new();
    if let Some(home) = home_dir() {
        let library = app.paths.library_dir();
        let dirs: Vec<(String, PathBuf)> = installed.iter().filter_map(|p| crate::core::path_safety::safe_join(&library, &p.folder).ok().map(|d| (p.key().to_string(), d))).collect();
        for account in crate::fork::port_mover::steam::device_accounts(&home) {
            let Ok(file) = crate::fork::store::ShortcutsFile::load(&account.shortcuts) else { continue };
            for s in crate::fork::shortcuts::list(&file.doc) {
                if s.last_play_time == 0 {
                    continue;
                }
                for (key, dir) in &dirs {
                    if inside(s.exe_path(), dir) {
                        let t = out.entry(key.clone()).or_insert(0i64);
                        *t = (*t).max(i64::from(s.last_play_time));
                    }
                }
            }
        }
    }
    *app.home.steam.borrow_mut() = Some((Instant::now(), out.clone()));
    out
}

fn inside(exe: &str, dir: &Path) -> bool {
    crate::fork::port_mover::steam::moved_path(exe, dir, dir).is_some()
}

// -- The update check -----------------------------------------------------------------------------------------

/// Once per start, in the background: the same check Play runs (the port's newest release against the installed
/// one), for the installed ports that follow a GitHub or GitLab release, when "check for updates" is on.
fn start_update_check(app: &Rc<AppState>) {
    let (wanted, tokens) = {
        let state = app.state.borrow();
        let wanted: Vec<(Port, Option<String>, String)> = if state.release_sync {
            app.catalog
                .borrow()
                .iter()
                .filter(|p| matches!(p.source_type, SourceType::Github | SourceType::Gitlab) && app.is_installed(p))
                .filter_map(|p| {
                    let info = state.get(p.key())?;
                    Some((p.clone(), info.installed_tag.clone(), info.installed_at.clone()))
                })
                .collect()
        } else {
            Vec::new()
        };
        (wanted, app.api_tokens())
    };
    if wanted.is_empty() {
        return;
    }
    let total = wanted.len();
    let results = app.home.updates.clone();
    std::thread::spawn(move || {
        for (port, tag, installed_at) in wanted {
            let available = crate::core::jobs::run_update_check(&port, tag.as_deref(), &installed_at, tokens.0.as_deref(), tokens.1.as_deref());
            if let Ok(mut r) = results.lock() {
                r.insert(port.key().to_string(), available);
            }
        }
    });
    let app = app.clone();
    UPDATE_POLL.with(|t| {
        t.start(slint::TimerMode::Repeated, Duration::from_secs(2), move || {
            let n = app.home.updates.lock().map(|r| r.len()).unwrap_or(0);
            if n != app.home.updates_seen.get() {
                app.home.updates_seen.set(n);
                if app.home.is_active() {
                    app.refresh_current_view();
                }
            }
            if n >= total {
                UPDATE_POLL.with(slint::Timer::stop);
            }
        });
    });
}
