//! The cards of Import ports (Settings > Import ports..., and the setup wizard's import step): what the scan
//! found, one folder at a time with its cover to confirm, and one page to start the move. It replaces the old
//! list. What each page says is `crate::fork::import::cards`; the window is `page_dialog::show_page`.
//!
//! Pages: found (covers of everything matched, or "No ports found"), one card per port ("Port 2 of 5": Yes /
//! Pick another port / Not a port), "Which port is this?" for a folder that fits several ports or none (the
//! likely ports as tiles, Another port, a GitHub link, Not a port), a filterable list of the other ports, and
//! "Move 4 ports into your ports folder?". The move itself, its progress, the summary and the old-copy page are
//! in `import_screen`; this file only walks the user to the point where they say Move.
//!
//! B goes back one page (from the first card to the found page; from the found page it leaves Import ports,
//! telling the setup wizard when it waits on this screen). The title bar's X leaves. "Not a port" is saved so
//! the scan stops offering that folder; the folder is never touched. A port picked for a folder is kept for
//! the rest of the screen's life, so a new scan does not ask again.

use super::dialogs::{build_text_entry_dialog, close_current_dialog, finish_dialog_open, DialogSlot};
use super::folder_picker_screen::open_folder_picker;
use super::import_screen::{
    build_plans, confirm_back, end_notice, home_dir, notice_back, run_job, save_folders, settings_path, show_folders, show_plans, start_move, start_scan, Ctx, FoldersState, JobSpec, Step, TITLE,
};
use super::page_dialog::{show_page, Handlers};
use super::state::AppState;
use crate::fork::folder_picker::{PickerRequest, Purpose};
use crate::fork::import::cards::{self, card_page, confirm_page, found_page, scan_notes, which_page, CardAction, Item, Match, Verdict};
use crate::fork::import::folders::AddOutcome;
use crate::fork::import::scan::{measure_choice, Choice};
use crate::fork::import::screen::{ChoiceTarget, ImportScreen, PortListScreen};
use crate::fork::page::Page;
use crate::fork::rom_screens::util::ellipsize;
use crate::fork::screens::short_path;
use crate::fork::settings::ForkSettings;
use crate::ui::gamepad_router::GamepadRouter;
use crate::PickerItem;
use slint::ComponentHandle;
use std::cell::RefCell;
use std::rc::Rc;

type Router = Rc<RefCell<GamepadRouter>>;
type Screen = Rc<RefCell<ImportScreen>>;
/// A step that shows a page again: what OK and B on a Details text go back to.
type Again = Step;

/// Leaves Import ports: the page closes and the setup wizard, if it waits, is told.
fn leave(app: &Rc<AppState>, router: &Router, ctx: &Ctx) {
    close_current_dialog(app, router);
    ctx.finish_quiet(app, router);
}

/// A page's handlers: `act` says what each item does; B goes `back`; the X leaves.
fn handlers(ctx: &Ctx, act: impl Fn(&Rc<AppState>, &Router, CardAction) + 'static, back: impl Fn(&Rc<AppState>, &Router) + 'static) -> Handlers<CardAction> {
    let ctx = ctx.clone();
    Handlers { on_action: Rc::new(act), on_back: Rc::new(back), on_exit: Rc::new(move |app, router| leave(app, router, &ctx)) }
}

/// If the page cannot be built nothing is lost: the flow ends as if the user had left.
fn show_or_end(app: &Rc<AppState>, router: &Router, ctx: &Ctx, page: Page<CardAction>, handlers: Handlers<CardAction>) {
    if !show_page(app, router, page, handlers) {
        ctx.finish_quiet(app, router);
    }
}

fn details_text(app: &Rc<AppState>, router: &Router, title: &str, text: &str, again: Again) {
    let on_ok = again.clone();
    confirm_back(app, router, title, text, "OK", false, move |app, router| on_ok(app, router), again);
}

// -- Found -----------------------------------------------------------------------------------------

/// The first page after the scan: how many ports were found, with their covers.
pub(super) fn show_found(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen) {
    let leave_label = if ctx.from_wizard { "Skip" } else { "Not now" };
    let page = found_page(&screen.borrow().cards, leave_label, screen.borrow().scan.deck_ready_note.as_deref());
    let again: Again = {
        let (ctx, screen) = (ctx.clone(), screen.clone());
        Rc::new(move |app, router| show_found(app, router, &ctx, screen.clone()))
    };
    let act = {
        let (ctx, screen, again) = (ctx.clone(), screen.clone(), again.clone());
        move |app: &Rc<AppState>, router: &Router, action: CardAction| match action {
            CardAction::Start => {
                screen.borrow_mut().cards.reset();
                show_step(app, router, &ctx, screen.clone(), 0);
            }
            CardAction::Leave => leave(app, router, &ctx),
            CardAction::AddFolder => add_folder_and_rescan(app, router, &ctx, screen.clone()),
            CardAction::Details => {
                let text = {
                    let s = screen.borrow();
                    format!("{}\n\nScan folders can be changed.", scan_notes(&s.scan, &s.home))
                };
                let (ctx2, screen2) = (ctx.clone(), screen.clone());
                confirm_back(app, router, TITLE, &text, "Change scan folders...", false, move |app, router| show_folders(app, router, &ctx2, FoldersState::new(screen2.clone())), again.clone());
            }
            _ => {}
        }
    };
    let back = {
        let ctx = ctx.clone();
        move |app: &Rc<AppState>, router: &Router| leave(app, router, &ctx)
    };
    show_or_end(app, router, ctx, page, handlers(ctx, act, back));
}

/// "Look in another folder...": the folder picker, then a new scan with the folder added.
fn add_folder_and_rescan(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen) {
    let start = home_dir().unwrap_or_else(|| std::path::PathBuf::from("/"));
    let (chosen_ctx, chosen_screen, cancel_ctx, cancel_screen) = (ctx.clone(), screen.clone(), ctx.clone(), screen);
    open_folder_picker(
        app,
        router,
        PickerRequest::new("Add a folder to look in", &start, Purpose::ScanFolder),
        move |app, router, path| {
            let home = home_dir().unwrap_or_default();
            let mut folders = chosen_screen.borrow().folders.clone();
            let outcome = folders.add_user(&path, &home);
            let again: Step = {
                let (ctx, screen) = (chosen_ctx.clone(), chosen_screen.clone());
                Rc::new(move |app, router| show_found(app, router, &ctx, screen.clone()))
            };
            match outcome {
                AddOutcome::Added | AddOutcome::DefaultTurnedOn(_) => {
                    if let Some(problem) = save_folders(app, &folders) {
                        return notice_back(app, router, &problem, again);
                    }
                    start_scan(app, router, &chosen_ctx, folders);
                }
                AddOutcome::AlreadyListed => notice_back(app, router, &format!("{} was already looked in.", short_path(&path, &home)), again),
                AddOutcome::NotAFolder => notice_back(app, router, "That is not a folder.", again),
            }
        },
        move |app, router| show_found(app, router, &cancel_ctx, cancel_screen.clone()),
    );
}

// -- The walk: one page per folder ------------------------------------------------------------------

/// Page number `pos` of the walk (the folders the user has not settled already), or the move page when the
/// walk is over.
fn show_step(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, pos: usize) {
    let walk = screen.borrow().cards.walk();
    let Some(&index) = walk.get(pos) else { return after_walk(app, router, ctx, screen) };
    let item = screen.borrow().cards.item(index).cloned();
    match item {
        Some(Item::Match(m)) => show_card(app, router, ctx, screen, pos, walk.len(), index, m),
        Some(Item::Unsure(u)) => show_which(app, router, ctx, screen, pos, walk.len(), index, u),
        None => after_walk(app, router, ctx, screen),
    }
}

/// B on page `pos`: the page before it, or the found page from the first.
fn step_back(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, pos: usize) {
    match pos.checked_sub(1) {
        Some(previous) => show_step(app, router, ctx, screen, previous),
        None => show_found(app, router, ctx, screen),
    }
}

/// Saves that the folder is not a port (the file is only the app's own settings), then goes on.
fn not_a_port(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, pos: usize, index: usize, again: Again) {
    let path = match screen.borrow().cards.item(index) {
        Some(item) => item.path().to_path_buf(),
        None => return,
    };
    let mut settings = ForkSettings::load(&settings_path(app));
    match cards::ignore_path(&mut settings, &path) {
        Ok(()) => {
            ctx.choices.borrow_mut().set(&path, Choice::NotAPort);
            {
                let mut s = screen.borrow_mut();
                s.cards.set(index, Verdict::NotAPort);
                s.leave_alone(&path);
            }
            show_step(app, router, ctx, screen, pos + 1);
        }
        // Not saved, so it would be offered again next time; the folder itself is untouched either way.
        Err(e) => notice_back(app, router, &format!("Could not save that this is not a port: {e}\nNothing was changed."), again),
    }
}

#[allow(clippy::too_many_arguments)]
fn show_card(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, pos: usize, total: usize, index: usize, m: Match) {
    let page = card_page(&m, pos, total);
    let again: Again = {
        let (ctx, screen, m) = (ctx.clone(), screen.clone(), m.clone());
        Rc::new(move |app, router| show_card(app, router, &ctx, screen.clone(), pos, total, index, m.clone()))
    };
    let act = {
        let (ctx, screen, again) = (ctx.clone(), screen.clone(), again.clone());
        move |app: &Rc<AppState>, router: &Router, action: CardAction| match action {
            CardAction::Yes => {
                screen.borrow_mut().cards.set(index, Verdict::Yes);
                show_step(app, router, &ctx, screen.clone(), pos + 1);
            }
            CardAction::NotAPort => not_a_port(app, router, &ctx, screen.clone(), pos, index, again.clone()),
            CardAction::PickAnother => show_port_list(app, router, &ctx, screen.clone(), pos, index, again.clone()),
            CardAction::Details => {
                let text = {
                    let s = screen.borrow();
                    m.details(&s.home, &s.ports_folder)
                };
                details_text(app, router, &m.port_name, &text, again.clone());
            }
            _ => {}
        }
    };
    let back = {
        let (ctx, screen) = (ctx.clone(), screen.clone());
        move |app: &Rc<AppState>, router: &Router| step_back(app, router, &ctx, screen.clone(), pos)
    };
    show_or_end(app, router, ctx, page, handlers(ctx, act, back));
}

#[allow(clippy::too_many_arguments)]
fn show_which(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, pos: usize, total: usize, index: usize, u: cards::Unsure) {
    let page = which_page(&u, pos, total);
    let again: Again = {
        let (ctx, screen, u) = (ctx.clone(), screen.clone(), u.clone());
        Rc::new(move |app, router| show_which(app, router, &ctx, screen.clone(), pos, total, index, u.clone()))
    };
    let act = {
        let (ctx, screen, again) = (ctx.clone(), screen.clone(), again.clone());
        move |app: &Rc<AppState>, router: &Router, action: CardAction| match action {
            CardAction::Pick(i) => {
                if let Some(maybe) = u.could_be.get(i) {
                    choose_port(app, router, &ctx, screen.clone(), pos, index, maybe.key.clone(), again.clone());
                }
            }
            CardAction::PickAnother => show_port_list(app, router, &ctx, screen.clone(), pos, index, again.clone()),
            CardAction::GithubLink => notice_back(app, router, "Adding a port from a GitHub link is not ready yet.", again.clone()),
            CardAction::NotAPort => not_a_port(app, router, &ctx, screen.clone(), pos, index, again.clone()),
            CardAction::Details => {
                let text = {
                    let s = screen.borrow();
                    format!("{}\n\nFound in: {}", u.dir_name, short_path(&u.path, &s.home))
                };
                details_text(app, router, &u.dir_name, &text, again.clone());
            }
            _ => {}
        }
    };
    let back = {
        let (ctx, screen) = (ctx.clone(), screen.clone());
        move |app: &Rc<AppState>, router: &Router| step_back(app, router, &ctx, screen.clone(), pos)
    };
    show_or_end(app, router, ctx, page, handlers(ctx, act, back));
}

// -- Another port ---------------------------------------------------------------------------------------

/// "Pick another port": the ports not in the ports folder yet, the ones named like the folder first, with a
/// text box to filter and "Use a GitHub link..." as the last row. B goes back to the page it came from.
fn show_port_list(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, pos: usize, index: usize, back: Again) {
    let path = match screen.borrow().cards.item(index) {
        Some(item) => item.path().to_path_buf(),
        None => return,
    };
    let target = ChoiceTarget { path, could_be: Vec::new() };
    let list = Rc::new(PortListScreen::new(&screen.borrow(), &target));
    let shown = Rc::new(RefCell::new(list.matching("")));
    let Some(dialog) = build_text_entry_dialog(app, router, "Pick another port", "Type to filter the list", "", &port_rows(&list, &shown.borrow()), false) else { return back(app, router) };
    {
        let (app, weak, list, shown) = (app.clone(), dialog.as_weak(), list.clone(), shown.clone());
        dialog.on_search_changed(move |query| {
            let Some(d) = weak.upgrade() else { return };
            *shown.borrow_mut() = list.matching(query.as_str());
            let items: Vec<PickerItem> = port_rows(&list, &shown.borrow()).into_iter().map(|label| PickerItem { label: label.into() }).collect();
            d.set_items(slint::ModelRc::new(slint::VecModel::from(items)));
            d.set_selected_index(0);
            app.dialog_nav.picker_index.set(0);
        });
    }
    {
        let (app, router, ctx, screen, list, shown, back) = (app.clone(), router.clone(), ctx.clone(), screen.clone(), list.clone(), shown.clone(), back.clone());
        dialog.on_item_selected(move |row| {
            let picked = shown.borrow().get(row as usize).copied();
            match picked {
                Some(i) => choose_port(&app, &router, &ctx, screen.clone(), pos, index, list.ports[i].key.clone(), back.clone()),
                None => notice_back(&app, &router, "Adding a port from a GitHub link is not ready yet.", back.clone()),
            }
        });
    }
    {
        let (b_app, b_router, b_back) = (app.clone(), router.clone(), back.clone());
        dialog.on_close_requested(move || {
            close_current_dialog(&b_app, &b_router);
            b_back(&b_app, &b_router);
        });
        // The title bar's X only hides the window; it goes where B goes.
        let (app, router) = (app.clone(), router.clone());
        dialog.window().on_close_requested(move || {
            close_current_dialog(&app, &router);
            back(&app, &router);
            slint::CloseRequestResponse::HideWindow
        });
    }
    let _ = dialog.show();
    finish_dialog_open(app, router, DialogSlot::SearchList(dialog));
}

/// The user picked port `key` for the folder at `index`: it is measured on the worker (size, Steam shortcuts),
/// then the folder counts as that port, said yes to, and goes through the same plan and checks as any other.
/// B on the measuring list goes back with nothing changed.
#[allow(clippy::too_many_arguments)]
fn choose_port(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, pos: usize, index: usize, key: String, back: Again) {
    let (port, shortcuts, new_entry, path) = {
        let s = screen.borrow();
        let Some(port) = s.port(&key).cloned() else { return };
        let Some(item) = s.cards.item(index) else { return };
        let new_entry = s.scan.new_entries.iter().find(|(k, _)| *k == key).map(|(_, e)| e.clone());
        (port, s.scan.shortcuts.clone(), new_entry, item.path().to_path_buf())
    };
    let (ctx2, screen2) = (ctx.clone(), screen.clone());
    let leave_to = back.clone();
    run_job(
        app,
        router,
        JobSpec {
            title: "Looking at the folder",
            initial: format!("Measuring {}...", ellipsize(&path.to_string_lossy(), 70)),
            cancel_label: "Cancel",
            deliver_after_close: false,
            leave: Some(leave_to),
            prompts: None,
            is_import: false,
            work: Box::new(move |ctl| measure_choice(&path, &port, &shortcuts, new_entry, &|| ctl.cancelled())),
        },
        move |app, router, result| match result {
            Ok(found) if found.size.complete => {
                ctx2.choices.borrow_mut().set(&found.path, Choice::Port(found.port_key.clone()));
                {
                    let mut s = screen2.borrow_mut();
                    let port = s.port(&found.port_key).cloned();
                    let at = s.add_found(found.clone());
                    s.cards.choose(index, Match::of_found(&found, at, port.as_ref()));
                }
                show_step(app, router, &ctx2, screen2.clone(), pos + 1);
            }
            _ => back(app, router),
        },
    );
}

// -- Move ---------------------------------------------------------------------------------------------

/// Every folder has an answer: move the ones said yes to, or say that nothing was chosen.
fn after_walk(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen) {
    if screen.borrow().cards.accepted_count() == 0 {
        return end_notice(app, router, ctx, TITLE, "Nothing was chosen, so nothing was moved.");
    }
    show_confirm(app, router, ctx, screen);
}

/// "Move 4 ports into your ports folder?": the covers of the ones accepted. Move starts the copies.
pub(super) fn show_confirm(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen) {
    let (plans, free) = build_plans(app, &screen.borrow());
    let home = screen.borrow().home.clone();
    let blocked: Vec<(String, String)> = plans.iter().filter_map(|p| p.block.as_ref().map(|b| (p.request.port_name.clone(), b.text(&home)))).collect();
    let page = confirm_page(&screen.borrow().cards, free, &blocked);
    let back_to_walk = {
        let (ctx, screen) = (ctx.clone(), screen.clone());
        move |app: &Rc<AppState>, router: &Router| {
            let last = screen.borrow().cards.walk().len();
            match last.checked_sub(1) {
                Some(last) => show_step(app, router, &ctx, screen.clone(), last),
                None => show_found(app, router, &ctx, screen.clone()),
            }
        }
    };
    let act = {
        let (ctx, screen) = (ctx.clone(), screen.clone());
        let back = back_to_walk.clone();
        move |app: &Rc<AppState>, router: &Router, action: CardAction| match action {
            CardAction::Move => start_move(app, router, &ctx, screen.clone()),
            CardAction::Back => back(app, router),
            // The step-by-step plan of each port (paths and all) is a list behind Details; B comes back here.
            CardAction::Details => show_plans(app, router, &ctx, screen.clone()),
            _ => {}
        }
    };
    show_or_end(app, router, ctx, page, handlers(ctx, act, back_to_walk));
}

/// The rows of the port list: the ports that match the filter, then the GitHub link row.
fn port_rows(list: &PortListScreen, shown: &[usize]) -> Vec<String> {
    let mut rows: Vec<String> = shown.iter().map(|i| ellipsize(&list.ports[*i].label(), 90)).collect();
    rows.push("Use a GitHub link...".to_string());
    rows
}
