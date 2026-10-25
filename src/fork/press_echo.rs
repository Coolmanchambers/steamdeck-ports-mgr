//! One physical press acts once. On a Steam Deck, Steam Input can type a key (Return for A, Escape for B) on top of
//! the controller press our gamepad reader (gilrs, which reads the pad whatever window has focus) sees. Both reach
//! the same action, so one press of A would choose twice: the first choice opens the next page and the echo chooses
//! on it. This filter drops a press that arrives from the *other* source (pad or keyboard) shortly after the same
//! kind of press was acted on, once per press. Presses from the same source always count, so a quick double press on
//! the pad, or typing on a keyboard, is never lost.
//!
//! Where it is asked: the pad side in `ui::gamepad_router::dispatch` (A and B); the keyboard side where Return and
//! Escape choose or go back: the guided page window (`app::page_dialog`), the upstream dialogs' keyboard wiring
//! (`app::dialogs`) and the main list (Return; Escape below). A pad press reaches some dialogs through the same Slint
//! callback a key does, so the keyboard check stands aside while a pad press is being dispatched ([`while_pad`]).
//!
//! **Escape on the main list** (windowed) closes the app, so an echo that arrives *before* the pad's B would close it
//! although the user pressed B. That Escape is held for [`ECHO_WINDOW`] before it acts ([`hold_main_escape`]); a pad B
//! in that time drops it and counts as the one B press (coordinator ruling, 2026-10-25).

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How close the echo follows the press. Steam's key arrives within a frame or two of the pad edge our 20 ms
/// poll sees; a person pressing A on the pad and then Return on a keyboard takes far longer than this.
pub const ECHO_WINDOW: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Pad,
    Key,
}

/// The two presses that Steam can echo: A / Return (choose) and B / Escape (back).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    Choose,
    Back,
}

#[derive(Debug, Default)]
pub struct EchoFilter {
    /// Presses acted on in the last [`ECHO_WINDOW`] whose echo has not come yet, oldest first.
    waiting: VecDeque<(Source, Press, Instant)>,
}

impl EchoFilter {
    /// True when the press should act; false when it is the echo of a press just acted on.
    pub fn allow(&mut self, source: Source, press: Press, now: Instant) -> bool {
        // Only presses within the window can still be echoed (and none "from the future", if the clock jumps).
        self.waiting.retain(|(_, _, at)| now.checked_duration_since(*at).is_some_and(|d| d <= ECHO_WINDOW));
        // Each press has at most one echo: an echo uses up the press it belongs to.
        if let Some(i) = self.waiting.iter().position(|(s, p, _)| *s != source && *p == press) {
            self.waiting.remove(i);
            return false;
        }
        self.waiting.push_back((source, press, now));
        if self.waiting.len() > 8 {
            self.waiting.pop_front();
        }
        true
    }

    /// Takes back the newest press of this kind from this source (a held Escape that a pad B replaced), so it is
    /// not waiting for an echo any more.
    pub fn forget(&mut self, source: Source, press: Press) {
        if let Some(i) = self.waiting.iter().rposition(|(s, p, _)| *s == source && *p == press) {
            self.waiting.remove(i);
        }
    }
}

/// An Escape on the main list waiting to act: its token, so a late timer of an earlier hold does nothing.
#[derive(Debug, Default)]
pub struct Held {
    token: u64,
    pending: Option<u64>,
}

impl Held {
    /// A new hold starts (it replaces one still waiting); the token its timer brings back.
    pub fn begin(&mut self) -> u64 {
        self.token += 1;
        self.pending = Some(self.token);
        self.token
    }

    /// A pad B came: true when an Escape was waiting (it is dropped).
    pub fn cancel(&mut self) -> bool {
        self.pending.take().is_some()
    }

    /// The hold's time is up: true when this hold is still the one waiting (the Escape acts now).
    pub fn expire(&mut self, token: u64) -> bool {
        if self.pending == Some(token) {
            self.pending = None;
            true
        } else {
            false
        }
    }
}

thread_local! {
    static FILTER: RefCell<EchoFilter> = RefCell::new(EchoFilter::default());
    static HELD: RefCell<Held> = RefCell::new(Held::default());
    /// A pad press is being dispatched: a callback it reaches is the pad's, not a key's.
    static PAD_DISPATCH: Cell<bool> = const { Cell::new(false) };
}

fn allow(source: Source, press: Press) -> bool {
    let allowed = FILTER.with(|f| f.borrow_mut().allow(source, press, Instant::now()));
    if !allowed {
        eprintln!("input: dropped a {press:?} from the {source:?} that echoed the one just acted on");
    }
    allowed
}

/// The pad side: A or B, before it is dispatched. A pad B also drops an Escape held on the main list.
pub fn pad(press: Press) -> bool {
    if press == Press::Back && HELD.with(|h| h.borrow_mut().cancel()) {
        FILTER.with(|f| f.borrow_mut().forget(Source::Key, Press::Back));
        eprintln!("input: dropped the Escape held on the main list; the controller's B counts instead");
    }
    allow(Source::Pad, press)
}

/// Runs `f` (the dispatch of a pad press that passed [`pad`]) so that the callbacks it reaches are not taken for keys.
pub fn while_pad<R>(f: impl FnOnce() -> R) -> R {
    let before = PAD_DISPATCH.with(|p| p.replace(true));
    let out = f();
    PAD_DISPATCH.with(|p| p.set(before));
    out
}

/// The keyboard side: Return (choose) or Escape (back) that is about to act. Always true for the pad's own press.
pub fn key(press: Press) -> bool {
    PAD_DISPATCH.with(Cell::get) || allow(Source::Key, press)
}

/// Escape on the main list in the windowed view (it closes the app): it acts after [`ECHO_WINDOW`] unless a pad B
/// comes first, in which case only the B counts. An Escape that is itself the echo of a B just pressed is dropped.
pub fn hold_main_escape(then: impl FnOnce() + 'static) {
    if !key(Press::Back) {
        return;
    }
    let token = HELD.with(|h| h.borrow_mut().begin());
    slint::Timer::single_shot(ECHO_WINDOW, move || {
        if HELD.with(|h| h.borrow_mut().expire(token)) {
            then();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_key_echo_right_after_a_pad_press_is_dropped_once() {
        let t = Instant::now();
        let mut f = EchoFilter::default();
        assert!(f.allow(Source::Pad, Press::Choose, t));
        assert!(!f.allow(Source::Key, Press::Choose, t + ms(30)), "Steam's Return for the same A press");
        assert!(f.allow(Source::Key, Press::Choose, t + ms(60)), "only one echo per press is swallowed");
    }

    #[test]
    fn the_echo_can_come_first_too() {
        let t = Instant::now();
        let mut f = EchoFilter::default();
        assert!(f.allow(Source::Key, Press::Back, t));
        assert!(!f.allow(Source::Pad, Press::Back, t + ms(20)), "the pad poll saw the same B a moment later");
    }

    #[test]
    fn presses_from_the_same_source_always_count() {
        let t = Instant::now();
        let mut f = EchoFilter::default();
        assert!(f.allow(Source::Pad, Press::Choose, t));
        assert!(f.allow(Source::Pad, Press::Choose, t + ms(40)), "a quick double press on the pad is two presses");
        let mut k = EchoFilter::default();
        assert!(k.allow(Source::Key, Press::Choose, t));
        assert!(k.allow(Source::Key, Press::Choose, t + ms(10)), "typing Return twice on a keyboard");
    }

    #[test]
    fn two_quick_pad_presses_each_lose_their_own_echo_and_nothing_more() {
        let t = Instant::now();
        let mut f = EchoFilter::default();
        assert!(f.allow(Source::Pad, Press::Choose, t));
        assert!(f.allow(Source::Pad, Press::Choose, t + ms(40)));
        assert!(!f.allow(Source::Key, Press::Choose, t + ms(50)));
        assert!(!f.allow(Source::Key, Press::Choose, t + ms(90)));
        assert!(f.allow(Source::Key, Press::Choose, t + ms(120)), "a third Return is a real press");
    }

    #[test]
    fn a_different_press_is_not_an_echo() {
        let t = Instant::now();
        let mut f = EchoFilter::default();
        assert!(f.allow(Source::Pad, Press::Choose, t));
        assert!(f.allow(Source::Key, Press::Back, t + ms(20)), "Escape after A is a real Back");
    }

    #[test]
    fn after_the_window_the_other_source_counts_again() {
        let t = Instant::now();
        let mut f = EchoFilter::default();
        assert!(f.allow(Source::Pad, Press::Choose, t));
        assert!(f.allow(Source::Key, Press::Choose, t + ECHO_WINDOW + ms(1)));
    }

    #[test]
    fn a_clock_that_seems_to_go_back_never_drops_a_press() {
        let t = Instant::now() + ms(500);
        let mut f = EchoFilter::default();
        assert!(f.allow(Source::Pad, Press::Choose, t));
        assert!(f.allow(Source::Key, Press::Choose, t - ms(100)));
    }

    #[test]
    fn a_forgotten_key_press_no_longer_swallows_the_pad_press() {
        let t = Instant::now();
        let mut f = EchoFilter::default();
        assert!(f.allow(Source::Key, Press::Back, t));
        f.forget(Source::Key, Press::Back);
        assert!(f.allow(Source::Pad, Press::Back, t + ms(50)), "the held Escape was dropped; the B is the press");
    }

    #[test]
    fn a_held_escape_acts_when_no_b_comes_and_not_when_one_does() {
        let mut h = Held::default();
        let first = h.begin();
        assert!(h.expire(first), "no B: the Escape acts");
        let second = h.begin();
        assert!(h.cancel(), "a B came while it waited");
        assert!(!h.expire(second), "so it never acts");
        assert!(!h.cancel(), "nothing is waiting any more");
    }

    #[test]
    fn a_late_timer_of_an_older_hold_does_nothing() {
        let mut h = Held::default();
        let old = h.begin();
        let new = h.begin();
        assert!(!h.expire(old));
        assert!(h.expire(new));
    }

    #[test]
    fn a_callback_reached_by_a_pad_press_is_not_taken_for_a_key() {
        assert!(pad(Press::Back), "the pad's B");
        assert!(while_pad(|| key(Press::Back)), "the dialog's close callback it reaches counts");
        assert!(!key(Press::Back), "while Steam's Escape for the same press is dropped");
    }
}
