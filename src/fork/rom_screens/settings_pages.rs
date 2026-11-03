//! Settings > Ports > Game files as card pages. Replaces the old picker lists (a sentence as a menu row, "Results by
//! system (scanned ...)", "Find game files for a port...", "12 not used by any port, 340 other files ignored", "Connect
//! game files: X" with "link" and "checksum" in its rows). One question a page, plain words, every place the human way.
//!
//! The pages: the Game files page itself (where your game files are, Look again, What was found, Ports that need game
//! files, Connected), what was found by console, the ports of one console, the ports that need game files, the connected
//! ports and the "forget this connection?" question. Choosing a port leads to the one-port pages (`super::pages`), which
//! also ask how a file reaches the port ([`super::pages::how_page`]).
//!
//! Pure data and words; `src/app/rom_screens.rs` runs them.

use super::connections::ConnGroup;
use super::find::count_files;
use super::report::{PortRow, SystemGroup};
use super::util::ellipsize;
use crate::fork::page::{Focus, Icon, Mark, Page, PageButton, PageOption, LEFT};
use crate::fork::rom_library::{Health, Method, PortStatus};

/// Said on the Game files page and the pages under it.
pub const READ_ONLY: &str = "Your game files are only read, never moved or deleted.";

/// The window title of the Game files page (Settings > Ports > Game files).
pub const MENU_WINDOW: &str = "Settings: Ports: Game files";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GsAct {
    /// Choose where the game files are.
    ChooseFolder,
    /// Look through the folder again.
    LookAgain,
    /// What the last look found, by console.
    WhatWasFound,
    /// The installed ports that need game files.
    NeedFiles,
    /// The connected ports.
    Connected,
    /// The `n`th console of the found page.
    System(usize),
    /// The `n`th port of a page that lists ports.
    Port(usize),
    /// The `n`th connection of the connected page.
    Connection(usize),
    /// Yes, forget that connection.
    Forget,
    /// No, keep it.
    Keep,
    /// Up one page (the Back button, B).
    Back,
}

fn option(title: &str, detail: impl Into<String>, action: GsAct) -> PageOption<GsAct> {
    PageOption { title: ellipsize(title, 40), detail: ellipsize(&detail.into(), 66), choice: None, enabled: true, recommended: false, action }
}

fn base(window: &str, title: &str) -> Page<GsAct> {
    Page {
        window_title: window.to_string(),
        large: true,
        header: "Game files".to_string(),
        title: title.to_string(),
        buttons: [Some(PageButton { label: "Back".to_string(), action: GsAct::Back }), None, None, None],
        initial: Focus::Option(0),
        back_hint: "Back".to_string(),
        ..Page::default()
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

// -- The Game files page ----------------------------------------------------------------------------------------------

/// What the Game files page says about each card.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MenuFacts {
    /// The game files folder the human way ("SD card > Emulation > roms"), when one is chosen.
    pub folder: Option<String>,
    /// When the folder was last looked through ("2026-10-30 14:22"), when it was.
    pub looked: Option<String>,
    /// Installed ports that need game files, and how many of them are not connected yet.
    pub need_total: usize,
    pub need_missing: usize,
    /// Ports with connected game files.
    pub connected: usize,
}

/// Settings > Ports > Game files: where the files are, Look again, what was found, the ports that need files, the
/// connected ones. B goes back to the Ports page of Settings.
pub fn menu_page(f: &MenuFacts) -> Page<GsAct> {
    let mut p = base(MENU_WINDOW, "Game files");
    p.header = "Settings".to_string();
    p.paragraphs.push(READ_ONLY.to_string());
    let mut folder = option("Where your game files are", f.folder.clone().unwrap_or_else(|| "Not chosen yet: choose the folder you keep them in".to_string()), GsAct::ChooseFolder);
    folder.recommended = f.folder.is_none();
    let look = match (&f.folder, &f.looked) {
        (None, _) => {
            let mut o = option("Look again", "Choose the folder first", GsAct::LookAgain);
            o.enabled = false;
            o
        }
        (Some(_), Some(when)) => option("Look again", format!("Last looked {when}"), GsAct::LookAgain),
        (Some(_), None) => {
            let mut o = option("Look through the folder", "Finds the game files your ports need", GsAct::LookAgain);
            o.recommended = true;
            o
        }
    };
    let mut found = option("What was found", "By console, from the last look", GsAct::WhatWasFound);
    if f.looked.is_none() {
        found.detail = "Nothing yet: look through the folder first".to_string();
        found.enabled = false;
    }
    let need = match (f.need_total, f.need_missing) {
        (0, _) => "None of your installed ports needs game files".to_string(),
        (_, 0) => "Every one of them has its game files".to_string(),
        (_, n) => format!("{} not connected yet", plural(n, "port", "ports")),
    };
    let connected = match f.connected {
        0 => "None yet".to_string(),
        n => format!("{} connected", plural(n, "port", "ports")),
    };
    p.options = vec![folder, look, found, option("Ports that need game files", need, GsAct::NeedFiles), option("Connected ports", connected, GsAct::Connected)];
    // The cursor starts on what to do next (the Recommended card), else on the first card.
    p.initial = Focus::Option(p.options.iter().position(|o| o.recommended).unwrap_or(0));
    p.icon(0, Icon::Folder).icon(1, Icon::Update).icon(2, Icon::Browse).icon(3, Icon::GameFiles).icon(4, Icon::Link)
}

// -- What was found ---------------------------------------------------------------------------------------------------

/// One console of what was found, counted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SystemLine {
    pub label: String,
    pub files: usize,
    pub ports: usize,
    pub connected: usize,
    pub found: usize,
    pub to_choose: usize,
    pub wrong: usize,
    pub missing: usize,
}

impl SystemLine {
    pub fn of(g: &SystemGroup) -> SystemLine {
        let count = |f: &dyn Fn(&PortRow) -> bool| g.ports.iter().filter(|r| f(r)).count();
        SystemLine {
            label: g.system.label().to_string(),
            files: g.files,
            ports: g.ports.len(),
            connected: count(&|r| r.connected),
            found: count(&|r| !r.connected && r.status == PortStatus::Found),
            to_choose: count(&|r| !r.connected && r.status == PortStatus::Ambiguous),
            wrong: count(&|r| !r.connected && matches!(r.status, PortStatus::Partial | PortStatus::WrongVersion)),
            missing: count(&|r| !r.connected && matches!(r.status, PortStatus::Missing | PortStatus::Unverifiable)),
        }
    }

    /// What its ports stand at ("1 connected, 2 found, 1 missing"), or "3 game files, no port of yours needs them". The
    /// number of files is on the page above; the ports are what the card is for.
    pub fn words(&self) -> String {
        if self.ports == 0 {
            return format!("{}, no port of yours needs them", count_files(self.files));
        }
        let mut parts = Vec::new();
        for (n, word) in [(self.connected, "connected"), (self.found, "found"), (self.to_choose, "to choose"), (self.wrong, if self.wrong == 1 { "needs a look" } else { "need a look" }), (self.missing, "missing")] {
            if n > 0 {
                parts.push(format!("{n} {word}"));
            }
        }
        parts.join(", ")
    }
}

/// What the last look found, ready for the page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FoundFacts {
    /// The folder the human way.
    pub place: String,
    pub total: usize,
    pub unused: usize,
    pub ignored: usize,
    /// Folders that could not be read.
    pub problems: usize,
    /// Something to know (the look could not be saved).
    pub note: Option<String>,
    /// Consoles with ports first.
    pub systems: Vec<SystemLine>,
}

/// "What was found": how many game files, then one card per console (a console no port needs is shown dimmed).
pub fn found_page(f: &FoundFacts) -> Page<GsAct> {
    let mut p = base("Game files: what was found", "What was found");
    p.paragraphs.push(format!("{} in {}.", capital(&count_files(f.total)), f.place));
    let left_alone = match f.ignored {
        0 => String::new(),
        1 => ", and 1 other file was left alone".to_string(),
        n => format!(", and {n} other files were left alone"),
    };
    p.paragraphs.push(format!("{} not used by any of your ports{left_alone}. Choose a console to see its ports.", plural_be(f.unused, "is", "are")));
    if f.problems > 0 {
        p.fact(Mark::Problem, format!("{} could not be read and {} skipped", plural(f.problems, "folder", "folders"), if f.problems == 1 { "was" } else { "were" }));
    }
    if let Some(note) = &f.note {
        p.fact(Mark::Problem, note.clone());
    }
    p.options = f
        .systems
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut o = option(&s.label, s.words(), GsAct::System(i));
            o.enabled = s.ports > 0;
            o
        })
        .collect();
    p.options.push(option("Look again", "Look through the folder once more", GsAct::LookAgain));
    p.initial = Focus::Option(p.options.iter().position(|o| o.enabled).unwrap_or(0));
    p
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// "1 is", "12 are".
fn plural_be(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// A port in a list of ports, as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortLine {
    pub name: String,
    pub words: String,
}

/// What was found for one port, in a few words.
pub fn port_words(row: &PortRow) -> String {
    let what = if row.connected {
        "Connected".to_string()
    } else {
        match row.status {
            PortStatus::Found if row.low_confidence => "Found. Not checked: the name matches".to_string(),
            PortStatus::Found => "Found. Checked: it is the right file".to_string(),
            PortStatus::Ambiguous => "Several files could be it: choose one".to_string(),
            PortStatus::Partial => "Some of its files were found".to_string(),
            PortStatus::WrongVersion => "Only another version of the game was found".to_string(),
            PortStatus::Missing => "Not found".to_string(),
            PortStatus::Unverifiable => "Cannot be checked".to_string(),
            PortStatus::Unspecified => "Needs no game files".to_string(),
        }
    };
    if row.installed { what } else { format!("{what} (not installed)") }
}

/// The ports of one console: a card each, what was found as its detail.
pub fn system_page(label: &str, files: usize, ports: &[PortLine]) -> Page<GsAct> {
    let mut p = base(&format!("Game files: {label}"), &ellipsize(&format!("Game files for {label}"), 40));
    p.paragraphs.push(format!("{} for {label}. Choose a port to see what was found and connect it.", capital(&count_files(files))));
    p.options = ports.iter().enumerate().map(|(i, l)| option(&l.name, l.words.clone(), GsAct::Port(i))).collect();
    p
}

// -- Ports that need game files -----------------------------------------------------------------------------------------

/// The installed ports that need game files, the ones not connected yet first. Choosing one looks for its files.
pub fn need_page(ports: &[(String, bool)]) -> Page<GsAct> {
    let mut p = base("Game files: ports that need them", "Ports that need game files");
    if ports.is_empty() {
        p.paragraphs.push("None of your installed ports needs game files.".to_string());
        p.initial = Focus::Button(LEFT);
        return p;
    }
    p.paragraphs.push("Choose a port to look for its game files and connect them.".to_string());
    p.options = ports.iter().enumerate().map(|(i, (name, connected))| option(name, if *connected { "Connected" } else { "Not connected yet" }, GsAct::Port(i))).collect();
    p
}

// -- Connected ports ----------------------------------------------------------------------------------------------------

/// How a connected file reaches its port, in plain words.
pub fn way_words(m: Method) -> &'static str {
    match m {
        Method::UseInPlace => "read where it is",
        Method::Symlink => "a shortcut in its folder leads to your file",
        Method::Copy => "copied into its folder",
    }
}

/// What is wrong with a connection, in plain words.
pub fn health_words(h: Health) -> &'static str {
    match h {
        Health::Ok => "fine",
        Health::SourceMissing => "your file is gone (card or drive out?)",
        Health::SourceChanged => "your file has changed",
        Health::DestMissing => "gone from the port's folder",
        Health::DestChanged => "changed in the port's folder",
    }
}

/// "oot.z64: a shortcut in its folder leads to your file", "game.cue and 1 more: your file is gone (card or drive out?)". The file
/// name is shortened first, so what happened always shows.
pub fn connection_words(c: &ConnGroup) -> String {
    let more = if c.more > 0 { format!(" and {} more", c.more) } else { String::new() };
    let what = match c.health {
        Health::Ok => way_words(c.method),
        h => health_words(h),
    };
    let room = 66usize.saturating_sub(more.chars().count() + what.chars().count() + 2).max(10);
    format!("{}{more}: {what}", ellipsize(&c.file, room))
}

/// The connected ports: one card each. Choosing one asks whether to forget it.
pub fn connected_page(conns: &[ConnGroup]) -> Page<GsAct> {
    let mut p = base("Game files: connected", "Connected ports");
    if conns.is_empty() {
        p.paragraphs.push("No game files are connected yet.".to_string());
        p.initial = Focus::Button(LEFT);
        return p;
    }
    p.paragraphs.push("Choose one to forget it. Forgetting never deletes a file.".to_string());
    p.options = conns.iter().enumerate().map(|(i, c)| option(&c.name, connection_words(c), GsAct::Connection(i))).collect();
    p
}

/// "Forget this connection?": only the app's note goes. No is where the cursor starts.
pub fn forget_page(c: &ConnGroup) -> Page<GsAct> {
    let mut p = base("Game files: forget", "Forget this connection?");
    p.header = c.name.clone();
    let file = if c.more > 0 { format!("{} and {} more", c.file, c.more) } else { c.file.clone() };
    p.paragraphs.push(format!("{} uses {file}.", c.name));
    p.paragraphs.push("Only this app's note of it is removed. Your game files, and what is in the port's folder, stay where they are.".to_string());
    p.options = vec![option("Yes, forget it", "Your files are not touched", GsAct::Forget), option("No, keep it", "Nothing changes", GsAct::Keep)];
    p.options[1].recommended = true;
    p.initial = Focus::Option(1);
    p.buttons = [None, None, None, None];
    p.b_does(&GsAct::Keep)
}

// -- A notice ------------------------------------------------------------------------------------------------------------

/// Something to say, then back (a folder that could not be used, no game files in it, a look that went wrong).
pub fn notice_page(title: &str, text: &str) -> Page<GsAct> {
    let mut p = base("Game files: notice", title);
    p.paragraphs = text.split("\n\n").map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect();
    p.initial = Focus::Button(LEFT);
    p
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::fork::page::{action_at, initial_focus};

    pub(crate) const JARGON: [&str; 13] = ["checksum", "sha", "method", "link", "symlink", "hash", "json", "scan file", "slot", "~/", "crc", "Method", "Link"];

    /// Every word a page shows: titles, paragraphs, summary lines, option titles and details, buttons.
    pub(crate) fn shown<A>(p: &Page<A>) -> Vec<String> {
        let mut out = vec![p.title.clone(), p.header.clone()];
        out.extend(p.paragraphs.iter().cloned());
        out.extend(p.facts.iter().cloned());
        out.extend(p.options.iter().flat_map(|o| [o.title.clone(), o.detail.clone()]));
        out.extend(p.buttons.iter().flatten().map(|b| b.label.clone()));
        out.extend(p.tiles.iter().flat_map(|t| [t.label.clone(), t.note.clone()]));
        out
    }

    /// No em dash, no double hyphen, none of the words the owner does not use. `allowed` are names (a port called
    /// "Linked" would be fine) that may hold one.
    pub(crate) fn assert_plain<A>(p: &Page<A>, allowed: &[&str]) {
        for t in shown(p) {
            assert!(!t.contains('\u{2014}') && !t.contains("--"), "no em dash or double hyphen: {t}");
            let mut cleaned = t.clone();
            for a in allowed {
                cleaned = cleaned.replace(a, "");
            }
            let lower = cleaned.to_lowercase();
            for j in JARGON {
                let hit = if j == "sha" { lower.split(|c: char| !c.is_alphanumeric()).any(|w| w == "sha" || w == "sha1") } else { lower.contains(&j.to_lowercase()) };
                assert!(!hit, "jargon {j:?} in {t:?} ({:?})", p.title);
            }
        }
    }

    pub(crate) fn assert_fits<A>(p: &Page<A>) {
        assert!(p.title.chars().count() <= 40, "title too long ({}): {}", p.title.chars().count(), p.title);
        for o in &p.options {
            assert!(!o.title.is_empty());
            assert!(o.title.chars().count() <= 40, "{:?}: option title too long: {}", p.title, o.title);
            assert!(o.detail.chars().count() <= 66, "{:?}: detail too long ({}): {}", p.title, o.detail.chars().count(), o.detail);
        }
        for b in p.buttons.iter().flatten() {
            assert!(b.label.chars().count() <= 16, "{}", b.label);
        }
    }

    const LONG: &str = "The Legend of Zelda: The Wind Waker HD (ZeldaWWHDRecomp)";
    const PLACE: &str = "SD card (SD512) > Emulation > roms > wiiu > The Legend of Zelda The Wind Waker HD [USA]";

    fn conn(name: &str, file: &str, more: usize, method: Method, health: Health) -> ConnGroup {
        ConnGroup { port_key: "k".into(), slot: "s".into(), health, name: name.into(), file: file.into(), more, method }
    }

    fn sys(label: &str, ports: usize) -> SystemLine {
        SystemLine { label: label.into(), files: 12_431, ports, connected: 1, found: 2, to_choose: 1, wrong: 1, missing: 3 }
    }

    fn all_pages() -> Vec<Page<GsAct>> {
        let mut pages = vec![
            menu_page(&MenuFacts::default()),
            menu_page(&MenuFacts { folder: Some(PLACE.into()), looked: Some("2026-10-30 14:22".into()), need_total: 12, need_missing: 3, connected: 9 }),
            menu_page(&MenuFacts { folder: Some("Internal storage > Home".into()), looked: None, need_total: 1, need_missing: 0, connected: 1 }),
            found_page(&FoundFacts { place: PLACE.into(), total: 12_431, unused: 1, ignored: 340, problems: 2, note: Some("This look could not be saved, so it runs again next time.".into()), systems: vec![sys("Master System", 3), sys("Game Boy Color", 0), sys("Xbox 360", 1)] }),
            system_page("Game Boy Color", 1, &[PortLine { name: LONG.into(), words: "Found. Not checked: the name matches (not installed)".into() }]),
            need_page(&[(LONG.into(), false), ("Dusklight".into(), true)]),
            need_page(&[]),
            connected_page(&[conn(LONG, "The Legend of Zelda - The Wind Waker HD (USA) (En,Fr,Es) (Rev 1).wua", 3, Method::Symlink, Health::SourceMissing), conn("SoH", "fake-n64-game.z64", 0, Method::Copy, Health::Ok)]),
            connected_page(&[]),
            forget_page(&conn(LONG, "fake.wua", 1, Method::UseInPlace, Health::Ok)),
            notice_page("No game files were found", "Nothing in SD card > Emulation.\n\nTry another folder."),
        ];
        for m in [Method::UseInPlace, Method::Symlink, Method::Copy] {
            for h in [Health::Ok, Health::SourceMissing, Health::SourceChanged, Health::DestMissing, Health::DestChanged] {
                pages.push(connected_page(&[conn("Very Long Port Name Number One (Recomp)", "A Long Fake Game Name (USA) (Rev 2).z64", 12, m, h)]));
            }
        }
        pages
    }

    #[test]
    fn every_settings_page_fits_and_says_it_plainly() {
        for p in all_pages() {
            assert_fits(&p);
            assert_plain(&p, &[]);
            assert!(p.window_title.starts_with("Game files") || p.window_title == MENU_WINDOW, "{}", p.window_title);
        }
    }

    #[test]
    fn the_game_files_page_has_its_five_cards_and_the_note_is_text_not_a_card() {
        let p = menu_page(&MenuFacts { folder: Some("SD card > Emulation > roms".into()), looked: Some("2026-10-30 14:22".into()), need_total: 4, need_missing: 2, connected: 2 });
        assert_eq!(p.window_title, "Settings: Ports: Game files");
        assert_eq!(p.paragraphs, [READ_ONLY]);
        let cards: Vec<(&str, &str)> = p.options.iter().map(|o| (o.title.as_str(), o.detail.as_str())).collect();
        assert_eq!(
            cards,
            [
                ("Where your game files are", "SD card > Emulation > roms"),
                ("Look again", "Last looked 2026-10-30 14:22"),
                ("What was found", "By console, from the last look"),
                ("Ports that need game files", "2 ports not connected yet"),
                ("Connected ports", "2 ports connected"),
            ]
        );
        assert_eq!(p.options.iter().map(|o| o.action).collect::<Vec<_>>(), [GsAct::ChooseFolder, GsAct::LookAgain, GsAct::WhatWasFound, GsAct::NeedFiles, GsAct::Connected]);
        assert_eq!(action_at(&p, initial_focus(&p)), Some(GsAct::ChooseFolder));
        assert_eq!(p.buttons[LEFT].as_ref().map(|b| b.action), Some(GsAct::Back));
        assert!(p.options.iter().all(|o| o.enabled));
    }

    #[test]
    fn with_no_folder_the_folder_card_is_recommended_and_look_again_waits_for_it() {
        let p = menu_page(&MenuFacts::default());
        assert!(p.options[0].recommended);
        assert_eq!(p.options[0].detail, "Not chosen yet: choose the folder you keep them in");
        assert!(!p.options[1].enabled && !p.options[2].enabled);
        let first = menu_page(&MenuFacts { folder: Some("SD card".into()), ..MenuFacts::default() });
        assert_eq!(first.options[1].title, "Look through the folder");
        assert!(first.options[1].recommended && first.options[1].enabled);
        assert_eq!(action_at(&first, initial_focus(&first)), Some(GsAct::LookAgain), "the cursor starts on the Recommended card");
        assert_eq!(action_at(&p, initial_focus(&p)), Some(GsAct::ChooseFolder));
        assert!(!first.options[2].enabled, "nothing found yet");
        assert_eq!(first.options[3].detail, "None of your installed ports needs game files");
        assert_eq!(first.options[4].detail, "None yet");
    }

    #[test]
    fn what_was_found_counts_in_words_and_dims_consoles_no_port_needs() {
        let p = found_page(&FoundFacts { place: "SD card > Emulation > roms".into(), total: 352, unused: 12, ignored: 340, problems: 1, note: None, systems: vec![sys("N64", 2), sys("SNES", 0)] });
        assert_eq!(p.paragraphs[0], "352 game files in SD card > Emulation > roms.");
        assert_eq!(p.paragraphs[1], "12 are not used by any of your ports, and 340 other files were left alone. Choose a console to see its ports.");
        assert_eq!(p.facts, ["1 folder could not be read and was skipped"]);
        let none_left = found_page(&FoundFacts { place: "SD card".into(), total: 4, unused: 2, systems: vec![sys("N64", 1)], ..FoundFacts::default() });
        assert_eq!(none_left.paragraphs[1], "2 are not used by any of your ports. Choose a console to see its ports.", "no \"0 other files\"");
        assert_eq!(p.options[0].detail, "1 connected, 2 found, 1 to choose, 1 needs a look, 3 missing");
        assert!(p.options[0].enabled && !p.options[1].enabled);
        assert_eq!(p.options[1].detail, "12,431 game files, no port of yours needs them");
        assert_eq!(p.options[2].action, GsAct::LookAgain);
        assert_eq!(action_at(&p, initial_focus(&p)), Some(GsAct::System(0)));
    }

    #[test]
    fn the_connected_page_says_how_each_file_reaches_its_port_and_forgetting_starts_on_no() {
        let p = connected_page(&[conn("SoH", "fake.z64", 0, Method::Symlink, Health::Ok), conn("Severed Chains", "fake.cue", 1, Method::UseInPlace, Health::SourceMissing)]);
        assert_eq!(p.options[0].detail, "fake.z64: a shortcut in its folder leads to your file");
        assert_eq!(p.options[1].detail, "fake.cue and 1 more: your file is gone (card or drive out?)");
        let f = forget_page(&conn("SoH", "fake.z64", 0, Method::Copy, Health::Ok));
        assert_eq!(f.title, "Forget this connection?");
        assert_eq!(action_at(&f, initial_focus(&f)), Some(GsAct::Keep));
        assert_eq!(f.back_hint, "No, keep it");
        assert!(f.paragraphs[1].contains("stay where they are"));
    }

    #[test]
    fn empty_lists_say_so_and_start_on_back() {
        for p in [need_page(&[]), connected_page(&[])] {
            assert!(p.options.is_empty());
            assert_eq!(action_at(&p, initial_focus(&p)), Some(GsAct::Back));
        }
        let need = need_page(&[("SoH".into(), false)]);
        assert_eq!((need.options[0].title.as_str(), need.options[0].detail.as_str()), ("SoH", "Not connected yet"));
    }
}
