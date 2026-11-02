//! "Add to Steam with artwork?" for a few named ports: one short page instead of the full Add to Steam list. Fix
//! problems' Steam fix (exactly the ports its problem names) and the Port page's Add to Steam (its one port) ask here;
//! the install wizard's Steam step shares the same decision and the same notes (its own page keeps its words).
//!
//! The page: the ports (one card, or a cover tile each), one plain paragraph, a line about Steam when it runs, "Add it"
//! or "Add them" (recommended) and "Not now". Nothing is asked when there is nothing to do: no Steam account, several
//! and none chosen, a problem reading Steam, already waiting for Steam to close, already in Steam. The caller shows the
//! note instead ([`ask`]).
//!
//! Everything here is plain data and words; `src/app/steam_add_screen.rs` runs the page and the work (the artwork
//! fetch, `setup_wizard::steam::add_all`, the Close Steam flow).

use crate::fork::art::thumbs::ArtRef;
use crate::fork::page::{Card, Focus, Mark, Page, PageOption, Tile};

/// What a choice on the page means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddAct {
    Add,
    NotNow,
}

/// The window title of the page and its working page.
pub const WINDOW_TITLE: &str = "Add to Steam";
/// Top left of the page when it shows several ports (one port: its name).
pub const HEADER: &str = "Deck Ports Manager";

/// One port the page shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub name: String,
    pub art: Option<ArtRef>,
}

/// Where Steam stands for these ports, read by the screen (`setup_wizard::steam::info` on a screen of just them).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    /// Steam accounts on this device.
    pub accounts: usize,
    /// One of them is the one to add to (the only one, or the one chosen in Settings > Add to Steam).
    pub chosen: bool,
    /// Why nothing can be done (a shortcuts file that cannot be read), in plain words.
    pub problem: Option<String>,
    /// Ports that would be added.
    pub to_add: usize,
    /// Of those, the ones already waiting for Steam to close.
    pub queued: usize,
    /// A shortcut with one of their names starts another program.
    pub other_by_name: bool,
    /// Steam is closed outside Game Mode: the shortcuts are written now.
    pub immediate: bool,
    pub game_mode: bool,
}

/// Whether to ask, or the note to show instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// Nothing to ask; the note says why (`None`: nothing worth saying, such as no ports that can go into Steam).
    Skip(Option<(Mark, String)>),
    /// Ask. `running`: Steam runs in Desktop Mode (it closes for a moment); `game_mode`: it is added in Desktop Mode.
    Ask { running: bool, game_mode: bool },
}

fn one(n: usize) -> bool {
    n <= 1
}

/// Why the page is skipped for want of an account: `accounts` found, and whether one is chosen. `n`: how many ports.
pub fn account_note(accounts: usize, chosen: bool, n: usize) -> Option<(Mark, String)> {
    match (accounts, chosen) {
        (0, _) if one(n) => Some((Mark::ToDo, "Steam: start Steam once, then its Port page can add it".to_string())),
        (0, _) => Some((Mark::ToDo, "Steam: start Steam once, then Fix problems can add them".to_string())),
        (_, false) => Some((Mark::ToDo, "Steam: choose your account in Settings > Add to Steam first".to_string())),
        _ => None,
    }
}

/// The line once the shortcuts are written: it says "with its artwork" only when the artwork is in the cache.
/// `names_art`: the catalog names a cover or icon; `art_missing`: one of them could not be fetched.
pub fn added_note(names_art: bool, art_missing: bool, n: usize) -> &'static str {
    match (names_art, art_missing, one(n)) {
        (false, _, _) => "Added to Steam",
        (true, false, true) => "Added to Steam with its artwork",
        (true, false, false) => "Added to Steam with their artwork",
        (true, true, true) => "Added to Steam; its artwork could not be fetched",
        (true, true, false) => "Added to Steam; some artwork could not be fetched",
    }
}

/// Waiting for Steam to close (queued in Desktop Mode, or the Close Steam flow left it waiting).
pub fn waiting_note(n: usize) -> &'static str {
    if one(n) { "Steam: it is added when Steam next closes" } else { "Steam: they are added when Steam next closes" }
}

/// Queued in Game Mode: added once the user is in Desktop Mode.
pub fn desktop_mode_note(n: usize) -> &'static str {
    if one(n) { "Steam: you are asked to add it in Desktop Mode" } else { "Steam: you are asked to add them in Desktop Mode" }
}

/// Nothing was added, and why.
pub fn problem_note(problem: &str) -> String {
    format!("Not added to Steam: {problem}")
}

/// The question that comes before closing Steam ("<name> is ready to go into Steam.").
pub fn close_lead(names: &[String]) -> String {
    match names {
        [name] => format!("{name} is ready to go into Steam."),
        _ => format!("{} ports are ready to go into Steam.", names.len()),
    }
}

/// Ask, or the note that says why not. The order is the install wizard's: an account to add to, Steam's files
/// readable, not already waiting for Steam, something to add.
pub fn ask(f: &Facts, n: usize) -> Ask {
    if let Some(note) = account_note(f.accounts, f.chosen, n) {
        return Ask::Skip(Some(note));
    }
    if let Some(problem) = &f.problem {
        return Ask::Skip(Some((Mark::Problem, problem_note(problem))));
    }
    if f.queued > 0 && f.queued >= f.to_add {
        return Ask::Skip(Some((Mark::Waiting, waiting_note(n).to_string())));
    }
    if f.to_add == 0 {
        return Ask::Skip(Some(if f.other_by_name {
            let whose = if one(n) { "its name" } else { "one of their names" };
            (Mark::ToDo, format!("Steam: a shortcut with {whose} starts another program; its Port page can fix it"))
        } else {
            (Mark::Done, "Already in Steam".to_string())
        }));
    }
    Ask::Ask { running: !f.immediate && !f.game_mode, game_mode: f.game_mode }
}

fn option(title: &str, detail: &str, action: AddAct) -> PageOption<AddAct> {
    PageOption { title: title.to_string(), detail: detail.to_string(), choice: None, enabled: true, recommended: false, action }
}

/// The ports on the page: one port is a card with its name in full; several are a cover tile each.
fn base(ports: &[Shown], title: &str) -> Page<AddAct> {
    let mut p = Page { window_title: WINDOW_TITLE.to_string(), large: true, title: title.to_string(), initial: Focus::Option(0), ..Page::default() };
    match ports {
        [port] => {
            p.header = port.name.clone();
            p.card = Some(Card { art: port.art.clone(), qr: None, title: port.name.clone(), lines: Vec::new() });
        }
        _ => {
            p.header = HEADER.to_string();
            p.tiles = ports.iter().map(|s| Tile::shown(s.art.clone(), s.name.clone())).collect();
        }
    }
    p
}

/// "Add to Steam with artwork?": `game_mode`: Steam cannot be changed now (it is added in Desktop Mode);
/// `steam_running`: Steam runs in Desktop Mode and closes for a moment.
pub fn page(ports: &[Shown], game_mode: bool, steam_running: bool) -> Page<AddAct> {
    let single = one(ports.len());
    let mut p = base(ports, "Add to Steam with artwork?");
    p.paragraphs.push(match ports {
        [port] => format!("{} is not in Steam yet.", port.name),
        _ => "These ports are not in Steam yet.".to_string(),
    });
    let them = if single { "it" } else { "them" };
    if game_mode {
        p.paragraphs.push(format!("You are asked to add {them} in Desktop Mode."));
    } else if steam_running {
        p.paragraphs.push(format!("Steam closes for a moment to add {them}; you can start it again after."));
    }
    let (add, detail, later) = if single {
        ("Add it", "Adds its shortcut and artwork; Steam's list is backed up first", "Its Port page can add it later")
    } else {
        ("Add them", "Adds their shortcuts and artwork; Steam's list is backed up first", "Fix problems can add them later")
    };
    let mut yes = option(add, detail, AddAct::Add);
    yes.recommended = true;
    p.options = vec![yes, option("Not now", later, AddAct::NotNow)];
    p.b_does(&AddAct::NotNow)
}

/// The page while the artwork is fetched: B does nothing.
pub fn working_page(ports: &[Shown]) -> Page<AddAct> {
    let mut p = base(ports, "Getting the artwork");
    p.paragraphs.push(if one(ports.len()) { "Fetching its cover and icon for Steam." } else { "Fetching their covers and icons for Steam." }.to_string());
    p.no_back = true;
    p.with_progress(None)
}

/// The title of the Port page's note after the flow: the worst of what happened.
pub fn result_title(notes: &[(Mark, String)]) -> &'static str {
    let has = |m: Mark| notes.iter().any(|(mark, _)| *mark == m);
    if has(Mark::Problem) {
        "Not added to Steam"
    } else if has(Mark::Waiting) {
        "Waiting for Steam"
    } else if has(Mark::ToDo) {
        "Not added yet"
    } else {
        "Done"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::install_wizard::TITLE_FIT;
    use crate::fork::page::{action_at, initial_focus};

    fn shown(name: &str) -> Shown {
        Shown { name: name.to_string(), art: Some(ArtRef { folder: name.to_lowercase(), url: None }) }
    }

    /// Names as long as real ones get (the catalog's longest kind) and short ones.
    const NAMES: [&str; 4] = ["X", "The Legend of Dragoon", "The Wind Waker HD", "The Legend of Zelda: The Wind Waker HD (ZeldaWWHDRecomp)"];

    fn sets() -> Vec<Vec<Shown>> {
        let mut out: Vec<Vec<Shown>> = NAMES.iter().map(|n| vec![shown(n)]).collect();
        out.push(vec![shown(NAMES[1]), shown(NAMES[2])]);
        out.push(vec![shown(NAMES[3]), shown(NAMES[1]), shown(NAMES[2])]);
        out
    }

    fn all_pages() -> Vec<Page<AddAct>> {
        let mut pages = Vec::new();
        for ports in sets() {
            for (game_mode, running) in [(false, false), (false, true), (true, true)] {
                pages.push(page(&ports, game_mode, running));
            }
            pages.push(working_page(&ports));
        }
        pages
    }

    /// The full-screen page's title is one line, elided past about 40 characters; an option's title (40) and detail
    /// (66) are one line each on the 800 px page.
    #[test]
    fn every_title_and_option_fits_with_long_names_and_one_or_three_ports() {
        for p in all_pages() {
            assert!(p.title.chars().count() <= TITLE_FIT, "title too long: {}", p.title);
            for o in &p.options {
                assert!(o.title.chars().count() <= 40, "{:?}: option title too long: {}", p.title, o.title);
                assert!(o.detail.chars().count() <= 66, "{:?}: detail too long ({}): {}", p.title, o.detail.chars().count(), o.detail);
                assert!(!o.title.is_empty());
            }
            for t in p.paragraphs.iter().chain(p.options.iter().map(|o| &o.detail)) {
                assert!(!t.contains('\u{2014}'), "no em dashes: {t}");
            }
        }
    }

    #[test]
    fn one_port_is_a_card_with_its_name_and_add_it() {
        let p = page(&[shown("The Legend of Dragoon")], false, false);
        assert_eq!(p.title, "Add to Steam with artwork?");
        assert_eq!(p.card.as_ref().unwrap().title, "The Legend of Dragoon");
        assert!(p.tiles.is_empty());
        assert_eq!(p.header, "The Legend of Dragoon");
        assert_eq!(p.paragraphs, ["The Legend of Dragoon is not in Steam yet."]);
        let opts: Vec<(&str, &str)> = p.options.iter().map(|o| (o.title.as_str(), o.detail.as_str())).collect();
        assert_eq!(opts, [("Add it", "Adds its shortcut and artwork; Steam's list is backed up first"), ("Not now", "Its Port page can add it later")]);
        assert!(p.options[0].recommended && !p.options[1].recommended);
        assert_eq!(action_at(&p, initial_focus(&p)), Some(AddAct::Add));
        assert_eq!(p.back_hint, "Not now");
        assert_eq!(p.window_title, WINDOW_TITLE);
    }

    #[test]
    fn several_ports_are_a_tile_each_and_add_them() {
        let p = page(&[shown("The Legend of Dragoon"), shown("The Wind Waker HD")], false, false);
        assert!(p.card.is_none());
        assert_eq!(p.tiles.iter().map(|t| t.label.as_str()).collect::<Vec<_>>(), ["The Legend of Dragoon", "The Wind Waker HD"]);
        assert!(p.tiles.iter().all(|t| t.action.is_none()), "the covers are shown, not chosen");
        assert_eq!(p.header, HEADER);
        assert_eq!(p.paragraphs, ["These ports are not in Steam yet."]);
        let opts: Vec<(&str, &str)> = p.options.iter().map(|o| (o.title.as_str(), o.detail.as_str())).collect();
        assert_eq!(opts, [("Add them", "Adds their shortcuts and artwork; Steam's list is backed up first"), ("Not now", "Fix problems can add them later")]);
        assert!(p.options[0].recommended);
        assert_eq!(p.options.iter().map(|o| o.action).collect::<Vec<_>>(), [AddAct::Add, AddAct::NotNow]);
    }

    #[test]
    fn steam_running_in_desktop_mode_says_it_closes_for_a_moment() {
        let one = page(&[shown("Dusklight")], false, true);
        assert_eq!(one.paragraphs[1], "Steam closes for a moment to add it; you can start it again after.");
        let two = page(&[shown("A"), shown("B")], false, true);
        assert_eq!(two.paragraphs[1], "Steam closes for a moment to add them; you can start it again after.");
        assert_eq!(page(&[shown("A"), shown("B")], false, false).paragraphs.len(), 1, "Steam closed: nothing to say");
    }

    #[test]
    fn game_mode_says_it_is_added_in_desktop_mode() {
        assert_eq!(page(&[shown("A"), shown("B")], true, true).paragraphs[1], "You are asked to add them in Desktop Mode.");
        assert_eq!(page(&[shown("A")], true, false).paragraphs[1], "You are asked to add it in Desktop Mode.");
    }

    #[test]
    fn the_working_page_waits() {
        let p = working_page(&[shown("A"), shown("B")]);
        assert_eq!(p.title, "Getting the artwork");
        assert!(p.no_back && p.options.is_empty());
        assert_eq!(p.paragraphs, ["Fetching their covers and icons for Steam."]);
        assert_eq!(working_page(&[shown("A")]).paragraphs, ["Fetching its cover and icon for Steam."]);
    }

    fn ready() -> Facts {
        Facts { accounts: 1, chosen: true, problem: None, to_add: 2, queued: 0, other_by_name: false, immediate: true, game_mode: false }
    }

    #[test]
    fn it_asks_when_there_is_something_to_add() {
        assert_eq!(ask(&ready(), 2), Ask::Ask { running: false, game_mode: false });
        assert_eq!(ask(&Facts { immediate: false, ..ready() }, 2), Ask::Ask { running: true, game_mode: false });
        assert_eq!(ask(&Facts { immediate: false, game_mode: true, ..ready() }, 2), Ask::Ask { running: false, game_mode: true });
        assert_eq!(ask(&Facts { queued: 1, ..ready() }, 2), Ask::Ask { running: false, game_mode: false }, "one of two waits: the other is still asked about");
    }

    #[test]
    fn each_case_with_nothing_to_ask_gives_a_note_instead() {
        let no_account = Facts { accounts: 0, chosen: false, ..ready() };
        assert_eq!(ask(&no_account, 1), Ask::Skip(Some((Mark::ToDo, "Steam: start Steam once, then its Port page can add it".into()))));
        assert_eq!(ask(&no_account, 2), Ask::Skip(Some((Mark::ToDo, "Steam: start Steam once, then Fix problems can add them".into()))));
        let several = Facts { accounts: 2, chosen: false, ..ready() };
        assert_eq!(ask(&several, 2), Ask::Skip(Some((Mark::ToDo, "Steam: choose your account in Settings > Add to Steam first".into()))));
        let problem = Facts { problem: Some("shortcuts.vdf cannot be read".into()), ..ready() };
        assert_eq!(ask(&problem, 2), Ask::Skip(Some((Mark::Problem, "Not added to Steam: shortcuts.vdf cannot be read".into()))));
        let queued = Facts { to_add: 1, queued: 1, ..ready() };
        assert_eq!(ask(&queued, 1), Ask::Skip(Some((Mark::Waiting, "Steam: it is added when Steam next closes".into()))));
        assert_eq!(ask(&Facts { to_add: 2, queued: 2, ..ready() }, 2), Ask::Skip(Some((Mark::Waiting, "Steam: they are added when Steam next closes".into()))));
        let already = Facts { to_add: 0, ..ready() };
        assert_eq!(ask(&already, 2), Ask::Skip(Some((Mark::Done, "Already in Steam".into()))));
        let other = Facts { to_add: 0, other_by_name: true, ..ready() };
        assert_eq!(ask(&other, 1), Ask::Skip(Some((Mark::ToDo, "Steam: a shortcut with its name starts another program; its Port page can fix it".into()))));
        assert_eq!(ask(&other, 2), Ask::Skip(Some((Mark::ToDo, "Steam: a shortcut with one of their names starts another program; its Port page can fix it".into()))));
    }

    #[test]
    fn steam_is_skipped_with_a_note_when_there_is_no_account_to_add_to() {
        assert_eq!(account_note(0, false, 1), Some((Mark::ToDo, "Steam: start Steam once, then its Port page can add it".to_string())));
        assert_eq!(account_note(2, false, 1), Some((Mark::ToDo, "Steam: choose your account in Settings > Add to Steam first".to_string())));
        assert_eq!(account_note(1, true, 1), None);
        assert_eq!(account_note(3, true, 2), None);
    }

    #[test]
    fn added_to_steam_says_with_its_artwork_only_when_it_is_there() {
        assert_eq!(added_note(true, false, 1), "Added to Steam with its artwork");
        assert_eq!(added_note(true, true, 1), "Added to Steam; its artwork could not be fetched");
        assert_eq!(added_note(false, false, 1), "Added to Steam");
        assert_eq!(added_note(true, false, 2), "Added to Steam with their artwork");
        assert_eq!(added_note(true, true, 3), "Added to Steam; some artwork could not be fetched");
    }

    #[test]
    fn notes_and_lead_read_for_one_or_several() {
        assert_eq!(desktop_mode_note(1), "Steam: you are asked to add it in Desktop Mode");
        assert_eq!(desktop_mode_note(2), "Steam: you are asked to add them in Desktop Mode");
        assert_eq!(close_lead(&["Dusklight".to_string()]), "Dusklight is ready to go into Steam.");
        assert_eq!(close_lead(&["A".to_string(), "B".to_string()]), "2 ports are ready to go into Steam.");
    }

    #[test]
    fn the_result_title_is_the_worst_of_what_happened() {
        assert_eq!(result_title(&[(Mark::Done, "Added to Steam".into())]), "Done");
        assert_eq!(result_title(&[(Mark::Done, "Already in Steam".into())]), "Done");
        assert_eq!(result_title(&[(Mark::Waiting, waiting_note(1).into())]), "Waiting for Steam");
        assert_eq!(result_title(&[(Mark::ToDo, "Steam: choose your account in Settings > Add to Steam first".into())]), "Not added yet");
        assert_eq!(result_title(&[(Mark::Done, "x".into()), (Mark::Problem, problem_note("y"))]), "Not added to Steam");
    }
}
