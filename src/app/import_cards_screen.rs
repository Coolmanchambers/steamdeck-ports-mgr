//! The cards of Import ports (Settings > Import ports..., and the setup wizard's import step): what the scan
//! found, one folder at a time with its cover to confirm, and one page to start the move. It replaces the old
//! list. What each page says is `crate::fork::import::cards`; the window is `page_dialog::show_page`.
//!
//! Pages: found (covers of everything matched, or "No ports found"), one card per port ("Port 2 of 5": Yes /
//! Pick another port / Not a port), "Which port is this?" for a folder that fits several ports or none (the
//! likely ports as tiles, Another port, a GitHub link, Not a port), a filterable list of the other ports, the
//! GitHub link pages, and "Move 4 ports into your ports folder?". The move itself, its progress, the summary
//! and the old-copy page are in `import_screen`; this file only walks the user to the point where they say Move.
//!
//! B goes back one page (from the first card to the found page; from the found page it leaves Import ports,
//! telling the setup wizard when it waits on this screen). The title bar's X leaves. A page is shown by the
//! index of its item in the cards; going forward skips the folders the user said are not ports, going back
//! shows them again (so the answer can be taken back). "Not a port" is saved so the scan stops offering that
//! folder, and it is not forgotten silently: Details of the found page lists the folders and "Ask again"
//! brings them back. The folder itself is never touched. A port picked for a folder is kept for the rest of
//! the screen's life, so a new scan does not ask again.

use super::dialogs::{build_text_entry_dialog, close_current_dialog, finish_dialog_open, DialogSlot};
use super::folder_picker_screen::open_folder_picker;
use super::import_screen::{
    build_plans, confirm_back, end_notice, home_dir, notice_back, run_job, save_folders, settings_path, show_folders, start_move, start_scan, Ctx, FoldersState, JobSpec, Step, TITLE,
};
use super::page_dialog::{show_page, Handlers};
use super::state::AppState;
use crate::fork::art::net::{Fetch, Net};
use crate::fork::custom_repo::check::{check, CheckError, Checked, ReplayFetch};
use crate::fork::custom_repo::entry as repo_entry;
use crate::fork::custom_repo::parse::parse_link;
use crate::fork::folder_picker::{PickerRequest, Purpose};
use crate::fork::import::cards::{self, card_page, confirm_page, details_page, found_page, scan_notes, steps_page, which_page, CardAction, Item, Match, Verdict};
use crate::fork::import::folders::AddOutcome;
use crate::fork::import::link::{self, LinkRow};
use crate::fork::import::plan::PortPlan;
use crate::fork::import::scan::{measure_choice, Choice};
use crate::fork::import::screen::{plan_text, ChoiceTarget, ImportScreen, PortListScreen};
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

fn ignored_now(app: &AppState) -> Vec<std::path::PathBuf> {
    cards::ignored_paths(&ForkSettings::load(&settings_path(app)))
}

// -- Found -----------------------------------------------------------------------------------------

/// The first page after the scan: how many ports were found, with their covers.
pub(super) fn show_found(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen) {
    let leave_label = if ctx.from_wizard { "Skip" } else { "Not now" };
    let left_alone = ignored_now(app).len();
    let page = found_page(&screen.borrow().cards, leave_label, screen.borrow().scan.deck_ready_note.as_deref(), left_alone);
    let again: Again = {
        let (ctx, screen) = (ctx.clone(), screen.clone());
        Rc::new(move |app, router| show_found(app, router, &ctx, screen.clone()))
    };
    let act = {
        let (ctx, screen, again) = (ctx.clone(), screen.clone(), again.clone());
        move |app: &Rc<AppState>, router: &Router, action: CardAction| match action {
            CardAction::Start => {
                screen.borrow_mut().cards.reset();
                let first = screen.borrow().cards.next_open(None);
                match first {
                    Some(index) => show_item(app, router, &ctx, screen.clone(), index),
                    None => after_walk(app, router, &ctx, screen.clone()),
                }
            }
            CardAction::Leave => leave(app, router, &ctx),
            CardAction::AddFolder => add_folder_and_rescan(app, router, &ctx, screen.clone()),
            CardAction::GithubLink => show_github(app, router, &ctx, screen.clone(), Target::NewFolder, again.clone(), String::new()),
            CardAction::AskAgain => ask_again(app, router, &ctx, screen.clone(), again.clone()),
            CardAction::Details => show_details(app, router, &ctx, screen.clone()),
            _ => {}
        }
    };
    let back = {
        let ctx = ctx.clone();
        move |app: &Rc<AppState>, router: &Router| leave(app, router, &ctx)
    };
    show_or_end(app, router, ctx, page, handlers(ctx, act, back));
}

/// Details of the found page: where it looked, what it left out, the folders said not to be ports (with a way to
/// ask again) and the scan folders.
fn show_details(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen) {
    let ignored = ignored_now(app);
    let notes = {
        let s = screen.borrow();
        scan_notes(&s.scan, &ignored, &s.home)
    };
    let page = details_page(&notes, ignored.len());
    let again: Again = {
        let (ctx, screen) = (ctx.clone(), screen.clone());
        Rc::new(move |app, router| show_details(app, router, &ctx, screen.clone()))
    };
    let act = {
        let (ctx, screen) = (ctx.clone(), screen.clone());
        move |app: &Rc<AppState>, router: &Router, action: CardAction| match action {
            CardAction::AskAgain => ask_again(app, router, &ctx, screen.clone(), again.clone()),
            CardAction::ScanFolders => show_folders(app, router, &ctx, FoldersState::new(screen.clone())),
            CardAction::Back => show_found(app, router, &ctx, screen.clone()),
            _ => {}
        }
    };
    let back = {
        let (ctx, screen) = (ctx.clone(), screen.clone());
        move |app: &Rc<AppState>, router: &Router| show_found(app, router, &ctx, screen.clone())
    };
    show_or_end(app, router, ctx, page, handlers(ctx, act, back));
}

/// "Ask again": forgets the not-a-port answers (the settings list and this screen's memory) and looks again.
fn ask_again(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, again: Again) {
    let mut settings = ForkSettings::load(&settings_path(app));
    if let Err(e) = cards::clear_ignored(&mut settings) {
        return notice_back(app, router, &format!("Could not forget those answers: {e}\nNothing was changed."), again);
    }
    ctx.choices.borrow_mut().clear_not_a_port();
    let folders = screen.borrow().folders.clone();
    start_scan(app, router, ctx, folders);
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

/// The page of item `index`: a port to confirm, or a folder to choose the port for.
fn show_item(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, index: usize) {
    let (item, (pos, total), said_not) = {
        let s = screen.borrow();
        (s.cards.item(index).cloned(), s.cards.position(index), s.cards.verdict(index) == Verdict::NotAPort)
    };
    match item {
        Some(Item::Match(m)) => show_card(app, router, ctx, screen, index, pos, total, said_not, m),
        Some(Item::Unsure(u)) => show_which(app, router, ctx, screen, index, pos, total, said_not, u),
        None => after_walk(app, router, ctx, screen),
    }
}

/// On from item `index`: the next one that needs an answer, or the move page.
fn go_next(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, index: usize) {
    let next = screen.borrow().cards.next_open(Some(index));
    match next {
        Some(i) => show_item(app, router, ctx, screen, i),
        None => after_walk(app, router, ctx, screen),
    }
}

/// B on item `index`: the page before it, or the found page from the first.
fn go_back(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, index: usize) {
    let previous = screen.borrow().cards.prev_item(index);
    match previous {
        Some(i) => show_item(app, router, ctx, screen, i),
        None => show_found(app, router, ctx, screen),
    }
}

/// Takes back "not a port" for item `index` when that was the answer (Yes or a pick): off the saved list, and
/// out of this screen's memory. A failure to save is said; the answer stays as it was then.
fn take_back_not_a_port(app: &Rc<AppState>, screen: &Screen, ctx: &Ctx, index: usize) -> Result<(), String> {
    let path = {
        let s = screen.borrow();
        if s.cards.verdict(index) != Verdict::NotAPort {
            return Ok(());
        }
        match s.cards.item(index) {
            Some(item) => item.path().to_path_buf(),
            None => return Ok(()),
        }
    };
    let mut settings = ForkSettings::load(&settings_path(app));
    cards::unignore_path(&mut settings, &path).map_err(|e| format!("Could not take back \"not a port\": {e}\nNothing was changed."))?;
    ctx.choices.borrow_mut().remove(&path);
    screen.borrow_mut().scan.left_alone.retain(|p| *p != path);
    Ok(())
}

/// Saves that the folder is not a port (the file is only the app's own settings), then goes on.
fn not_a_port(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, index: usize, again: Again) {
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
            go_next(app, router, ctx, screen, index);
        }
        // Not saved, so it would be offered again next time; the folder itself is untouched either way.
        Err(e) => notice_back(app, router, &format!("Could not save that this is not a port: {e}\nNothing was changed."), again),
    }
}

#[allow(clippy::too_many_arguments)]
fn show_card(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, index: usize, pos: usize, total: usize, said_not: bool, m: Match) {
    let page = card_page(&m, pos, total, said_not);
    let again: Again = {
        let (ctx, screen, m) = (ctx.clone(), screen.clone(), m.clone());
        Rc::new(move |app, router| show_item_again(app, router, &ctx, screen.clone(), index, m.clone()))
    };
    let act = {
        let (ctx, screen, again) = (ctx.clone(), screen.clone(), again.clone());
        move |app: &Rc<AppState>, router: &Router, action: CardAction| match action {
            CardAction::Yes => match take_back_not_a_port(app, &screen, &ctx, index) {
                Ok(()) => {
                    screen.borrow_mut().cards.set(index, Verdict::Yes);
                    go_next(app, router, &ctx, screen.clone(), index);
                }
                Err(why) => notice_back(app, router, &why, again.clone()),
            },
            CardAction::NotAPort if m.strong && !said_not => {
                // A real match: one question before the answer is saved.
                let (ctx2, screen2, again2) = (ctx.clone(), screen.clone(), again.clone());
                let text = format!("{} looks like a real match. Leave this folder alone and do not ask again?\n\nYou can ask again later from Details on the first page.", m.port_name);
                confirm_back(app, router, TITLE, &text, "Leave it alone", true, move |app, router| not_a_port(app, router, &ctx2, screen2.clone(), index, again2.clone()), again.clone());
            }
            CardAction::NotAPort => not_a_port(app, router, &ctx, screen.clone(), index, again.clone()),
            CardAction::PickAnother => show_port_list(app, router, &ctx, screen.clone(), index, again.clone()),
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
        move |app: &Rc<AppState>, router: &Router| go_back(app, router, &ctx, screen.clone(), index)
    };
    show_or_end(app, router, ctx, page, handlers(ctx, act, back));
}

/// Shows item `index` again as it is now (after a notice or Details).
fn show_item_again(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, index: usize, _was: Match) {
    show_item(app, router, ctx, screen, index);
}

#[allow(clippy::too_many_arguments)]
fn show_which(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, index: usize, pos: usize, total: usize, said_not: bool, u: cards::Unsure) {
    let page = which_page(&u, pos, total, said_not);
    let again: Again = {
        let (ctx, screen) = (ctx.clone(), screen.clone());
        Rc::new(move |app, router| show_item(app, router, &ctx, screen.clone(), index))
    };
    let act = {
        let (ctx, screen, again) = (ctx.clone(), screen.clone(), again.clone());
        move |app: &Rc<AppState>, router: &Router, action: CardAction| match action {
            CardAction::Pick(i) => {
                if let Some(maybe) = u.could_be.get(i) {
                    choose_port(app, router, &ctx, screen.clone(), index, maybe.key.clone(), again.clone());
                }
            }
            CardAction::PickAnother => show_port_list(app, router, &ctx, screen.clone(), index, again.clone()),
            CardAction::GithubLink => show_github(app, router, &ctx, screen.clone(), Target::Folder { index }, again.clone(), String::new()),
            CardAction::NotAPort => not_a_port(app, router, &ctx, screen.clone(), index, again.clone()),
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
        move |app: &Rc<AppState>, router: &Router| go_back(app, router, &ctx, screen.clone(), index)
    };
    show_or_end(app, router, ctx, page, handlers(ctx, act, back));
}

// -- Another port ---------------------------------------------------------------------------------------

/// What the last rows of the port list are: the keyboard (Game Mode only), then the GitHub link.
fn extra_rows(game_mode: bool) -> usize {
    if game_mode { 2 } else { 1 }
}

/// The rows of the port list: the ports that match the filter, Show keyboard (Game Mode), the GitHub link row.
fn port_rows(list: &PortListScreen, shown: &[usize], game_mode: bool) -> Vec<String> {
    let mut rows: Vec<String> = shown.iter().map(|i| ellipsize(&list.ports[*i].label(), 90)).collect();
    if game_mode {
        rows.push("Show keyboard".to_string());
    }
    rows.push("Use a GitHub link...".to_string());
    rows
}

/// Asks Steam for its on-screen keyboard. Unverified on a Deck, so the pages also say how to bring it up by hand.
fn show_keyboard() -> bool {
    std::process::Command::new("xdg-open").arg("steam://open/keyboard").spawn().is_ok()
}

/// "Pick another port": the ports not in the ports folder yet, the ones named like the folder first, with a
/// text box to filter and "Use a GitHub link..." as the last row. B goes back to the page it came from.
fn show_port_list(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, index: usize, back: Again) {
    let path = match screen.borrow().cards.item(index) {
        Some(item) => item.path().to_path_buf(),
        None => return,
    };
    let game_mode = crate::fork::steam_queue::game_mode_from_env();
    let target = ChoiceTarget { path, could_be: Vec::new() };
    let list = Rc::new(PortListScreen::new(&screen.borrow(), &target));
    let shown = Rc::new(RefCell::new(list.matching("")));
    let placeholder = if game_mode { "Type to filter. Steam + X brings up the keyboard" } else { "Type to filter the list" };
    let Some(dialog) = build_text_entry_dialog(app, router, "Pick another port", placeholder, "", &port_rows(&list, &shown.borrow(), game_mode), false) else { return back(app, router) };
    {
        let (app, weak, list, shown) = (app.clone(), dialog.as_weak(), list.clone(), shown.clone());
        dialog.on_search_changed(move |query| {
            let Some(d) = weak.upgrade() else { return };
            *shown.borrow_mut() = list.matching(query.as_str());
            let items: Vec<PickerItem> = port_rows(&list, &shown.borrow(), game_mode).into_iter().map(|label| PickerItem { label: label.into() }).collect();
            d.set_items(slint::ModelRc::new(slint::VecModel::from(items)));
            d.set_selected_index(0);
            app.dialog_nav.picker_index.set(0);
        });
    }
    {
        let (app, router, ctx, screen, list, shown, back) = (app.clone(), router.clone(), ctx.clone(), screen.clone(), list.clone(), shown.clone(), back.clone());
        dialog.on_item_selected(move |row| {
            let row = row.max(0) as usize;
            let (count, picked) = {
                let shown = shown.borrow();
                (shown.len() + extra_rows(game_mode), shown.get(row).copied())
            };
            // A row from before the list was filtered shorter is not a row any more.
            if row >= count {
                return;
            }
            match picked {
                Some(i) => choose_port(&app, &router, &ctx, screen.clone(), index, list.ports[i].key.clone(), back.clone()),
                None if game_mode && row == count - 2 => {
                    show_keyboard();
                }
                None => show_github(&app, &router, &ctx, screen.clone(), Target::Folder { index }, back.clone(), String::new()),
            }
        });
    }
    {
        let (b_app, b_router, b_back) = (app.clone(), router.clone(), back.clone());
        dialog.on_close_requested(move || {
            close_current_dialog(&b_app, &b_router);
            b_back(&b_app, &b_router);
        });
        // The title bar X only hides the window; it goes where B goes.
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
/// B on the measuring list goes back with nothing changed; a folder that cannot be measured says why.
fn choose_port(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, index: usize, key: String, back: Again) {
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
                if let Err(why) = take_back_not_a_port(app, &screen2, &ctx2, index) {
                    return notice_back(app, router, &why, back.clone());
                }
                ctx2.choices.borrow_mut().set(&found.path, Choice::Port(found.port_key.clone()));
                {
                    let mut s = screen2.borrow_mut();
                    let port = s.port(&found.port_key).cloned();
                    let at = s.add_found(found.clone());
                    s.cards.choose(index, Match::of_found(&found, at, port.as_ref()));
                }
                go_next(app, router, &ctx2, screen2.clone(), index);
            }
            Ok(_) => notice_back(app, router, "That folder is too big to look through here, so it was not used. Nothing was changed.", back.clone()),
            Err(e) => notice_back(app, router, &format!("Could not look at that folder ({e}). Nothing was changed."), back.clone()),
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
            let last = screen.borrow().cards.prev_item(usize::MAX);
            match last {
                Some(last) => show_item(app, router, &ctx, screen.clone(), last),
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
            // What will happen to each port, step by step (paths and all), on a plain page; B comes back here.
            CardAction::Details => show_steps(app, router, &ctx, screen.clone(), &plans),
            _ => {}
        }
    };
    show_or_end(app, router, ctx, page, handlers(ctx, act, back_to_walk));
}

/// The steps of each port's move as text, behind Details of the move page.
fn show_steps(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, plans: &[PortPlan]) {
    let home = screen.borrow().home.clone();
    let texts: Vec<String> = plans.iter().map(|p| plan_text(p, &home)).collect();
    let back = {
        let (ctx, screen) = (ctx.clone(), screen.clone());
        move |app: &Rc<AppState>, router: &Router| show_confirm(app, router, &ctx, screen.clone())
    };
    let act = {
        let back = back.clone();
        move |app: &Rc<AppState>, router: &Router, action: CardAction| {
            if action == CardAction::Back {
                back(app, router);
            }
        }
    };
    show_or_end(app, router, ctx, steps_page(&texts), handlers(ctx, act, back));
}

// -- Use a GitHub link ----------------------------------------------------------------------------------

/// What the project from the link is for.
#[derive(Clone, Copy)]
enum Target {
    /// The folder at `index` of the cards: it becomes this project.
    Folder { index: usize },
    /// Nothing was found, so the user picks the folder after the project is confirmed.
    NewFolder,
}

/// GitHub, or the saved answers of the `DPM_GITHUB_REPLAY` test switch.
fn github() -> Box<dyn Fetch> {
    match ReplayFetch::from_env() {
        Some(replay) => Box::new(replay),
        None => Box::new(Net::new()),
    }
}

/// The text box: paste or type the project address. Rows under it: Check this link, Show keyboard (Game
/// Mode), What should I look for?, Cancel. B and the X go back to where the user came from (`back`).
fn show_github(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, target: Target, back: Again, text: String) {
    let rows = link::rows(crate::fork::steam_queue::game_mode_from_env());
    let labels: Vec<String> = rows.iter().map(|r| r.1.to_string()).collect();
    let Some(dialog) = build_text_entry_dialog(app, router, link::WINDOW_TITLE, link::PLACEHOLDER, &text, &labels, false) else { return back(app, router) };
    let typed = Rc::new(RefCell::new(text));
    {
        let typed = typed.clone();
        dialog.on_search_changed(move |query| *typed.borrow_mut() = query.to_string());
    }
    let again: Again = {
        let (ctx, screen, back, typed) = (ctx.clone(), screen.clone(), back.clone(), typed.clone());
        Rc::new(move |app, router| show_github(app, router, &ctx, screen.clone(), target, back.clone(), typed.borrow().clone()))
    };
    {
        let (app, router, ctx, screen, back, typed, again) = (app.clone(), router.clone(), ctx.clone(), screen.clone(), back.clone(), typed.clone(), again.clone());
        dialog.on_item_selected(move |row| {
            let Some((action, _)) = rows.get(row as usize) else { return };
            match action {
                LinkRow::Check => check_link(&app, &router, &ctx, screen.clone(), target, &back, &typed.borrow(), again.clone()),
                LinkRow::ShowKeyboard => {
                    // Slint does not raise the Steam keyboard by itself; ask Steam to. Unverified on a Deck, so the
                    // help text also says how to bring it up by hand.
                    let opened = std::process::Command::new("xdg-open").arg("steam://open/keyboard").spawn().is_ok();
                    if !opened {
                        notice_back(&app, &router, link::KEYBOARD_HINT, again.clone());
                    }
                }
                LinkRow::Help => details_text(&app, &router, "What should I look for?", link::HELP, again.clone()),
                LinkRow::Cancel => {
                    close_current_dialog(&app, &router);
                    back(&app, &router);
                }
            }
        });
    }
    {
        let (b_app, b_router, b_back) = (app.clone(), router.clone(), back.clone());
        dialog.on_close_requested(move || {
            close_current_dialog(&b_app, &b_router);
            b_back(&b_app, &b_router);
        });
        // The title bar X only hides the window; it goes where B goes.
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

/// Reads the text; a bad one gets its reason. A good one is asked about on the worker (a project already in
/// the port list needs no asking: it is that port).
#[allow(clippy::too_many_arguments)]
fn check_link(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, target: Target, back: &Again, text: &str, again: Again) {
    let repo = match parse_link(text) {
        Ok(repo) => repo,
        Err(e) => return notice_back(app, router, &e.text(), again),
    };
    let known = repo_entry::existing(&screen.borrow().scan.ports, &repo).map(|p| p.key().to_string());
    if let Some(key) = known {
        return use_port(app, router, ctx, screen, target, key, back.clone(), again);
    }
    let (ctx2, screen2, back2, again2, asked) = (ctx.clone(), screen.clone(), back.clone(), again.clone(), repo.clone());
    run_job(
        app,
        router,
        JobSpec {
            title: "Checking the link",
            initial: link::checking_text(&repo.slug()),
            cancel_label: "Cancel",
            deliver_after_close: false,
            leave: Some(again.clone()),
            prompts: None,
            is_import: false,
            work: Box::new(move |_| check(github().as_ref(), &asked)),
        },
        move |app, router, result| match result {
            Ok(Ok(checked)) => {
                // GitHub may know the project under another name (a rename): that one may be in the list.
                let known = repo_entry::existing(&screen2.borrow().scan.ports, &checked.repo).map(|p| p.key().to_string());
                match known {
                    Some(key) => use_port(app, router, &ctx2, screen2.clone(), target, key, back2.clone(), again2.clone()),
                    None => show_github_card(app, router, &ctx2, screen2.clone(), target, checked, back2.clone(), again2.clone()),
                }
            }
            Ok(Err(e)) => notice_back(app, router, &e.text(), again2.clone()),
            Err(_) => notice_back(app, router, &CheckError::Unreadable.text(), again2.clone()),
        },
    );
}

/// "Is this the project?": what GitHub said. Yes adds it to this scan's ports (the port list gets it just before
/// the move), Change the link goes back to the text box.
#[allow(clippy::too_many_arguments)]
fn show_github_card(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, target: Target, checked: Checked, back: Again, again: Again) {
    let page = link::confirm_card(&checked);
    let act = {
        let (ctx, screen, back, again, checked) = (ctx.clone(), screen.clone(), back.clone(), again.clone(), checked.clone());
        move |app: &Rc<AppState>, router: &Router, action: CardAction| match action {
            CardAction::UseIt => {
                let built = {
                    let s = screen.borrow();
                    let known: Vec<&str> = s.scan.ports.iter().map(|p| p.folder.as_str()).collect();
                    let folder = repo_entry::folder_for(&checked.repo, |f| repo_entry::folder_taken(&s.ports_folder, known.iter().copied(), f));
                    folder.ok_or_else(|| "No folder name is free for this project in your ports folder.".to_string()).and_then(|f| repo_entry::new_entry(&checked, &f))
                };
                match built {
                    Ok((new, port)) => {
                        let key = port.key().to_string();
                        ctx.extra.borrow_mut().push((port.clone(), new.clone()));
                        {
                            let mut s = screen.borrow_mut();
                            s.scan.new_entries.push((key.clone(), new));
                            s.scan.ports.push(port);
                        }
                        use_port(app, router, &ctx, screen.clone(), target, key, back.clone(), again.clone());
                    }
                    Err(why) => notice_back(app, router, &why, again.clone()),
                }
            }
            CardAction::ChangeLink => again(app, router),
            _ => {}
        }
    };
    let back_to_link = {
        let again = again.clone();
        move |app: &Rc<AppState>, router: &Router| again(app, router)
    };
    show_or_end(app, router, ctx, page, handlers(ctx, act, back_to_link));
}

/// The project (`key`, now among the scan's ports) is the port: for a folder of the walk it is measured like
/// any pick; with no folder yet the user picks the folder first.
#[allow(clippy::too_many_arguments)]
fn use_port(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, target: Target, key: String, back: Again, again: Again) {
    match target {
        Target::Folder { index } => choose_port(app, router, ctx, screen, index, key, again),
        Target::NewFolder => {
            let start = home_dir().unwrap_or_else(|| std::path::PathBuf::from("/"));
            let (ctx2, screen2, back2, again2) = (ctx.clone(), screen.clone(), back.clone(), again.clone());
            open_folder_picker(
                app,
                router,
                PickerRequest::new("Pick the folder this port is in", &start, Purpose::ScanFolder),
                move |app, router, path| {
                    if !path.is_dir() {
                        return notice_back(app, router, "That is not a folder.", again2.clone());
                    }
                    let index = screen2.borrow_mut().cards.add_folder(path);
                    choose_port(app, router, &ctx2, screen2.clone(), index, key.clone(), again2.clone());
                },
                move |app, router| back2(app, router),
            );
        }
    }
}
