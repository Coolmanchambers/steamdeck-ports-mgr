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
    /// Drawn as the recommended choice (a small "Recommended" tag and an accent edge; focus is the ring alone).
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

/// A mark drawn before a line of the summary box: what state that thing is in. Drawn as a small picture (a check, a
/// clock, a dash, an exclamation mark), never a text character, so it cannot fall back to an empty box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mark {
    #[default]
    None,
    /// Set and finished.
    Done,
    /// Still to do: the user can do it now.
    ToDo,
    /// Waiting on something else (Steam to close, a first start).
    Waiting,
    /// Skipped on purpose.
    Skipped,
    /// Something went wrong or needs a look.
    Problem,
}

impl Mark {
    /// The number the window draws by (0 none, 1 done, 2 to do, 3 waiting, 4 skipped, 5 problem).
    pub fn code(self) -> i32 {
        match self {
            Mark::None => 0,
            Mark::Done => 1,
            Mark::ToDo => 2,
            Mark::Waiting => 3,
            Mark::Skipped => 4,
            Mark::Problem => 5,
        }
    }
}

/// A small picture drawn at the left of an option (a card for a group of things: Ports, Steam, Saves). Drawn as paths
/// by the window, so it never depends on a font.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Icon {
    #[default]
    None,
    /// A grid of four squares: every port there is.
    Browse,
    /// A handheld with a check: ports that run on the Deck.
    DeckReady,
    /// An arrow into a tray: bring in what is already here.
    Import,
    /// A chain link.
    Link,
    /// A folder.
    Folder,
    /// A screen with a play triangle: launching from Steam.
    Steam,
    /// A floppy disk: saves.
    Saves,
    /// A gear.
    Gear,
    /// An arrow circling: updates.
    Update,
    /// A check in a circle: the Fix problems checkup.
    Fix,
    /// A play triangle in a circle (the Port page's Play card).
    Play,
    /// A program window with a prompt (Program to start).
    Program,
    /// A disc (Game files).
    GameFiles,
    /// A page of text (a port's settings files).
    TextFile,
    /// Three dots (More).
    More,
    /// A circle with a minus (Uninstall).
    Remove,
}

impl Icon {
    /// The number the window draws by (0 none, then in declaration order).
    pub fn code(self) -> i32 {
        match self {
            Icon::None => 0,
            Icon::Browse => 1,
            Icon::DeckReady => 2,
            Icon::Import => 3,
            Icon::Link => 4,
            Icon::Folder => 5,
            Icon::Steam => 6,
            Icon::Saves => 7,
            Icon::Gear => 8,
            Icon::Update => 9,
            Icon::Fix => 10,
            Icon::Play => 11,
            Icon::Program => 12,
            Icon::GameFiles => 13,
            Icon::TextFile => 14,
            Icon::More => 15,
            Icon::Remove => 16,
        }
    }
}

/// A progress bar under the summary box: how far in thousandths, when the work knows, or a moving bar when it
/// does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Part(u16),
    Busy,
}

impl Progress {
    /// 0 to 1 for the window (`None` for the moving bar).
    pub fn fraction(self) -> Option<f32> {
        match self {
            Progress::Part(p) => Some(f32::from(p.min(1000)) / 1000.0),
            Progress::Busy => None,
        }
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
    /// The mark before each line of `facts` (missing ones are `Mark::None`).
    pub marks: Vec<Mark>,
    /// A progress bar under the summary box.
    pub progress: Option<Progress>,
    pub card: Option<Card>,
    /// A text box under the text. It is the first thing the cursor can be on; Down or A leaves it for the choices.
    pub input: Option<InputBox>,
    pub tiles: Vec<Tile<A>>,
    pub options: Vec<PageOption<A>>,
    /// The picture before each option (missing ones are `Icon::None`).
    pub icons: Vec<Icon>,
    /// An On or Off switch at the right of each option that is one (missing ones are `None`: not a switch).
    pub toggles: Vec<Option<bool>>,
    pub footnote: String,
    /// Left, middle, extra, right.
    pub buttons: [Option<PageButton<A>>; SLOTS],
    /// Which button is drawn as the primary one.
    pub primary: Option<usize>,
    /// Where the cursor starts.
    pub initial: Focus,
    /// What B does, in a word or two, for the controller hint bar ("Back" when empty). See [`Page::b_does`].
    pub back_hint: String,
    /// B does nothing here (a short check that cannot be stopped): the hint bar leaves B out.
    pub no_back: bool,
    /// One window: a "Show the app" button in the footer (left of the primary button), and Y does the same. Set by the
    /// page window itself (`page_dialog`) on full-screen pages in Desktop Mode, never by a page's owner.
    pub show_app: bool,
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
            marks: Vec::new(),
            progress: None,
            card: None,
            input: None,
            tiles: Vec::new(),
            options: Vec::new(),
            icons: Vec::new(),
            toggles: Vec::new(),
            footnote: String::new(),
            buttons: [None, None, None, None],
            primary: None,
            initial: Focus::Button(RIGHT),
            back_hint: String::new(),
            no_back: false,
            show_app: false,
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
    /// The footer's "Show the app" button ([`Page::show_app`]).
    ShowApp,
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

impl<A> Page<A> {
    /// A bar showing how far the work is: `Some(fraction)` when it knows, `None` for a moving bar.
    pub fn with_progress(mut self, fraction: Option<f32>) -> Page<A> {
        self.progress = Some(fraction.map_or(Progress::Busy, |f| Progress::Part(if f.is_finite() { (f.clamp(0.0, 1.0) * 1000.0).round() as u16 } else { 0 })));
        self
    }

    /// Adds a line to the summary box with its mark.
    pub fn fact(&mut self, mark: Mark, text: impl Into<String>) {
        self.marks.resize(self.facts.len(), Mark::None);
        self.facts.push(text.into());
        self.marks.push(mark);
    }

    /// The mark of line `i` of the summary box.
    pub fn mark_of(&self, i: usize) -> Mark {
        self.marks.get(i).copied().unwrap_or_default()
    }

    /// Gives option `index` a picture.
    pub fn icon(mut self, index: usize, icon: Icon) -> Page<A> {
        if self.icons.len() <= index {
            self.icons.resize(index + 1, Icon::None);
        }
        self.icons[index] = icon;
        self
    }

    /// The picture before option `i`.
    pub fn icon_of(&self, i: usize) -> Icon {
        self.icons.get(i).copied().unwrap_or_default()
    }

    /// Makes option `index` an On or Off switch showing `on`.
    pub fn toggle(mut self, index: usize, on: bool) -> Page<A> {
        if self.toggles.len() <= index {
            self.toggles.resize(index + 1, None);
        }
        self.toggles[index] = Some(on);
        self
    }

    /// The switch of option `i`: `Some(on)`, or `None` when it is not a switch.
    pub fn toggle_of(&self, i: usize) -> Option<bool> {
        self.toggles.get(i).copied().flatten()
    }

    /// Says what B does on this page, for the hint bar ("Cancel", "Keep them").
    pub fn b_means(mut self, words: impl Into<String>) -> Page<A> {
        self.back_hint = words.into();
        self
    }
}

impl<A: PartialEq> Page<A> {
    /// Says what B does by the label of the button or option that does the same thing (B on a question whose
    /// safe answer is "Keep them" reads "B Keep them"). Left as it was when nothing on the page does that.
    pub fn b_does(mut self, action: &A) -> Page<A> {
        let label = self.buttons.iter().flatten().find(|b| b.action == *action).map(|b| b.label.clone()).or_else(|| self.options.iter().find(|o| o.action == *action).map(|o| o.title.clone()));
        if let Some(label) = label {
            self.back_hint = label.trim_end_matches("...").trim_end_matches('\u{2026}').trim().to_string();
        }
        self
    }
}

/// The controller hint bar along the bottom of the page: what A, B and the D-pad do here, and on a page with a text
/// box how to bring up Steam's keyboard. Only the D-pad, A and B reach the window, so these are all the buttons.
pub fn hints<A>(page: &Page<A>) -> String {
    let can_focus = focusables(page).len();
    let back = if page.back_hint.is_empty() { "Back" } else { page.back_hint.as_str() };
    let mut parts: Vec<String> = Vec::new();
    if can_focus > 0 {
        parts.push("A Choose".into());
    }
    if !page.no_back {
        parts.push(format!("B {back}"));
    }
    if can_focus > 1 {
        parts.push("D-pad Move".into());
    }
    if page.input.is_some() {
        parts.push("Steam + X Keyboard".into());
    }
    if page.show_app {
        parts.push("Y Show the app".into());
    }
    if parts.is_empty() {
        return "Please wait".into();
    }
    parts.join(HINT_GAP)
}

/// Between two hints in the bar.
pub const HINT_GAP: &str = "   \u{b7}   ";

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
    out.extend(footer_row(page));
    out
}

/// The footer, left to right: the buttons that exist, with "Show the app" just left of the primary (right) button.
fn footer_row<A>(page: &Page<A>) -> Vec<Focus> {
    let mut row: Vec<Focus> = (0..RIGHT).filter(|s| page.buttons[*s].is_some()).map(Focus::Button).collect();
    if page.show_app {
        row.push(Focus::ShowApp);
    }
    if page.buttons[RIGHT].is_some() {
        row.push(Focus::Button(RIGHT));
    }
    row
}

/// The focus number of the "Show the app" button, or -1 when the page has none.
pub fn show_app_number<A>(page: &Page<A>) -> i32 {
    focusables(page).iter().position(|f| *f == Focus::ShowApp).map_or(-1, |n| n as i32)
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
    let row = footer_row(page);
    let to_buttons = || page.primary.map(Focus::Button).filter(|f| row.contains(f)).or_else(|| row.iter().rev().find(|f| **f != Focus::ShowApp).copied()).or_else(|| row.last().copied());
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
        Focus::Button(_) | Focus::ShowApp if dy < 0 => enabled.last().map(|o| Focus::Option(*o)).or_else(|| tiles.first().map(|t| Focus::Tile(*t))).or(input),
        Focus::Button(_) | Focus::ShowApp if dx != 0 => {
            let at = row.iter().position(|f| *f == here).unwrap_or(0) as i32;
            let next = at + dx.signum();
            (next >= 0).then(|| row.get(next as usize)).flatten().copied()
        }
        _ => None,
    };
    target.and_then(index_of).unwrap_or(current)
}

/// What choosing the focused item does.
pub fn action_at<A: Clone>(page: &Page<A>, number: usize) -> Option<A> {
    match focus_at(page, number)? {
        Focus::Input | Focus::ShowApp => None,
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
    fn summary_lines_keep_their_marks_even_after_unmarked_ones() {
        let mut p = Page::<Act>::default();
        p.facts.push("plain".into());
        p.fact(Mark::Done, "Ports folder: SD card");
        p.fact(Mark::Problem, "Steam: could not be read");
        assert_eq!(p.facts.len(), 3);
        assert_eq!((p.mark_of(0), p.mark_of(1), p.mark_of(2), p.mark_of(9)), (Mark::None, Mark::Done, Mark::Problem, Mark::None));
        assert_eq!((Mark::None.code(), Mark::Done.code(), Mark::Problem.code()), (0, 1, 5));
    }

    #[test]
    fn options_can_carry_a_picture_and_an_on_off_switch() {
        let p = card_page().icon(1, Icon::Folder).toggle(2, true);
        assert_eq!((p.icon_of(0), p.icon_of(1), p.icon_of(2), p.icon_of(9)), (Icon::None, Icon::Folder, Icon::None, Icon::None));
        assert_eq!((p.toggle_of(0), p.toggle_of(1), p.toggle_of(2), p.toggle_of(9)), (None, None, Some(true), None));
        assert_eq!(p.clone().toggle(2, false).toggle_of(2), Some(false), "a switch can be turned over");
        assert_eq!((Icon::None.code(), Icon::Browse.code(), Icon::Update.code()), (0, 1, 9));
    }

    #[test]
    fn a_progress_bar_knows_its_part_or_moves() {
        assert_eq!(Page::<Act>::default().with_progress(Some(0.25)).progress, Some(Progress::Part(250)));
        assert_eq!(Progress::Part(250).fraction(), Some(0.25));
        assert_eq!(Page::<Act>::default().with_progress(Some(7.0)).progress, Some(Progress::Part(1000)));
        assert_eq!(Progress::Busy.fraction(), None);
        assert_eq!(Page::<Act>::default().with_progress(None).progress, Some(Progress::Busy));
        assert_eq!(Page::<Act>::default().progress, None);
    }

    #[test]
    fn the_hint_bar_names_a_b_and_the_d_pad_and_what_b_does_here() {
        let bar = |parts: &[&str]| parts.join(HINT_GAP);
        assert_eq!(hints(&card_page()), bar(&["A Choose", "B Back", "D-pad Move"]));
        let skip = card_page().b_does(&Act::Skip);
        assert_eq!(hints(&skip), bar(&["A Choose", "B Skip", "D-pad Move"]), "B takes the label of what it does");
        assert_eq!(card_page().b_means("Cancel").back_hint, "Cancel");
        let mut dots = card_page();
        dots.buttons[LEFT] = Some(PageButton { label: "Skip setup...".into(), action: Act::Details });
        assert_eq!(dots.b_does(&Act::Details).back_hint, "Skip setup", "no trailing dots in the bar");
        let mut none = card_page();
        none.buttons[LEFT] = None;
        assert_eq!(none.b_does(&Act::Details).back_hint, "", "nothing on the page does it: the bar says Back");
    }

    #[test]
    fn the_hint_bar_leaves_out_what_does_nothing_and_names_the_keyboard_on_a_text_page() {
        assert_eq!(hints(&Page::<Act>::default()), "B Back", "nothing to choose: only B");
        let mut one = Page::<Act>::default();
        one.buttons[RIGHT] = Some(PageButton { label: "Cancel".into(), action: Act::Skip });
        assert_eq!(hints(&one.b_means("Cancel")), ["A Choose", "B Cancel"].join(HINT_GAP), "one thing to choose: no D-pad");
        assert!(hints(&input_page()).ends_with("Steam + X Keyboard"));
        assert_eq!(hints(&Page::<Act> { no_back: true, ..Page::default() }), "Please wait", "a check that cannot be stopped");
    }

    #[test]
    fn a_page_in_the_wizard_says_which_step_it_is_and_keeps_its_own_counter() {
        let p = Page::<u8> { header: "Import ports".into(), step_text: "Port 2 of 5".into(), ..Page::default() }.in_wizard(2, 5);
        assert_eq!((p.header.as_str(), p.step_text.as_str(), p.step_number, p.step_count), ("Deck Ports Manager", "Step 2 of 5 - Port 2 of 5", 2, 5));
        let q = Page::<u8> { header: "Import ports".into(), ..Page::default() }.in_wizard(4, 5);
        assert_eq!(q.step_text, "Step 4 of 5");
    }

    fn footer_page(show_app: bool) -> Page<u8> {
        let mut p = Page::<u8> { show_app, ..Page::default() };
        p.options = vec![PageOption { title: "o".into(), detail: String::new(), choice: None, enabled: true, recommended: false, action: 1 }];
        p.buttons[LEFT] = Some(PageButton { label: "Back".into(), action: 2 });
        p.buttons[RIGHT] = Some(PageButton { label: "Next".into(), action: 3 });
        p.primary = Some(RIGHT);
        p
    }

    #[test]
    fn show_the_app_sits_left_of_the_primary_button_and_the_hint_names_y() {
        let p = footer_page(true);
        assert_eq!(focusables(&p), vec![Focus::Option(0), Focus::Button(LEFT), Focus::ShowApp, Focus::Button(RIGHT)]);
        assert_eq!(show_app_number(&p), 2);
        assert!(hints(&p).ends_with("Y Show the app"), "{}", hints(&p));
        assert_eq!(action_at(&p, 2), None, "the page window does it, not the page's owner");
        let q = footer_page(false);
        assert_eq!(show_app_number(&q), -1);
        assert!(!hints(&q).contains("Show the app"));
        assert_eq!(focusables(&q), vec![Focus::Option(0), Focus::Button(LEFT), Focus::Button(RIGHT)], "pages without it are as before");
    }

    #[test]
    fn the_d_pad_reaches_show_the_app_along_the_footer_and_down_still_lands_on_the_primary() {
        let p = footer_page(true);
        let next = focus_number(&p, Focus::Button(RIGHT));
        assert_eq!(move_focus(&p, 0, 0, 1), next, "down from the last option: the primary button, not Show the app");
        let show = move_focus(&p, next, -1, 0);
        assert_eq!(focus_at(&p, show), Some(Focus::ShowApp));
        assert_eq!(focus_at(&p, move_focus(&p, show, -1, 0)), Some(Focus::Button(LEFT)));
        assert_eq!(focus_at(&p, move_focus(&p, show, 0, -1)), Some(Focus::Option(0)));
        // No primary: down goes to the last real button, never to Show the app.
        let mut r = footer_page(true);
        r.primary = None;
        r.buttons[RIGHT] = None;
        assert_eq!(focus_at(&r, move_focus(&r, 0, 0, 1)), Some(Focus::Button(LEFT)));
    }
}
