//! The main window's dim layer while a page or dialog is open (docs/design/port-page-and-one-window.md, part 1).
//!
//! Pages and dialogs are top-level windows of their own. When the main window is clicked, tapped or Alt+Tabbed to,
//! it comes in front of them (in full screen it even covers them), and before this layer it looked usable while it
//! ignored the controller. Now, while anything is open, the main window shows a dim layer with one line ("Settings is
//! open") and a "Go back to it" button. A, Return, Space, a click or a tap anywhere on the layer brings the page back
//! in front with the keyboard. Looking at the main window alone does not raise the page: one press on the layer does.
//!
//! The layer follows the dialog slot: [`sync`] runs when a dialog opens (`finish_dialog_open`), when it closes
//! (`close_current_dialog`), when the page window's short park ends, on every page shown, and every
//! [`RESYNC`] while the layer is up, so no way of closing can leave it behind. While the page window is parked
//! (a hand-off between two screens) the layer stays as it was, so it does not flicker.
//!
//! In Desktop Mode the windows are no longer marked as owned by the main window ([`keep_owner`]), so each one has its
//! own taskbar entry; Game Mode (gamescope, one window at a time) keeps upstream's owner, which gamescope uses.

use super::dialogs::{dialog_is_open, DialogSlot};
use super::state::AppState;
use crate::ui::chrome;
use slint::ComponentHandle;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

/// How often the layer checks itself against the dialog slot while it is up.
const RESYNC: Duration = Duration::from_millis(250);

thread_local! {
    static TIMER: slint::Timer = slint::Timer::default();
    static WIRED: Cell<bool> = const { Cell::new(false) };
}

/// What the layer should show.
#[derive(Debug, PartialEq)]
enum Layer {
    Hidden,
    Shown(String),
    /// The page window is parked between two screens: leave the layer as it is.
    Keep,
}

/// The layer's line for the open window's title.
fn layer_text(title: &str) -> String {
    let title = title.trim();
    if title.is_empty() {
        "A window of this app is open".into()
    } else {
        format!("{title} is open")
    }
}

/// `open`: the title of the dialog in the slot, when there is one; `parked`: the page window waits for its next page.
fn layer_for(open: Option<&str>, parked: bool) -> Layer {
    match open {
        Some(title) => Layer::Shown(layer_text(title)),
        None if parked => Layer::Keep,
        None => Layer::Hidden,
    }
}

/// The main window is in front of an open dialog, so the controller is meant for the layer: Desktop Mode only. Under
/// gamescope (one window at a time) the controller keeps going to the open page as it always did.
fn decide_main_in_front(game_mode: bool, main: Option<bool>, dialog: Option<bool>) -> bool {
    !game_mode && main == Some(true) && dialog != Some(true)
}

/// Whether a dialog is marked as owned by the main window (upstream's `own_window`). An owned (transient) window has no
/// taskbar entry in KDE, so Desktop Mode leaves it out; gamescope uses it to show the dialog, so Game Mode keeps it, and so
/// does Windows, where an owned window always stays above its owner.
pub(crate) fn keep_owner() -> bool {
    !cfg!(target_os = "linux") || crate::fork::steam_queue::game_mode_from_env()
}

fn slot_title(slot: &DialogSlot) -> Option<String> {
    let title = match slot {
        DialogSlot::None => return None,
        DialogSlot::Message(d) => d.get_dialog_title(),
        DialogSlot::Confirm(d) => d.get_dialog_title(),
        DialogSlot::Error(d) => d.get_dialog_title(),
        DialogSlot::Info(d) => d.get_dialog_title(),
        DialogSlot::Progress(d) => d.get_dialog_title(),
        DialogSlot::Picker(d) => d.get_dialog_title(),
        DialogSlot::SearchList(d) => d.get_dialog_title(),
        DialogSlot::DeckReadyInfo(d) => d.get_dialog_title(),
        DialogSlot::FolderPicker(d) => d.get_dialog_title(),
        DialogSlot::SetupWizard(d) => d.get_dialog_title(),
    };
    Some(title.to_string())
}

/// Puts the layer in step with what is open. Cheap; safe to call from anywhere on the UI thread.
pub(crate) fn sync(app: &Rc<AppState>) {
    let Some(main) = app.window.upgrade() else { return };
    wire_once(app, &main);
    // Called from inside other dialog code: a slot borrowed right now is read again by the timer.
    let Ok(slot) = app.dialog_nav.dialogs.try_borrow() else { return };
    let open = slot_title(&slot);
    drop(slot);
    match layer_for(open.as_deref(), super::page_dialog::parked()) {
        Layer::Keep => {}
        Layer::Hidden => {
            if main.get_dialog_layer_shown() {
                main.set_dialog_layer_shown(false);
                eprintln!("dialog layer: hidden");
            }
            TIMER.with(|t| t.stop());
        }
        Layer::Shown(text) => {
            if main.get_dialog_layer_text() != text.as_str() || !main.get_dialog_layer_shown() {
                eprintln!("dialog layer: shown {text:?}");
                main.set_dialog_layer_text(text.into());
                main.set_dialog_layer_shown(true);
            }
            if !TIMER.with(|t| t.running()) {
                let app = app.clone();
                TIMER.with(|t| t.start(slint::TimerMode::Repeated, RESYNC, move || sync(&app)));
            }
        }
    }
}

fn wire_once(app: &Rc<AppState>, main: &crate::AppWindow) {
    if WIRED.with(|w| w.replace(true)) {
        return;
    }
    let app = app.clone();
    main.on_dialog_layer_pressed(move || go_back(&app));
}

/// "Go back to it": the open page or dialog comes back in front with the keyboard.
pub(crate) fn go_back(app: &Rc<AppState>) {
    let slot = app.dialog_nav.dialogs.borrow().clone_strong();
    let Some(window) = slot.window() else {
        sync(app);
        return;
    };
    let how = chrome::bring_back(window);
    eprintln!("dialog layer: back to {:?} ({how})", slot_title(&slot).unwrap_or_default());
}

/// True while a dialog is open and the main window, with its layer, is the one in front (Desktop Mode).
pub(crate) fn main_in_front(app: &AppState) -> bool {
    if !dialog_is_open(app) {
        return false;
    }
    let Some(main) = app.window.upgrade() else { return false };
    let dialog = app.dialog_nav.dialogs.borrow().window().and_then(chrome::window_in_front);
    decide_main_in_front(crate::fork::steam_queue::game_mode_from_env(), chrome::window_in_front(main.window()), dialog)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layer_names_the_open_page() {
        assert_eq!(layer_for(Some("Settings"), false), Layer::Shown("Settings is open".into()));
        assert_eq!(layer_for(Some("Fix problems"), true), Layer::Shown("Fix problems is open".into()), "a page in the slot wins over a park");
        assert_eq!(layer_for(Some("  "), false), Layer::Shown("A window of this app is open".into()));
    }

    #[test]
    fn the_layer_goes_when_nothing_is_open_and_waits_through_a_park() {
        assert_eq!(layer_for(None, false), Layer::Hidden);
        assert_eq!(layer_for(None, true), Layer::Keep, "a hand-off between two pages does not flicker the layer");
    }

    #[test]
    fn the_controller_goes_to_the_layer_only_when_the_main_window_is_in_front_on_the_desktop() {
        assert!(decide_main_in_front(false, Some(true), Some(false)));
        assert!(decide_main_in_front(false, Some(true), None));
        assert!(!decide_main_in_front(false, Some(false), Some(true)), "the page is in front: it gets the controller");
        assert!(!decide_main_in_front(false, Some(false), Some(false)), "another app is in front");
        assert!(!decide_main_in_front(false, None, None), "nobody can say (no window manager): as before, the page");
        assert!(!decide_main_in_front(false, Some(true), Some(true)), "both claim focus: the page");
        assert!(!decide_main_in_front(true, Some(true), Some(false)), "Game Mode: the page, always");
    }
}
