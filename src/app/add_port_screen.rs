//! "Add a port": the page the Home screen's plus tile opens (and Settings > Ports > Add a port). One question and a few
//! cards (`crate::fork::add_port` says what they show; the window is `page_dialog::show_page`). Each card hands over to
//! a screen that already exists: the Deck-ready list, the catalog list, Import ports, and Import ports' GitHub link
//! page. B returns to where the page was opened from: the Home screen (the cursor still on the plus tile), or Settings'
//! Ports page.

use super::dialogs::{close_current_dialog, dialog_is_open};
use super::page_dialog::{show_page, Handlers};
use super::state::AppState;
use crate::fork::add_port::{self, Act};
use crate::fork::home::List;
use crate::fork::settings_menu::Group;
use crate::ui::gamepad_router::GamepadRouter;
use std::cell::RefCell;
use std::rc::Rc;

type Router = Rc<RefCell<GamepadRouter>>;

/// Opens the page. `from_settings`: it was reached from Settings, so B goes back there instead of to Home.
pub(crate) fn open(app: &Rc<AppState>, router: &Router, from_settings: bool) {
    if dialog_is_open(app) {
        return;
    }
    show(app, router, from_settings);
}

fn show(app: &Rc<AppState>, router: &Router, from_settings: bool) {
    let page = add_port::page(app.window().get_big_mode(), if from_settings { "Back" } else { "Home" });
    let handlers = Handlers {
        on_action: Rc::new(move |app, router, act| match act {
            Act::Back => back(app, router, from_settings),
            Act::DeckReady => {
                close_current_dialog(app, router);
                super::home_screen::show_list(app, router, List::DeckReady);
            }
            Act::Browse => {
                close_current_dialog(app, router);
                super::home_screen::show_list(app, router, List::AllPorts);
            }
            Act::Import => {
                close_current_dialog(app, router);
                super::import_screen::open(app, router, false);
            }
            Act::GithubLink => {
                close_current_dialog(app, router);
                // B on the link page (and a failed or cancelled start) comes back to this page.
                let again: super::import_screen::Step = Rc::new(move |app, router| show(app, router, from_settings));
                super::import_screen::open_link(app, router, again);
            }
        }),
        on_back: Rc::new(move |app, router| back(app, router, from_settings)),
        on_exit: Rc::new(|app, router| close_current_dialog(app, router)),
        on_input: None,
    };
    show_page(app, router, page, handlers);
}

fn back(app: &Rc<AppState>, router: &Router, from_settings: bool) {
    close_current_dialog(app, router);
    if from_settings {
        super::settings_menu_screen::open_group(app, router, Group::Ports);
    }
}
