//! One page of the guided screens (the setup wizard and the Import ports cards), as plain data. A [`Page`]
//! has a header line, a title, wrapped paragraphs, a summary box, an optional card (one port with its cover),
//! a row of cover tiles, a few big options and up to four footer buttons. Only enabled options and buttons
//! take the controller's focus (D-pad, A, B reach the window and nothing else); [`move_focus`] decides where
//! the D-pad goes. `A` is whatever the screen wants to hear when an item is chosen: the wizard's actions, the
//! import cards' actions.
//!
//! The window that draws it is `ui/fork/setup-wizard.slint`; `src/app/page_dialog.rs` fills it from a page.

use crate::fork::art::thumbs::ArtRef;

/// Footer slots, left to right.
pub const LEFT: usize = 0;
pub const MIDDLE: usize = 1;
pub const EXTRA: usize = 2;
pub const RIGHT: usize = 3;
pub const SLOTS: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageOption<A> {
    pub title: String,
    pub detail: String,
    /// `Some(chosen)` draws a radio mark (a choice); `None` is an action that opens something.
    pub choice: Option<bool>,
    /// False: shown dimmed, cannot be chosen or focused (the folder in use, a drive that cannot hold ports).
    pub enabled: bool,
    /// Drawn as the recommended choice (filled with the accent colour).
    pub recommended: bool,
    pub action: A,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageButton<A> {
    pub label: String,
    pub action: A,
}

/// One port shown big: its cover (when there is one), its name and a few short lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub art: Option<ArtRef>,
    pub title: String,
    pub lines: Vec<String>,
}

/// A cover with its name under it. Tiles are shown, not chosen, unless a screen gives them an action later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    pub art: Option<ArtRef>,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<A> {
    /// The window's title (also its name to the window system, which the smoke test looks for).
    pub window_title: String,
    /// Bigger text, covers and choices (the Import ports pages are read from further away than the wizard).
    pub large: bool,
    /// Top left of the page ("Deck Ports Manager", "Import ports").
    pub header: String,
    /// Top right ("Step 2 of 6", "Port 2 of 5").
    pub step_text: String,
    /// Segments of the progress bar (0: no bar) and how many are filled, counting from 1.
    pub step_number: usize,
    pub step_count: usize,
    pub title: String,
    pub paragraphs: Vec<String>,
    /// Lines of the summary box (what is set, what was found).
    pub facts: Vec<String>,
    pub card: Option<Card>,
    pub tiles: Vec<Tile>,
    pub options: Vec<PageOption<A>>,
    pub footnote: String,
    /// Left, middle, extra, right.
    pub buttons: [Option<PageButton<A>>; SLOTS],
    /// Which button is drawn as the primary one.
    pub primary: Option<usize>,
    /// Where the cursor starts.
    pub initial: Focus,
}

impl<A> Default for Page<A> {
    fn default() -> Page<A> {
        Page {
            window_title: String::new(),
            large: false,
            header: String::new(),
            step_text: String::new(),
            step_number: 0,
            step_count: 0,
            title: String::new(),
            paragraphs: Vec::new(),
            facts: Vec::new(),
            card: None,
            tiles: Vec::new(),
            options: Vec::new(),
            footnote: String::new(),
            buttons: [None, None, None, None],
            primary: None,
            initial: Focus::Button(RIGHT),
        }
    }
}

/// Something that can take focus: option `n`, or the button in slot `n`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Option(usize),
    Button(usize),
}

impl<A: Clone> Page<A> {
    /// Every cover the page shows, once each (the card first), for the image loader.
    pub fn art_refs(&self) -> Vec<ArtRef> {
        let mut out: Vec<ArtRef> = Vec::new();
        for art in self.card.iter().filter_map(|c| c.art.as_ref()).chain(self.tiles.iter().filter_map(|t| t.art.as_ref())) {
            if !out.contains(art) {
                out.push(art.clone());
            }
        }
        out
    }
}

/// Everything that can take focus, in order: the enabled options, then the buttons that exist.
pub fn focusables<A>(page: &Page<A>) -> Vec<Focus> {
    let mut out: Vec<Focus> = (0..page.options.len()).filter(|i| page.options[*i].enabled).map(Focus::Option).collect();
    out.extend((0..SLOTS).filter(|s| page.buttons[*s].is_some()).map(Focus::Button));
    out
}

/// Position of `focus` in [`focusables`] (the number the window draws as selected).
pub fn focus_number<A>(page: &Page<A>, focus: Focus) -> usize {
    focusables(page).iter().position(|f| *f == focus).unwrap_or(0)
}

/// The focus number of option `index`, or -1 when it cannot take focus.
pub fn option_number<A>(page: &Page<A>, index: usize) -> i32 {
    focusables(page).iter().position(|f| *f == Focus::Option(index)).map_or(-1, |n| n as i32)
}

/// The number of the button in `slot`, or -1 when it is not shown.
pub fn button_number<A>(page: &Page<A>, slot: usize) -> i32 {
    focusables(page).iter().position(|f| *f == Focus::Button(slot)).map_or(-1, |n| n as i32)
}

/// What a focus number stands for.
pub fn focus_at<A>(page: &Page<A>, number: usize) -> Option<Focus> {
    focusables(page).get(number).copied()
}

pub fn initial_focus<A>(page: &Page<A>) -> usize {
    let items = focusables(page);
    let Some(first) = items.first().copied() else { return 0 };
    let wanted = if items.contains(&page.initial) { page.initial } else { page.primary.map_or(first, Focus::Button) };
    focus_number(page, wanted)
}

/// The D-pad: up and down walk the options and then drop to the buttons (the primary one first), left
/// and right move between buttons. It never lands on text or a disabled option, and stays put at an edge.
pub fn move_focus<A>(page: &Page<A>, current: usize, dx: i32, dy: i32) -> usize {
    let items = focusables(page);
    let Some(here) = items.get(current).copied() else { return initial_focus(page) };
    let index_of = |f: Focus| items.iter().position(|x| *x == f);
    let enabled: Vec<usize> = (0..page.options.len()).filter(|i| page.options[*i].enabled).collect();
    let present: Vec<usize> = (0..SLOTS).filter(|s| page.buttons[*s].is_some()).collect();
    let target = match here {
        Focus::Option(i) if dy > 0 => match enabled.iter().find(|o| **o > i) {
            Some(next) => Some(Focus::Option(*next)),
            None => page.primary.filter(|s| present.contains(s)).or_else(|| present.last().copied()).map(Focus::Button),
        },
        Focus::Option(i) if dy < 0 => enabled.iter().rev().find(|o| **o < i).map(|o| Focus::Option(*o)),
        Focus::Button(_) if dy < 0 => enabled.last().map(|o| Focus::Option(*o)),
        Focus::Button(slot) if dx != 0 => {
            let at = present.iter().position(|s| *s == slot).unwrap_or(0) as i32;
            let next = at + dx.signum();
            (next >= 0).then(|| present.get(next as usize)).flatten().map(|s| Focus::Button(*s))
        }
        _ => None,
    };
    target.and_then(index_of).unwrap_or(current)
}

/// What choosing the focused item does.
pub fn action_at<A: Clone>(page: &Page<A>, number: usize) -> Option<A> {
    match focus_at(page, number)? {
        Focus::Option(i) => page.options.get(i).map(|o| o.action.clone()),
        Focus::Button(s) => page.buttons[s].as_ref().map(|b| b.action.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Act {
        Yes,
        Other,
        Skip,
        Details,
    }

    fn option(title: &str, action: Act, enabled: bool) -> PageOption<Act> {
        PageOption { title: title.into(), detail: String::new(), choice: None, enabled, recommended: false, action }
    }

    fn card_page() -> Page<Act> {
        let mut p = Page::<Act>::default();
        p.options = vec![option("Yes", Act::Yes, true), option("Other", Act::Other, true), option("Skip", Act::Skip, true)];
        p.buttons[LEFT] = Some(PageButton { label: "Details".into(), action: Act::Details });
        p.initial = Focus::Option(0);
        p
    }

    #[test]
    fn the_cursor_starts_where_the_page_says() {
        let mut p = card_page();
        assert_eq!(action_at(&p, initial_focus(&p)), Some(Act::Yes));
        p.initial = Focus::Option(1);
        assert_eq!(action_at(&p, initial_focus(&p)), Some(Act::Other));
    }

    #[test]
    fn a_page_with_nothing_to_focus_does_not_panic() {
        let p = Page::<Act>::default();
        assert_eq!(initial_focus(&p), 0);
        assert_eq!(move_focus(&p, 0, 0, 1), 0);
        assert_eq!(action_at(&p, 0), None);
    }

    #[test]
    fn the_d_pad_walks_the_options_then_the_buttons_and_never_off_the_page() {
        let p = card_page();
        let mut at = initial_focus(&p);
        at = move_focus(&p, at, 0, 1);
        assert_eq!(action_at(&p, at), Some(Act::Other));
        at = move_focus(&p, at, 0, 1);
        at = move_focus(&p, at, 0, 1);
        assert_eq!(action_at(&p, at), Some(Act::Details), "down from the last option drops to the footer");
        assert_eq!(move_focus(&p, at, 0, 1), at, "stays put at the bottom");
        at = move_focus(&p, at, 0, -1);
        assert_eq!(action_at(&p, at), Some(Act::Skip));
        assert_eq!(move_focus(&p, initial_focus(&p), 0, -1), initial_focus(&p), "stays put at the top");
    }

    #[test]
    fn a_disabled_option_is_skipped() {
        let mut p = card_page();
        p.options[1].enabled = false;
        let at = move_focus(&p, initial_focus(&p), 0, 1);
        assert_eq!(action_at(&p, at), Some(Act::Skip));
        assert_eq!(option_number(&p, 1), -1);
    }

    #[test]
    fn the_covers_of_a_page_are_listed_once_each_card_first() {
        let art = |f: &str| Some(ArtRef { folder: f.into(), url: None });
        let mut p = card_page();
        p.card = Some(Card { art: art("dusk"), title: "Dusklight".into(), lines: vec![] });
        p.tiles = vec![
            Tile { art: art("soh"), label: "SoH".into() },
            Tile { art: art("dusk"), label: "Dusklight".into() },
            Tile { art: None, label: "No art".into() },
        ];
        let folders: Vec<String> = p.art_refs().into_iter().map(|a| a.folder).collect();
        assert_eq!(folders, ["dusk", "soh"]);
    }
}
