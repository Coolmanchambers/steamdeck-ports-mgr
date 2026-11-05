//! The Deck-ready list screen: a second list in the main window beside All ports.
//!
//! L1/R1 (or Tab, or the footer button) switch lists. In this list A installs or plays, X opens Info,
//! Y opens Filters and B goes back to All ports. What each row shows and does is decided in
//! `crate::fork::deck_ready::view` (tested without any UI); this file draws it and runs the actions.
//! Ports the app already has go through the normal install and launch flow; an entry it doesn't have is
//! first added to `ports.local.json` as the user's own entry. The text is English only.

use super::dialogs::{close_current_dialog, dialog_is_open, open_message_dialog};
use super::page_dialog::{show_page, Handlers};
use crate::fork::deck_ready::pages::{self, DrAct};
use super::events::{lock, AppEvent};
use super::install_launch::activate_port;
use super::state::AppState;
use crate::fork::deck_ready::index::{Host, Index};
use crate::fork::deck_ready::sync::{self, Meta, Outcome};
use crate::fork::deck_ready::view::{self, Action, Filters, KnownPort, Row, TagKind};
use crate::fork::deck_ready::entry;
use crate::ui::gamepad_router::GamepadRouter;
use crate::{AppWindow, DeckReadyInfoDialog, DeckReadyRow, DeckReadyTag};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

type Router = Rc<RefCell<GamepadRouter>>;

const PAGE_ROWS: i32 = 10;

#[derive(Default)]
pub(crate) struct DeckReadyUi {
    active: Cell<bool>,
    loaded: Cell<bool>,
    fetching: Cell<bool>,
    index: RefCell<Option<Index>>,
    meta: RefCell<Meta>,
    error: RefCell<Option<String>>,
    filters: RefCell<Filters>,
    rows: RefCell<Vec<Row>>,
    selected: Cell<usize>,
}

impl DeckReadyUi {
    pub(crate) fn is_active(&self) -> bool {
        self.active.get()
    }
}

pub(crate) fn wire(window: &AppWindow, app: &Rc<AppState>, router: &Router) {
    {
        let (app, router) = (app.clone(), router.clone());
        // The lists go round: Home, All ports, Deck-ready (src/app/home_screen.rs).
        window.on_deck_ready_toggle_requested(move || super::home_screen::next_list(&app, &router));
    }
    {
        let app = app.clone();
        window.on_deck_ready_row_hovered(move |i| select(&app, i));
    }
    {
        let (app, router) = (app.clone(), router.clone());
        window.on_deck_ready_row_info(move |i| {
            select(&app, i);
            show_info(&app, &router);
        });
    }
    {
        let (app, router) = (app.clone(), router.clone());
        window.on_deck_ready_row_action(move |i| {
            select(&app, i);
            activate(&app, &router);
        });
    }
    {
        let (app, router) = (app.clone(), router.clone());
        window.on_deck_ready_filters_requested(move || open_filters(&app, &router));
    }
    {
        let (app, router) = (app.clone(), router.clone());
        window.on_deck_ready_row_options(move |i| row_options(&app, &router, i));
    }
    super::home_screen::wire(window, app, router);
}

/// Switches between All ports and the Deck-ready list. The search text is cleared either way.
pub(crate) fn toggle(app: &Rc<AppState>, _router: &Router) {
    if dialog_is_open(app) {
        return;
    }
    let ui = &app.deck_ready;
    let now_active = !ui.active.get();
    ui.active.set(now_active);
    let window = app.window();
    window.invoke_clear_search();
    app.windowed_nav.search_query.borrow_mut().clear();
    window.set_deck_ready_mode(now_active);
    if now_active {
        if !ui.loaded.get() {
            ui.loaded.set(true);
            *ui.index.borrow_mut() = sync::load_cached(&app.paths.cache_dir);
            *ui.meta.borrow_mut() = sync::load_meta(&app.paths.cache_dir);
        }
        ui.selected.set(0);
        start_fetch(app, false);
    }
    app.refresh_current_view();
}

/// The Deck-ready list for Import ports: the one this screen holds, else the saved copy (`None` when the
/// list was never fetched).
pub(crate) fn index_for_import(app: &AppState) -> Option<Index> {
    app.deck_ready.index.borrow().clone().or_else(|| sync::load_cached(&app.paths.cache_dir))
}

/// Fetches in the background when the saved copy is missing or older than 12 hours (or always with
/// `force`). The saved copy stays on screen meanwhile.
fn start_fetch(app: &AppState, force: bool) {
    let ui = &app.deck_ready;
    if ui.fetching.get() || (!force && ui.index.borrow().is_some() && !sync::is_due(&ui.meta.borrow(), chrono::Utc::now())) {
        return;
    }
    ui.fetching.set(true);
    let cache_dir = app.paths.cache_dir.clone();
    let events = app.events.clone();
    std::thread::spawn(move || {
        let outcome = sync::refresh(&cache_dir, chrono::Utc::now(), sync::fetch_from_github);
        lock(&events).push(AppEvent::DeckReadyFetched(Box::new(outcome)));
    });
}

pub(crate) fn on_fetched(app: &AppState, outcome: Outcome) {
    let ui = &app.deck_ready;
    ui.fetching.set(false);
    if let Some(index) = outcome.index {
        *ui.index.borrow_mut() = Some(index);
    }
    *ui.meta.borrow_mut() = outcome.meta;
    *ui.error.borrow_mut() = outcome.error;
    if ui.active.get() {
        rebuild(app);
    }
}

/// The catalog's ports as the list matches them, and (fork, same game) the installed ones as games: one look at the
/// disk per port for both.
fn known_ports(app: &AppState) -> (Vec<KnownPort>, Vec<crate::fork::same_game::Owned>) {
    let on_disk = super::same_game_screen::Installed::read(app);
    let known = app
        .catalog
        .borrow()
        .iter()
        .map(|p| KnownPort { key: p.key().to_string(), folder: p.folder.clone(), repo: p.repo.clone(), user_managed: p.user_managed, installed: on_disk.has(p) })
        .collect();
    (known, on_disk.owned)
}

fn tag_of(t: &crate::fork::deck_ready::view::Tag) -> DeckReadyTag {
    DeckReadyTag {
        label: t.label.into(),
        kind: match t.kind {
            TagKind::Plain => 0,
            TagKind::Dashed => 1,
            TagKind::Warning => 2,
            TagKind::Muted => 3,
            TagKind::Info => 4,
        },
    }
}

fn tag_model(row: &Row) -> slint::ModelRc<DeckReadyTag> {
    let tags: Vec<DeckReadyTag> = row.tags.iter().map(tag_of).collect();
    slint::ModelRc::new(slint::VecModel::from(tags))
}

fn status_text(ui: &DeckReadyUi, count: usize) -> String {
    let shown = if count == 1 { "1 port".to_string() } else { format!("{count} ports") };
    let index = ui.index.borrow();
    let error = ui.error.borrow();
    let Some(index) = index.as_ref() else {
        return if ui.fetching.get() { "Loading the Deck-ready list...".to_string() } else { String::new() };
    };
    let changed = view::date_long(&index.generated).unwrap_or_else(|| "unknown".to_string());
    if let Some(e) = error.as_ref() {
        let why = if e.contains("not usable") { "the new download was not usable" } else { "GitHub could not be reached" };
        return format!("{shown}  ·  showing the saved list from {changed}; {why}");
    }
    let checked = view::checked_label(&ui.meta.borrow().checked_at, &chrono::Local::now());
    let checking = if ui.fetching.get() { "  ·  checking for a newer list" } else { "" };
    format!("{shown}  ·  list changed {changed}  ·  checked {checked}{checking}")
}

fn empty_text(ui: &DeckReadyUi) -> String {
    if ui.index.borrow().is_some() {
        return "Nothing matches. Press Y to change the filters, or clear the search.".to_string();
    }
    match (ui.fetching.get(), ui.error.borrow().as_ref()) {
        (true, _) => String::new(),
        (false, Some(e)) => format!("Can't load the Deck-ready list yet ({e}). Check your connection, then press A to try again."),
        (false, None) => "The Deck-ready list isn't loaded yet. Press A to load it.".to_string(),
    }
}

/// Recomputes the rows (filters, search, what the app has installed) and pushes them to the window,
/// keeping the selection on the same entry when it is still shown. A rebuild (the background download, a search) that
/// moves the selected row brings it into view; one that leaves it where it was does not touch the scroll.
pub(crate) fn rebuild(app: &AppState) {
    if rebuild_rows(app) {
        scroll_to_selection(&app.window());
    }
}

/// [`rebuild`] without the scroll: true when the selection is on another row number now.
fn rebuild_rows(app: &AppState) -> bool {
    let ui = &app.deck_ready;
    let previous = ui.rows.borrow().get(ui.selected.get()).map(|r| r.id.clone());
    let rows = match ui.index.borrow().as_ref() {
        Some(index) => {
            let (known, owned) = known_ports(app);
            let mut rows = view::rows(index, &known, &ui.filters.borrow(), &app.windowed_nav.search_query.borrow());
            // Same game: rows for a game installed under another project say so (src/fork/same_game.rs).
            let log = view::mark_same_game(&mut rows, index, &owned);
            super::same_game_screen::log_matches("deck-ready", log);
            rows
        }
        None => Vec::new(),
    };
    let selected = previous.and_then(|id| rows.iter().position(|r| r.id == id)).unwrap_or(0).min(rows.len().saturating_sub(1));
    let items: Vec<DeckReadyRow> =
        rows.iter().map(|r| DeckReadyRow { name: r.name.clone().into(), sub: r.sub.clone().into(), deck: r.deck, tags: tag_model(r), badge: r.badge.as_ref().map(tag_of).unwrap_or_default(), button: r.button.into() }).collect();
    let window = app.window();
    window.set_deck_ready_rows(slint::ModelRc::new(slint::VecModel::from(items)));
    window.set_deck_ready_status(status_text(ui, rows.len()).into());
    window.set_deck_ready_filters(view::filter_summary(&ui.filters.borrow()).into());
    window.set_deck_ready_empty_text(empty_text(ui).into());
    window.set_deck_ready_selected(if rows.is_empty() { -1 } else { selected as i32 });
    let moved = selection_moved(ui.selected.get(), selected, rows.len());
    ui.selected.set(selected);
    *ui.rows.borrow_mut() = rows;
    moved
}

/// True when a rebuild put the selection on another row number (the same port moved, or another port took over).
fn selection_moved(before: usize, after: usize, rows: usize) -> bool {
    rows > 0 && before != after
}

fn scroll_to_selection(window: &AppWindow) {
    window.set_scroll_trigger(!window.get_scroll_trigger());
}

pub(crate) fn select(app: &AppState, index: i32) {
    let ui = &app.deck_ready;
    if index < 0 || index as usize >= ui.rows.borrow().len() {
        return;
    }
    ui.selected.set(index as usize);
    app.window().set_deck_ready_selected(index);
}

/// D-pad up and down move one row; left and right move a page, as in All ports.
pub(crate) fn move_selection(app: &AppState, dx: i32, dy: i32) {
    let ui = &app.deck_ready;
    let len = ui.rows.borrow().len();
    if len == 0 {
        return;
    }
    let delta = if dx != 0 { dx * PAGE_ROWS } else { dy };
    let next = (ui.selected.get() as i32 + delta).clamp(0, len as i32 - 1) as usize;
    if next != ui.selected.get() {
        ui.selected.set(next);
        let window = app.window();
        window.set_deck_ready_selected(next as i32);
        scroll_to_selection(&window);
    }
}

/// Fork (port page): the cursor is on the first row (or there is none): Up from here reaches the Home button.
pub(crate) fn at_top(app: &AppState) -> bool {
    app.deck_ready.selected.get() == 0 || app.deck_ready.rows.borrow().is_empty()
}

/// Fork (port page): a right-click on a row: the Port page of a port the app has installed, else its project page.
pub(crate) fn row_options(app: &Rc<AppState>, router: &Router, index: i32) {
    if dialog_is_open(app) {
        return;
    }
    select(app, index);
    let Some(row) = selected_row(app) else { return };
    let installed = match &row.action {
        Action::Existing { key } => app.find_port(key).filter(|p| app.is_installed(p)),
        _ => None,
    };
    if let Some(port) = installed {
        return super::port_page_screen::open_from_list(app, router, &port);
    }
    let project = app.deck_ready.index.borrow().as_ref().and_then(|i| i.get(&row.id).cloned()).and_then(|e| e.website.clone().or_else(|| entry::source_url(&e)).or_else(|| e.repo_url.clone()));
    if let Some(url) = project.filter(|u| crate::core::launch::is_web_url(u)) {
        super::port_page_screen::open_link(app, router, &row.name, &url, None);
    }
}

fn selected_row(app: &AppState) -> Option<Row> {
    let ui = &app.deck_ready;
    ui.rows.borrow().get(ui.selected.get()).cloned()
}

/// A: install or play the selected row. With no list loaded, A retries the download.
pub(crate) fn activate(app: &Rc<AppState>, router: &Router) {
    if dialog_is_open(app) {
        return;
    }
    let Some(row) = selected_row(app) else {
        if app.deck_ready.index.borrow().is_none() {
            start_fetch(app, true);
            rebuild(app);
        }
        return;
    };
    run_action(app, router, &row);
}

fn run_action(app: &Rc<AppState>, router: &Router, row: &Row) {
    match &row.action {
        Action::Existing { key } => {
            if let Some(port) = app.find_port(key) {
                activate_port(app, router, &port);
            }
        }
        Action::AddAndInstall => confirm_add(app, router, &row.id),
        Action::InfoOnly => show_info(app, router),
    }
}

fn folder_taken(app: &AppState, folder: &str) -> bool {
    let catalog = app.catalog.borrow();
    entry::folder_taken(&app.paths.library_dir(), catalog.iter().map(|p| p.folder.as_str()), folder)
}

/// The install wizard for a project the app does not have yet: its first page ("Install <name>?", with the same-game
/// note) shows the entry it would add; Install writes it to `ports.local.json` and installs.
fn confirm_add(app: &Rc<AppState>, router: &Router, id: &str) {
    let Some((entry, name)) = app.deck_ready.index.borrow().as_ref().and_then(|i| i.get(id)).map(|e| (e.clone(), view::display_name(e))) else { return };
    let Some(folder) = entry::folder_for(&entry, |f| folder_taken(app, f)) else {
        return open_message_dialog(app, router, &name, "There is no free folder name for this port in your ports folder, so nothing was added.");
    };
    // Built and checked before anything is written, so a failure leaves ports.local.json alone.
    // Fork: the cover, icon and notes of the catalog's entry for this folder, even when upstream turned it off.
    let raw = crate::fork::catalog_extras::RawCatalog::load(&app.paths.config_dir.join("ports.json"));
    let Some((value, mut port)) = entry::local_entry_with(&entry, &name, &folder, &raw).and_then(|v| crate::core::models::port_from_value(&v).ok().map(|p| (v, p))) else {
        return open_message_dialog(app, router, &name, "Nothing was added: the list's entry for this port is missing something the app needs to install it.");
    };
    port.user_managed = true;
    let to_add = port.clone();
    let prepare: super::install_wizard_screen::Prepare = Rc::new(move |app, router| {
        let local_json = app.paths.config_dir.join("ports.local.json");
        if let Err(e) = entry::add_to_local_file(&local_json, value.clone(), &crate::fork::store::backup_stamp()) {
            open_message_dialog(app, router, &name, &format!("Nothing was added: {e}."));
            return None;
        }
        {
            let mut catalog = app.catalog.borrow_mut();
            let old = std::mem::take(&mut *catalog);
            *catalog = crate::core::config::merge_local_catalog(old, vec![to_add.clone()]);
        }
        // Fork (save finder): an answer given earlier for this folder applies to the entry just added.
        super::save_finder_screen::reapply(app);
        app.refresh_current_view();
        Some(to_add.clone())
    });
    let version = entry.release.as_ref().map(|r| r.tag.clone());
    let back: super::install_wizard_screen::Back = Rc::new(|app, router| close_current_dialog(app, router));
    super::install_wizard_screen::open_entry(app, router, port, entry.system.clone(), version, prepare, back);
}

fn releases_url(entry: &crate::fork::deck_ready::index::Entry) -> Option<String> {
    let source = entry::source_url(entry)?;
    Some(match entry.host {
        Host::Gitlab => format!("{source}/-/releases"),
        _ => format!("{source}/releases"),
    })
}

/// X: the Info page for the selected row (`fork::deck_ready::pages::info_page`): a few plain facts, then Install / Play,
/// the project page and the releases page (each shown with its address and a QR code first). B goes back to the list.
pub(crate) fn show_info(app: &Rc<AppState>, router: &Router) {
    if dialog_is_open(app) {
        return;
    }
    let Some(row) = selected_row(app) else { return };
    let ui = &app.deck_ready;
    let Some(entry) = ui.index.borrow().as_ref().and_then(|i| i.get(&row.id).cloned()) else { return };
    let project = entry.website.clone().or_else(|| entry::source_url(&entry)).or_else(|| entry.repo_url.clone()).filter(|u| crate::core::launch::is_web_url(u));
    let releases = releases_url(&entry).filter(|u| crate::core::launch::is_web_url(u));
    let facts = pages::InfoFacts { name: &row.name, standing: row.standing, button: row.button, has_project: project.is_some(), has_releases: releases.is_some() };
    let page = pages::info_page(&entry, &facts);
    let again: super::port_page_screen::Back = Rc::new(|app, router| {
        close_current_dialog(app, router);
        show_info(app, router);
    });
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| match act {
            DrAct::Main => {
                close_current_dialog(app, router);
                run_action(app, router, &row);
            }
            DrAct::ProjectPage => {
                if let Some(url) = &project {
                    super::port_page_screen::open_link(app, router, &row.name, url, Some(again.clone()));
                }
            }
            DrAct::ReleasesPage => {
                if let Some(url) = &releases {
                    super::port_page_screen::open_link_with(app, router, &row.name, url, "Releases page", "Every version of it and its download files.", Some(again.clone()));
                }
            }
            DrAct::Back => close_current_dialog(app, router),
            _ => {}
        }),
        on_back: Rc::new(close_current_dialog),
        on_exit: Rc::new(close_current_dialog),
        on_input: None,
    };
    show_page(app, router, page, handlers);
}

/// Which of the old Info dialog's four buttons (action, project page, releases page, close) can be chosen. Fork (steam,
/// art and settings pages): Info is a page now; kept for the dialog's controller code.
pub(crate) fn info_buttons(d: &DeckReadyInfoDialog) -> [bool; 4] {
    [d.get_action_enabled() && !d.get_action_label().is_empty(), d.get_project_enabled(), d.get_releases_enabled(), true]
}

pub(crate) fn info_step(d: &DeckReadyInfoDialog, current: i32, dx: i32) -> Option<i32> {
    let enabled = info_buttons(d);
    let candidates: Vec<i32> = (0..4).filter(|&i| enabled[i as usize]).collect();
    let pos = candidates.iter().position(|&i| i == current).unwrap_or(0) as i32;
    let next = (pos + dx).clamp(0, candidates.len() as i32 - 1);
    (next != pos).then(|| candidates[next as usize])
}

pub(crate) fn info_activate(d: &DeckReadyInfoDialog, index: i32) {
    if info_buttons(d).get(index as usize) != Some(&true) {
        return;
    }
    match index {
        0 => d.invoke_action_requested(),
        1 => d.invoke_project_requested(),
        2 => d.invoke_releases_requested(),
        _ => d.invoke_close_requested(),
    }
}

/// The list behind follows a filter at once.
fn refilter(app: &AppState) {
    app.deck_ready.selected.set(0);
    // One scroll, always (two toggles of the trigger in one go would cancel out).
    rebuild_rows(app);
    scroll_to_selection(&app.window());
}

/// System: a choice page (All systems, then each system), the one in use marked; choosing one goes back to Filters.
fn open_system_page(app: &Rc<AppState>, router: &Router) {
    let ui = &app.deck_ready;
    let community = ui.filters.borrow().community;
    let systems = ui.index.borrow().as_ref().map(|i| view::systems(i, community)).unwrap_or_default();
    let current = ui.filters.borrow().system.clone();
    let page = pages::system_page(&systems, current.as_deref());
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| match act {
            DrAct::PickSystem(choice) => {
                app.deck_ready.filters.borrow_mut().system = choice.checked_sub(1).and_then(|i| systems.get(i)).cloned();
                refilter(app);
                show_filters(app, router);
            }
            DrAct::Back => show_filters(app, router),
            _ => {}
        }),
        on_back: Rc::new(show_filters),
        on_exit: Rc::new(close_current_dialog),
        on_input: None,
    };
    show_page(app, router, page, handlers);
}

/// Y: Filters, as a page of three cards. Builds and Community finds change in place (the list behind updates); System
/// opens its choice page; Done or B closes.
pub(crate) fn open_filters(app: &Rc<AppState>, router: &Router) {
    if dialog_is_open(app) {
        return;
    }
    show_filters(app, router);
}

fn show_filters(app: &Rc<AppState>, router: &Router) {
    let page = pages::filters_page(&app.deck_ready.filters.borrow());
    let handlers = Handlers {
        on_action: Rc::new(|app, router, act| {
            match act {
                DrAct::Builds => {
                    let next = view::next_build(app.deck_ready.filters.borrow().build);
                    app.deck_ready.filters.borrow_mut().build = next;
                }
                DrAct::Community => {
                    let on = !app.deck_ready.filters.borrow().community;
                    app.deck_ready.filters.borrow_mut().community = on;
                }
                DrAct::System => return open_system_page(app, router),
                DrAct::Done => return close_current_dialog(app, router),
                _ => return,
            }
            refilter(app);
            show_filters(app, router);
        }),
        on_back: Rc::new(close_current_dialog),
        on_exit: Rc::new(close_current_dialog),
        on_input: None,
    };
    show_page(app, router, page, handlers);
}

#[cfg(test)]
mod tests {
    use super::selection_moved;

    #[test]
    fn a_refresh_scrolls_only_when_the_selected_row_moved() {
        assert!(selection_moved(3, 7, 20), "the download put rows above it: bring it into view");
        assert!(selection_moved(12, 0, 5), "its port is gone: the first row, in view");
        assert!(!selection_moved(4, 4, 20), "it stayed: the scroll is left alone");
        assert!(!selection_moved(4, 0, 0), "nothing to show");
    }

    #[test]
    fn the_keyboard_reaches_port_options_and_settings_like_x_and_y() {
        let slint = include_str!("../../ui/app-window.slint");
        let keys = &slint[slint.find("search-input := TextInput").expect("the search box's keys")..];
        let line_of = |shortcut: &str| keys.lines().position(|l| l.contains(shortcut)).unwrap_or_else(|| panic!("no {shortcut}"));
        let info = line_of("event.modifiers.control && event.text == \"i\"");
        let settings = line_of("event.modifiers.control && event.text == \"p\"");
        let lines: Vec<&str> = keys.lines().collect();
        assert!(lines[info..info + 4].iter().any(|l| l.contains("root.info-requested()")), "Ctrl+I: Port options / Info");
        assert!(lines[settings..settings + 4].iter().any(|l| l.contains("root.settings-requested()")), "Ctrl+P: Settings");
        assert!(info < line_of("event.text == Key.UpArrow"), "before the Ctrl+W/A/S/D moves");
    }
}
