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

use super::dialogs::close_current_dialog;
use super::folder_picker_screen::open_folder_picker;
use super::import_screen::{Outcome,
    build_plans, confirm_back, end_notice, home_dir, notice_back, run_job, save_folders, settings_path, show_folders, start_move, start_scan, Ctx, FoldersState, JobSpec, Step, TITLE,
};
use super::page_dialog::{show_page, Handlers};
use super::state::AppState;
use crate::fork::art::net::{Fetch, Net};
use crate::fork::custom_repo::check::{check_with, CheckError, Checked, ReplayFetch};
use crate::fork::custom_repo::entry as repo_entry;
use crate::fork::custom_repo::parse::parse_link;
use crate::fork::folder_picker::{PickerRequest, Purpose};
use crate::fork::art::thumbs::ArtRef;
use crate::fork::import::cards::{self, card_page, confirm_page, details_page, finished_page, found_page, scan_notes, steps_page, which_page, CardAction, Item, Match, SteamWaiting, Verdict};
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
use std::cell::RefCell;
use std::rc::Rc;

type Router = Rc<RefCell<GamepadRouter>>;
type Screen = Rc<RefCell<ImportScreen>>;
/// A step that shows a page again: what OK and B on a Details text go back to.
type Again = Step;

thread_local! {
    /// The Steam close flow started Steam again: the finished page it returns to says so, once.
    static STEAM_STARTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Leaves Import ports: the page closes and the setup wizard, if it waits, is told.
fn leave(app: &Rc<AppState>, router: &Router, ctx: &Ctx) {
    close_current_dialog(app, router);
    ctx.finish_quiet(app, router);
}

/// A page's handlers: `act` says what each item does; B goes `back`; the X leaves.
fn handlers<A>(ctx: &Ctx, act: impl Fn(&Rc<AppState>, &Router, A) + 'static, back: impl Fn(&Rc<AppState>, &Router) + 'static) -> Handlers<A> {
    let ctx = ctx.clone();
    // The title bar X ends Import ports, and in the setup wizard the whole wizard, as the X on its own pages does.
    let on_exit: Rc<dyn Fn(&Rc<AppState>, &Router)> = Rc::new(move |app, router| if ctx.from_wizard { super::setup_wizard_screen::exit_wizard(app, router) } else { leave(app, router, &ctx) });
    Handlers { on_action: Rc::new(act), on_back: Rc::new(back), on_exit, on_input: None }
}

/// If the page cannot be built nothing is lost: the flow ends as if the user had left.
fn show_or_end<A: Clone + 'static>(app: &Rc<AppState>, router: &Router, ctx: &Ctx, page: Page<A>, handlers: Handlers<A>) {
    // In the setup wizard these pages are its second step.
    let page = if ctx.from_wizard { page.in_wizard(2, crate::fork::setup_wizard::flow::Step::COUNTED) } else { page };
    if !show_page(app, router, page, handlers) {
        ctx.finish_quiet(app, router);
    }
}

/// The help for the link page, as a page with one OK.
fn show_help(app: &Rc<AppState>, router: &Router, ctx: &Ctx, again: Again) {
    let on_ok = again.clone();
    let act = move |app: &Rc<AppState>, router: &Router, _action: CardAction| on_ok(app, router);
    show_or_end(app, router, ctx, link::help_page(), handlers(ctx, act, move |app, router| again(app, router)));
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
        move |app: &Rc<AppState>, router: &Router| {
            // In the setup wizard this page is the first of its Your ports step: B goes back to the question before.
            if ctx.from_wizard {
                super::import_screen::note_went_back();
            }
            leave(app, router, &ctx)
        }
    };
    show_or_end(app, router, ctx, page, handlers(ctx, act, back));
}

/// The first page of "From a GitHub link" on the Add a port page: the link page itself. B and Cancel leave Import ports and
/// run `back` (the Add a port page again).
pub(super) fn show_link_first(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, back: super::import_screen::Step) {
    let leave_to: Again = {
        let ctx = ctx.clone();
        Rc::new(move |app, router| {
            leave(app, router, &ctx);
            back(app, router);
        })
    };
    show_github(app, router, ctx, screen, Target::NewFolder, leave_to, String::new());
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
            CardAction::PickAnother => show_port_list(app, router, &ctx, screen.clone(), index, again.clone(), String::new()),
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
                    choose_port(app, router, &ctx, screen.clone(), Folder::Index(index), maybe.key.clone(), again.clone());
                }
            }
            CardAction::PickAnother => show_port_list(app, router, &ctx, screen.clone(), index, again.clone(), String::new()),
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

/// Asks Steam for its on-screen keyboard. Unverified on a Deck, so a failure says how to bring it up by hand.
fn open_keyboard(app: &Rc<AppState>, router: &Router, again: Again) {
    let opened = std::process::Command::new("xdg-open").arg("steam://open/keyboard").spawn().is_ok();
    if !opened {
        notice_back(app, router, link::KEYBOARD_HINT, again);
    }
}

/// "Pick another port": the ports not in the ports folder yet, the ones named like the folder first, with a
/// text box in the page to filter and "Use a GitHub link..." beside Back. B goes back to the page it came from.
fn show_port_list(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, index: usize, back: Again, filter: String) {
    let path = match screen.borrow().cards.item(index) {
        Some(item) => item.path().to_path_buf(),
        None => return,
    };
    let game_mode = crate::fork::steam_queue::game_mode_from_env();
    let target = ChoiceTarget { path, could_be: Vec::new() };
    let list = Rc::new(PortListScreen::new(&screen.borrow(), &target));
    let page_for = {
        let list = list.clone();
        move |filter: &str| {
            let matching = list.matching(filter);
            let rows: Vec<(usize, String)> = matching.iter().take(cards::MAX_LIST_ROWS).map(|i| (*i, ellipsize(&list.ports[*i].label(), 90))).collect();
            cards::port_list_page(&rows, matching.len(), filter, game_mode)
        }
    };
    let page = page_for(&filter);
    let typed = Rc::new(RefCell::new(filter));
    let again: Again = {
        let (ctx, screen, back, typed) = (ctx.clone(), screen.clone(), back.clone(), typed.clone());
        Rc::new(move |app, router| show_port_list(app, router, &ctx, screen.clone(), index, back.clone(), typed.borrow().clone()))
    };
    let act = {
        let (ctx, screen, list, back, again) = (ctx.clone(), screen.clone(), list.clone(), back.clone(), again.clone());
        move |app: &Rc<AppState>, router: &Router, action: CardAction| match action {
            CardAction::ListPort(i) => {
                if let Some(port) = list.ports.get(i) {
                    choose_port(app, router, &ctx, screen.clone(), Folder::Index(index), port.key.clone(), back.clone());
                }
            }
            CardAction::GithubLink => show_github(app, router, &ctx, screen.clone(), Target::Folder { index }, again.clone(), String::new()),
            CardAction::ShowKeyboard => open_keyboard(app, router, again.clone()),
            CardAction::Back => back(app, router),
            _ => {}
        }
    };
    let b = back.clone();
    let mut h = handlers(ctx, act, move |app, router| b(app, router));
    h.on_input = Some(Rc::new(move |_, _, text| {
        *typed.borrow_mut() = text.to_string();
        Some(page_for(text))
    }));
    show_or_end(app, router, ctx, page, h);
}

/// The user picked port `key` for the folder at `index`: it is measured on the worker (size, Steam shortcuts),
/// then the folder counts as that port, said yes to, and goes through the same plan and checks as any other.
/// B on the measuring list goes back with nothing changed; a folder that cannot be measured says why.
/// Which folder a picked port is for: one of the walk, or one the user chose by hand (it joins the walk only
/// once it has been measured, so a failed or cancelled pick leaves nothing behind).
enum Folder {
    Index(usize),
    New(std::path::PathBuf),
}

fn choose_port(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, folder: Folder, key: String, back: Again) {
    let (port, shortcuts, new_entry, path) = {
        let s = screen.borrow();
        let Some(port) = s.port(&key).cloned() else { return };
        let path = match &folder {
            Folder::Index(index) => match s.cards.item(*index) {
                Some(item) => item.path().to_path_buf(),
                None => return,
            },
            Folder::New(path) => path.clone(),
        };
        let new_entry = s.scan.new_entries.iter().find(|(k, _)| *k == key).map(|(_, e)| e.clone());
        (port, s.scan.shortcuts.clone(), new_entry, path)
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
                let known = match &folder {
                    Folder::Index(index) => Some(*index),
                    Folder::New(path) => screen2.borrow().cards.index_of(path),
                };
                if let Some(known) = known {
                    if let Err(why) = take_back_not_a_port(app, &screen2, &ctx2, known) {
                        return notice_back(app, router, &why, back.clone());
                    }
                }
                let index = known.unwrap_or_else(|| screen2.borrow_mut().cards.add_folder(found.path.clone()));
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

/// The text box page: paste or type the project address. Choices: Check this link, Show keyboard (Game
/// Mode), More help; Cancel. B goes back to where the user came from (`back`).
fn show_github(app: &Rc<AppState>, router: &Router, ctx: &Ctx, screen: Screen, target: Target, back: Again, text: String) {
    let page = link::entry_page(&text, crate::fork::steam_queue::game_mode_from_env());
    let typed = Rc::new(RefCell::new(text));
    let again: Again = {
        let (ctx, screen, back, typed) = (ctx.clone(), screen.clone(), back.clone(), typed.clone());
        Rc::new(move |app, router| show_github(app, router, &ctx, screen.clone(), target, back.clone(), typed.borrow().clone()))
    };
    let act = {
        let (ctx, screen, back, typed, again) = (ctx.clone(), screen.clone(), back.clone(), typed.clone(), again.clone());
        move |app: &Rc<AppState>, router: &Router, action: LinkRow| match action {
            LinkRow::Check => {
                let text = typed.borrow().clone();
                check_link(app, router, &ctx, screen.clone(), target, &back, &text, again.clone());
            }
            LinkRow::ShowKeyboard => open_keyboard(app, router, again.clone()),
            LinkRow::Help => show_help(app, router, &ctx, again.clone()),
            LinkRow::Cancel => back(app, router),
        }
    };
    let b = back.clone();
    let mut h = handlers(ctx, act, move |app, router| b(app, router));
    h.on_input = Some(Rc::new(move |_, _, text| {
        *typed.borrow_mut() = text.to_string();
        None
    }));
    show_or_end(app, router, ctx, page, h);
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
    let token = app.api_tokens().0;
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
            work: Box::new(move |ctl| check_with(github().as_ref(), &asked, token.as_deref(), &|| ctl.cancelled())),
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
        Target::Folder { index } => choose_port(app, router, ctx, screen, Folder::Index(index), key, again),
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
                    choose_port(app, router, &ctx2, screen2.clone(), Folder::New(path), key.clone(), again2.clone());
                },
                move |app, router| back2(app, router),
            );
        }
    }
}

// -- The end -------------------------------------------------------------------------------------------

/// The end of an import: "4 ports moved" with the covers, a short line for anything that went wrong, and one
/// button. The long text (with paths) is behind Details. Replaces the old dense "Import finished" dialog.
pub(super) fn show_finished(app: &Rc<AppState>, router: &Router, ctx: &Ctx, text: &str, keys: &[String], outcome: &Outcome) {
    let ports_now: Vec<(String, Option<ArtRef>)> = {
        let catalog = app.catalog.borrow();
        keys.iter().filter_map(|k| catalog.iter().find(|p| p.key() == k)).map(|p| (p.display_name().to_string(), Some(ArtRef::of_port(p)))).collect()
    };
    let waiting = crate::fork::steam_queue::load(&crate::fork::steam_queue::queue_path(&app.paths.config_dir)).queue.len();
    let steam = match (waiting, crate::fork::steam_queue::game_mode_from_env()) {
        (0, _) => SteamWaiting::Nothing,
        (n, true) => SteamWaiting::GameMode(n),
        (n, false) => SteamWaiting::Desktop(n),
    };
    let starting = STEAM_STARTING.with(|s| s.replace(false));
    let find_files = {
        let catalog = app.catalog.borrow();
        keys.iter().filter_map(|k| catalog.iter().find(|p| p.key() == k)).filter(|p| super::rom_screens::port_missing_game_files(app, p)).count()
    };
    let page = finished_page(outcome.moved, &ports_now, &outcome.lines, ctx.from_wizard, steam, starting, find_files);
    let full = text.to_string();
    let this: Again = {
        let (ctx, keys, outcome, full) = (ctx.clone(), keys.to_vec(), outcome.clone(), full.clone());
        Rc::new(move |app, router| show_finished(app, router, &ctx, &full, &keys, &outcome))
    };
    let act = {
        let ctx = ctx.clone();
        let keys_now = keys.to_vec();
        move |app: &Rc<AppState>, router: &Router, action: CardAction| match action {
            CardAction::Leave => leave(app, router, &ctx),
            CardAction::Details => details_text(app, router, "Import ports", &format!("{full}\n"), this.clone()),
            // The game file search, as its own page after this one (no summary on top of it).
            CardAction::FindGameFiles => {
                let leave_after: Step = {
                    let ctx = ctx.clone();
                    Rc::new(move |app, router| leave(app, router, &ctx))
                };
                // Searched even when these ports were looked for before (the user asked for it now).
                if !super::rom_screens::search_ports_now(app, router, &keys_now, Some(leave_after)) {
                    super::import_screen::message(app, router, "Game files", "Another task is still running. Try Find their game files again in a moment.");
                }
            }
            // The shared flow; this page comes back after it, saying what is left.
            CardAction::CloseSteam => {
                let again = this.clone();
                let then: super::steam_close_screen::Then = Rc::new(move |app, router, done| {
                    STEAM_STARTING.with(|s| s.set(done.steam_started));
                    again(app, router)
                });
                super::steam_close_screen::open(app, router, super::steam_close_screen::Opts::default(), then);
            }
            _ => {}
        }
    };
    // B is the one button, and so is the X: nothing here asks anything, so leaving is the only way on.
    let back = {
        let ctx = ctx.clone();
        move |app: &Rc<AppState>, router: &Router| leave(app, router, &ctx)
    };
    show_or_end(app, router, ctx, page, handlers(ctx, act, back));
}
