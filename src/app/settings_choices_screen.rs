//! Settings > App > Themes, Language, Tools and Files as pages (`crate::fork::settings_choices` says what they show).
//! B and Back go up to the App page of Settings. Choosing a theme applies and saves it at once and the page stays (so
//! another can be tried); choosing a language saves it and goes back; a tool shows its address and a QR code first
//! (`port_page_screen::open_link`), then opens in the browser from there; a file opens in a text editor (the card is not
//! offered in Game Mode). What is saved is as before (`dialogs::choose_theme`, `dialogs::choose_language`).

use super::dialogs::{close_current_dialog, LANGUAGES};
use super::page_dialog::{show_page, Handlers};
use super::state::AppState;
use crate::fork::settings_choices::{self as choices, ChoiceAct};
use crate::fork::settings_menu::Group;
use crate::ui::gamepad_router::GamepadRouter;
use crate::Tr;
use slint::ComponentHandle;
use std::cell::RefCell;
use std::rc::Rc;

type Router = Rc<RefCell<GamepadRouter>>;

/// Up to the App page of Settings (closing the page first: `open_group` gives way to an open dialog).
fn to_app_page(app: &Rc<AppState>, router: &Router) {
    close_current_dialog(app, router);
    super::settings_menu_screen::open_group(app, router, Group::App);
}

fn header(app: &AppState) -> String {
    app.window().global::<Tr>().invoke_dialog_title_settings().to_string()
}

fn handlers(on_pick: impl Fn(&Rc<AppState>, &Router, usize) + 'static) -> Handlers<ChoiceAct> {
    Handlers {
        on_action: Rc::new(move |app, router, act| match act {
            ChoiceAct::Pick(i) => on_pick(app, router, i),
            ChoiceAct::Back => to_app_page(app, router),
        }),
        on_back: Rc::new(to_app_page),
        on_exit: Rc::new(close_current_dialog),
        on_input: None,
    }
}

pub(crate) fn open_themes(app: &Rc<AppState>, router: &Router) {
    let names = crate::ui::theme::list_theme_names(&app.theme.theme_config.borrow());
    let current = app.state.borrow().active_theme.clone();
    let title = app.window().global::<Tr>().invoke_label_themes().to_string();
    let page = choices::theme_page(&header(app), &title, &names, &current);
    show_page(
        app,
        router,
        page,
        handlers(move |app, router, i| {
            if let Some(name) = names.get(i) {
                super::dialogs::choose_theme(app, name);
                open_themes(app, router);
            }
        }),
    );
}

pub(crate) fn open_language(app: &Rc<AppState>, router: &Router) {
    let (title, system) = {
        let window = app.window();
        let tr = window.global::<Tr>();
        (tr.invoke_dialog_title_language().to_string(), tr.invoke_language_default_system().to_string())
    };
    // Upstream's "Default (System)" in English reads as our plain words; another language keeps its translation.
    let system = crate::fork::settings_menu::plain_or_translated(&system, "Default (System)", "Same as the system");
    let current = app.state.borrow().language.clone();
    let page = choices::language_page(&header(app), &title, &system, LANGUAGES, &current);
    show_page(
        app,
        router,
        page,
        handlers(|app, router, i| {
            if let Some(code) = choices::language_code(LANGUAGES, i) {
                super::dialogs::choose_language(app, code);
            }
            to_app_page(app, router);
        }),
    );
}

pub(crate) fn open_tools(app: &Rc<AppState>, router: &Router) {
    let tools: Vec<(String, String)> = crate::core::config::load_tools(&app.paths.config_dir.join("ports.json"))
        .into_iter()
        .filter(|t| crate::core::launch::is_web_url(&t.source))
        .map(|t| (t.name, t.source))
        .collect();
    let title = app.window().global::<Tr>().invoke_label_tools().to_string();
    let page = choices::tools_page(&header(app), &title, &tools);
    show_page(
        app,
        router,
        page,
        handlers(move |app, router, i| {
            if let Some((name, url)) = tools.get(i) {
                let back: super::port_page_screen::Back = Rc::new(open_tools);
                super::port_page_screen::open_link_with(app, router, name, url, "Tool page", "This tool's own web page.", Some(back));
            }
        }),
    );
}

pub(crate) fn open_files(app: &Rc<AppState>, router: &Router) {
    let title = app.window().global::<Tr>().invoke_label_files().to_string();
    let page = choices::files_page(&header(app), &title);
    show_page(
        app,
        router,
        page,
        handlers(|app, _, i| {
            if let Some((file, _, _)) = choices::FILES.get(i) {
                crate::core::launch::open_file(&app.paths.config_dir.join(file));
            }
        }),
    );
}
