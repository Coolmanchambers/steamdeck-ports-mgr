//! The main window's dim layer while a page or dialog is open (docs/design/port-page-and-one-window.md, part 1).
//!
//! Pages and dialogs are top-level windows of their own. When the main window is clicked, tapped or Alt+Tabbed to,
//! it comes in front of them (in full screen it even covers them), and before this layer it looked usable while it
//! ignored the controller. Now, while anything is open, the main window shows a dim layer with one line ("Settings is
//! open") and a "Go back to it" button. A, B, Return, Space, Escape, a click or a tap anywhere on the layer brings the
//! page back in front with the keyboard. Looking at the main window alone does not raise the page: one press does.
//!
//! The layer follows the dialog slot: [`sync`] runs when a dialog opens (`finish_dialog_open`), when it closes
//! (`close_current_dialog`), when the page window's short park ends, on every page shown, and every
//! [`RESYNC`] while the layer is up, so no way of closing can leave it behind. While the page window is parked
//! (a hand-off between two screens) the layer stays as it was, so it does not flicker.
//!
//! In Desktop Mode guided pages are no longer marked as owned by the main window ([`keep_owner`]), so each has its own
//! taskbar entry and goes full screen over a full-screen main window; smaller dialogs keep upstream's owner so they stay
//! above the main window. Game Mode (gamescope, one window at a time) keeps the owner for everything.

use super::dialogs::{dialog_is_open, DialogSlot};
use super::state::AppState;
use crate::fork::press_echo::{self, Press};
use crate::ui::chrome;
use slint::ComponentHandle;
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// How often the layer checks itself against the dialog slot while it is up.
const RESYNC: Duration = Duration::from_millis(250);
/// One answer of [`main_in_front`] serves every input of one controller poll (they arrive together, 20 ms apart).
const IN_FRONT_CACHE: Duration = Duration::from_millis(10);

thread_local! {
    static TIMER: slint::Timer = slint::Timer::default();
    static IN_FRONT: Cell<Option<(Instant, bool)>> = const { Cell::new(None) };
}

/// What the layer should show.
#[derive(Debug, PartialEq)]
enum Layer {
    Hidden,
    Shown(String),
    /// The page window is parked between two screens: leave the layer as it is.
    Keep,
}

/// What is open, as the layer names it.
#[derive(Debug, PartialEq)]
enum Open {
    /// A page or dialog with this window title.
    Titled(String),
    /// A yes/no question (Confirm): its title is often the answer's verb ("Apply"), so it is not used.
    Question,
    /// An error message.
    Message,
}

/// The layer's line. Only the part of the title before its first ':' ("Settings: Ports" is Settings).
fn layer_text(open: &Open) -> String {
    match open {
        Open::Question => "A question is open".into(),
        Open::Message => "A message is open".into(),
        Open::Titled(title) => {
            // A Port page names its port, never its card ("Port: Dusklight is open").
            if let Some(port) = crate::fork::port_page::port_in_window_title(title) {
                return format!("{}: {port} is open", crate::fork::port_page::TITLE);
            }
            let name = title.split(':').next().unwrap_or("").trim();
            if name.is_empty() {
                "A window of this app is open".into()
            } else {
                format!("{name} is open")
            }
        }
    }
}

/// `open`: what the slot holds, when anything; `parked`: the page window waits for its next page.
fn layer_for(open: Option<&Open>, parked: bool) -> Layer {
    match open {
        Some(open) => Layer::Shown(layer_text(open)),
        None if parked => Layer::Keep,
        None => Layer::Hidden,
    }
}

/// The main window is in front of an open dialog, so the controller is meant for the layer, and A or B only brings the
/// dialog back. On the desktop the window manager answers (`_NET_ACTIVE_WINDOW`), falling back to window focus.
/// Under gamescope (one window at a time) both windows must answer, so a page that is on screen never loses its A.
fn decide_main_in_front(game_mode: bool, main: Option<bool>, dialog: Option<bool>) -> bool {
    if game_mode {
        main == Some(true) && dialog == Some(false)
    } else {
        main == Some(true) && dialog != Some(true)
    }
}

/// Whether this dialog is marked as owned by the main window (upstream's `own_window`). In Desktop Mode guided pages drop
/// it, since an owned (transient) window has no taskbar entry in KDE; every other dialog keeps it so it stays above the
/// main window. Game Mode keeps it for all (gamescope uses it to show the dialog), and so does Windows.
pub(crate) fn keep_owner(slot: &DialogSlot) -> bool {
    owner_rule(cfg!(target_os = "linux") && !crate::fork::steam_queue::game_mode_from_env(), matches!(slot, DialogSlot::SetupWizard(_)))
}

fn owner_rule(linux_desktop: bool, page: bool) -> bool {
    !(linux_desktop && page)
}

fn slot_open(slot: &DialogSlot) -> Option<Open> {
    let title = match slot {
        DialogSlot::None => return None,
        // Upstream's one TwoButtonDialog (2026-10-30) is both the confirm ("Question") and the launch error
        // dialog; the latter has the Info shortcut (Y opens Info).
        DialogSlot::TwoButton(d) if d.get_info_shortcut() => return Some(Open::Message),
        DialogSlot::TwoButton(_) => return Some(Open::Question),
        DialogSlot::Message(d) => d.get_dialog_title(),
        DialogSlot::Info(d) => d.get_dialog_title(),
        DialogSlot::Progress(d) => d.get_dialog_title(),
        DialogSlot::Picker(d) => d.get_dialog_title(),
        DialogSlot::SearchList(d) => d.get_dialog_title(),
        DialogSlot::DeckReadyInfo(d) => d.get_dialog_title(),
        DialogSlot::FolderPicker(d) => d.get_dialog_title(),
        DialogSlot::SetupWizard(d) => d.get_dialog_title(),
    };
    Some(Open::Titled(title.to_string()))
}

/// Puts the layer in step with what is open. Cheap; safe to call from anywhere on the UI thread.
pub(crate) fn sync(app: &Rc<AppState>) {
    let Some(main) = app.window.upgrade() else { return };
    // Called from inside other dialog code: a slot borrowed right now is read again by the timer.
    let Ok(slot) = app.dialog_nav.dialogs.try_borrow() else { return };
    let open = slot_open(&slot);
    drop(slot);
    match layer_for(open.as_ref(), super::page_dialog::parked()) {
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

/// How the layer was pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayerPress {
    /// A click or a tap.
    Click,
    /// Return or Space.
    Chosen,
    /// Escape.
    Backed,
}

/// The one-press check a press on the layer goes through: a key may be Steam's echo of a pad A or B (and when the key
/// comes first, the pad's own press must then be dropped, or it would press the page just brought back). A click is
/// never an echo: it is not filtered, so it can neither be dropped nor leave a press waiting for an echo.
fn echo_check(how: LayerPress) -> Option<Press> {
    match how {
        LayerPress::Click => None,
        LayerPress::Chosen => Some(Press::Choose),
        LayerPress::Backed => Some(Press::Back),
    }
}

/// What the layer says under its button: the pad's buttons and the pointer; outside Game Mode the keys too.
fn layer_hint(game_mode: bool) -> &'static str {
    if game_mode {
        "A or B, click or tap"
    } else {
        "A or B, Enter or Esc, click or tap"
    }
}

/// The main window's "Go back to it" (wired with the window's other callbacks in `main.rs`).
pub(crate) fn wire(main: &crate::AppWindow, app: &Rc<AppState>) {
    main.set_dialog_layer_hint(layer_hint(crate::fork::steam_queue::game_mode_from_env()).into());
    let pressed = |how: LayerPress| {
        let app = app.clone();
        move || {
            if echo_check(how).is_none_or(press_echo::key) {
                go_back(&app);
            }
        }
    };
    main.on_dialog_layer_pressed(pressed(LayerPress::Click));
    main.on_dialog_layer_chosen(pressed(LayerPress::Chosen));
    main.on_dialog_layer_backed(pressed(LayerPress::Backed));
}

/// "Go back to it": the open page or dialog comes back in front with the keyboard.
pub(crate) fn go_back(app: &Rc<AppState>) {
    let slot = app.dialog_nav.dialogs.borrow().clone_strong();
    let Some(window) = slot.window() else {
        sync(app);
        return;
    };
    let how = chrome::bring_back(window);
    IN_FRONT.with(|c| c.set(None));
    let title = match slot_open(&slot) {
        Some(Open::Titled(t)) => t,
        Some(other) => layer_text(&other),
        None => String::new(),
    };
    eprintln!("dialog layer: back to {title:?} ({how})");
}

/// True while a dialog is open and the main window, with its layer, is the one in front.
pub(crate) fn main_in_front(app: &AppState) -> bool {
    if !dialog_is_open(app) {
        return false;
    }
    let now = Instant::now();
    if let Some((_, answer)) = IN_FRONT.with(Cell::get).filter(|(at, _)| now.duration_since(*at) < IN_FRONT_CACHE) {
        return answer;
    }
    let Some(main) = app.window.upgrade() else { return false };
    let dialog = app.dialog_nav.dialogs.borrow().window().and_then(chrome::window_in_front);
    let answer = decide_main_in_front(crate::fork::steam_queue::game_mode_from_env(), chrome::window_in_front(main.window()), dialog);
    IN_FRONT.with(|c| c.set(Some((now, answer))));
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titled(t: &str) -> Open {
        Open::Titled(t.into())
    }

    #[test]
    fn the_layer_names_the_open_page() {
        assert_eq!(layer_for(Some(&titled("Settings")), false), Layer::Shown("Settings is open".into()));
        assert_eq!(layer_for(Some(&titled("Fix problems")), true), Layer::Shown("Fix problems is open".into()), "a page in the slot wins over a park");
        assert_eq!(layer_for(Some(&titled("  ")), false), Layer::Shown("A window of this app is open".into()));
    }

    #[test]
    fn the_layer_names_the_page_not_its_step() {
        assert_eq!(layer_text(&titled("Settings: Ports")), "Settings is open");
        assert_eq!(layer_text(&titled("Import ports: nothing found")), "Import ports is open");
        assert_eq!(layer_text(&titled(": odd")), "A window of this app is open");
        // A Port page names its port (whose own name may hold ':'), never its card or the card's step.
        assert_eq!(layer_text(&titled("Port: Dusklight")), "Port: Dusklight is open");
        assert_eq!(layer_text(&titled("Port: 3D Pinball for Windows: Space Cadet: Saves: Backup made")), "Port: 3D Pinball for Windows: Space Cadet is open");
        assert_eq!(layer_text(&titled("Port: Dusklight: Uninstall")), "Port: Dusklight is open");
    }

    #[test]
    fn questions_and_errors_are_named_by_kind_not_by_their_verb() {
        assert_eq!(layer_text(&Open::Question), "A question is open");
        assert_eq!(layer_text(&Open::Message), "A message is open");
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
    }

    #[test]
    fn in_game_mode_both_windows_must_answer_before_the_layer_takes_the_controller() {
        assert!(decide_main_in_front(true, Some(true), Some(false)), "gamescope shows the main window: A and B only bring the page back");
        assert!(!decide_main_in_front(true, Some(true), None), "the page cannot say: it keeps its A");
        assert!(!decide_main_in_front(true, None, Some(false)));
        assert!(!decide_main_in_front(true, Some(false), Some(true)));
    }

    #[test]
    fn only_guided_pages_on_the_linux_desktop_drop_the_owner() {
        assert!(!owner_rule(true, true), "a page in Desktop Mode: its own taskbar entry");
        assert!(owner_rule(true, false), "a Confirm, Progress, Picker... stays above the main window");
        assert!(owner_rule(false, true), "Game Mode and Windows keep the owner");
        assert!(owner_rule(false, false));
    }

    #[test]
    fn keys_on_the_layer_go_through_the_one_press_check_and_a_click_never_does() {
        assert_eq!(echo_check(LayerPress::Chosen), Some(Press::Choose));
        assert_eq!(echo_check(LayerPress::Backed), Some(Press::Back));
        assert_eq!(echo_check(LayerPress::Click), None, "a click is never an echo: never dropped, never leaves a press waiting");
        // Steam's Return comes first and brings the page back; the pad's own A a moment later is its echo and is dropped,
        // so it cannot press the page's focused button.
        let t = Instant::now();
        let mut f = press_echo::EchoFilter::default();
        assert!(f.allow(press_echo::Source::Key, Press::Choose, t));
        assert!(!f.allow(press_echo::Source::Pad, Press::Choose, t + Duration::from_millis(20)));
    }

    #[test]
    fn the_layer_names_the_keys_outside_game_mode() {
        assert_eq!(layer_hint(true), "A or B, click or tap");
        assert!(layer_hint(false).contains("Enter or Esc"));
    }

    #[test]
    fn the_layer_splits_keys_from_clicks_and_ignores_held_keys() {
        let slint = include_str!("../../ui/fork/dialog-layer.slint");
        let touch = &slint[slint.find("TouchArea {").expect("the layer's touch area")..];
        assert!(touch[..touch.find('}').unwrap()].contains("root.clicked()"), "a click is its own callback");
        let keys = &slint[slint.find("key-pressed(event)").expect("the layer's keys")..];
        let repeat = keys.find("event.repeat").expect("held keys are ignored");
        assert!(repeat < keys.find("Key.Return").unwrap() && repeat < keys.find("Key.Escape").unwrap());
        assert!(keys.contains("root.chosen()") && keys.contains("root.backed()"));
    }
}
