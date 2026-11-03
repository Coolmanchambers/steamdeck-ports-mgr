//! The pages of the wizard's first question once a place is chosen: the place cannot be used, it needs to be made,
//! "Move your 3 ports there?", a game file problem before a copy, the progress of the move, and what to do when
//! some ports did not move. Settings > Ports > Ports folder shows the same pages outside the wizard ([`for_settings`]:
//! its own header and window titles, no step bar, the chosen place named the human way); `ports_folder_screen` runs
//! them. Plain words, no file paths.

use crate::fork::page::{Focus, Page, PageButton, PageOption, LEFT};
use crate::fork::port_page::paths::size_text;

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
    /// OK on a notice.
    Ok,
    Back,
}

/// The header of these pages outside the setup wizard.
pub const SETTINGS_HEADER: &str = "Ports folder";

/// A wizard page of this flow as Settings > Ports > Ports folder shows it: "Ports folder" as the header, no step bar,
/// the window title without "Setup wizard: ". `place` is the folder the user chose, the human way ("SD card > Ports"):
/// said first, so the page names where it is about.
pub fn for_settings(mut p: Page<FolderAction>, place: Option<&str>) -> Page<FolderAction> {
    p.window_title = p.window_title.strip_prefix("Setup wizard: ").unwrap_or(&p.window_title).to_string();
    p.header = SETTINGS_HEADER.to_string();
    p.step_text.clear();
    p.step_number = 0;
    p.step_count = 0;
    if let Some(place) = place.filter(|s| !s.is_empty()) {
        p.paragraphs.insert(0, format!("The folder you chose: {place}."));
    }
    p
}

/// Something to say, with OK (B does the same): a reason the move cannot start, that it was cancelled, a folder that
/// is already the ports folder. `text` is paragraphs split at blank lines.
pub fn notice_page(window: &str, title: &str, text: &str) -> Page<FolderAction> {
    let mut p = base(window, title);
    p.paragraphs = text.split("\n\n").map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect();
    p.options = vec![option("OK", "", FolderAction::Ok, true)];
    p.initial = Focus::Option(0);
    p.b_does(&FolderAction::Ok)
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
        p.paragraphs = vec![format!("Your {} need {}, and that place has {} free.", ports(f.ports), size_text(f.needed), size_text(f.free.unwrap_or(0)))];
        p.options = vec![option("Pick another place", "", FolderAction::PickAnother, true)];
        // A copy kept earlier sits where a port would go: using the place without moving would use that old copy.
        if f.stale {
            p.paragraphs.push("A copy you kept earlier is already there, so that place can only be used once your ports are moved into it.".to_string());
        } else {
            p.options.push(option("Use it without moving my ports", "They stay where they are and show as not installed", FolderAction::LeaveThem, false));
        }
        back_button(&mut p);
        p.initial = Focus::Option(0);
        return p;
    }
    if f.stale {
        let mut p = base("Ports folder: old copy in the way", "An old copy is in the way");
        p.paragraphs = vec!["A copy you kept earlier is already there. It is replaced by the one in use, and its saves are backed up first.".to_string()];
        p.options = vec![option("Replace and move", "", FolderAction::ReplaceAndMove, false), option("Pick another place", "", FolderAction::PickAnother, true)];
        back_button(&mut p);
        p.initial = Focus::Option(1);
        return p;
    }
    let mut p = base("Ports folder: move your ports", &format!("Move your {} there?", ports(f.ports)));
    p.paragraphs = vec!["Each one is copied and checked first. The old copies are kept until you choose to remove them.".to_string()];
    p.facts = vec![match f.free {
        Some(free) => format!("About {}. {} free there.", size_text(f.bytes), size_text(free)),
        None => format!("About {}.", size_text(f.bytes)),
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

/// The same question for a port whose kept old copy already sits in the new place: it cannot be skipped (the place
/// would then use that old copy), so the choices are moving it anyway or another place.
pub fn game_file_page_kept_copy(port: &str, problem: &str, more: usize) -> Page<FolderAction> {
    let mut p = game_file_page(port, problem, more);
    p.paragraphs.push("A copy you kept earlier is already in the new place, so this port cannot stay behind.".to_string());
    p.options = vec![option("Pick another place", "", FolderAction::PickAnother, true), option("Move anyway", "Copy it without that file", FolderAction::MoveAnyway, false)];
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
        "The old copy you kept is replaced first (its saves are backed up). Then each port is copied and checked.".to_string()
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
    p.b_means("Cancel")
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
        assert_eq!(p.facts, ["About 34 MB. 900 GB free there."]);
        assert!(all_text(&p).contains("copied and checked first") && all_text(&p).contains("kept until you choose to remove them"));
        assert_eq!(move_page(&facts(1, None)).title, "Move your 1 port there?");
        assert_eq!(move_page(&facts(1, None)).facts, ["About 34 MB."]);
        assert_eq!(p.window_title, "Setup wizard: Ports folder: move your ports");
        assert_eq!((p.step_text.as_str(), p.step_count), ("Step 1 of 5", 5));
    }

    #[test]
    fn not_enough_room_says_how_much_and_offers_another_place_or_leaving_the_ports() {
        let p = move_page(&facts(2, Some(10_000_000)));
        assert_eq!(p.title, "Not enough room there");
        assert!(p.paragraphs[0].contains("2 ports need") && p.paragraphs[0].contains("10 MB free"), "{:?}", p.paragraphs);
        assert_eq!(actions(&p), [FolderAction::PickAnother, FolderAction::LeaveThem]);
        assert!(!actions(&p).contains(&FolderAction::MoveThem), "a move that cannot fit is not offered");
    }

    #[test]
    fn a_kept_copy_in_the_way_is_explained_and_replacing_is_never_the_default() {
        let mut f = facts(1, Some(900_000_000_000));
        f.stale = true;
        let p = move_page(&f);
        assert!(all_text(&p).contains("saves are backed up first"));
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
        assert!(replacing.contains("replaced first") && replacing.contains("saves are backed up") && replacing.contains("The copy in progress is removed"));
    }

    #[test]
    fn a_kept_copy_in_the_way_never_offers_using_the_place_without_moving() {
        // Not enough room and a kept old copy at the new place: using it without moving would use that old copy.
        let mut f = facts(2, Some(10_000_000));
        f.stale = true;
        let p = move_page(&f);
        assert_eq!(p.title, "Not enough room there");
        assert_eq!(actions(&p), [FolderAction::PickAnother]);
        assert!(all_text(&p).contains("A copy you kept earlier is already there"));
        f.free = Some(900_000_000_000);
        assert!(!actions(&move_page(&f)).contains(&FolderAction::LeaveThem));
        f.free = None;
        assert!(!actions(&move_page(&f)).contains(&FolderAction::LeaveThem), "unknown room too");
        // Its game file question cannot skip it either.
        let q = game_file_page_kept_copy("Dusk", "x.", 0);
        assert_eq!(actions(&q), [FolderAction::PickAnother, FolderAction::MoveAnyway]);
        assert!(!actions(&q).contains(&FolderAction::SkipPort));
        // Without a kept copy the old choices stay.
        let mut ok = facts(2, Some(10_000_000));
        ok.stale = false;
        assert_eq!(actions(&move_page(&ok)), [FolderAction::PickAnother, FolderAction::LeaveThem]);
    }

    fn every_page() -> Vec<Page<FolderAction>> {
        let long = "The Legend of Zelda: The Wind Waker HD (ZeldaWWHDRecomp)".to_string();
        let mut stale = facts(12, Some(900_000_000_000));
        stale.stale = true;
        vec![
            cannot_use_page(&["That is a system folder. Pick or create a folder of your own, like Games/Ports in your home folder or a folder on the SD card.".to_string()]),
            create_page(),
            move_page(&facts(12, Some(900_000_000_000))),
            move_page(&facts(12, Some(1))),
            move_page(&stale),
            game_file_page(&long, "oot.z64: the original game file is missing.", 3),
            already_there_page(3),
            nothing_moved_page(2),
            moving_page(&[long.clone(), "Dusk".into()], "Copying 1 of 2: x\n1.0 MB of 2.0 MB", false, true),
            moving_page(&[long.clone()], "", true, false),
            some_failed_page(&[(long.clone(), "the copy could not be checked".into())], 2),
            notice_page("Ports folder: notice", "Move cancelled", "Nothing changed.\n\nThe copies made so far were removed."),
        ]
    }

    #[test]
    fn every_page_fits_and_says_it_plainly_in_the_wizard_and_in_settings() {
        let place = "SD card (SD512) > Emulation > Ports for the Deck > A long folder name";
        for page in every_page() {
            for p in [page.clone(), for_settings(page, Some(place))] {
                crate::fork::rom_screens::settings_pages::tests::assert_fits(&p);
                crate::fork::rom_screens::settings_pages::tests::assert_plain(&p, &["ZeldaWWHDRecomp"]);
                for t in crate::fork::rom_screens::settings_pages::tests::shown(&p) {
                    let lower = t.to_lowercase();
                    for word in ["zip", "/run/media", "/home/", "repoint", "staging", "(s)", "json"] {
                        assert!(!lower.contains(word), "{word:?} in {t:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn outside_the_wizard_the_pages_have_their_own_header_and_name_the_place() {
        let p = for_settings(move_page(&facts(3, Some(900_000_000_000))), Some("SD card > Ports"));
        assert_eq!(p.window_title, "Ports folder: move your ports");
        assert_eq!((p.header.as_str(), p.step_text.as_str(), p.step_count, p.step_number), ("Ports folder", "", 0, 0));
        assert_eq!(p.paragraphs[0], "The folder you chose: SD card > Ports.");
        assert_eq!(p.title, "Move your 3 ports there?");
        assert_eq!(for_settings(create_page(), None).paragraphs.len(), 1, "no place, no extra line");
        let n = notice_page("Ports folder: notice", "Move cancelled", "a\n\nb");
        assert_eq!((n.paragraphs.len(), n.back_hint.as_str()), (2, "OK"));
        assert_eq!(n.options.iter().map(|o| o.action).collect::<Vec<_>>(), [FolderAction::Ok]);
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
