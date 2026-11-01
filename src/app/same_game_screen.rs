//! Fork (same game): the installed games the lists compare with, the badges of All ports, and the "You already have
//! this game" question before an install. The rules are in `crate::fork::same_game`.

use super::dialogs::close_current_dialog;
use super::page_dialog::{show_page, Handlers};
use super::state::AppState;
use crate::core::models::Port;
use crate::fork::art::thumbs::ArtRef;
use crate::fork::same_game::{self, Badge, Game, Owned, SameAct, Verdict};
use crate::ui::gamepad_router::GamepadRouter;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

type Router = Rc<RefCell<GamepadRouter>>;

/// At most this many match lines per list rebuild go to the log.
const LOG_LINES: usize = 3;

thread_local! {
    /// The lines each list logged last, so typing in the search box does not repeat them.
    static LOGGED: RefCell<HashMap<&'static str, Vec<String>>> = RefCell::new(HashMap::new());
}

/// The installed ports (the user's own entries too), as games. Built once per list rebuild.
pub(crate) fn owned_games(app: &AppState) -> Vec<Owned> {
    app.catalog
        .borrow()
        .iter()
        .filter(|p| app.is_installed(p))
        .map(|p| Owned { art: Some(ArtRef::of_port(p)), ..Owned::new(p.key(), &p.name, &p.tags_lower) })
        .collect()
}

/// Logs a list's matches (`same game: <verdict> <port key> vs <installed key>`) when they changed since its last rebuild.
pub(crate) fn log_matches(list: &'static str, lines: Vec<String>) {
    LOGGED.with(|l| {
        let mut logged = l.borrow_mut();
        if logged.get(list) == Some(&lines) {
            return;
        }
        for line in lines.iter().take(LOG_LINES) {
            eprintln!("{line}");
        }
        logged.insert(list, lines);
    });
}

/// The badge of each port in `ports` (All ports and its search): `None` for an installed port and for a game you do
/// not have.
pub(crate) fn list_badges(app: &AppState, ports: &[Port]) -> Vec<Option<Badge>> {
    let owned = owned_games(app);
    let mut log = Vec::new();
    let badges = ports
        .iter()
        .map(|p| {
            if app.is_installed(p) {
                return None;
            }
            let found = same_game::find(&owned, p.key(), &Game::of_port(&p.name, &p.tags_lower))?;
            log.push(same_game::log_line(found.verdict, p.key(), &found.owned.key));
            Some(same_game::badge(&found))
        })
        .collect();
    log_matches("all ports", log);
    badges
}

/// Before installing `port`: when it is not installed and is the same game as an installed port, the question page
/// first ("Not now" or "Install it as well"); otherwise `then` at once. Updates and reinstalls never come here.
pub(crate) fn confirm_then(app: &Rc<AppState>, router: &Router, port: &Port, then: impl Fn(&Rc<AppState>, &Router) + 'static) {
    if app.is_installed(port) {
        return then(app, router);
    }
    ask_or_go(app, router, port.key(), port.display_name(), &Game::of_port(&port.name, &port.tags_lower), then);
}

/// [`confirm_then`] for a Deck-ready entry the app does not have yet (it is added as the user's own entry after).
pub(crate) fn confirm_entry_then(app: &Rc<AppState>, router: &Router, name: &str, entry_name: &str, system: Option<&str>, then: impl Fn(&Rc<AppState>, &Router) + 'static) {
    ask_or_go(app, router, "", name, &Game::of_entry(entry_name, system), then);
}

fn ask_or_go(app: &Rc<AppState>, router: &Router, key: &str, name: &str, game: &Game, then: impl Fn(&Rc<AppState>, &Router) + 'static) {
    let owned = owned_games(app);
    let Some(found) = same_game::find(&owned, key, game).filter(|f| f.verdict == Verdict::SameGame) else {
        return then(app, router);
    };
    eprintln!("same game: asking before installing {name} ({})", found.owned.key);
    let page = same_game::confirm_page(name, found.owned);
    let then = Rc::new(then);
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| {
            close_current_dialog(app, router);
            if act == SameAct::InstallAnyway {
                then(app, router);
            }
        }),
        on_back: Rc::new(|app, router| close_current_dialog(app, router)),
        on_exit: Rc::new(|app, router| close_current_dialog(app, router)),
        on_input: None,
    };
    show_page(app, router, page, handlers);
}
