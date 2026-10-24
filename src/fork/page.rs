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
    /// A QR code shown where the cover goes (the cloud sign-in page); wins over `art`.
    pub qr: Option<crate::fork::cloud::qr::QrMatrix>,
    pub title: String,
    pub lines: Vec<String>,
}

/// A cover with its name under it. A tile with an action can be chosen (the ports a folder could be); one
/// without is only shown (the ports found, the ports to move).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile<A> {
    pub art: Option<ArtRef>,
    pub label: String,
    /// A small line under the label ("already found", "installed"); empty for none.
    pub note: String,
    pub action: Option<A>,
}

impl<A> Tile<A> {
    pub fn shown(art: Option<ArtRef>, label: impl Into<String>) -> Tile<A> {
        Tile { art, label: label.into(), note: String::new(), action: None }
    }

    pub fn choice(art: Option<ArtRef>, label: impl Into<String>, action: A) -> Tile<A> {
        Tile { art, label: label.into(), note: String::new(), action: Some(action) }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Tile<A> {
        self.note = note.into();
        self
    }
}

/// A text box in the page (a link to paste, a name to filter by): what it shows when empty, and what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputBox {
    pub placeholder: String,
    pub text: String,
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
    /// A text box under the text. It is the first thing the cursor can be on; Down or A leaves it for the choices.
    pub input: Option<InputBox>,
    pub tiles: Vec<Tile<A>>,
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
            input: None,
            tiles: Vec::new(),
            options: Vec::new(),
            footnote: String::new(),
            buttons: [None, None, None, None],
            primary: None,
            initial: Focus::Button(RIGHT),
        }
    }
}

/// Something that can take focus: tile `n`, option `n`, or the button in slot `n`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The page's text box.
    Input,
    Tile(usize),
    Option(usize),
    Button(usize),
}

impl<A> Page<A> {
    /// The page as a part of the setup wizard (Import ports and the old copy question open from it): the wizard's
    /// name in the header, "Step 2 of 5" at the top right (the page's own counter, "Port 2 of 5", follows it) and
    /// the wizard's progress bar.
    pub fn in_wizard(mut self, step: usize, count: usize) -> Page<A> {
        self.header = "Deck Ports Manager".to_string();
        let own = std::mem::take(&mut self.step_text);
        self.step_text = if own.is_empty() { format!("Step {step} of {count}") } else { format!("Step {step} of {count} - {own}") };
        self.step_number = step;
        self.step_count = count;
        self
    }
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

fn tile_indexes<A>(page: &Page<A>) -> Vec<usize> {
    (0..page.tiles.len()).filter(|i| page.tiles[*i].action.is_some()).collect()
}

fn enabled_options<A>(page: &Page<A>) -> Vec<usize> {
    (0..page.options.len()).filter(|i| page.options[*i].enabled).collect()
}

/// Everything that can take focus, in order: the text box, the tiles that can be chosen, the enabled options,
/// then the buttons that exist.
pub fn focusables<A>(page: &Page<A>) -> Vec<Focus> {
    let mut out: Vec<Focus> = Vec::new();
    if page.input.is_some() {
        out.push(Focus::Input);
    }
    out.extend(tile_indexes(page).into_iter().map(Focus::Tile));
    out.extend(enabled_options(page).into_iter().map(Focus::Option));
    out.extend((0..SLOTS).filter(|s| page.buttons[*s].is_some()).map(Focus::Button));
    out
}

/// Position of `focus` in [`focusables`] (the number the window draws as selected).
pub fn focus_number<A>(page: &Page<A>, focus: Focus) -> usize {
    focusables(page).iter().position(|f| *f == focus).unwrap_or(0)
}

/// The focus number of the text box, or -1 when the page has none.
pub fn input_number<A>(page: &Page<A>) -> i32 {
    focusables(page).iter().position(|f| *f == Focus::Input).map_or(-1, |n| n as i32)
}

/// The focus number of tile `index`, or -1 when it cannot be chosen.
pub fn tile_number<A>(page: &Page<A>, index: usize) -> i32 {
    focusables(page).iter().position(|f| *f == Focus::Tile(index)).map_or(-1, |n| n as i32)
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

/// The D-pad. The page is a column: a row of tiles you can choose, the options, the buttons. Up and down move
/// between those groups and along the options (down from the last option drops to the primary button, down
/// from a tile goes to the first option); left and right move along the tiles and along the buttons. It never
/// lands on text or a disabled option, and stays put at an edge.
pub fn move_focus<A>(page: &Page<A>, current: usize, dx: i32, dy: i32) -> usize {
    let items = focusables(page);
    let Some(here) = items.get(current).copied() else { return initial_focus(page) };
    let index_of = |f: Focus| items.iter().position(|x| *x == f);
    let tiles = tile_indexes(page);
    let enabled = enabled_options(page);
    let present: Vec<usize> = (0..SLOTS).filter(|s| page.buttons[*s].is_some()).collect();
    let to_buttons = || page.primary.filter(|s| present.contains(s)).or_else(|| present.last().copied()).map(Focus::Button);
    let input = page.input.is_some().then_some(Focus::Input);
    let first_below_input = || tiles.first().map(|t| Focus::Tile(*t)).or_else(|| enabled.first().map(|o| Focus::Option(*o))).or_else(to_buttons);
    let target = match here {
        Focus::Input if dy > 0 => first_below_input(),
        Focus::Tile(i) if dx != 0 => {
            let at = tiles.iter().position(|t| *t == i).unwrap_or(0) as i32 + dx.signum();
            (at >= 0).then(|| tiles.get(at as usize)).flatten().map(|t| Focus::Tile(*t))
        }
        Focus::Tile(_) if dy < 0 => input,
        Focus::Tile(_) if dy > 0 => enabled.first().map(|o| Focus::Option(*o)).or_else(to_buttons),
        Focus::Option(i) if dy > 0 => match enabled.iter().find(|o| **o > i) {
            Some(next) => Some(Focus::Option(*next)),
            None => to_buttons(),
        },
        Focus::Option(i) if dy < 0 => match enabled.iter().rev().find(|o| **o < i) {
            Some(prev) => Some(Focus::Option(*prev)),
            None => tiles.first().map(|t| Focus::Tile(*t)).or(input),
        },
        Focus::Button(_) if dy < 0 => enabled.last().map(|o| Focus::Option(*o)).or_else(|| tiles.first().map(|t| Focus::Tile(*t))).or(input),
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
        Focus::Input => None,
        Focus::Tile(i) => page.tiles.get(i).and_then(|t| t.action.clone()),
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
        p.card = Some(Card { art: art("dusk"), qr: None, title: "Dusklight".into(), lines: vec![] });
        p.tiles = vec![
            Tile::shown(art("soh"), "SoH"),
            Tile::shown(art("dusk"), "Dusklight"),
            Tile::shown(None, "No art"),
        ];
        let folders: Vec<String> = p.art_refs().into_iter().map(|a| a.folder).collect();
        assert_eq!(folders, ["dusk", "soh"]);
    }

    fn tile_page() -> Page<Act> {
        let mut p = card_page();
        p.tiles = vec![Tile::choice(None, "A", Act::Yes), Tile::shown(None, "shown only"), Tile::choice(None, "B", Act::Other), Tile::choice(None, "C", Act::Skip)];
        p.initial = Focus::Tile(0);
        p
    }

    #[test]
    fn only_tiles_with_an_action_can_be_focused_and_they_come_first() {
        let p = tile_page();
        assert_eq!(focusables(&p)[..3], [Focus::Tile(0), Focus::Tile(2), Focus::Tile(3)]);
        assert_eq!(tile_number(&p, 0), 0);
        assert_eq!(tile_number(&p, 1), -1, "a shown-only tile");
        assert_eq!(tile_number(&p, 2), 1);
        assert_eq!(option_number(&p, 0), 3, "the options follow the tiles");
        assert_eq!(action_at(&p, 1), Some(Act::Other));
    }

    #[test]
    fn left_and_right_walk_the_tiles_down_goes_to_the_options_and_up_comes_back() {
        let p = tile_page();
        let mut at = initial_focus(&p);
        assert_eq!(action_at(&p, at), Some(Act::Yes));
        at = move_focus(&p, at, 1, 0);
        assert_eq!(action_at(&p, at), Some(Act::Other), "right skips the tile that cannot be chosen");
        at = move_focus(&p, at, 1, 0);
        assert_eq!(action_at(&p, at), Some(Act::Skip));
        assert_eq!(move_focus(&p, at, 1, 0), at, "stays put at the end of the row");
        assert_eq!(move_focus(&p, initial_focus(&p), -1, 0), initial_focus(&p), "and at the start");
        let down = move_focus(&p, at, 0, 1);
        assert_eq!(focus_at(&p, down), Some(Focus::Option(0)));
        assert_eq!(focus_at(&p, move_focus(&p, down, 0, -1)), Some(Focus::Tile(0)), "up from the first option reaches the tiles");
        assert_eq!(move_focus(&p, initial_focus(&p), 0, -1), initial_focus(&p), "nothing above the tiles");
    }

    #[test]
    fn without_options_down_from_a_tile_reaches_the_buttons() {
        let mut p = tile_page();
        p.options.clear();
        let down = move_focus(&p, initial_focus(&p), 0, 1);
        assert_eq!(focus_at(&p, down), Some(Focus::Button(LEFT)));
        assert_eq!(focus_at(&p, move_focus(&p, down, 0, -1)), Some(Focus::Tile(0)));
    }

    fn input_page() -> Page<Act> {
        let mut p = card_page();
        p.input = Some(InputBox { placeholder: "github.com/owner/project".into(), text: String::new() });
        p.initial = Focus::Input;
        p
    }

    #[test]
    fn a_page_with_a_text_box_starts_on_it_and_the_box_has_no_action() {
        let p = input_page();
        assert_eq!(focusables(&p)[0], Focus::Input);
        assert_eq!(input_number(&p), 0);
        assert_eq!(input_number(&card_page()), -1);
        assert_eq!(option_number(&p, 0), 1, "the options follow the box");
        assert_eq!(focus_at(&p, initial_focus(&p)), Some(Focus::Input));
        assert_eq!(action_at(&p, 0), None, "A on the box is not a choice; the screen moves on to the choices");
    }

    #[test]
    fn down_leaves_the_box_for_the_choices_and_up_comes_back_to_it() {
        let p = input_page();
        let down = move_focus(&p, 0, 0, 1);
        assert_eq!(focus_at(&p, down), Some(Focus::Option(0)));
        assert_eq!(move_focus(&p, down, 0, -1), 0, "up from the first choice is the box again");
        assert_eq!(move_focus(&p, 0, 0, -1), 0, "nothing above the box");
        assert_eq!(move_focus(&p, 0, 1, 0), 0, "left and right stay in the text");
        let mut bare = input_page();
        bare.options.clear();
        let down = move_focus(&bare, 0, 0, 1);
        assert_eq!(focus_at(&bare, down), Some(Focus::Button(LEFT)));
        assert_eq!(move_focus(&bare, down, 0, -1), 0, "up from the footer reaches the box when there is nothing between");
    }
    #[test]
    fn a_page_in_the_wizard_says_which_step_it_is_and_keeps_its_own_counter() {
        let p = Page::<u8> { header: "Import ports".into(), step_text: "Port 2 of 5".into(), ..Page::default() }.in_wizard(2, 5);
        assert_eq!((p.header.as_str(), p.step_text.as_str(), p.step_number, p.step_count), ("Deck Ports Manager", "Step 2 of 5 - Port 2 of 5", 2, 5));
        let q = Page::<u8> { header: "Import ports".into(), ..Page::default() }.in_wizard(4, 5);
        assert_eq!(q.step_text, "Step 4 of 5");
    }

}
