//! The Deck-ready list screen: a second list in the main window beside All ports.
//!
//! L1/R1 (or Tab, or the footer button) switch lists. In this list A installs or plays, X opens Info,
//! Y opens Filters and B goes back to All ports. What each row shows and does is decided in
//! `crate::fork::deck_ready::view` (tested without any UI); this file draws it and runs the actions.
//! Ports the app already has go through the normal install and launch flow; an entry it doesn't have is
//! first added to `ports.local.json` as the user's own entry. The text is English only.

use super::dialogs::{
    build_deck_ready_info_dialog, build_picker_dialog, close_current_dialog, dialog_is_open, finish_dialog_open, open_confirm_dialog, open_message_dialog, open_picker_dialog,
    set_picker_items, DialogSlot,
};
use super::events::{lock, AppEvent};
use super::install_launch::{activate_port, start_install};
use super::state::AppState;
use crate::core::launch::open_url;
use crate::fork::deck_ready::index::{Host, Index};
use crate::fork::deck_ready::sync::{self, Meta, Outcome};
use crate::fork::deck_ready::view::{self, Action, Filters, KnownPort, Row, TagKind};
use crate::fork::deck_ready::entry;
use crate::ui::gamepad_router::GamepadRouter;
use crate::{AppWindow, DeckReadyInfoDialog, DeckReadyRow, DeckReadyTag};
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

type Router = Rc<RefCell<GamepadRouter>>;

const PAGE_ROWS: i32 = 10;
const FILTERS_TITLE: &str = "Deck-ready filters";

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
        window.on_deck_ready_toggle_requested(move || toggle(&app, &router));
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

fn known_ports(app: &AppState) -> Vec<KnownPort> {
    app.catalog
        .borrow()
        .iter()
        .map(|p| KnownPort {
            key: p.key().to_string(),
            folder: p.folder.clone(),
            repo: p.repo.clone(),
            user_managed: p.user_managed,
            installed: app.is_installed(p),
        })
        .collect()
}

fn tag_model(row: &Row) -> slint::ModelRc<DeckReadyTag> {
    let tags: Vec<DeckReadyTag> = row
        .tags
        .iter()
        .map(|t| DeckReadyTag {
            label: t.label.into(),
            kind: match t.kind {
                TagKind::Plain => 0,
                TagKind::Dashed => 1,
                TagKind::Warning => 2,
                TagKind::Muted => 3,
            },
        })
        .collect();
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
/// keeping the selection on the same entry when it is still shown.
pub(crate) fn rebuild(app: &AppState) {
    let ui = &app.deck_ready;
    let previous = ui.rows.borrow().get(ui.selected.get()).map(|r| r.id.clone());
    let rows = match ui.index.borrow().as_ref() {
        Some(index) => view::rows(index, &known_ports(app), &ui.filters.borrow(), &app.windowed_nav.search_query.borrow()),
        None => Vec::new(),
    };
    let selected = previous.and_then(|id| rows.iter().position(|r| r.id == id)).unwrap_or(0).min(rows.len().saturating_sub(1));
    let items: Vec<DeckReadyRow> =
        rows.iter().map(|r| DeckReadyRow { name: r.name.clone().into(), sub: r.sub.clone().into(), deck: r.deck, tags: tag_model(r), button: r.button.into() }).collect();
    let window = app.window();
    window.set_deck_ready_rows(slint::ModelRc::new(slint::VecModel::from(items)));
    window.set_deck_ready_status(status_text(ui, rows.len()).into());
    window.set_deck_ready_filters(view::filter_summary(&ui.filters.borrow()).into());
    window.set_deck_ready_empty_text(empty_text(ui).into());
    window.set_deck_ready_selected(if rows.is_empty() { -1 } else { selected as i32 });
    ui.selected.set(selected);
    *ui.rows.borrow_mut() = rows;
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

fn confirm_add(app: &Rc<AppState>, router: &Router, id: &str) {
    let Some((entry, name)) = app.deck_ready.index.borrow().as_ref().and_then(|i| i.get(id)).map(|e| (e.clone(), view::display_name(e))) else { return };
    let Some(folder) = entry::folder_for(&entry, |f| folder_taken(app, f)) else {
        return open_message_dialog(app, router, &name, "There is no free folder name for this port in Library, so nothing was added.\n");
    };
    let text = format!("{}\n", view::confirm_text(&entry, &folder));
    open_confirm_dialog(app, router, format!("Add {name}?").into(), text.into(), "Add and install".into(), move |app, router| {
        // Built and checked before anything is written, so a failure leaves ports.local.json alone.
        let Some((value, mut port)) = entry::local_entry(&entry, &name, &folder).and_then(|v| crate::core::models::port_from_value(&v).ok().map(|p| (v, p))) else {
            return open_message_dialog(app, router, &name, "Nothing was added: the entry could not be built from the list.\n");
        };
        let local_json = app.paths.config_dir.join("ports.local.json");
        if let Err(e) = entry::add_to_local_file(&local_json, value, &crate::fork::store::backup_stamp()) {
            return open_message_dialog(app, router, &name, &format!("Nothing was added: {e}.\n"));
        }
        port.user_managed = true;
        {
            let mut catalog = app.catalog.borrow_mut();
            let old = std::mem::take(&mut *catalog);
            *catalog = crate::core::config::merge_local_catalog(old, vec![port.clone()]);
        }
        app.refresh_current_view();
        start_install(app, router, port, None, None);
    });
}

fn releases_url(entry: &crate::fork::deck_ready::index::Entry) -> Option<String> {
    let source = entry::source_url(entry)?;
    Some(match entry.host {
        Host::Gitlab => format!("{source}/-/releases"),
        _ => format!("{source}/releases"),
    })
}

/// X: the Info dialog for the selected row.
pub(crate) fn show_info(app: &Rc<AppState>, router: &Router) {
    if dialog_is_open(app) {
        return;
    }
    let Some(row) = selected_row(app) else { return };
    let ui = &app.deck_ready;
    let Some((entry, generated)) = ui.index.borrow().as_ref().and_then(|i| i.get(&row.id).map(|e| (e.clone(), i.generated.clone()))) else { return };
    let checked = view::checked_label(&ui.meta.borrow().checked_at, &chrono::Local::now());
    let body = view::info_text(&entry, row.standing, &generated, &checked);
    let project = entry.website.clone().or_else(|| entry::source_url(&entry)).or_else(|| entry.repo_url.clone());
    let releases = releases_url(&entry);

    let Some(dialog) = build_deck_ready_info_dialog(app, router) else { return };
    dialog.set_dialog_title(format!("{}  ({})", row.name, view::rating_name(entry.rating)).into());
    dialog.set_body_text(body.into());
    dialog.set_action_label(row.button.into());
    dialog.set_action_enabled(!row.button.is_empty());
    dialog.set_project_enabled(project.is_some());
    dialog.set_releases_enabled(releases.is_some());
    let first = info_buttons(&dialog).iter().position(|&on| on).unwrap_or(3) as i32;
    app.dialog_nav.info_nav_index.set(first);
    dialog.set_selected_index(first);
    {
        let (app, router) = (app.clone(), router.clone());
        dialog.on_action_requested(move || {
            close_current_dialog(&app, &router);
            run_action(&app, &router, &row);
        });
    }
    if let Some(url) = project {
        dialog.on_project_requested(move || open_url(&url));
    }
    if let Some(url) = releases {
        dialog.on_releases_requested(move || open_url(&url));
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::DeckReadyInfo(dialog));
}

/// Which of the Info dialog's four buttons (action, project page, releases page, close) can be chosen.
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

/// System is chosen from its own list (about 25 systems), then Filters reopens.
fn open_system_picker(app: &Rc<AppState>, router: &Router) {
    let ui = &app.deck_ready;
    let community = ui.filters.borrow().community;
    let systems = ui.index.borrow().as_ref().map(|i| view::systems(i, community)).unwrap_or_default();
    let labels = std::iter::once("All systems".to_string()).chain(systems.iter().cloned()).collect();
    open_picker_dialog(app, router, "System", labels, move |app, router, choice| {
        app.deck_ready.filters.borrow_mut().system = choice.checked_sub(1).and_then(|i| systems.get(i)).cloned();
        app.deck_ready.selected.set(0);
        rebuild(app);
        open_filters(app, router);
    });
}

/// Y: Filters. Choosing a row changes its value at once (the list behind updates); Done or B closes.
pub(crate) fn open_filters(app: &Rc<AppState>, router: &Router) {
    if dialog_is_open(app) {
        return;
    }
    let labels = view::filter_lines(&app.deck_ready.filters.borrow());
    let Some(dialog) = build_picker_dialog(app, router, FILTERS_TITLE, &labels) else { return };
    {
        let (app, router, weak) = (app.clone(), router.clone(), dialog.as_weak());
        dialog.on_item_selected(move |index| {
            let ui = &app.deck_ready;
            {
                let mut filters = ui.filters.borrow_mut();
                match index {
                    0 => filters.build = view::next_build(filters.build),
                    1 => {
                        drop(filters);
                        return open_system_picker(&app, &router);
                    }
                    2 => filters.community = !filters.community,
                    _ => {
                        drop(filters);
                        return close_current_dialog(&app, &router);
                    }
                }
            }
            ui.selected.set(0);
            rebuild(&app);
            scroll_to_selection(&app.window());
            if let Some(d) = weak.upgrade() {
                set_picker_items(&d, &view::filter_lines(&ui.filters.borrow()));
            }
        });
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::Picker(dialog));
}
