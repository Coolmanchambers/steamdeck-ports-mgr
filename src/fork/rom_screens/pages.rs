//! The game files result for one port as card pages (the Port page's Game files card, the search after an install,
//! Info > Game files..., Settings > Game files > Find game files for a port). Replaces the old picker list whose sentences
//! read like menu rows. One question per page, plain words, every place the human way ("SD card > Emulation > roms").
//! The flow that shows them is `src/app/rom_screens/find_flow.rs`; what was found is `find::FindOutcome`.

use super::find::Need;
use super::util::ellipsize;
use crate::fork::page::{Focus, Mark, Page, PageButton, PageOption, LEFT};
use crate::fork::rom_library::requirements::SlotKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GfAct {
    /// Found: connect the file(s) found.
    UseIt,
    /// Pick the file in a file list.
    ChooseFile,
    /// Search the game files folder again.
    LookAgain,
    /// Choose (another) game files folder.
    ChooseFolder,
    /// Leave it for now.
    Later,
    /// One of several matching files.
    Pick(usize),
    /// Copy the files (the copy question).
    Copy,
    /// Done with a result page.
    Done,
    /// Choose the file myself: up one folder, into folder `n`, file `n`.
    Up,
    Folder(usize),
    File(usize),
    /// A hand-picked file that does not match: use it for the `n`th thing it could be, or choose another.
    UseAnyway(usize),
    OtherFile,
}

/// The window title of every page of the flow.
pub fn window_title(name: &str) -> String {
    format!("Game files for {name}")
}

fn base(name: &str, title: &str) -> Page<GfAct> {
    Page { window_title: window_title(name), large: true, header: name.to_string(), title: title.to_string(), initial: Focus::Option(0), back_hint: "Later".into(), ..Page::default() }
}

fn option(title: &str, detail: &str, action: GfAct) -> PageOption<GfAct> {
    PageOption { title: title.to_string(), detail: detail.to_string(), choice: None, enabled: true, recommended: false, action }
}

/// One file the port needs, in plain words: the file's name, or for a port that only names its game, "your own copy of
/// <game>". No checksums, no "(only the game is named...)".
pub fn need_words(n: &Need, port_name: &str) -> String {
    let bios = if n.kind == SlotKind::Bios { "BIOS: " } else { "" };
    if n.title_only {
        let game = if n.name.trim().is_empty() { port_name } else { n.name.trim() };
        return match &n.kind_hint {
            Some(kind) => format!("{bios}Your own copy of {game}: {kind}"),
            None => format!("{bios}Your own copy of {game}"),
        };
    }
    let others = match n.other_choices {
        0 => String::new(),
        1 => " (or 1 other file it accepts)".to_string(),
        k => format!(" (or one of {k} other files it accepts)"),
    };
    format!("{bios}{}{others}", n.name)
}

fn needs_facts(page: &mut Page<GfAct>, needs: &[Need], port_name: &str, mark: Mark) {
    for n in needs.iter().take(4) {
        page.fact(mark, need_words(n, port_name));
    }
    if needs.len() > 4 {
        page.fact(mark, format!("And {} more files", needs.len() - 4));
    }
}

const LATER_DETAIL: &str = "The game will not start until you do; this page is on the port's Game files card";

fn later() -> PageOption<GfAct> {
    option("Add it later", LATER_DETAIL, GfAct::Later)
}

/// Found everything: "Found your copy: <file> in SD card > Emulation > roms. Use it?" `files` are (file name, place).
pub fn found_page(name: &str, files: &[(String, String)], checked_by_name: bool) -> Page<GfAct> {
    let mut p = base(name, "Found your copy");
    match files {
        [(file, place)] => p.paragraphs.push(format!("Found your copy: {file} in {place}. Use it?")),
        many => {
            p.paragraphs.push(format!("Found the {} files {name} needs. Use them?", many.len()));
            for (file, place) in many.iter().take(6) {
                p.fact(Mark::Done, format!("{file} in {place}"));
            }
        }
    }
    if checked_by_name {
        p.paragraphs.push("It was matched by its name only; if the game does not start, choose the file yourself.".into());
    }
    p.paragraphs.push("Your game files are only read, never moved or deleted.".into());
    let mut use_it = option(if files.len() == 1 { "Use it" } else { "Use them" }, "Connects it to the port (a link, or a copy if you say yes)", GfAct::UseIt);
    use_it.recommended = true;
    p.options = vec![use_it, option("Choose the file myself", "Pick another file from a list", GfAct::ChooseFile), later()];
    p
}

/// Every file is connected already.
pub fn connected_page(name: &str, files: &[(String, String)]) -> Page<GfAct> {
    let mut p = base(name, "Game files are connected");
    for (file, place) in files.iter().take(6) {
        p.fact(Mark::Done, format!("{file} in {place}"));
    }
    if files.is_empty() {
        p.fact(Mark::Done, format!("{name} has what it needs"));
    }
    p.options = vec![option("Look again", "Search the game files folder again", GfAct::LookAgain), option("Choose the file myself", "Connect another file instead", GfAct::ChooseFile)];
    p.buttons[LEFT] = Some(PageButton { label: "Done".into(), action: GfAct::Done });
    p.back_hint = "Done".into();
    p.initial = Focus::Button(LEFT);
    p
}

/// Nothing in the folder: "Your game files folder (SD card > Emulation > roms) has no copy of The Wind Waker HD."
pub fn not_found_page(name: &str, needs: &[Need], folder: &str, notes: &[String], problems: usize) -> Page<GfAct> {
    let mut p = base(name, "Your copy was not found");
    let what = match needs {
        [one] if one.title_only => format!("copy of {}", if one.name.trim().is_empty() { name } else { one.name.trim() }),
        [one] => one.name.clone(),
        _ => "the files it needs".to_string(),
    };
    p.paragraphs.push(format!("Your game files folder ({folder}) has no {what}."));
    p.paragraphs.push(format!("{name} needs a file from your own copy of the game:"));
    needs_facts(&mut p, needs, name, Mark::ToDo);
    for n in notes.iter().take(3) {
        p.fact(Mark::None, n.clone());
    }
    if problems > 0 {
        p.fact(Mark::Problem, format!("{problems} folders or files could not be read and were skipped"));
    }
    p.options = vec![
        option("Choose the file myself", "Pick it from a list of folders and files", GfAct::ChooseFile),
        option("Look again", "After you put the file in that folder", GfAct::LookAgain),
        option("Look in another folder", "Choose where you keep your game files", GfAct::ChooseFolder),
        later(),
    ];
    p
}

/// No game files folder chosen yet.
pub fn no_folder_page(name: &str, needs: &[Need]) -> Page<GfAct> {
    let mut p = base(name, "Where are your game files?");
    p.paragraphs.push(format!("{name} needs a file from your own copy of the game, and no game files folder is chosen yet."));
    needs_facts(&mut p, needs, name, Mark::ToDo);
    let mut choose = option("Choose the folder", "Where you keep your game files (on EmuDeck: SD card > Emulation > roms)", GfAct::ChooseFolder);
    choose.recommended = true;
    p.options = vec![choose, option("Choose the file myself", "Pick the file from a list", GfAct::ChooseFile), later()];
    p
}

/// The folder could not be read, or reads empty although it had files: the card or drive is most likely out.
pub fn unreadable_page(name: &str, needs: &[Need], folder: &str, why: &str) -> Page<GfAct> {
    let mut p = base(name, "Your game files folder could not be read");
    p.paragraphs.push(format!("{folder}: {why}"));
    p.paragraphs.push("Is the SD card or drive in? Put it in, then choose Look again.".into());
    needs_facts(&mut p, needs, name, Mark::ToDo);
    p.options = vec![
        option("Look again", "After the card or drive is in", GfAct::LookAgain),
        option("Look in another folder", "Choose where you keep your game files", GfAct::ChooseFolder),
        option("Choose the file myself", "Pick it from a list", GfAct::ChooseFile),
        later(),
    ];
    p
}

/// Several files match one thing the port needs: one card each.
pub fn pick_page(name: &str, what: &str, files: &[(String, String)]) -> Page<GfAct> {
    let mut p = base(name, "Which one is your copy?");
    p.paragraphs.push(format!("More than one file could be {what}. Choose the one to use."));
    p.options = files.iter().take(12).enumerate().map(|(i, (file, place))| option(file, place, GfAct::Pick(i))).collect();
    p.options.push(later());
    p
}

/// The copy question: some files cannot be linked and would be copied (the text says how much, and where).
pub fn copy_page(name: &str, text: &str) -> Page<GfAct> {
    let mut p = base(name, "Copy the game files?");
    p.paragraphs = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
    let mut copy = option("Copy", "Makes the copy; the original stays where it is", GfAct::Copy);
    copy.recommended = true;
    p.options = vec![copy, option("Not now", "Nothing is copied", GfAct::Later)];
    p.back_hint = "Not now".into();
    p
}

/// A result: connected, or why not. One Done button.
pub fn result_page(name: &str, title: &str, text: &str) -> Page<GfAct> {
    let mut p = base(name, title);
    p.paragraphs = text.split("\n\n").map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    p.buttons[LEFT] = Some(PageButton { label: "Done".into(), action: GfAct::Done });
    p.back_hint = "Done".into();
    p.initial = Focus::Button(LEFT);
    p
}

/// What one folder of "Choose the file myself" holds, as the page shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Folder {
    /// The folder the human way ("SD card > Emulation > roms").
    pub place: String,
    pub can_go_up: bool,
    pub folders: Vec<String>,
    /// (file name, its size in words).
    pub files: Vec<(String, String)>,
    /// Entries left out (a folder of thousands).
    pub hidden: usize,
    pub error: Option<String>,
}

/// "Choose the file myself" as cards: the folders and files of one folder (game files first), Up one level, Cancel. A
/// file is only read once chosen; nothing is moved.
pub fn browse_page(name: &str, f: &Folder) -> Page<GfAct> {
    let mut p = base(name, "Choose your game file");
    // The folder's own name at the top right: each folder is its own page, so the cursor memory never carries a place
    // from one folder into another.
    p.step_text = f.place.rsplit(" > ").next().unwrap_or_default().to_string();
    p.paragraphs.push(match &f.error {
        Some(e) => format!("{}: {e}", f.place),
        None if f.folders.is_empty() && f.files.is_empty() => format!("{} is empty.", f.place),
        None => format!("In {}. Your game files are only read, never moved.", f.place),
    });
    if f.can_go_up {
        p.options.push(option("Up one level", "The folder this one is in", GfAct::Up));
    }
    p.options.extend(f.folders.iter().enumerate().map(|(i, d)| option(&ellipsize(d, 40), "Folder", GfAct::Folder(i))));
    p.options.extend(f.files.iter().enumerate().map(|(i, (file, size))| option(&ellipsize(file, 40), size, GfAct::File(i))));
    if f.hidden > 0 {
        p.fact(Mark::None, format!("{} more are not shown; open a folder further down", f.hidden));
    }
    let first_file = f.can_go_up as usize + f.folders.len();
    p.initial = if f.files.is_empty() { Focus::Option(0) } else { Focus::Option(first_file) };
    p.buttons[LEFT] = Some(PageButton { label: "Cancel".into(), action: GfAct::Later });
    if p.options.is_empty() {
        p.initial = Focus::Button(LEFT);
    }
    p.b_does(&GfAct::Later)
}

/// A hand-picked file that is not one the port lists: what it is, what the port wants, and "Use it anyway" for each
/// thing it could be. `uses` are those things' names (one: just "Use it anyway").
pub fn no_match_page(name: &str, file: &str, wrong_version: bool, needs: &[Need], uses: &[String]) -> Page<GfAct> {
    let mut p = base(name, "This file does not match");
    let what = if wrong_version { "has the right name, but it is another version than the one" } else { "is not a file" };
    p.paragraphs.push(format!("{file} {what} {name} lists. It needs:"));
    needs_facts(&mut p, needs, name, Mark::ToDo);
    p.paragraphs.push("Using it anyway may not work. Your file is only read either way.".into());
    p.options = uses
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let title = if uses.len() == 1 { "Use it anyway".to_string() } else { format!("Use it for: {}", ellipsize(label, 28)) };
            option(&title, "If you are sure it is your copy", GfAct::UseAnyway(i))
        })
        .collect();
    p.options.push(option("Choose another file", "Back to the list of files", GfAct::OtherFile));
    p.buttons[LEFT] = Some(PageButton { label: "Cancel".into(), action: GfAct::Later });
    p.initial = Focus::Option(p.options.len() - 1);
    p.b_does(&GfAct::Later)
}

/// The port names no game files.
pub fn none_needed_page(name: &str) -> Page<GfAct> {
    result_page(name, "No game files needed", &format!("{name} does not name any game files it needs, so there is nothing to connect."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::page::{action_at, initial_focus};

    fn need(name: &str, title_only: bool) -> Need {
        Need { name: name.into(), crc32: vec![], has_sha1: false, kind: SlotKind::Game, title_only, other_choices: 0, kind_hint: None }
    }

    #[test]
    fn the_not_found_page_says_it_plainly_and_offers_the_three_ways() {
        let p = not_found_page("The Wind Waker HD", &[need("The Legend of Zelda: The Wind Waker HD", true)], "SD card > Emulation > roms", &[], 0);
        assert_eq!(p.window_title, "Game files for The Wind Waker HD");
        assert_eq!(p.paragraphs[0], "Your game files folder (SD card > Emulation > roms) has no copy of The Legend of Zelda: The Wind Waker HD.");
        assert_eq!(p.facts, ["Your own copy of The Legend of Zelda: The Wind Waker HD"]);
        assert!(p.facts.iter().chain(p.paragraphs.iter()).all(|t| !t.contains("checksum") && !t.contains("c14f9023")));
        let acts: Vec<GfAct> = p.options.iter().map(|o| o.action).collect();
        assert_eq!(acts, [GfAct::ChooseFile, GfAct::LookAgain, GfAct::ChooseFolder, GfAct::Later]);
        assert_eq!(p.options[0].title, "Choose the file myself");
        assert_eq!(p.options[3].title, "Add it later");
        assert_eq!(action_at(&p, initial_focus(&p)), Some(GfAct::ChooseFile));
    }

    #[test]
    fn the_found_page_names_the_file_and_the_place_and_use_it_comes_first() {
        let p = found_page("Dusklight", &[("Legend of Zelda (USA).rvz".into(), "SD card > Emulation > roms > gc".into())], false);
        assert_eq!(p.paragraphs[0], "Found your copy: Legend of Zelda (USA).rvz in SD card > Emulation > roms > gc. Use it?");
        assert_eq!(action_at(&p, initial_focus(&p)), Some(GfAct::UseIt));
        assert!(p.options[0].recommended);
        let two = found_page("X", &[("a".into(), "p".into()), ("b".into(), "q".into())], true);
        assert_eq!(two.options[0].title, "Use them");
        assert_eq!(two.facts.len(), 2);
    }

    #[test]
    fn need_words_never_mention_checksums() {
        let mut n = need("oot.z64", false);
        n.crc32 = vec![0xAABBCCDD];
        assert_eq!(need_words(&n, "SoH"), "oot.z64");
        n.other_choices = 2;
        assert_eq!(need_words(&n, "SoH"), "oot.z64 (or one of 2 other files it accepts)");
        n.kind = SlotKind::Bios;
        assert!(need_words(&n, "SoH").starts_with("BIOS: "));
        assert_eq!(need_words(&need("", true), "Dusklight"), "Your own copy of Dusklight");
    }

    #[test]
    fn choose_the_file_myself_is_cards_with_the_place_the_human_way() {
        let f = Folder {
            place: "SD card > Emulation > roms".into(),
            can_go_up: true,
            folders: vec!["wiiu".into()],
            files: vec![("The Legend of Zelda - The Wind Waker HD (USA) (En,Fr,Es).wua".into(), "6.2 GB".into())],
            hidden: 3,
            error: None,
        };
        let p = browse_page("The Wind Waker HD", &f);
        assert_eq!(p.title, "Choose your game file");
        assert_eq!(p.paragraphs[0], "In SD card > Emulation > roms. Your game files are only read, never moved.");
        let acts: Vec<GfAct> = p.options.iter().map(|o| o.action).collect();
        assert_eq!(acts, [GfAct::Up, GfAct::Folder(0), GfAct::File(0)]);
        assert_eq!(action_at(&p, initial_focus(&p)), Some(GfAct::File(0)), "the cursor starts on the first file");
        assert!(p.options.iter().all(|o| o.title.chars().count() <= 40), "one line each on the 800 px page");
        assert_eq!(p.options[2].detail, "6.2 GB");
        assert_eq!(p.facts, ["3 more are not shown; open a folder further down"]);
        assert_eq!(p.back_hint, "Cancel");
        assert!(p.paragraphs.iter().chain(p.options.iter().map(|o| &o.title)).all(|t| !t.contains("/run/media")));
        let empty = browse_page("X", &Folder { place: "Internal storage > Home".into(), ..Folder::default() });
        assert_eq!(empty.paragraphs[0], "Internal storage > Home is empty.");
        assert_eq!(action_at(&empty, initial_focus(&empty)), Some(GfAct::Later));
        let broken = browse_page("X", &Folder { place: "SD card".into(), error: Some("Could not open this folder".into()), ..Folder::default() });
        assert_eq!(broken.paragraphs[0], "SD card: Could not open this folder");
    }

    #[test]
    fn a_file_that_does_not_match_says_so_plainly_and_offers_use_anyway() {
        let p = no_match_page("SoH", "mario.z64", false, &[need("oot.z64", false)], &["Ocarina of Time".into()]);
        assert_eq!(p.paragraphs[0], "mario.z64 is not a file SoH lists. It needs:");
        assert_eq!(p.facts, ["oot.z64"]);
        let acts: Vec<GfAct> = p.options.iter().map(|o| o.action).collect();
        assert_eq!(acts, [GfAct::UseAnyway(0), GfAct::OtherFile]);
        assert_eq!(p.options[0].title, "Use it anyway");
        assert_eq!(action_at(&p, initial_focus(&p)), Some(GfAct::OtherFile), "the safe choice first");
        assert!(p.paragraphs.iter().chain(p.facts.iter()).all(|t| !t.contains("CRC")));
        let two = no_match_page("SoH", "x.z64", true, &[], &["Ocarina of Time".into(), "Master Quest".into()]);
        assert_eq!(two.options[1].title, "Use it for: Master Quest");
        assert!(two.paragraphs[0].contains("another version"));
    }

    #[test]
    fn b_means_later_and_result_pages_have_one_done() {
        let p = no_folder_page("SoH", &[need("oot.z64", false)]);
        assert_eq!(p.back_hint, "Later");
        assert_eq!(p.options[0].action, GfAct::ChooseFolder);
        let r = result_page("SoH", "Game files connected", "One.\n\nTwo.");
        assert_eq!(r.paragraphs, ["One.", "Two."]);
        assert_eq!(action_at(&r, initial_focus(&r)), Some(GfAct::Done));
        assert_eq!(pick_page("SoH", "oot.z64", &[("a".into(), "here".into())]).options[0].action, GfAct::Pick(0));
    }
}

#[cfg(test)]
mod kind_hint_tests {
    use super::*;
    use crate::fork::rom_library::requirements::{parse_instructions, Alt};
    use crate::fork::rom_screens::find::kind_hint;

    #[test]
    fn a_title_only_need_says_what_kind_of_file_it_wants() {
        let alt = |exts: &[&str]| Alt { display: "Game".into(), title_only: true, ext_hint: exts.iter().map(|e| e.to_string()).collect(), ..Alt::default() };
        assert_eq!(kind_hint(&alt(&["wua", "wud"])).as_deref(), Some("a Wii U copy, .wua or .wud"));
        assert_eq!(kind_hint(&alt(&["iso"])).as_deref(), Some("a .iso file"));
        assert_eq!(kind_hint(&alt(&[])), None);
        // From the catalog's own words, as the Game files pages get it.
        let r = parse_instructions("Required files: The Legend of Dragoon .iso");
        let alt = &r.slots[0].alternatives[0];
        let n = Need { name: alt.display.clone(), crc32: vec![], has_sha1: false, kind: SlotKind::Game, title_only: true, other_choices: 0, kind_hint: kind_hint(alt) };
        assert_eq!(need_words(&n, "Severed Chains"), "Your own copy of The Legend of Dragoon: a .iso file");
    }
}
