//! The pages of the wizard's first question once a place is chosen: the place cannot be used, it needs to be made,
//! "Move your 3 ports there?", a game file problem before a copy, the progress of the move, and what to do when
//! some ports did not move. They replace the lists of Settings > Ports folder inside the wizard (the Settings screen
//! keeps its lists); the decisions are the same ones (`ports_folder_screen` runs them). Plain words, no file paths.

use crate::fork::page::{Focus, Page, PageButton, PageOption, LEFT};
use crate::fork::rom_screens::util::human_bytes;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderAction {
    /// Make the folder, then check it again.
    CreateIt,
    /// Choose another place (the folder picker).
    PickAnother,
    /// Copy the ports there, check them, switch.
    MoveThem,
    /// Use the place without moving anything (the ports stay in the old folder).
    LeaveThem,
    /// A copy kept earlier is in the way: replace it, then move.
    ReplaceAndMove,
    /// Leave this port out of the move.
    SkipPort,
    /// Move it although its game files were not all found.
    MoveAnyway,
    /// Go on with the ports that did move.
    UseMoved,
    /// Use the place as it is (the ports in it stay where they are).
    UseIt,
    /// Take back what was copied (the new copies are removed; the originals were never touched).
    Undo,
    /// Stop the copy in progress.
    CancelMove,
    Back,
}

fn base(window: &str, title: &str) -> Page<FolderAction> {
    Page {
        window_title: format!("Setup wizard: {window}"),
        large: true,
        header: "Deck Ports Manager".to_string(),
        step_number: 1,
        step_count: 5,
        step_text: "Step 1 of 5".to_string(),
        title: title.to_string(),
        ..Page::default()
    }
}

fn option(title: &str, detail: &str, action: FolderAction, recommended: bool) -> PageOption<FolderAction> {
    PageOption { title: title.to_string(), detail: detail.to_string(), choice: None, enabled: true, recommended, action }
}

fn back_button(p: &mut Page<FolderAction>) {
    p.buttons[LEFT] = Some(PageButton { label: "Back".to_string(), action: FolderAction::Back });
}

fn ports(n: usize) -> String {
    if n == 1 { "1 port".to_string() } else { format!("{n} ports") }
}

/// The place failed its checks; `problems` are the reasons, in the checks' own words.
pub fn cannot_use_page(problems: &[String]) -> Page<FolderAction> {
    let mut p = base("Ports folder: cannot use it", "That place can't be used");
    p.paragraphs = problems.iter().take(2).cloned().collect();
    p.options = vec![option("Pick another place", "", FolderAction::PickAnother, true)];
    back_button(&mut p);
    p.initial = Focus::Option(0);
    p
}

/// The folder does not exist yet.
pub fn create_page() -> Page<FolderAction> {
    let mut p = base("Ports folder: new folder", "Make a new folder?");
    p.paragraphs = vec!["A new Ports folder will be made there, and your ports will be kept in it.".to_string()];
    p.options = vec![option("Make it", "", FolderAction::CreateIt, true), option("Pick another place", "", FolderAction::PickAnother, false)];
    back_button(&mut p);
    p.initial = Focus::Option(0);
    p
}

/// What the move needs to say: how many ports, how big, how much room.
pub struct MoveFacts {
    pub ports: usize,
    pub bytes: u64,
    /// What is free in the new place, when known.
    pub free: Option<u64>,
    /// Needed: the ports plus a margin.
    pub needed: u64,
    /// A copy kept earlier sits where a port is going.
    pub stale: bool,
}

/// "Move your 3 ports there?", or why that cannot be asked as it is.
pub fn move_page(f: &MoveFacts) -> Page<FolderAction> {
    let fits = f.free.is_none_or(|free| free >= f.needed);
    if !fits {
        let mut p = base("Ports folder: not enough room", "Not enough room there");
        p.paragraphs = vec![format!("Your {} need {}, and that place has {} free.", ports(f.ports), human_bytes(f.needed), human_bytes(f.free.unwrap_or(0)))];
        p.options = vec![
            option("Pick another place", "", FolderAction::PickAnother, true),
            option("Use it without moving my ports", "They stay where they are and show as not installed", FolderAction::LeaveThem, false),
        ];
        back_button(&mut p);
        p.initial = Focus::Option(0);
        return p;
    }
    if f.stale {
        let mut p = base("Ports folder: old copy in the way", "An old copy is in the way");
        p.paragraphs = vec!["A copy you kept earlier is already there. It is replaced by the one in use, and its saves are zipped to a backup first.".to_string()];
        p.options = vec![option("Replace and move", "", FolderAction::ReplaceAndMove, false), option("Pick another place", "", FolderAction::PickAnother, true)];
        back_button(&mut p);
        p.initial = Focus::Option(1);
        return p;
    }
    let mut p = base("Ports folder: move your ports", &format!("Move your {} there?", ports(f.ports)));
    p.paragraphs = vec!["Each one is copied and checked first. The old copies are kept until you choose to remove them.".to_string()];
    p.facts = vec![match f.free {
        Some(free) => format!("About {}. {} free there.", human_bytes(f.bytes), human_bytes(free)),
        None => format!("About {}.", human_bytes(f.bytes)),
    }];
    p.options = vec![
        option("Move them", "", FolderAction::MoveThem, true),
        option("Leave them", "They stay where they are and show as not installed", FolderAction::LeaveThem, false),
    ];
    back_button(&mut p);
    p.initial = Focus::Option(0);
    p
}

/// A game file the port needs was not found where the copy starts: skip the port or move it anyway.
pub fn game_file_page(port: &str, problem: &str, more: usize) -> Page<FolderAction> {
    let mut p = base("Ports folder: game files", "A game file may be missing");
    p.paragraphs = vec![format!("{port}: {problem}")];
    if more > 0 {
        p.paragraphs.push(format!("{} more {} the same kind of problem.", more, if more == 1 { "port has" } else { "ports have" }));
    }
    p.options = vec![
        option("Skip this port", "It stays where it is", FolderAction::SkipPort, true),
        option("Move anyway", "Copy it without that file", FolderAction::MoveAnyway, false),
    ];
    back_button(&mut p);
    p.initial = Focus::Option(0);
    p
}

/// Ports are in that place already: nothing is copied.
pub fn already_there_page(n: usize) -> Page<FolderAction> {
    let mut p = base("Ports folder: already there", "Your ports are already there");
    p.paragraphs = vec![format!("{} in that place already, and {} used from there. Nothing is copied.", if n == 1 { "1 port is".to_string() } else { format!("{n} ports are") }, if n == 1 { "it is" } else { "they are" })];
    p.options = vec![option("Use that place", "", FolderAction::UseIt, true), option("Pick another place", "", FolderAction::PickAnother, false)];
    back_button(&mut p);
    p.initial = Focus::Option(0);
    p
}

/// Every port was skipped for its game files: nothing moves.
pub fn nothing_moved_page(skipped: usize) -> Page<FolderAction> {
    let mut p = base("Ports folder: nothing moved", "No ports were moved");
    p.paragraphs = vec![format!("The {} you skipped {} where {} and show{} as not installed in the new place.", ports(skipped), if skipped == 1 { "stays" } else { "stay" }, if skipped == 1 { "it is" } else { "they are" }, if skipped == 1 { "s" } else { "" })];
    p.options = vec![option("Use the new place anyway", "", FolderAction::UseIt, false), option("Pick another place", "", FolderAction::PickAnother, true)];
    back_button(&mut p);
    p.initial = Focus::Option(1);
    p
}

/// The state of each port while the copy runs, from the worker's progress text
/// ("Copying 2 of 3: Name\n12 MB of 28 MB"). Ports before the current one are done, the current one shows its
/// bytes, the rest wait. A text that does not look like that (a kept copy being replaced first) leaves all waiting.
pub fn progress_lines(names: &[String], text: &str) -> Vec<String> {
    let (first, second) = text.split_once('\n').unwrap_or((text, ""));
    let current = first
        .strip_prefix("Copying ")
        .and_then(|rest| rest.split_once(" of "))
        .and_then(|(i, _)| i.trim().parse::<usize>().ok())
        .filter(|i| *i >= 1 && *i <= names.len());
    names
        .iter()
        .enumerate()
        .map(|(idx, name)| match current {
            Some(i) if idx + 1 < i => format!("{name}: done"),
            Some(i) if idx + 1 == i && !second.trim().is_empty() => format!("{name}: copying, {}", second.trim()),
            Some(i) if idx + 1 == i => format!("{name}: copying"),
            _ => format!("{name}: waiting"),
        })
        .collect()
}

/// "Moving your ports" with one line per port and a Cancel. `replaces` is a kept old copy being replaced first,
/// which is the one thing in this flow that removes anything, so the page does not claim "nothing is deleted".
pub fn moving_page(names: &[String], text: &str, cancelling: bool, replaces: bool) -> Page<FolderAction> {
    let mut p = base("Ports folder: moving", "Moving your ports");
    p.paragraphs = vec![if replaces {
        "The old copy you kept is replaced first (its saves are zipped). Then each port is copied and checked.".to_string()
    } else {
        "Each port is copied and checked. Nothing is deleted.".to_string()
    }];

    p.facts = progress_lines(names, text);
    p.options = vec![if cancelling {
        PageOption { enabled: false, ..option("Cancelling...", "", FolderAction::CancelMove, false) }
    } else {
        option("Cancel", if replaces { "The copy in progress is removed" } else { "Nothing is changed" }, FolderAction::CancelMove, false)
    }];
    p.initial = Focus::Option(0);
    p
}

/// Some ports did not move. `failed` is name and a short reason; `moved` is how many did.
pub fn some_failed_page(failed: &[(String, String)], moved: usize) -> Page<FolderAction> {
    let mut p = base("Ports folder: some ports did not move", &format!("{} did not move", ports(failed.len())));
    p.facts = failed.iter().take(3).map(|(name, why)| format!("{name}: {why}")).collect();
    if failed.len() > 3 {
        p.facts.push(format!("And {} more.", failed.len() - 3));
    }
    p.options = Vec::new();
    if moved > 0 {
        p.options.push(option(&format!("Go on with the {} that moved", ports(moved)), "The others stay where they are", FolderAction::UseMoved, true));
    }
    p.options.push(option("Undo", "The new copies are removed; the ports you use are untouched", FolderAction::Undo, moved == 0));
    p.initial = Focus::Option(0);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_text(p: &Page<FolderAction>) -> String {
        let mut parts = vec![p.title.clone()];
        parts.extend(p.paragraphs.iter().cloned());
        parts.extend(p.facts.iter().cloned());
        parts.extend(p.options.iter().flat_map(|o| [o.title.clone(), o.detail.clone()]));
        parts.join(" ")
    }

    fn actions(p: &Page<FolderAction>) -> Vec<FolderAction> {
        p.options.iter().map(|o| o.action).collect()
    }

    fn facts(n: usize, free: Option<u64>) -> MoveFacts {
        MoveFacts { ports: n, bytes: 33_500_000, free, needed: 40_000_000, stale: false }
    }

    #[test]
    fn the_move_question_names_the_ports_the_size_and_the_room_and_starts_on_move_them() {
        let p = move_page(&facts(3, Some(900_000_000_000)));
        assert_eq!(p.title, "Move your 3 ports there?");
        assert_eq!(actions(&p), [FolderAction::MoveThem, FolderAction::LeaveThem]);
        assert!(p.options[0].recommended && p.initial == Focus::Option(0));
        assert!(p.facts[0].starts_with("About 33") && p.facts[0].contains("free there"), "{:?}", p.facts);
        assert!(all_text(&p).contains("copied and checked first") && all_text(&p).contains("kept until you choose to remove them"));
        assert_eq!(move_page(&facts(1, None)).title, "Move your 1 port there?");
        assert_eq!(move_page(&facts(1, None)).facts, ["About 33.5 MB."]);
        assert_eq!(p.window_title, "Setup wizard: Ports folder: move your ports");
        assert_eq!((p.step_text.as_str(), p.step_count), ("Step 1 of 5", 5));
    }

    #[test]
    fn not_enough_room_says_how_much_and_offers_another_place_or_leaving_the_ports() {
        let p = move_page(&facts(2, Some(10_000_000)));
        assert_eq!(p.title, "Not enough room there");
        assert!(p.paragraphs[0].contains("2 ports need") && p.paragraphs[0].contains("10.0 MB free"), "{:?}", p.paragraphs);
        assert_eq!(actions(&p), [FolderAction::PickAnother, FolderAction::LeaveThem]);
        assert!(!actions(&p).contains(&FolderAction::MoveThem), "a move that cannot fit is not offered");
    }

    #[test]
    fn a_kept_copy_in_the_way_is_explained_and_replacing_is_never_the_default() {
        let mut f = facts(1, Some(900_000_000_000));
        f.stale = true;
        let p = move_page(&f);
        assert!(all_text(&p).contains("saves are zipped to a backup first"));
        assert_eq!(actions(&p), [FolderAction::ReplaceAndMove, FolderAction::PickAnother]);
        assert_eq!(p.initial, Focus::Option(1));
    }

    #[test]
    fn a_game_file_problem_skips_the_port_by_default_and_says_how_many_more() {
        let p = game_file_page("Demo Wolf", "its game file is missing.", 2);
        assert_eq!(actions(&p), [FolderAction::SkipPort, FolderAction::MoveAnyway]);
        assert_eq!(p.initial, Focus::Option(0));
        assert!(p.paragraphs[1].starts_with("2 more ports have"));
        assert!(game_file_page("A", "x", 1).paragraphs[1].starts_with("1 more port has"));
        assert_eq!(game_file_page("A", "x", 0).paragraphs.len(), 1);
    }

    #[test]
    fn the_progress_page_has_one_line_per_port_and_cancel_is_all_there_is_to_press() {
        let names: Vec<String> = ["Demo Blocks", "Demo Severed", "Demo Wolf"].iter().map(|s| s.to_string()).collect();
        let p = moving_page(&names, "Copying 2 of 3: Demo Severed\n12.0 MB of 28.0 MB", false, false);
        assert_eq!(p.facts, ["Demo Blocks: done", "Demo Severed: copying, 12.0 MB of 28.0 MB", "Demo Wolf: waiting"]);
        assert_eq!(actions(&p), [FolderAction::CancelMove]);
        assert!(p.options[0].enabled && p.options[0].title == "Cancel");
        let c = moving_page(&names, "", true, false);
        assert!(!c.options[0].enabled && c.options[0].title == "Cancelling...");
        assert!(c.facts.iter().all(|l| l.ends_with("waiting")));
        // A kept copy being replaced first leaves every port waiting.
        assert!(progress_lines(&names, "Checking and replacing the old copy of X\n/some/where").iter().all(|l| l.ends_with("waiting")));
        // Nonsense numbers never panic.
        assert_eq!(progress_lines(&names, "Copying 9 of 3: Z").len(), 3);
        assert_eq!(progress_lines(&names, "Copying 0 of 3: Z")[0], "Demo Blocks: waiting");
    }

    #[test]
    fn some_ports_not_moving_offers_going_on_with_the_rest_or_undo() {
        let failed = vec![("A".to_string(), "its game file is missing".to_string())];
        let p = some_failed_page(&failed, 2);
        assert_eq!(p.title, "1 port did not move");
        assert_eq!(actions(&p), [FolderAction::UseMoved, FolderAction::Undo]);
        assert!(p.options[0].title.contains("2 ports"));
        let none = some_failed_page(&failed, 0);
        assert_eq!(actions(&none), [FolderAction::Undo], "nothing moved, so nothing to go on with");
        assert!(none.options[0].recommended);
        let many: Vec<(String, String)> = (0..5).map(|i| (format!("P{i}"), "x".to_string())).collect();
        let p = some_failed_page(&many, 1);
        assert_eq!(p.facts.len(), 4);
        assert_eq!(p.facts[3], "And 2 more.");
    }

    #[test]
    fn the_other_pages_have_a_way_back_and_no_paths() {
        let mut pages = vec![create_page(), cannot_use_page(&["The folder is read-only.".to_string()]), game_file_page("P", "x", 0), move_page(&facts(2, None))];
        pages.push(some_failed_page(&[("P".to_string(), "x".to_string())], 1));
        for p in &pages {
            let t = all_text(p);
            assert!(!t.contains('/') && !t.contains(".json"), "{t}");
            assert!(p.large && p.header == "Deck Ports Manager");
        }
        for p in [create_page(), cannot_use_page(&[]), game_file_page("P", "x", 0), move_page(&facts(2, None))] {
            assert_eq!(p.buttons[LEFT].as_ref().map(|b| b.action), Some(FolderAction::Back));
        }
    }
    #[test]
    fn replacing_a_kept_copy_does_not_claim_that_nothing_is_deleted_or_changed() {
        let names = vec!["A".to_string()];
        let plain = all_text(&moving_page(&names, "", false, false));
        assert!(plain.contains("Nothing is deleted") && plain.contains("Nothing is changed"));
        let replacing = all_text(&moving_page(&names, "", false, true));
        assert!(!replacing.contains("Nothing is deleted") && !replacing.contains("Nothing is changed"), "{replacing}");
        assert!(replacing.contains("replaced first") && replacing.contains("saves are zipped") && replacing.contains("The copy in progress is removed"));
    }

    #[test]
    fn ports_already_there_and_ports_all_skipped_say_what_the_switch_does() {
        let p = already_there_page(2);
        assert!(p.paragraphs[0].starts_with("2 ports are in that place already, and they are used from there"));
        assert_eq!(actions(&p), [FolderAction::UseIt, FolderAction::PickAnother]);
        assert!(already_there_page(1).paragraphs[0].starts_with("1 port is in that place already, and it is used"));
        let q = nothing_moved_page(2);
        assert!(q.paragraphs[0].contains("The 2 ports you skipped stay where they are"));
        assert_eq!(q.initial, Focus::Option(1), "using the new place anyway is not the default");
        assert!(nothing_moved_page(1).paragraphs[0].contains("stays where it is"));
    }

}
