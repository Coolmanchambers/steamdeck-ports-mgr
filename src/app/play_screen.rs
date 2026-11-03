//! Play's two questions on screen (`crate::fork::play_pages` says what they show): "Could not start <name>" with Reinstall,
//! Game files and Back, and "Which program starts <name>?" with one option per program. They replace upstream's "Error -
//! X" dialog and its bare list of file names. Both are guided pages (`page_dialog::show_page`); B and the title bar's X
//! leave them with nothing started and nothing changed.

use super::dialogs::close_current_dialog;
use super::page_dialog::{show_page, Handlers};
use super::state::AppState;
use crate::core::models::Port;
use crate::fork::art::thumbs::ArtRef;
use crate::fork::play_pages::{self as pages, PlayAct, Problem};
use crate::fork::port_page::paths::Places;
use crate::ui::gamepad_router::GamepadRouter;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

type Router = Rc<RefCell<GamepadRouter>>;
type Back = Rc<dyn Fn(&Rc<AppState>, &Router)>;
/// What is done with the program chosen: `Some(path)`, or `None` for "Ask every time".
pub(crate) type Picked = Rc<dyn Fn(&Rc<AppState>, &Router, Option<PathBuf>)>;

fn handlers(on_action: impl Fn(&Rc<AppState>, &Router, PlayAct) + 'static) -> Handlers<PlayAct> {
    let on_action: Rc<dyn Fn(&Rc<AppState>, &Router, PlayAct)> = Rc::new(on_action);
    let (b, x) = (on_action.clone(), on_action.clone());
    Handlers { on_action, on_back: Rc::new(move |app, router| b(app, router, PlayAct::Back)), on_exit: Rc::new(move |app, router| x(app, router, PlayAct::Back)), on_input: None }
}

/// The page again, for the pages that Reinstall and Game files lead to when they are left with B.
fn back_here(port: Port, problem: Problem) -> Back {
    Rc::new(move |app, router| could_not_start(app, router, &port, problem.clone()))
}

/// "Could not start <name>": why, and Reinstall (the install wizard's "Update this port?"), Game files (the one-port game
/// files page) and Back. Both lead back here when left with B.
pub(crate) fn could_not_start(app: &Rc<AppState>, router: &Router, port: &Port, problem: Problem) {
    eprintln!("play: could not start {}: {problem:?}", port.display_name());
    let name = port.display_name().to_string();
    let page = pages::could_not_start_page(&name, &Some(ArtRef::of_port(port)), &problem, super::rom_screens::port_needs_game_files(app, port));
    let port = port.clone();
    show_page(
        app,
        router,
        page,
        handlers(move |app, router, act| match act {
            PlayAct::Reinstall => super::install_wizard_screen::open(app, router, port.clone(), back_here(port.clone(), problem.clone())),
            PlayAct::GameFiles => super::rom_screens::find_for_port(app, router, port.key(), back_here(port.clone(), problem.clone())),
            _ => close_current_dialog(app, router),
        }),
    );
}

/// "Which program starts <name>?" for `candidates` (paths inside `game_dir`). A choice closes the page and goes to
/// `picked`; B just closes it. `ask_every_time`: the page also offers to leave the choice open (Fix problems; Play's own
/// question has no such answer).
pub(crate) fn choose_program(app: &Rc<AppState>, router: &Router, port: &Port, game_dir: &Path, candidates: Vec<PathBuf>, ask_every_time: bool, picked: Picked) {
    let choices = pages::choices_of(&candidates, &Places::system());
    let likely = pages::likely_program(game_dir, &candidates, &port.folder, port.display_name());
    let page = pages::which_program_page(port.display_name(), &Some(ArtRef::of_port(port)), &choices, likely, ask_every_time);
    show_page(
        app,
        router,
        page,
        handlers(move |app, router, act| {
            let choice = match act {
                PlayAct::Program(i) => match candidates.get(i) {
                    Some(path) => Some(Some(path.clone())),
                    None => return,
                },
                PlayAct::AskEveryTime => Some(None),
                _ => None,
            };
            close_current_dialog(app, router);
            if let Some(path) = choice {
                picked(app, router, path);
            }
        }),
    );
}
