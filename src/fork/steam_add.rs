//! "Add to Steam with artwork?" for a few named ports: one short page instead of the full Add to Steam list. Fix
//! problems' Steam fix (exactly the ports its problem names) and the Port page's Add to Steam (its one port) ask here;
//! the install wizard's Steam step shares the same decision and the same notes (its own page keeps its words). The Port
//! page's "Update the shortcut" and "Use that shortcut for this port" ask on a page of the same kind ([`change_page`]).
//!
//! The page: the ports (one card, or a cover tile each), one plain paragraph, a line about Steam when it runs, "Add it"
//! or "Add them" (recommended) and "Not now". Nothing is asked when there is nothing to do: no Steam account, several
//! and none chosen, a problem reading Steam, no way to tell whether Steam runs, already waiting for Steam to close,
//! already in Steam. The caller shows the note instead ([`ask`]).
//!
//! Everything here is plain data and words; `src/app/steam_add_screen.rs` runs the page and the work (the artwork
//! fetch, `setup_wizard::steam::add_selected`, the Close Steam flow).

use crate::fork::art::thumbs::ArtRef;
use crate::fork::page::{Card, Focus, Mark, Page, PageOption, Tile};

/// What a choice on the page means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddAct {
    Add,
    NotNow,
}

/// Who asks: the words that point elsewhere differ ("its Port page can add it" is wrong on the Port page itself).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caller {
    Wizard,
    Checkup,
    PortPage,
}

/// What is done to Steam: ports added, or one shortcut that is there changed (out of date, or found by its name).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job {
    Add,
    Change,
}

/// The window title of the page and its working page.
pub const WINDOW_TITLE: &str = "Add to Steam";
/// Top left of the page when it shows several ports (one port: its name).
pub const HEADER: &str = "Deck Ports Manager";
/// Why nothing is done when Steam's process check failed.
pub const CHECK_FAILED: &str = "could not tell whether Steam is running";

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
    /// Ports that would be added (or shortcuts changed).
    pub to_add: usize,
    /// Of those, the ones already waiting for Steam to close.
    pub queued: usize,
    /// A shortcut with one of their names starts another program.
    pub other_by_name: bool,
    /// Steam is closed outside Game Mode: the shortcuts are written now.
    pub immediate: bool,
    pub game_mode: bool,
    /// Every Steam change already waiting for Steam to close (these ports' own included).
    pub waiting: usize,
    /// Whether Steam runs could not be told (outside Game Mode): nothing is written or queued.
    pub check_failed: bool,
}

/// Whether to ask, or the note to show instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// Nothing to ask; the note says why (`None`: nothing worth saying, such as no ports that can go into Steam).
    Skip(Option<(Mark, String)>),
    /// Ask. `running`: Steam runs in Desktop Mode (it closes for a moment); `game_mode`: it is done in Desktop Mode;
    /// `others`: other Steam changes already waiting, made at the same time.
    Ask { running: bool, game_mode: bool, others: usize },
}

fn one(n: usize) -> bool {
    n <= 1
}

/// A port can be offered: installed (it may have been uninstalled meanwhile) and not Android only.
pub fn can_go_in(installed: bool, android_only: bool) -> bool {
    installed && !android_only
}

/// Why the page is skipped for want of an account: `accounts` found, and whether one is chosen. `n`: how many ports.
pub fn account_note(accounts: usize, chosen: bool, n: usize, caller: Caller) -> Option<(Mark, String)> {
    let start_first = match caller {
        Caller::PortPage => "Steam: start Steam once, then try again here",
        _ if one(n) => "Steam: start Steam once, then its Port page can add it",
        _ => "Steam: start Steam once, then Fix problems can add them",
    };
    match (accounts, chosen) {
        (0, _) => Some((Mark::ToDo, start_first.to_string())),
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

/// The line once a shortcut that was there is changed.
pub const CHANGED_NOTE: &str = "Steam shortcut updated";

/// Waiting for Steam to close (queued in Desktop Mode, or the Close Steam flow left it waiting).
pub fn waiting_note(n: usize, job: Job) -> &'static str {
    match (job, one(n)) {
        (Job::Change, _) => "Steam: its shortcut is updated when Steam next closes",
        (Job::Add, true) => "Steam: it is added when Steam next closes",
        (Job::Add, false) => "Steam: they are added when Steam next closes",
    }
}

/// Queued in Game Mode: done once the user is in Desktop Mode.
pub fn desktop_mode_note(n: usize, job: Job) -> &'static str {
    match (job, one(n)) {
        (Job::Change, _) => "Steam: you are asked to update its shortcut in Desktop Mode",
        (Job::Add, true) => "Steam: you are asked to add it in Desktop Mode",
        (Job::Add, false) => "Steam: you are asked to add them in Desktop Mode",
    }
}

/// Nothing to do.
pub fn already_note(job: Job) -> &'static str {
    match job {
        Job::Add => "Already in Steam",
        Job::Change => "Its Steam shortcut is up to date",
    }
}

/// Nothing was added (or changed), and why.
pub fn problem_note(problem: &str, job: Job) -> String {
    match job {
        Job::Add => format!("Not added to Steam: {problem}"),
        Job::Change => format!("Not updated in Steam: {problem}"),
    }
}

/// Ask, or the note that says why not. The order is the install wizard's: an account to add to, Steam's files
/// readable, Steam's process check working, not already waiting for Steam, something to add.
pub fn ask(f: &Facts, n: usize, caller: Caller, job: Job) -> Ask {
    if let Some(note) = account_note(f.accounts, f.chosen, n, caller) {
        return Ask::Skip(Some(note));
    }
    if let Some(problem) = &f.problem {
        return Ask::Skip(Some((Mark::Problem, problem_note(problem, job))));
    }
    if f.check_failed && !f.game_mode {
        return Ask::Skip(Some((Mark::Problem, problem_note(CHECK_FAILED, job))));
    }
    if f.queued > 0 && f.queued >= f.to_add {
        return Ask::Skip(Some((Mark::Waiting, waiting_note(n, job).to_string())));
    }
    if f.to_add == 0 {
        return Ask::Skip(Some(if f.other_by_name {
            let whose = if one(n) { "its name" } else { "one of their names" };
            let fix = if caller == Caller::PortPage { "this card can fix it" } else { "its Port page can fix it" };
            (Mark::ToDo, format!("Steam: a shortcut with {whose} starts another program; {fix}"))
        } else {
            (Mark::Done, already_note(job).to_string())
        }));
    }
    Ask::Ask { running: !f.immediate && !f.game_mode, game_mode: f.game_mode, others: f.waiting.saturating_sub(f.queued) }
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

/// The lines about Steam: Game Mode, Steam running, other changes made at the same time. `what`: "add it", "add them",
/// "update it".
fn steam_lines(p: &mut Page<AddAct>, what: &str, game_mode: bool, steam_running: bool, others: usize) {
    if game_mode {
        p.paragraphs.push(format!("You are asked to {what} in Desktop Mode."));
        return;
    }
    if steam_running {
        p.paragraphs.push(format!("Steam closes for a moment to {what}; you can start it again after."));
    }
    match others {
        0 => {}
        1 => p.paragraphs.push("1 other Steam change already waiting is made too.".to_string()),
        n => p.paragraphs.push(format!("{n} other Steam changes already waiting are made too.")),
    }
}

fn finish(mut p: Page<AddAct>, yes: &str, detail: &str, later: &str) -> Page<AddAct> {
    let mut o = option(yes, detail, AddAct::Add);
    o.recommended = true;
    p.options = vec![o, option("Not now", later, AddAct::NotNow)];
    p.b_does(&AddAct::NotNow)
}

/// "Add to Steam with artwork?": `game_mode`: Steam cannot be changed now (it is added in Desktop Mode);
/// `steam_running`: Steam runs in Desktop Mode and closes for a moment; `others`: other changes already waiting.
pub fn page(ports: &[Shown], game_mode: bool, steam_running: bool, others: usize, caller: Caller) -> Page<AddAct> {
    let single = one(ports.len());
    let mut p = base(ports, "Add to Steam with artwork?");
    p.paragraphs.push(match ports {
        [port] => format!("{} is not in Steam yet.", port.name),
        _ => "These ports are not in Steam yet.".to_string(),
    });
    steam_lines(&mut p, if single { "add it" } else { "add them" }, game_mode, steam_running, others);
    let yes = if single { "Add it" } else { "Add them" };
    let detail = match (game_mode, single) {
        (true, true) => "Saved; it is added in Desktop Mode",
        (true, false) => "Saved; they are added in Desktop Mode",
        (false, true) => "Adds its shortcut and artwork; Steam's list is backed up first",
        (false, false) => "Adds their shortcuts and artwork; Steam's list is backed up first",
    };
    let later = match caller {
        Caller::PortPage => "Nothing is changed",
        _ if single => "Its Port page can add it later",
        _ => "Fix problems can add them later",
    };
    finish(p, yes, detail, later)
}

/// What changes on a shortcut that is there, for [`change_page`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Change {
    /// Found by its name only: it starts another program today.
    pub by_name: bool,
    /// The program it starts now and the one it starts after (file names), when the program changes.
    pub from: Option<String>,
    pub to: Option<String>,
    /// The other things that change ("start folder", "icon").
    pub fields: Vec<String>,
}

/// The Port page's "Update the shortcut" / "Use that shortcut for this port": what changes, in plain words.
pub fn change_page(port: &Shown, c: &Change, game_mode: bool, steam_running: bool, others: usize) -> Page<AddAct> {
    let title = if c.by_name { "Use that shortcut for this port?" } else { "Update its Steam shortcut?" };
    let mut p = base(std::slice::from_ref(port), title);
    let name = &port.name;
    p.paragraphs.push(match (c.by_name, &c.from, &c.to) {
        (true, Some(from), Some(to)) => format!("A Steam shortcut named {name} starts {from} now. After this it starts {to}, this port's program."),
        (true, _, _) => format!("A Steam shortcut named {name} starts another program now. After this it starts this port."),
        (false, Some(from), Some(to)) => format!("Its Steam shortcut starts {from} now. After this it starts {to}, this port's program."),
        (false, _, _) if c.fields.is_empty() => "Its Steam shortcut is out of date. After this it matches this port.".to_string(),
        (false, _, _) => format!("Its Steam shortcut is out of date ({}). After this it matches this port.", c.fields.join(", ")),
    });
    p.paragraphs.push("Its name, play time and launch options stay.".to_string());
    steam_lines(&mut p, "update it", game_mode, steam_running, others);
    let detail = if game_mode { "Saved; it is updated in Desktop Mode" } else { "Steam's list is backed up first" };
    finish(p, "Update it", detail, "Nothing is changed")
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
        "Not changed in Steam"
    } else if has(Mark::Waiting) {
        "Waiting for Steam"
    } else if has(Mark::ToDo) {
        "Not added yet"
    } else {
        "Done"
    }
}

/// The file name of a program path, for the change page.
pub fn program_name(path: &str) -> String {
    let path = path.trim().trim_matches('"');
    path.rsplit(['/', '\\']).next().filter(|n| !n.is_empty()).unwrap_or(path).to_string()
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

    fn changes() -> Vec<Change> {
        let long = |s: &str| Some(s.to_string());
        vec![
            Change { by_name: true, from: long("SomethingElseEntirely-x86_64.AppImage"), to: long("ZeldaWWHDRecomp-linux-x86_64.AppImage"), fields: vec!["program".into()] },
            Change { by_name: true, ..Change::default() },
            Change { by_name: false, from: long("dusk"), to: long("dusk.sh"), fields: vec!["program".into(), "start folder".into()] },
            Change { by_name: false, from: None, to: None, fields: vec!["start folder".into(), "icon".into()] },
            Change::default(),
        ]
    }

    fn all_pages() -> Vec<Page<AddAct>> {
        let mut pages = Vec::new();
        for ports in sets() {
            for caller in [Caller::Checkup, Caller::PortPage] {
                for (game_mode, running, others) in [(false, false, 0), (false, true, 0), (true, true, 0), (false, false, 3), (false, true, 1)] {
                    pages.push(page(&ports, game_mode, running, others, caller));
                }
            }
            pages.push(working_page(&ports));
        }
        for name in NAMES {
            for c in changes() {
                for (game_mode, running, others) in [(false, false, 0), (false, true, 2), (true, true, 0)] {
                    pages.push(change_page(&shown(name), &c, game_mode, running, others));
                }
            }
        }
        pages
    }

    /// The full-screen page's title is one line, elided past about 40 characters; an option's title (40) and detail
    /// (66) are one line each on the 800 px page. Every note fits a summary line too.
    #[test]
    fn every_title_option_and_note_fits_with_long_names_and_one_or_three_ports() {
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
        let mut notes: Vec<String> = vec![CHANGED_NOTE.into(), problem_note(CHECK_FAILED, Job::Add), problem_note(CHECK_FAILED, Job::Change)];
        for job in [Job::Add, Job::Change] {
            for n in [1, 3] {
                notes.extend([waiting_note(n, job), desktop_mode_note(n, job), already_note(job)].map(String::from));
            }
        }
        for caller in [Caller::Wizard, Caller::Checkup, Caller::PortPage] {
            for n in [1, 3] {
                notes.extend(account_note(0, false, n, caller).map(|(_, t)| t));
                if let Ask::Skip(Some((_, t))) = ask(&Facts { to_add: 0, other_by_name: true, ..ready() }, n, caller, Job::Add) {
                    notes.push(t);
                }
            }
        }
        for t in &notes {
            assert!(t.chars().count() <= 90 && !t.contains('\u{2014}'), "note too long or with an em dash: {t}");
        }
    }

    #[test]
    fn one_port_is_a_card_with_its_name_and_add_it() {
        let p = page(&[shown("The Legend of Dragoon")], false, false, 0, Caller::Checkup);
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
        assert_eq!(page(&[shown("A")], false, false, 0, Caller::PortPage).options[1].detail, "Nothing is changed", "never points at the page it is on");
    }

    #[test]
    fn several_ports_are_a_tile_each_and_add_them() {
        let p = page(&[shown("The Legend of Dragoon"), shown("The Wind Waker HD")], false, false, 0, Caller::Checkup);
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
        let one = page(&[shown("Dusklight")], false, true, 0, Caller::Checkup);
        assert_eq!(one.paragraphs[1], "Steam closes for a moment to add it; you can start it again after.");
        let two = page(&[shown("A"), shown("B")], false, true, 0, Caller::Checkup);
        assert_eq!(two.paragraphs[1], "Steam closes for a moment to add them; you can start it again after.");
        assert_eq!(page(&[shown("A"), shown("B")], false, false, 0, Caller::Checkup).paragraphs.len(), 1, "Steam closed: nothing to say");
        let others = page(&[shown("A")], false, false, 2, Caller::Checkup);
        assert_eq!(others.paragraphs[1], "2 other Steam changes already waiting are made too.");
        assert_eq!(page(&[shown("A")], false, true, 1, Caller::Checkup).paragraphs[2], "1 other Steam change already waiting is made too.");
    }

    #[test]
    fn game_mode_says_it_is_added_in_desktop_mode() {
        let two = page(&[shown("A"), shown("B")], true, true, 3, Caller::Checkup);
        assert_eq!(two.paragraphs, ["These ports are not in Steam yet.", "You are asked to add them in Desktop Mode."]);
        assert_eq!(two.options[0].detail, "Saved; they are added in Desktop Mode");
        let one = page(&[shown("A")], true, false, 0, Caller::Checkup);
        assert_eq!(one.paragraphs[1], "You are asked to add it in Desktop Mode.");
        assert_eq!(one.options[0].detail, "Saved; it is added in Desktop Mode");
    }

    #[test]
    fn the_change_page_says_what_it_starts_now_and_after() {
        let c = Change { by_name: true, from: Some("other.sh".into()), to: Some("dusk".into()), fields: vec!["program".into()] };
        let p = change_page(&shown("Dusklight"), &c, false, true, 0);
        assert_eq!(p.title, "Use that shortcut for this port?");
        assert_eq!(p.card.as_ref().unwrap().title, "Dusklight");
        assert_eq!(
            p.paragraphs,
            [
                "A Steam shortcut named Dusklight starts other.sh now. After this it starts dusk, this port's program.",
                "Its name, play time and launch options stay.",
                "Steam closes for a moment to update it; you can start it again after.",
            ]
        );
        let opts: Vec<(&str, &str)> = p.options.iter().map(|o| (o.title.as_str(), o.detail.as_str())).collect();
        assert_eq!(opts, [("Update it", "Steam's list is backed up first"), ("Not now", "Nothing is changed")]);
        assert!(p.options[0].recommended);
        assert_eq!(p.back_hint, "Not now");
        let stale = Change { by_name: false, from: None, to: None, fields: vec!["start folder".into()] };
        let p = change_page(&shown("Dusklight"), &stale, true, false, 0);
        assert_eq!(p.title, "Update its Steam shortcut?");
        assert_eq!(p.paragraphs[0], "Its Steam shortcut is out of date (start folder). After this it matches this port.");
        assert_eq!(p.paragraphs[2], "You are asked to update it in Desktop Mode.");
        assert_eq!(p.options[0].detail, "Saved; it is updated in Desktop Mode");
        let moved = Change { by_name: false, from: Some("dusk".into()), to: Some("dusk.sh".into()), fields: vec!["program".into()] };
        assert_eq!(change_page(&shown("D"), &moved, false, false, 0).paragraphs[0], "Its Steam shortcut starts dusk now. After this it starts dusk.sh, this port's program.");
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
        Facts { accounts: 1, chosen: true, problem: None, to_add: 2, queued: 0, other_by_name: false, immediate: true, game_mode: false, waiting: 0, check_failed: false }
    }

    #[test]
    fn it_asks_when_there_is_something_to_add() {
        let ask2 = |f: &Facts| ask(f, 2, Caller::Checkup, Job::Add);
        assert_eq!(ask2(&ready()), Ask::Ask { running: false, game_mode: false, others: 0 });
        assert_eq!(ask2(&Facts { immediate: false, ..ready() }), Ask::Ask { running: true, game_mode: false, others: 0 });
        assert_eq!(ask2(&Facts { immediate: false, game_mode: true, ..ready() }), Ask::Ask { running: false, game_mode: true, others: 0 });
        assert_eq!(ask2(&Facts { queued: 1, waiting: 1, ..ready() }), Ask::Ask { running: false, game_mode: false, others: 0 }, "one of two waits: the other is still asked about");
        assert_eq!(ask2(&Facts { waiting: 3, ..ready() }), Ask::Ask { running: false, game_mode: false, others: 3 }, "other changes waiting with Steam closed: made at the same time");
    }

    #[test]
    fn each_case_with_nothing_to_ask_gives_a_note_instead() {
        let ask_n = |f: &Facts, n: usize| ask(f, n, Caller::Checkup, Job::Add);
        let no_account = Facts { accounts: 0, chosen: false, ..ready() };
        assert_eq!(ask_n(&no_account, 1), Ask::Skip(Some((Mark::ToDo, "Steam: start Steam once, then its Port page can add it".into()))));
        assert_eq!(ask_n(&no_account, 2), Ask::Skip(Some((Mark::ToDo, "Steam: start Steam once, then Fix problems can add them".into()))));
        assert_eq!(ask(&no_account, 1, Caller::PortPage, Job::Add), Ask::Skip(Some((Mark::ToDo, "Steam: start Steam once, then try again here".into()))));
        let several = Facts { accounts: 2, chosen: false, ..ready() };
        assert_eq!(ask_n(&several, 2), Ask::Skip(Some((Mark::ToDo, "Steam: choose your account in Settings > Add to Steam first".into()))));
        let problem = Facts { problem: Some("shortcuts.vdf cannot be read".into()), ..ready() };
        assert_eq!(ask_n(&problem, 2), Ask::Skip(Some((Mark::Problem, "Not added to Steam: shortcuts.vdf cannot be read".into()))));
        let unknown = Facts { immediate: false, check_failed: true, ..ready() };
        assert_eq!(ask_n(&unknown, 2), Ask::Skip(Some((Mark::Problem, "Not added to Steam: could not tell whether Steam is running".into()))), "never \"Steam closes for a moment\"");
        assert!(matches!(ask_n(&Facts { game_mode: true, ..unknown.clone() }, 2), Ask::Ask { game_mode: true, .. }), "Game Mode always queues");
        let queued = Facts { to_add: 1, queued: 1, ..ready() };
        assert_eq!(ask_n(&queued, 1), Ask::Skip(Some((Mark::Waiting, "Steam: it is added when Steam next closes".into()))));
        assert_eq!(ask_n(&Facts { to_add: 2, queued: 2, ..ready() }, 2), Ask::Skip(Some((Mark::Waiting, "Steam: they are added when Steam next closes".into()))));
        assert_eq!(ask(&queued, 1, Caller::PortPage, Job::Change), Ask::Skip(Some((Mark::Waiting, "Steam: its shortcut is updated when Steam next closes".into()))));
        let already = Facts { to_add: 0, ..ready() };
        assert_eq!(ask_n(&already, 2), Ask::Skip(Some((Mark::Done, "Already in Steam".into()))));
        assert_eq!(ask(&already, 1, Caller::PortPage, Job::Change), Ask::Skip(Some((Mark::Done, "Its Steam shortcut is up to date".into()))));
        let other = Facts { to_add: 0, other_by_name: true, ..ready() };
        assert_eq!(ask_n(&other, 1), Ask::Skip(Some((Mark::ToDo, "Steam: a shortcut with its name starts another program; its Port page can fix it".into()))));
        assert_eq!(ask_n(&other, 2), Ask::Skip(Some((Mark::ToDo, "Steam: a shortcut with one of their names starts another program; its Port page can fix it".into()))));
        assert_eq!(ask(&other, 1, Caller::PortPage, Job::Add), Ask::Skip(Some((Mark::ToDo, "Steam: a shortcut with its name starts another program; this card can fix it".into()))));
    }

    #[test]
    fn a_port_uninstalled_meanwhile_or_android_only_is_not_offered() {
        assert!(can_go_in(true, false));
        assert!(!can_go_in(false, false), "uninstalled while the page was open");
        assert!(!can_go_in(true, true));
    }

    #[test]
    fn steam_is_skipped_with_a_note_when_there_is_no_account_to_add_to() {
        assert_eq!(account_note(0, false, 1, Caller::Wizard), Some((Mark::ToDo, "Steam: start Steam once, then its Port page can add it".to_string())));
        assert_eq!(account_note(2, false, 1, Caller::Wizard), Some((Mark::ToDo, "Steam: choose your account in Settings > Add to Steam first".to_string())));
        assert_eq!(account_note(1, true, 1, Caller::Wizard), None);
        assert_eq!(account_note(3, true, 2, Caller::Checkup), None);
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
    fn notes_read_for_one_or_several() {
        assert_eq!(desktop_mode_note(1, Job::Add), "Steam: you are asked to add it in Desktop Mode");
        assert_eq!(desktop_mode_note(2, Job::Add), "Steam: you are asked to add them in Desktop Mode");
        assert_eq!(desktop_mode_note(1, Job::Change), "Steam: you are asked to update its shortcut in Desktop Mode");
        assert_eq!(program_name("\"/home/x/Ports/dusk/dusk.sh\""), "dusk.sh");
        assert_eq!(program_name("C:\\Games\\a.exe"), "a.exe");
    }

    #[test]
    fn the_result_title_is_the_worst_of_what_happened() {
        assert_eq!(result_title(&[(Mark::Done, "Added to Steam".into())]), "Done");
        assert_eq!(result_title(&[(Mark::Done, "Already in Steam".into())]), "Done");
        assert_eq!(result_title(&[(Mark::Waiting, waiting_note(1, Job::Add).into())]), "Waiting for Steam");
        assert_eq!(result_title(&[(Mark::ToDo, "Steam: choose your account in Settings > Add to Steam first".into())]), "Not added yet");
        assert_eq!(result_title(&[(Mark::Done, "x".into()), (Mark::Problem, problem_note("y", Job::Add))]), "Not changed in Steam");
        assert_eq!(problem_note("y", Job::Change), "Not updated in Steam: y");
    }
}
