//! Fork (same game): the installed games the lists compare with, the badges of All ports, and the "You already have
//! this game" question before Info's Select version (installs ask on the install wizard's first page). The rules are in
//! `crate::fork::same_game`.

use super::dialogs::close_current_dialog;
use super::page_dialog::{show_page, Handlers};
use super::state::AppState;
use crate::core::models::Port;
use crate::fork::art::thumbs::ArtRef;
use crate::fork::same_game::{self, Badge, Game, Owned, SameAct, Verdict};
use crate::ui::gamepad_router::GamepadRouter;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

type Router = Rc<RefCell<GamepadRouter>>;
type Back = Rc<dyn Fn(&Rc<AppState>, &Router)>;

/// At most this many match lines per list rebuild go to the log.
const LOG_LINES: usize = 3;

thread_local! {
    /// The lines each list logged last, so typing in the search box does not repeat them.
    static LOGGED: RefCell<HashMap<&'static str, Vec<String>>> = RefCell::new(HashMap::new());
}

/// What is installed, read once per list rebuild: one look at the disk per catalog port, shared by the rows' own
/// "installed" and the same-game badges.
pub(crate) struct Installed {
    folders: HashSet<String>,
    pub(crate) owned: Vec<Owned>,
}

impl Installed {
    pub(crate) fn read(app: &AppState) -> Installed {
        let mut folders = HashSet::new();
        let mut owned = Vec::new();
        for p in app.catalog.borrow().iter().filter(|p| app.is_installed(p)) {
            folders.insert(p.folder.clone());
            owned.push(Owned { art: Some(ArtRef::of_port(p)), ..Owned::new(p.key(), &p.name, &p.folder, &p.tags_lower) });
        }
        Installed { folders, owned }
    }

    /// The same answer as `AppState::is_installed` gave when this was read (installed is a folder in the ports folder).
    pub(crate) fn has(&self, port: &Port) -> bool {
        self.folders.contains(&port.folder)
    }

    /// The badge of a port in All ports (and its search): `None` when it is installed or its game is not.
    pub(crate) fn badge(&self, port: &Port, log: &mut Vec<String>) -> Option<Badge> {
        if self.has(port) {
            return None;
        }
        let found = same_game::find(&self.owned, port.key(), &Game::of_port(&port.name, &port.tags_lower))?;
        log.push(same_game::log_line(found.verdict, port.key(), &found.owned.key));
        Some(same_game::badge(&found))
    }
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

/// Before Info's Select version on a port that is not installed: when it is the same game as an installed port, the
/// question page first ("Not now" or "Install it as well"); otherwise `then` at once. Every other install asks on the
/// install wizard's first page instead (`install_wizard_screen`, the note folded into it).
pub(crate) fn confirm_then(app: &Rc<AppState>, router: &Router, port: &Port, then: impl Fn(&Rc<AppState>, &Router) + 'static) {
    // Safety net: callers only come here for a port that is not installed, and an installed one never asks.
    if app.is_installed(port) {
        return then(app, router);
    }
    let not_now: Back = Rc::new(|app, router| close_current_dialog(app, router));
    let project = same_game::project_name(&port.name, &port.folder);
    ask_or_go(app, router, port.key(), &port.name, &project, &Game::of_port(&port.name, &port.tags_lower), not_now, then);
}

#[allow(clippy::too_many_arguments)]
fn ask_or_go(app: &Rc<AppState>, router: &Router, key: &str, name: &str, project: &str, game: &Game, not_now: Back, then: impl Fn(&Rc<AppState>, &Router) + 'static) {
    let owned = Installed::read(app).owned;
    let Some(found) = same_game::find(&owned, key, game).filter(|f| f.verdict == Verdict::SameGame) else {
        return then(app, router);
    };
    eprintln!("same game: asking before installing {name} ({})", found.owned.key);
    let page = same_game::confirm_page(name, project, found.owned);
    let then = Rc::new(then);
    let (n1, n2, n3) = (not_now.clone(), not_now.clone(), not_now);
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| match act {
            SameAct::InstallAnyway => {
                close_current_dialog(app, router);
                then(app, router);
            }
            SameAct::NotNow => n1(app, router),
        }),
        on_back: Rc::new(move |app, router| n2(app, router)),
        on_exit: Rc::new(move |app, router| n3(app, router)),
        on_input: None,
    };
    // A dialog still open (Info, for Select version) gives way to the page; a page window is only parked and taken back.
    close_current_dialog(app, router);
    show_page(app, router, page, handlers);
}
