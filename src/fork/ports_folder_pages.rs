//! Settings > Ports > Ports folder as card pages, outside the setup wizard. Replaces the old lists ("Ports folder:
//! /run/media/deck/1234-ABCD/Ports" as a row, "Finish switching Dusk (Steam and game files still point at the old
//! copy)", "Old copy: {path}", "Remove old copy (its saves are zipped to Saves Backup/Update Backups first)").
//!
//! The pages: the Ports folder page itself (where the ports are now and how much room is left, Finish switching X when
//! a switch was cut short, Choose another folder, Open the ports folder in Desktop Mode, Old copies kept after moves, Use
//! the default folder), the old copies (one card each: the port, where the old copy is, its size, whether it can go),
//! "this old copy is gone", the yes/no before old copies are removed, and what a change did. The move itself uses the
//! setup wizard's pages (`setup_wizard::ports_pages`, through `for_settings`).
//!
//! Pure data and words; `src/app/ports_folder_screen.rs` runs them. Every place is told the human way by the caller
//! (`port_page::paths::human_path`).

use crate::fork::page::{Focus, Icon, Mark, Page, PageButton, PageOption, LEFT};
use crate::fork::rom_screens::util::{ellipsize, human_bytes};

/// The window title of the Ports folder page.
pub const MENU_WINDOW: &str = "Settings: Ports: Ports folder";
/// Its header and the header of the pages under it.
pub const HEADER: &str = "Ports folder";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PfAct {
    /// Finish the cut-short switch `n` of the page's list.
    FinishSwitch(usize),
    ChooseAnother,
    OpenFolder,
    OldCopies,
    UseDefault,
    /// The old copy `n` of the old copies page.
    Copy(usize),
    /// Forget the record of an old copy that is gone.
    Forget,
    /// Yes, remove.
    Remove,
    /// No, keep (and OK on a page that only says something).
    Keep,
    Ok,
    Back,
}

fn option(title: &str, detail: impl Into<String>, action: PfAct) -> PageOption<PfAct> {
    PageOption { title: ellipsize(title, 40), detail: ellipsize(&detail.into(), 66), choice: None, enabled: true, recommended: false, action }
}

fn base(window: &str, title: &str) -> Page<PfAct> {
    Page {
        window_title: window.to_string(),
        large: true,
        header: HEADER.to_string(),
        title: ellipsize(title, 40),
        buttons: [Some(PageButton { label: "Back".to_string(), action: PfAct::Back }), None, None, None],
        initial: Focus::Option(0),
        back_hint: "Back".to_string(),
        ..Page::default()
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

// -- The Ports folder page --------------------------------------------------------------------------------------------

/// What the Ports folder page says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderFacts {
    /// Where the ports are, the human way ("SD card > Ports").
    pub place: String,
    /// The drive and room there ("12.3 GB free, SD card"), or `None` when the folder cannot be reached now.
    pub room: Option<String>,
    /// It is the default folder, next to the app.
    pub is_default: bool,
    /// The default folder, the human way (for "Use the default folder").
    pub default_place: String,
    /// Game Mode: no file manager to open.
    pub game_mode: bool,
    /// Old copies kept after moves: how many and their size.
    pub old_copies: usize,
    pub old_bytes: u64,
    /// Ports whose switch was cut short (the app closed between the setting and Steam).
    pub unfinished: Vec<String>,
}

/// Settings > Ports > Ports folder. B goes back to the Ports page of Settings.
pub fn menu_page(f: &FolderFacts) -> Page<PfAct> {
    let mut p = base(MENU_WINDOW, "Ports folder");
    p.header = "Settings".to_string();
    p.paragraphs.push(format!("Your ports are kept in {}.", f.place));
    match &f.room {
        Some(room) => p.fact(Mark::Done, if f.is_default { format!("{room}. The default folder, next to the app") } else { room.clone() }),
        None => p.fact(Mark::Problem, "Not available now. Is the SD card in?"),
    }
    for (i, name) in f.unfinished.iter().enumerate() {
        let mut o = option(&format!("Finish switching {name}"), "Steam and its game files still use the old copy", PfAct::FinishSwitch(i));
        o.recommended = i == 0;
        p.options.push(o);
    }
    let mut icons = vec![Icon::Update; p.options.len()];
    p.options.push(option("Choose another folder", "Your ports can move there, or stay where they are", PfAct::ChooseAnother));
    icons.push(Icon::Folder);
    if !f.game_mode {
        p.options.push(option("Open the ports folder", "Show it in the file manager", PfAct::OpenFolder));
        icons.push(Icon::Browse);
    }
    if f.old_copies > 0 {
        p.options.push(option("Old copies kept after moves", format!("{}, {}", plural(f.old_copies, "copy", "copies"), human_bytes(f.old_bytes)), PfAct::OldCopies));
        icons.push(Icon::Remove);
    }
    if !f.is_default {
        p.options.push(option("Use the default folder", f.default_place.clone(), PfAct::UseDefault));
        icons.push(Icon::Folder);
    }
    // The cursor starts on the first card: a switch to finish when there is one, else Choose another folder.
    p.initial = Focus::Option(0);
    for (i, icon) in icons.into_iter().enumerate() {
        p = p.icon(i, icon);
    }
    p
}

// -- Old copies -------------------------------------------------------------------------------------------------------

/// One old copy as its card shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyLine {
    pub name: String,
    /// Where the old copy is, the human way.
    pub place: String,
    pub bytes: u64,
    /// "Ready to remove", "Start it once first", "Waiting for Steam", or "Gone already".
    pub state: String,
    /// It only points to a folder elsewhere (removing it frees no space).
    pub link_only: bool,
}

/// "<state>, <size>: <place>" in 66 characters; the place is shortened in the middle, so its start and the port's own
/// folder at the end both stay.
pub fn copy_detail(c: &CopyLine) -> String {
    let size = if c.link_only { "frees no space".to_string() } else { human_bytes(c.bytes) };
    let lead = format!("{}, {size}: ", c.state);
    let room = 66usize.saturating_sub(lead.chars().count()).max(12);
    ellipsize(&format!("{lead}{}", ellipsize(&c.place, room)), 66)
}

/// The old copies kept after moves: one card each. Choosing one asks about it (its status, Remove when it is ready).
pub fn old_copies_page(copies: &[CopyLine]) -> Page<PfAct> {
    let mut p = base("Ports folder: old copies kept", "Old copies kept after moves");
    if copies.is_empty() {
        p.paragraphs.push("No old copies are kept. Everything was moved and checked, or removed already.".to_string());
        p.initial = Focus::Button(LEFT);
        return p;
    }
    p.paragraphs.push("When ports move, the old copies stay until you remove them. Choose one to see whether it can go.".to_string());
    p.options = copies.iter().enumerate().map(|(i, c)| option(&c.name, copy_detail(c), PfAct::Copy(i))).collect();
    p
}

/// The old copy of `name` is not there any more: forget its record? Keep is where the cursor starts.
pub fn gone_page(name: &str, place: &str) -> Page<PfAct> {
    let mut p = base("Ports folder: old copy gone", "This old copy is gone");
    p.header = name.to_string();
    p.paragraphs.push(format!("The old copy of {name} is not in {place} any more."));
    p.paragraphs.push("Forget it, so it is no longer listed? Nothing else is changed.".to_string());
    p.options = vec![option("Forget it", "It is no longer listed", PfAct::Forget), option("Keep it listed", "Nothing changes", PfAct::Keep)];
    p.options[1].recommended = true;
    p.initial = Focus::Option(1);
    p.buttons = [None, None, None, None];
    p.b_does(&PfAct::Keep)
}

/// What a removal will do, for the yes/no page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoveFacts {
    /// The ports whose old copies go.
    pub names: Vec<String>,
    /// What it frees (0 for links only).
    pub bytes: u64,
    /// Where their saves are backed up first, the human way.
    pub saves_place: String,
}

/// "Remove the old copy of Dusklight?" before anything is removed. No is where the cursor starts, and what B does.
pub fn remove_page(f: &RemoveFacts) -> Page<PfAct> {
    let one = f.names.len() == 1;
    let title = if one { format!("Remove the old copy of {}?", f.names[0]) } else { format!("Remove {} old copies?", f.names.len()) };
    let mut p = base("Ports folder: remove old copies", &title);
    if one && title.chars().count() > 40 {
        // A long name goes to the header, where it has room.
        p.title = "Remove this old copy?".to_string();
        p.header = f.names[0].clone();
    }
    let whose = if one { "Its".to_string() } else { "Their".to_string() };
    p.paragraphs.push(format!("{whose} saves are backed up first, to {}. If that backup does not work, nothing is removed.", f.saves_place));
    let freed = if f.bytes > 0 { format!(" This frees {}.", human_bytes(f.bytes)) } else { String::new() };
    p.paragraphs.push(format!("Then the old {} removed. The copy you use now is not touched.{freed}", if one { "copy is" } else { "copies are" }));
    if !one {
        p.facts = f.names.iter().take(6).cloned().collect();
        if f.names.len() > 6 {
            p.facts.push(format!("And {} more.", f.names.len() - 6));
        }
    }
    p.options = vec![
        option(if one { "Yes, remove it" } else { "Yes, remove them" }, "After the saves are backed up", PfAct::Remove),
        option(if one { "No, keep it" } else { "No, keep them" }, "Nothing is removed", PfAct::Keep),
    ];
    p.options[1].recommended = true;
    p.initial = Focus::Option(1);
    p.buttons = [None, None, None, None];
    p.b_does(&PfAct::Keep)
}

// -- What a change did ------------------------------------------------------------------------------------------------

/// Something to say, then OK: what a move or a switch did, why it could not start. `paragraphs` are shown as they are.
pub fn result_page(window: &str, title: &str, paragraphs: Vec<String>, facts: Vec<(Mark, String)>) -> Page<PfAct> {
    let mut p = base(window, title);
    p.paragraphs = paragraphs.into_iter().filter(|t| !t.trim().is_empty()).collect();
    for (mark, line) in facts {
        p.fact(mark, line);
    }
    p.options = vec![option("OK", "", PfAct::Ok)];
    p.options[0].recommended = true;
    p.buttons = [None, None, None, None];
    p.initial = Focus::Option(0);
    p.b_does(&PfAct::Ok)
}

/// One moved port, as the closing page says it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MovedPort {
    pub name: String,
    /// Steam shortcuts that now start the new copy.
    pub steam_changed: usize,
    /// Steam was running: its shortcuts still start the old copy until they are updated.
    pub steam_waiting: bool,
    /// Steam shortcuts that could not be changed or were left alone.
    pub steam_problems: usize,
    /// Connected game files that follow the port.
    pub game_files: usize,
    /// Shortcuts inside the port's folder that were fixed so they keep working.
    pub inner_links: usize,
    /// Moved although its game file check found a problem.
    pub moved_anyway: bool,
    /// Something went wrong with its records (the details are in the log).
    pub record_problem: bool,
}

impl MovedPort {
    /// The port's line, or `None` when there is nothing to say beyond "moved".
    pub fn line(&self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if self.steam_waiting {
            parts.push("Steam still starts the old copy until its shortcuts are updated (close Steam, then open this app)".to_string());
        } else if self.steam_changed > 0 {
            parts.push(format!("Steam now starts it from here ({})", plural(self.steam_changed, "shortcut", "shortcuts")));
        }
        if self.steam_problems > 0 {
            parts.push(format!("{} could not be changed", plural(self.steam_problems, "Steam shortcut", "Steam shortcuts")));
        }
        if self.game_files > 0 {
            parts.push("its game files follow it".to_string());
        }
        if self.inner_links > 0 {
            parts.push(format!("{} inside its folder kept working", plural(self.inner_links, "shortcut", "shortcuts")));
        }
        if self.moved_anyway {
            parts.push("moved although a game file may be missing".to_string());
        }
        if self.record_problem {
            parts.push("one of its records could not be updated".to_string());
        }
        (!parts.is_empty()).then(|| format!("{}: {}.", self.name, parts.join("; ")))
    }
}

/// What the closing page of a change says: the new place, how many ports moved and how big, a line per port, and
/// anything else to know (ports that stayed behind).
pub fn switched_paragraphs(place: &str, moved: &[MovedPort], bytes: u64, notes: &[String]) -> Vec<String> {
    let mut out = vec![format!("Your ports folder is now {place}.")];
    if !moved.is_empty() {
        out.push(format!("{} moved there ({}), each one copied and checked. The old copies are kept until you remove them.", plural(moved.len(), "port", "ports"), human_bytes(bytes)));
    }
    out.extend(moved.iter().filter_map(MovedPort::line));
    out.extend(notes.iter().cloned());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::page::{action_at, initial_focus};
    use crate::fork::rom_screens::settings_pages::tests::{assert_fits, assert_plain, shown};

    const LONG: &str = "The Legend of Zelda: The Wind Waker HD (ZeldaWWHDRecomp)";
    const PLACE: &str = "SD card (SD512) > Emulation > Ports for the Deck > The Legend of Zelda The Wind Waker HD";

    fn facts() -> FolderFacts {
        FolderFacts {
            place: PLACE.into(),
            room: Some("123.4 GB free, SD card (SD512)".into()),
            is_default: false,
            default_place: "Internal storage > Applications > Deck Ports Manager > Library".into(),
            game_mode: false,
            old_copies: 2,
            old_bytes: 1_200_000_000,
            unfinished: vec![],
        }
    }

    fn copy(name: &str, state: &str) -> CopyLine {
        CopyLine { name: name.into(), place: PLACE.into(), bytes: 1_234_000_000, state: state.into(), link_only: false }
    }

    fn all_pages() -> Vec<Page<PfAct>> {
        let mut unfinished = facts();
        unfinished.unfinished = vec![LONG.into(), "Dusk".into()];
        let mut default = facts();
        default.is_default = true;
        default.game_mode = true;
        default.old_copies = 0;
        let mut gone = facts();
        gone.room = None;
        vec![
            menu_page(&facts()),
            menu_page(&unfinished),
            menu_page(&default),
            menu_page(&gone),
            old_copies_page(&[copy(LONG, "Ready to remove"), copy("Dusk", "Start it once from its new place"), CopyLine { link_only: true, ..copy("SoH", "Waiting for Steam to update") }]),
            old_copies_page(&[]),
            gone_page(LONG, PLACE),
            remove_page(&RemoveFacts { names: vec![LONG.into()], bytes: 1_000_000_000, saves_place: "Internal storage > Applications > Deck Ports Manager > Saves Backup".into() }),
            remove_page(&RemoveFacts { names: vec!["Dusk".into()], bytes: 0, saves_place: "SD card > Saves Backup".into() }),
            remove_page(&RemoveFacts { names: (0..9).map(|i| format!("Port {i}")).collect(), bytes: 5, saves_place: "x".into() }),
            result_page("Ports folder: done", "Ports folder changed", switched_paragraphs(PLACE, &[MovedPort { name: LONG.into(), steam_waiting: true, game_files: 2, inner_links: 1, moved_anyway: true, record_problem: true, steam_problems: 1, ..MovedPort::default() }], 5_000_000_000, &["Dusk stays in the old folder.".into()]), vec![]),
        ]
    }

    #[test]
    fn every_ports_folder_page_fits_and_says_it_plainly() {
        for p in all_pages() {
            assert_fits(&p);
            assert_plain(&p, &["ZeldaWWHDRecomp"]);
            for t in shown(&p) {
                let lower = t.to_lowercase();
                for word in ["zip", "/run/media", "/home/", "~/", "repoint", "staging", "(s)", "json", "display()", "symlink", "queue"] {
                    assert!(!lower.contains(word), "{word:?} in {t:?}");
                }
            }
            assert!(p.window_title.starts_with("Ports folder: ") || p.window_title == MENU_WINDOW, "{}", p.window_title);
        }
    }

    #[test]
    fn the_ports_folder_page_says_where_the_ports_are_and_starts_on_choose_another_folder() {
        let p = menu_page(&facts());
        assert_eq!(p.window_title, "Settings: Ports: Ports folder");
        assert_eq!(p.paragraphs, [format!("Your ports are kept in {PLACE}.")]);
        assert_eq!(p.facts, ["123.4 GB free, SD card (SD512)"]);
        let titles: Vec<&str> = p.options.iter().map(|o| o.title.as_str()).collect();
        assert_eq!(titles, ["Choose another folder", "Open the ports folder", "Old copies kept after moves", "Use the default folder"]);
        assert_eq!(p.options[2].detail, "2 copies, 1.2 GB");
        assert_eq!(p.options[3].detail, "Internal storage > Applications > Deck Ports Manager > Library");
        assert_eq!(action_at(&p, initial_focus(&p)), Some(PfAct::ChooseAnother));
        assert_eq!(p.buttons[LEFT].as_ref().map(|b| b.action), Some(PfAct::Back));
        let mut d = facts();
        d.is_default = true;
        d.game_mode = true;
        let q = menu_page(&d);
        assert!(q.facts[0].ends_with("The default folder, next to the app"));
        assert!(!q.options.iter().any(|o| matches!(o.action, PfAct::UseDefault | PfAct::OpenFolder)), "no file manager in Game Mode, no default when it is the default");
    }

    #[test]
    fn a_switch_cut_short_comes_first_and_a_missing_folder_says_so() {
        let mut f = facts();
        f.unfinished = vec!["Dusk".into()];
        let p = menu_page(&f);
        assert_eq!((p.options[0].title.as_str(), p.options[0].detail.as_str()), ("Finish switching Dusk", "Steam and its game files still use the old copy"));
        assert!(p.options[0].recommended);
        assert_eq!(action_at(&p, initial_focus(&p)), Some(PfAct::FinishSwitch(0)));
        f.room = None;
        assert_eq!(menu_page(&f).facts, ["Not available now. Is the SD card in?"]);
        assert_eq!(menu_page(&f).mark_of(0), Mark::Problem);
    }

    #[test]
    fn each_old_copy_is_a_card_with_its_state_size_and_place() {
        let p = old_copies_page(&[copy("Dusk", "Ready to remove")]);
        assert_eq!(p.options[0].title, "Dusk");
        assert!(p.options[0].detail.starts_with("Ready to remove, 1.2 GB: SD card"), "{}", p.options[0].detail);
        assert!(p.options[0].detail.chars().count() <= 66);
        assert!(p.options[0].detail.ends_with("Wind Waker HD"), "the port's own folder stays: {}", p.options[0].detail);
        let link = copy_detail(&CopyLine { link_only: true, ..copy("Dusk", "Ready to remove") });
        assert!(link.starts_with("Ready to remove, frees no space: "), "{link}");
        let none = old_copies_page(&[]);
        assert!(none.options.is_empty());
        assert_eq!(action_at(&none, initial_focus(&none)), Some(PfAct::Back));
    }

    #[test]
    fn removing_asks_first_says_where_the_saves_go_and_starts_on_no() {
        let p = remove_page(&RemoveFacts { names: vec!["Dusk".into()], bytes: 2_000_000_000, saves_place: "Internal storage > Saves Backup".into() });
        assert_eq!(p.title, "Remove the old copy of Dusk?");
        assert_eq!(p.paragraphs[0], "Its saves are backed up first, to Internal storage > Saves Backup. If that backup does not work, nothing is removed.");
        assert!(p.paragraphs[1].contains("The copy you use now is not touched. This frees 2.0 GB."));
        assert_eq!(action_at(&p, initial_focus(&p)), Some(PfAct::Keep));
        assert_eq!(p.back_hint, "No, keep it");
        let long = remove_page(&RemoveFacts { names: vec![LONG.into()], ..RemoveFacts::default() });
        assert_eq!((long.title.as_str(), long.header.as_str()), ("Remove this old copy?", LONG));
        let many = remove_page(&RemoveFacts { names: (0..8).map(|i| format!("P{i}")).collect(), ..RemoveFacts::default() });
        assert_eq!(many.title, "Remove 8 old copies?");
        assert_eq!(many.facts.last().map(String::as_str), Some("And 2 more."));
    }

    #[test]
    fn a_gone_copy_is_forgotten_only_when_asked() {
        let p = gone_page("Dusk", "SD card > Ports > Dusk");
        assert_eq!(action_at(&p, initial_focus(&p)), Some(PfAct::Keep));
        assert_eq!(p.back_hint, "Keep it listed");
        assert!(p.paragraphs[0].contains("not in SD card > Ports > Dusk any more"));
    }

    #[test]
    fn the_closing_page_names_the_place_the_ports_and_what_to_know() {
        let moved = [MovedPort { name: "Dusk".into(), steam_changed: 2, game_files: 1, ..MovedPort::default() }, MovedPort { name: "SoH".into(), ..MovedPort::default() }];
        let paragraphs = switched_paragraphs("SD card > Ports", &moved, 1_500_000_000, &[]);
        assert_eq!(paragraphs[0], "Your ports folder is now SD card > Ports.");
        assert_eq!(paragraphs[1], "2 ports moved there (1.5 GB), each one copied and checked. The old copies are kept until you remove them.");
        assert_eq!(paragraphs[2], "Dusk: Steam now starts it from here (2 shortcuts); its game files follow it.");
        assert_eq!(paragraphs.len(), 3, "nothing to say about SoH");
        let waiting = MovedPort { name: "X".into(), steam_waiting: true, steam_changed: 1, ..MovedPort::default() };
        assert!(waiting.line().unwrap().contains("Steam still starts the old copy"));
        assert_eq!(switched_paragraphs("Internal storage > Games", &[], 0, &["a".into()]), ["Your ports folder is now Internal storage > Games.", "a"]);
    }
}
