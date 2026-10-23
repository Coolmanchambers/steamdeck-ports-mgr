//! Fork: the in-app folder picker. The logic (places, listing, rows, naming rules) is in
//! `crate::fork::folder_picker`; this file draws it in `ui/fork/folder-picker.slint` and wires the
//! controller, keyboard and mouse.
//!
//! Use from any screen:
//! ```ignore
//! open_folder_picker(app, router, PickerRequest::new("Choose the ports folder", &start, Purpose::PortsFolder),
//!     |app, router, folder| { /* the chosen folder */ },
//!     |app, router| { /* Cancel or B */ });
//! ```
//! Exactly one of the two callbacks runs, once. The picker closes itself before calling either, so a
//! callback can open the next dialog straight away.

use super::dialogs::{build_folder_picker_dialog, build_text_entry_dialog, close_current_dialog, finish_dialog_open, DialogSlot};
use super::state::AppState;
use crate::fork::folder_picker::{create_folder, Env, Picker, PickerRequest, Row};
use crate::fork::rom_screens::util::ellipsize;
use crate::ui::gamepad_router::GamepadRouter;
use crate::{FolderPickerDialog, FolderRow, PickerItem};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

type Router = Rc<RefCell<GamepadRouter>>;
type OnChosen = Rc<dyn Fn(&Rc<AppState>, &Router, PathBuf)>;
type OnCancel = Rc<dyn Fn(&Rc<AppState>, &Router)>;

struct Session {
    title: String,
    picker: RefCell<Picker>,
    on_chosen: OnChosen,
    on_cancel: OnCancel,
}

thread_local! {
    /// The picker on screen, for the controller's move step (which only gets the dialog).
    static CURRENT: RefCell<Option<Rc<Session>>> = const { RefCell::new(None) };
}

pub(crate) fn open_folder_picker(
    app: &Rc<AppState>,
    router: &Router,
    request: PickerRequest,
    on_chosen: impl Fn(&Rc<AppState>, &Router, PathBuf) + 'static,
    on_cancel: impl Fn(&Rc<AppState>, &Router) + 'static,
) {
    let picker = Picker::open_with(&request.start, request.purpose, Env::system(), request.extra_places);
    let session = Rc::new(Session { title: request.title, picker: RefCell::new(picker), on_chosen: Rc::new(on_chosen), on_cancel: Rc::new(on_cancel) });
    let start = session.picker.borrow().row_of(Row::UseThis).unwrap_or(0);
    show(app, router, session, start);
}

fn rows_model(picker: &Picker) -> slint::ModelRc<FolderRow> {
    let rows: Vec<FolderRow> = picker
        .row_views()
        .into_iter()
        .map(|r| FolderRow { label: r.label.into(), detail: r.detail.into(), heading: r.heading, enabled: r.enabled })
        .collect();
    slint::ModelRc::new(slint::VecModel::from(rows))
}

fn fill(app: &AppState, dialog: &FolderPickerDialog, picker: &Picker, selected: usize) {
    dialog.set_path_text(shown_path(&picker.current).into());
    dialog.set_info_text(picker.info_line().into());
    dialog.set_hint_text(picker.hint_line().into());
    dialog.set_rows(rows_model(picker));
    let selected = selected.min(picker.rows().len().saturating_sub(1)) as i32;
    dialog.set_selected_index(selected);
    app.dialog_nav.picker_index.set(selected);
    dialog.set_scroll_trigger(!dialog.get_scroll_trigger());
}

fn show(app: &Rc<AppState>, router: &Router, session: Rc<Session>, selected: usize) {
    let Some(dialog) = build_folder_picker_dialog(app, router) else { return };
    dialog.set_dialog_title(session.title.clone().into());
    fill(app, &dialog, &session.picker.borrow(), selected);
    {
        let (app, weak) = (app.clone(), dialog.as_weak());
        let session = session.clone();
        dialog.on_item_hovered(move |index| {
            let selectable = session.picker.borrow().rows().get(index as usize).is_some_and(|r| r.selectable());
            if let (true, Some(d)) = (selectable, weak.upgrade()) {
                app.dialog_nav.picker_index.set(index);
                d.set_selected_index(index);
            }
        });
    }
    {
        let (app, router, weak) = (app.clone(), router.clone(), dialog.as_weak());
        let session = session.clone();
        dialog.on_item_selected(move |index| {
            if let Some(d) = weak.upgrade() {
                activate(&app, &router, &session, &d, index as usize);
            }
        });
    }
    {
        let (app, router) = (app.clone(), router.clone());
        let session = session.clone();
        dialog.on_close_requested(move || finish(&app, &router, &session, None));
    }
    CURRENT.with(|c| *c.borrow_mut() = Some(session));
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::FolderPicker(dialog));
}

/// Closes the picker and runs one of the callbacks.
fn finish(app: &Rc<AppState>, router: &Router, session: &Rc<Session>, chosen: Option<PathBuf>) {
    CURRENT.with(|c| *c.borrow_mut() = None);
    close_current_dialog(app, router);
    match chosen {
        Some(path) => (session.on_chosen)(app, router, path),
        None => (session.on_cancel)(app, router),
    }
}

fn activate(app: &Rc<AppState>, router: &Router, session: &Rc<Session>, dialog: &FolderPickerDialog, index: usize) {
    let row = session.picker.borrow().rows().get(index).copied();
    let next = match row {
        Some(Row::Place(i)) => {
            let mut picker = session.picker.borrow_mut();
            let Some(path) = picker.places().get(i).map(|p| p.path.clone()) else { return };
            picker.go_to(&path);
            picker.default_row()
        }
        Some(Row::Folder(i)) => {
            let mut picker = session.picker.borrow_mut();
            picker.enter(i);
            picker.default_row()
        }
        Some(Row::Up) => {
            let mut picker = session.picker.borrow_mut();
            let came_from = picker.up();
            came_from.and_then(|name| picker.row_of_folder(&name)).unwrap_or_else(|| picker.default_row())
        }
        Some(Row::ToggleHidden) => {
            let mut picker = session.picker.borrow_mut();
            picker.toggle_hidden();
            picker.row_of(Row::ToggleHidden).unwrap_or(0)
        }
        Some(Row::UseThis) => {
            let (ok, path) = {
                let picker = session.picker.borrow();
                (picker.can_choose().is_ok(), picker.current.clone())
            };
            if ok {
                return finish(app, router, session, Some(path));
            }
            index
        }
        Some(Row::NewFolder) => return ask_new_folder(app, router, session.clone()),
        Some(Row::Cancel) => return finish(app, router, session, None),
        Some(Row::Heading(_) | Row::More) | None => return,
    };
    fill(app, dialog, &session.picker.borrow(), next);
}

/// The controller's Up/Down on the picker: skips headings, stops at the ends.
pub(crate) fn move_selection(app: &AppState, dialog: &FolderPickerDialog, dy: i32) {
    let Some(session) = CURRENT.with(|c| c.borrow().clone()) else { return };
    let current = app.dialog_nav.picker_index.get().max(0) as usize;
    let next = session.picker.borrow().step(current, dy);
    if next != current {
        app.dialog_nav.picker_index.set(next as i32);
        dialog.set_selected_index(next as i32);
        dialog.set_scroll_trigger(!dialog.get_scroll_trigger());
    }
}

const NEW_FOLDER_ROWS: [&str; 2] = ["Create the folder", "Cancel"];

fn ask_new_folder(app: &Rc<AppState>, router: &Router, session: Rc<Session>) {
    let parent = session.picker.borrow().current.clone();
    let title = format!("New folder in {}", shown_path(&parent));
    let labels: Vec<String> = NEW_FOLDER_ROWS.iter().map(|s| s.to_string()).collect();
    let Some(dialog) = build_text_entry_dialog(app, router, &title, "Folder name, like Ports", "", &labels, false) else { return };
    let text = Rc::new(RefCell::new(String::new()));
    {
        let text = text.clone();
        dialog.on_search_changed(move |t| *text.borrow_mut() = t.to_string());
    }
    let session_for_close = session.clone();
    let reopen = {
        let session = session.clone();
        move |app: &Rc<AppState>, router: &Router, select: usize| show(app, router, session.clone(), select)
    };
    {
        let (app, router, weak) = (app.clone(), router.clone(), dialog.as_weak());
        let reopen = reopen.clone();
        dialog.on_item_selected(move |index| {
            if index != 0 {
                let select = session.picker.borrow().row_of(Row::NewFolder).unwrap_or(0);
                return reopen(&app, &router, select);
            }
            let name = text.borrow().clone();
            match create_folder(&parent, &name) {
                Ok(path) => {
                    {
                        let mut picker = session.picker.borrow_mut();
                        picker.go_to(&path);
                    }
                    let select = session.picker.borrow().row_of(Row::UseThis).unwrap_or(0);
                    reopen(&app, &router, select);
                }
                Err(problem) => {
                    // Shown as a third row; the text box keeps what was typed.
                    if let Some(d) = weak.upgrade() {
                        let mut items: Vec<PickerItem> = NEW_FOLDER_ROWS.iter().map(|s| PickerItem { label: (*s).into() }).collect();
                        items.push(PickerItem { label: problem.into() });
                        d.set_items(slint::ModelRc::new(slint::VecModel::from(items)));
                    }
                }
            }
        });
    }
    {
        let (app, router) = (app.clone(), router.clone());
        dialog.on_close_requested(move || {
            let select = session_for_close.picker.borrow().row_of(Row::NewFolder).unwrap_or(0);
            show(&app, &router, session_for_close.clone(), select);
        });
    }
    // While the name is typed the picker is not on screen; the controller's move step must not use it.
    CURRENT.with(|c| *c.borrow_mut() = None);
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::SearchList(dialog));
}

/// The path for the header: the home folder as `~`, long paths shortened from the middle so the end
/// (the part that tells folders apart) stays.
fn shown_path(path: &std::path::Path) -> String {
    let text = path.to_string_lossy().into_owned();
    let home = std::env::var("HOME").unwrap_or_default();
    let text = match text.strip_prefix(&home) {
        Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => format!("~{rest}"),
        _ => text,
    };
    ellipsize(&text, 58)
}
