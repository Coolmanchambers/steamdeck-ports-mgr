use super::pages::{self, Act};
use super::*;
use crate::fork::page::{action_at, initial_focus, Mark};
use crate::fork::port_mover::old_copies::StartedBy;
use std::collections::HashSet;
use std::path::PathBuf;

fn named(key: &str) -> Named {
    Named::new(key, &key.to_uppercase())
}

fn finding(port: &str, state: IconState) -> IconFinding {
    IconFinding {
        port_key: port.into(),
        port_name: port.to_uppercase(),
        port_folder: port.into(),
        account_id: 1,
        account_label: "Tim".into(),
        shortcuts_file: PathBuf::from("/s/shortcuts.vdf"),
        shortcut_name: format!("{port} shortcut"),
        app_id: Some(7),
        stored: String::new(),
        state,
        at_old_copy: false,
    }
}

fn ok_icon(port: &str) -> IconFinding {
    finding(port, IconState::Ok(format!("/lib/{port}/i.png")))
}

fn queued(port: &str, kind: QueuedKind) -> QueuedFact {
    QueuedFact { port_key: port.into(), name: port.to_uppercase(), kind }
}

fn problem_of(o: Outcome) -> Problem {
    match o {
        Outcome::Problem(p) => p,
        other => panic!("expected a problem, got {other:?}"),
    }
}

/// A setup with nothing wrong.
fn fine() -> Facts {
    Facts {
        installed: vec![named("soh"), named("dusk")],
        shortcuts: Ok(vec![ok_icon("soh"), ok_icon("dusk")]),
        backup_days: Some(2),
        ..Facts::default()
    }
}

#[test]
fn a_setup_with_nothing_wrong_is_fine_on_every_line() {
    let r = run(&fine());
    assert_eq!(r.items.len(), 10);
    assert!(r.items.iter().all(|(_, o)| matches!(o, Outcome::Ok(_)) && o.mark() == Mark::Done), "{:?}", r.items);
    assert_eq!(r.problem_count(), 0);
    assert_eq!(r.summary(), "Everything looks fine");
    assert!(r.home_tile().is_none(), "no tile on Home when there is nothing to fix");
    assert!(!r.any_red());
}

#[test]
fn the_checks_come_in_the_order_of_the_design() {
    let names: Vec<&str> = Check::ALL.iter().map(|c| c.title()).collect();
    assert_eq!(
        names,
        ["Ports folder", "Steam changes", "Steam icons", "Steam shortcuts", "Program to start", "Game files", "Saves", "Old copies", "Updates", "Save backups"]
    );
    let r = run(&fine());
    assert_eq!(r.items.iter().map(|(c, _)| *c).collect::<Vec<_>>(), Check::ALL);
}

#[test]
fn a_missing_ports_folder_is_red_and_the_checks_that_need_it_say_they_could_not_look() {
    let mut f = fine();
    f.folder = Folder::DriveOut;
    f.saves = vec![named("soh")];
    f.files = vec![(named("soh"), FilesState::Missing)];
    let r = run(&f);
    let p = problem_of(r.outcome(Check::PortsFolder).unwrap().clone());
    assert_eq!(p.mark, Mark::Problem, "no game starts without its folder");
    assert!(p.summary.contains("SD card"));
    assert_eq!(p.fix, Fix::PortsFolderGone);
    assert_eq!(p.ports, vec!["soh", "dusk"]);
    for c in [Check::SteamIcons, Check::Programs, Check::GameFiles, Check::Saves, Check::OldCopies] {
        assert_eq!(r.outcome(c), Some(&Outcome::Unknown("the ports folder is not there".into())), "{c:?} must not guess about a folder that is not there");
    }
    assert!(matches!(r.outcome(Check::Updates), Some(Outcome::Ok(_))), "updates do not need the folder");
    assert!(r.any_red());
    assert_eq!(r.summary(), "1 thing to fix");
    assert_eq!(r.home_tile().unwrap().mark, Mark::Problem);
    let line = pages::line(Check::GameFiles, r.outcome(Check::GameFiles).unwrap());
    assert_eq!(line, "Game files: could not check (the ports folder is not there)");
}

#[test]
fn a_gone_folder_a_file_in_its_place_and_a_cut_short_move_each_say_so() {
    let mut f = fine();
    f.folder = Folder::Gone;
    assert!(problem_of(check(Check::PortsFolder, &f)).summary.contains("gone"));
    f.folder = Folder::NotAFolder;
    assert!(problem_of(check(Check::PortsFolder, &f)).summary.contains("not a folder"));
    f.folder = Folder::Here;
    f.unfinished_moves = vec!["Dusklight".into(), "SoH".into()];
    let p = problem_of(check(Check::PortsFolder, &f));
    assert_eq!((p.mark, p.fix.clone()), (Mark::ToDo, Fix::PortsFolderMenu));
    assert_eq!(p.summary, "A move did not finish: Dusklight and SoH");
    assert_eq!(Folder::from_missing(None), Folder::Here);
    assert_eq!(Folder::from_missing(Some(&crate::fork::ports_folder::Missing::DriveNotMounted("/run/media".into()))), Folder::DriveOut);
}

#[test]
fn queued_steam_changes_wait_and_game_mode_says_how_to_finish() {
    let mut f = fine();
    f.queue = Ok(vec![queued("soh", QueuedKind::Repoint), queued("soh", QueuedKind::Icon), queued("dusk", QueuedKind::Add)]);
    let p = problem_of(check(Check::SteamWaiting, &f));
    assert_eq!(p.mark, Mark::Waiting);
    assert_eq!(p.summary, "3 changes waiting for Steam to close: SOH and DUSK");
    assert_eq!((p.fix.clone(), p.fix_label.as_str()), (Fix::CloseSteam, "Close Steam and update shortcuts"));
    assert_eq!(p.ports, vec!["soh", "dusk"], "each port once");
    f.game_mode = true;
    assert_eq!(problem_of(check(Check::SteamWaiting, &f)).fix_label, "See how to finish this");
    f.queue = Ok(vec![queued("soh", QueuedKind::Add)]);
    assert_eq!(problem_of(check(Check::SteamWaiting, &f)).summary, "1 change waiting for Steam to close: SOH");
    f.queue = Err("The waiting list could not be read.".into());
    assert_eq!(check(Check::SteamWaiting, &f), Outcome::Unknown("The waiting list could not be read.".into()));
}

#[test]
fn a_blank_icon_names_the_port_and_the_reason_and_keeps_the_path_for_the_step_page() {
    let mut f = fine();
    f.shortcuts = Ok(vec![finding("soh", IconState::Missing("/run/media/x/tp.png".into())), ok_icon("dusk")]);
    let p = problem_of(check(Check::SteamIcons, &f));
    assert_eq!(p.summary, "Icon file is missing for SOH");
    assert!(!p.summary.contains('/'), "the line has no path");
    assert!(p.detail[0].contains("/run/media/x/tp.png"), "the real value is on the step page: {:?}", p.detail);
    assert_eq!((p.mark, p.fix.clone()), (Mark::ToDo, Fix::SteamIcons), "a blank icon does not stop a game");
    f.shortcuts = Ok(vec![
        finding("soh", IconState::Empty),
        finding("dusk", IconState::Quoted { path: "/x.png".into(), exists: true }),
        finding("abuse", IconState::NotAnImage("/y.png".into())),
        finding("quake", IconState::Outside("/home/q.png".into())),
    ]);
    let p = problem_of(check(Check::SteamIcons, &f));
    assert_eq!(p.summary, "3 icons show nothing for SOH, DUSK and ABUSE", "an icon outside the folder is left alone");
    assert_eq!(p.ports, vec!["soh", "dusk", "abuse"]);
}

#[test]
fn an_icon_repair_that_is_already_queued_is_counted_once_under_steam_changes() {
    let mut f = fine();
    f.shortcuts = Ok(vec![finding("soh", IconState::Empty), ok_icon("dusk")]);
    f.queue = Ok(vec![queued("soh", QueuedKind::Icon)]);
    assert!(matches!(check(Check::SteamIcons, &f), Outcome::Queued { .. }), "made, waiting for Steam: not fine, not a second problem");
    assert!(check(Check::SteamWaiting, &f).is_problem());
    f.queue = Ok(vec![queued("soh", QueuedKind::Repoint)]);
    assert!(check(Check::SteamIcons, &f).is_problem(), "a different kind of change does not repair the icon");
}

#[test]
fn steam_that_cannot_be_read_or_found_is_could_not_check_never_fine() {
    let mut f = fine();
    f.shortcuts = Err("Could not read the Steam shortcuts of Tim: bad file".into());
    for c in [Check::SteamIcons, Check::SteamShortcuts] {
        assert_eq!(check(c, &f), Outcome::Unknown("Could not read the Steam shortcuts of Tim: bad file".into()));
    }
    let mut f = fine();
    f.steam_found = false;
    for c in [Check::SteamIcons, Check::SteamShortcuts] {
        assert_eq!(check(c, &f), Outcome::Unknown("Steam's folder was not found".into()));
    }
    assert_eq!(run(&f).summary(), "Looks fine (2 could not be checked)");
}

#[test]
fn an_installed_port_with_no_shortcut_is_flagged_once_other_ports_are_in_steam() {
    let mut f = fine();
    f.installed.push(named("abuse"));
    f.shortcuts = Ok(vec![ok_icon("soh")]);
    let p = problem_of(check(Check::SteamShortcuts, &f));
    assert_eq!(p.summary, "Not in Steam: DUSK and ABUSE");
    assert_eq!((p.fix.clone(), p.mark), (Fix::AddToSteam, Mark::ToDo));
    f.queue = Ok(vec![queued("dusk", QueuedKind::Add)]);
    assert_eq!(problem_of(check(Check::SteamShortcuts, &f)).summary, "Not in Steam: ABUSE", "an Add that is waiting is not missing");
    f.shortcuts = Ok(Vec::new());
    assert_eq!(check(Check::SteamShortcuts, &f), Outcome::Ok("No ports in Steam yet".into()), "nobody asked for Steam: nothing to nag about");
    f.shortcuts = Ok(vec![ok_icon("soh"), ok_icon("dusk"), ok_icon("abuse")]);
    f.queue = Ok(Vec::new());
    assert_eq!(check(Check::SteamShortcuts, &f), Outcome::Ok("Every port is in Steam".into()));
}

#[test]
fn programs_missing_files_and_saves_name_their_ports_and_fixes() {
    let mut f = fine();
    f.programs = vec![named("severed"), named("soh")];
    let p = problem_of(check(Check::Programs, &f));
    assert_eq!(p.fix, Fix::PickProgram("severed".into()));
    assert_eq!(p.summary, "Not chosen for SEVERED and SOH");

    f.files = vec![(named("dusk"), FilesState::Unverifiable)];
    let p = problem_of(check(Check::GameFiles, &f));
    assert_eq!((p.mark, p.summary.as_str()), (Mark::ToDo, "Not connected yet: DUSK"), "a title only is not red");
    f.files = vec![(named("dusk"), FilesState::Unverifiable), (named("soh"), FilesState::Missing), (named("abuse"), FilesState::Missing), (named("q"), FilesState::Missing)];
    let p = problem_of(check(Check::GameFiles, &f));
    assert_eq!(p.mark, Mark::Problem, "a named file that is not there: the game will not start");
    assert_eq!(p.summary, "Missing for SOH, ABUSE and Q; not connected yet: DUSK");
    assert_eq!(p.ports.len(), 4);
    assert!(run(&f).any_red());
    assert_eq!(run(&f).home_tile().unwrap().mark, Mark::Problem);

    f.saves = vec![named("soh"), named("dusk"), named("a"), named("b"), named("c")];
    let p = problem_of(check(Check::Saves, &f));
    assert_eq!(p.summary, "Not found for SOH, DUSK and 3 more");
    assert_eq!(p.fix, Fix::FindSaves("soh".into()));
}

#[test]
fn old_copies_say_what_each_needs_and_wait_unless_one_can_be_acted_on() {
    let mut f = fine();
    f.old_copies = Ok(vec![CopyFact { name: "Dusklight".into(), state: CopyState::StartOnce }]);
    let p = problem_of(check(Check::OldCopies, &f));
    assert_eq!(p.summary, "Start Dusklight once, from here or from Steam");
    assert_eq!(p.mark, Mark::Waiting, "nothing to do yet");
    f.old_copies = Ok(vec![
        CopyFact { name: "A".into(), state: CopyState::Ready },
        CopyFact { name: "B".into(), state: CopyState::StartOnce },
        CopyFact { name: "C".into(), state: CopyState::WaitingForSteam },
        CopyFact { name: "D".into(), state: CopyState::Gone },
    ]);
    let p = problem_of(check(Check::OldCopies, &f));
    assert_eq!(p.summary, "4 old copies kept: 1 ready to remove, 1 to start once, 1 waiting for Steam, 1 already gone");
    assert_eq!((p.mark, p.fix.clone()), (Mark::ToDo, Fix::OldCopies));
    assert_eq!(p.detail.len(), 4);
    assert_eq!(p.detail[0], "The old copy of A can be removed");
    f.old_copies = Err("The list of old copies could not be read.".into());
    assert!(matches!(check(Check::OldCopies, &f), Outcome::Unknown(_)));
    assert_eq!(CopyState::of(Readiness::Ready(StartedBy::User)), CopyState::Ready);
    assert_eq!(CopyState::of(Readiness::StartOnce), CopyState::StartOnce);
    assert_eq!(CopyState::of(Readiness::WaitingForSteam), CopyState::WaitingForSteam);
}

#[test]
fn updates_and_backups() {
    let mut f = fine();
    f.updates = vec![named("soh"), named("dusk")];
    let p = problem_of(check(Check::Updates, &f));
    assert_eq!((p.summary.as_str(), p.fix.clone(), p.fix_label.as_str()), ("Update ready: SOH and DUSK", Fix::Update("soh".into()), "Update SOH"));

    f.backup_days = None;
    f.saves_known = 1;
    let p = problem_of(check(Check::Backups, &f));
    assert_eq!((p.summary.as_str(), p.fix.clone()), ("No save backup yet", Fix::BackUpNow));
    f.backup_days = Some(BACKUP_STALE_DAYS);
    assert_eq!(check(Check::Backups, &f), Outcome::Ok("Backed up 14 days ago".into()), "the limit itself is still fine");
    f.backup_days = Some(BACKUP_STALE_DAYS + 1);
    assert_eq!(problem_of(check(Check::Backups, &f)).summary, "Last save backup was 15 days ago");
    f.cloud_problem = true;
    assert_eq!(problem_of(check(Check::Backups, &f)).summary, "Last save backup was 15 days ago; cloud backup has a problem");
    f.backup_days = Some(0);
    let p = problem_of(check(Check::Backups, &f));
    assert_eq!((p.fix.clone(), p.summary.as_str()), (Fix::Cloud, "Cloud backup has a problem"));
    f.cloud_problem = false;
    assert_eq!(check(Check::Backups, &f), Outcome::Ok("Backed up today".into()));
    f.installed.clear();
    f.backup_days = None;
    assert_eq!(check(Check::Backups, &f), Outcome::Ok("No ports yet".into()), "nothing to back up");
}

#[test]
fn the_summary_and_the_home_tile_count_the_checks_with_something_to_fix() {
    let mut f = fine();
    f.updates = vec![named("soh")];
    let r = run(&f);
    assert_eq!(r.summary(), "1 thing to fix");
    let tile = r.home_tile().unwrap();
    assert_eq!((tile.title.as_str(), tile.note.as_str(), tile.mark), ("Fix 1 problem", "Updates", Mark::ToDo));
    f.saves = vec![named("soh")];
    f.queue = Ok(vec![queued("soh", QueuedKind::Add)]);
    let r = run(&f);
    assert_eq!(r.summary(), "2 to do now, 1 reminder");
    let tile = r.home_tile().unwrap();
    assert_eq!((tile.title.as_str(), tile.note.as_str()), ("Fix 2 problems", "Steam changes"));
    assert_eq!(r.problem_count(), 3, "saves not found shows in the checkup but is not worth a tile by itself");
    assert_eq!(tile.mark, Mark::ToDo, "red only when a game will not start");
    assert_eq!(r.problems().iter().map(|(c, _)| *c).collect::<Vec<_>>(), [Check::SteamWaiting, Check::Saves, Check::Updates], "problems keep the design's order");
}

#[test]
fn the_overview_line_reads_on_after_the_colon_without_breaking_names_and_numbers() {
    let words = |w: &str| pages::line(Check::SteamIcons, &Outcome::Ok(w.into()));
    assert_eq!(words("Icon file is missing for SOH"), "Steam icons: icon file is missing for SOH");
    assert_eq!(words("3 icons show nothing for A"), "Steam icons: 3 icons show nothing for A");
    assert_eq!(words("SoH is fine"), "Steam icons: SoH is fine", "not a word of ours: left alone");
    assert_eq!(words("All fine"), "Steam icons: all fine");
    assert_eq!(words("Steam still starts the old copy of A"), "Steam icons: Steam still starts the old copy of A", "Steam is a name");
    assert_eq!(words("A"), "Steam icons: A");
    assert_eq!(pages::line(Check::Updates, &Outcome::Unknown("x".into())), "Updates: could not check (x)");
}

#[test]
fn names_read_like_a_sentence() {
    let v = |l: &[&str]| names(&l.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    assert_eq!(v(&[]), "");
    assert_eq!(v(&["A"]), "A");
    assert_eq!(v(&["A", "B"]), "A and B");
    assert_eq!(v(&["A", "B", "C"]), "A, B and C");
    assert_eq!(v(&["A", "B", "C", "D", "E"]), "A, B and 3 more");
}

#[test]
fn the_overview_has_one_line_per_check_and_leads_with_fixing_when_there_is_something() {
    let mut f = fine();
    f.updates = vec![named("soh")];
    f.shortcuts = Err("Could not read Steam".into());
    let r = run(&f);
    let page = pages::overview_page(&r);
    assert_eq!(page.facts.len(), 4, "a line for each check that needs a look, then one for the rest");
    assert_eq!(page.title, "Checkup");
    let at = |text: &str| page.facts.iter().position(|l| l == text).unwrap_or_else(|| panic!("{text}: {:?}", page.facts));
    let updates = at("Updates: Update ready: SOH");
    assert_eq!(page.mark_of(updates), Mark::ToDo);
    let icons = at("Steam icons: could not check (Could not read Steam)");
    assert_eq!(page.mark_of(icons), Mark::Skipped, "a check that cannot run says why on its own line");
    assert_eq!(page.facts[3], "7 other checks are fine");
    assert_eq!(page.paragraphs, vec!["I found 1 thing to fix."]);
    assert_eq!(page.options.len(), 1);
    assert_eq!((page.options[0].title.as_str(), page.options[0].recommended, page.options[0].action), ("Fix it", true, Act::FixAll));
    assert_eq!(page.buttons[crate::fork::page::RIGHT].as_ref().map(|b| (b.label.as_str(), b.action)), Some(("Not now", Act::NotNow)), "Not now is in the footer, in reach without scrolling");
    assert_eq!(action_at(&page, initial_focus(&page)), Some(Act::FixAll));
    assert_eq!(page.back_hint, "Not now");

    f.saves = vec![named("soh")];
    let page = pages::overview_page(&run(&f));
    assert_eq!(page.options[0].title, "Fix them one by one");
    assert_eq!(page.paragraphs, vec!["I found 2 things to fix: 1 to do now, 1 reminder."], "saves not found is a reminder");
}

#[test]
fn when_everything_is_fine_the_overview_says_so_and_offers_done() {
    let page = pages::overview_page(&run(&fine()));
    assert_eq!(page.paragraphs, vec!["Everything looks fine."]);
    assert_eq!(page.options.iter().map(|o| o.action).collect::<Vec<_>>(), [Act::Done]);
    assert!(page.options[0].recommended);
    assert_eq!(page.buttons[crate::fork::page::LEFT].as_ref().map(|b| b.action), Some(Act::CheckAgain));
    assert!((0..10).all(|i| page.mark_of(i) == Mark::Done));
}

#[test]
fn a_step_page_shows_the_problem_the_fix_card_and_a_way_to_skip() {
    let mut f = fine();
    f.shortcuts = Ok(vec![finding("soh", IconState::Missing("/run/media/x/tp.png".into())), ok_icon("dusk")]);
    let p = problem_of(check(Check::SteamIcons, &f));
    let page = pages::step_page(Check::SteamIcons, &p, 2, 3);
    assert_eq!(page.title, "Steam icons");
    assert_eq!(page.step_text, "Problem 2 of 3");
    assert_eq!((page.step_number, page.step_count), (2, 3));
    assert_eq!(page.facts, vec!["Icon file is missing for SOH"]);
    assert!(page.footnote.contains("/run/media/x/tp.png"), "the real value sits in the small text: {:?}", page.footnote);
    assert!(page.paragraphs.is_empty());
    assert_eq!((page.options[0].title.as_str(), page.options[0].action, page.options[0].recommended), ("Fix the icons", Act::Fix, true));
    assert_eq!(page.options[1].action, Act::Skip);
    assert_eq!(page.back_hint, "Checkup");
    let single = pages::step_page(Check::SteamIcons, &p, 1, 1);
    assert_eq!((single.step_text.as_str(), single.step_count), ("", 0), "no counter for a single problem");
}

#[test]
fn after_a_fix_the_line_turns_green_or_offers_another_try() {
    let green = pages::after_page(Check::SteamIcons, &Outcome::Ok("All fine".into()), 1, 2, false, false);
    assert_eq!(green.title, "That is fixed");
    assert_eq!(green.mark_of(0), Mark::Done);
    assert_eq!(green.facts, vec!["Steam icons: all fine"]);
    assert_eq!(green.options.len(), 1);
    assert_eq!((green.options[0].title.as_str(), green.options[0].action), ("Next problem", Act::Next));
    let last = pages::after_page(Check::SteamIcons, &Outcome::Ok("All fine".into()), 2, 2, true, false);
    assert_eq!(last.options[0].title, "Finish");

    let mut f = fine();
    f.updates = vec![named("soh")];
    let still = pages::after_page(Check::Updates, &check(Check::Updates, &f), 1, 1, true, false);
    assert_eq!(still.title, "Not fixed yet");
    assert_eq!(still.mark_of(0), Mark::ToDo);
    assert_eq!(still.options.iter().map(|o| o.action).collect::<Vec<_>>(), [Act::TryAgain, Act::CheckAgain, Act::Next]);
    assert_eq!(still.options[0].detail, "Update SOH");

    let cannot = pages::after_page(Check::Saves, &Outcome::Unknown("the ports folder is not there".into()), 1, 1, true, false);
    assert_eq!(cannot.title, "Could not check again");
    assert_eq!(cannot.mark_of(0), Mark::Skipped);
    assert_eq!(cannot.options.iter().map(|o| o.action).collect::<Vec<_>>(), [Act::CheckAgain, Act::Next]);
}

#[test]
fn the_last_page_counts_what_is_left_and_the_confirm_page_asks_first() {
    let done = pages::finished_page(&[(Check::Updates, Outcome::Ok("Up to date".into()))]);
    assert_eq!(done.title, "All done");
    assert_eq!(done.mark_of(0), Mark::Done);
    let mut f = fine();
    f.updates = vec![named("soh")];
    let left = pages::finished_page(&[(Check::Updates, check(Check::Updates, &f)), (Check::Saves, Outcome::Ok("All found".into()))]);
    assert_eq!(left.title, "Done for now");
    assert!(left.paragraphs[0].starts_with("Still left: 1 to fix"), "{:?}", left.paragraphs);
    assert_eq!((left.mark_of(0), left.mark_of(1)), (Mark::ToDo, Mark::Done));

    let ask = pages::confirm_page(Check::Updates, "Update SOH to the newest version?", vec!["Your saves are kept.".into()], "Update", 1, 1);
    assert_eq!(ask.paragraphs.len(), 2);
    assert_eq!((ask.options[0].action, ask.options[1].action), (Act::Yes, Act::Next), "Not now never remembers a skip");
    assert_eq!(ask.back_hint, "Not now", "B never changes anything");
    let wait = pages::looking_page();
    assert!(wait.options.is_empty() && wait.buttons[crate::fork::page::RIGHT].is_some(), "the wait can be cancelled");
    assert!(wait.progress.is_some());
}

#[cfg(target_os = "linux")]
#[test]
fn only_a_folder_walk_of_bounded_size_decides_whether_there_is_a_program_to_choose() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(shallow_programs(dir.path()).unwrap().len(), 0);
    std::fs::write(dir.path().join("a.sh"), b"#!/bin/sh\n").unwrap();
    assert_eq!(shallow_programs(dir.path()).unwrap().len(), 1);
    std::fs::write(dir.path().join("b.sh"), b"#!/bin/sh\n").unwrap();
    assert_eq!(shallow_programs(dir.path()).unwrap().len(), 2, "two at the top: ask");
    std::fs::write(dir.path().join("unins000.sh"), b"#!/bin/sh\n").unwrap();
    assert_eq!(shallow_programs(dir.path()).unwrap().len(), 2, "an uninstaller is not a choice");
    let deep = dir.path().join("jdk").join("bin");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("java.sh"), b"#!/bin/sh\n").unwrap();
    assert_eq!(shallow_programs(dir.path()).unwrap().len(), 2, "only the shallowest folder with any counts");
    assert!(shallow_programs(&dir.path().join("missing")).is_err(), "an unreadable folder is an error, not 'none'");
    std::fs::create_dir_all(dir.path().join(".previous")).unwrap();
    std::fs::write(dir.path().join(".previous").join("old.sh"), b"#!/bin/sh\n").unwrap();
    assert_eq!(shallow_programs(dir.path()).unwrap().len(), 2, "superseded releases are never candidates");
}

// -- The review round: skips, advisory lines, queued repairs, old copies, backups ----------------------------

fn found_saves(f: &mut Facts) {
    f.saves_known = 2;
}

#[test]
fn a_skipped_problem_is_out_of_the_way_until_it_reads_differently() {
    let mut f = fine();
    f.updates = vec![named("soh")];
    let first = problem_of(check(Check::Updates, &f));
    f.skips.insert(Check::Updates.key().to_string(), first.fingerprint());
    let r = run(&f);
    assert!(matches!(r.outcome(Check::Updates), Some(Outcome::Skipped(p)) if p.summary == first.summary));
    assert_eq!((r.problem_count(), r.skipped_count()), (0, 1), "a skipped problem is not counted");
    assert!(r.home_tile().is_none(), "and does not raise the tile");
    assert_eq!(r.summary(), "Everything looks fine, 1 skipped");
    assert_eq!(r.outcome(Check::Updates).unwrap().mark(), Mark::Skipped);
    assert_eq!(pages::line(Check::Updates, r.outcome(Check::Updates).unwrap()), "Updates: skipped (Update ready: SOH)");
    assert!(r.problems().is_empty(), "the run does not visit it");
    // Another port with an update: the line changed, so it is a problem again.
    f.updates = vec![named("soh"), named("dusk")];
    let r = run(&f);
    assert!(r.outcome(Check::Updates).unwrap().is_problem());
    // Skips remembered for another check do not touch this one.
    f.skips.clear();
    f.skips.insert(Check::Saves.key().to_string(), "x".into());
    assert!(run(&f).outcome(Check::Updates).unwrap().is_problem());
    // The settings file round trip.
    let mut skips = Skips::new();
    skips.insert("saves".into(), "Not found for A|a".into());
    assert_eq!(skips_from_json(Some(&skips_to_json(&skips))), skips);
    assert_eq!(skips_from_json(None), Skips::new());
}

#[test]
fn the_overview_says_skipped_and_offers_to_bring_them_back() {
    let mut f = fine();
    f.updates = vec![named("soh")];
    f.skips.insert(Check::Updates.key().to_string(), problem_of(check(Check::Updates, &f)).fingerprint());
    let page = pages::overview_page(&run(&f));
    assert_eq!(page.paragraphs, vec!["Nothing new to fix. 1 skipped earlier."]);
    assert!(page.facts.iter().any(|l| l == "Updates: skipped (Update ready: SOH)"));
    assert_eq!(page.options.iter().map(|o| o.action).collect::<Vec<_>>(), [Act::Done, Act::Unskip]);
    f.saves = vec![named("soh")];
    let page = pages::overview_page(&run(&f));
    assert_eq!(page.options.iter().map(|o| o.action).collect::<Vec<_>>(), [Act::FixAll, Act::Unskip]);
    assert_eq!(page.paragraphs[0], "Nothing to do right now: 1 reminder. 1 skipped earlier.");
}

#[test]
fn nudges_and_things_that_cannot_be_done_now_show_in_the_checkup_but_do_not_raise_the_tile() {
    let mut f = fine();
    f.saves = vec![named("soh")];
    f.backup_days = Some(40);
    found_saves(&mut f);
    f.old_copies = Ok(vec![CopyFact { name: "A".into(), state: CopyState::StartOnce }]);
    let r = run(&f);
    assert_eq!(r.problem_count(), 3, "saves not found, an old backup, an old copy waiting for a first start");
    assert!(r.problems().iter().all(|(_, p)| p.advisory), "all three are nudges or waiting");
    assert!(r.home_tile().is_none(), "none of them raises the tile by itself");
    assert_eq!(r.summary(), "3 reminders", "but Settings' line counts them");
    assert_eq!(pages::overview_page(&r).paragraphs, vec!["Nothing to do right now: 3 reminders."]);
    // A real fix next to them: the tile counts only that one.
    f.updates = vec![named("soh")];
    let tile = run(&f).home_tile().unwrap();
    assert_eq!((tile.title.as_str(), tile.note.as_str(), tile.mark), ("Fix 1 problem", "Updates", Mark::ToDo));
    // The checkup and Settings say how the problems split, so the tile's count is explained.
    assert_eq!(run(&f).summary(), "1 to do now, 3 reminders");
    assert_eq!(pages::overview_page(&run(&f)).paragraphs, vec!["I found 4 things to fix: 1 to do now, 3 reminders."]);
    // A red problem always counts, advisory or not.
    f.updates.clear();
    f.files = vec![(named("soh"), FilesState::Missing)];
    let tile = run(&f).home_tile().unwrap();
    assert_eq!((tile.title.as_str(), tile.mark), ("Fix 1 problem", Mark::Problem));
    // An old copy that can be removed is something to do now.
    f.files.clear();
    f.old_copies = Ok(vec![CopyFact { name: "A".into(), state: CopyState::Ready }]);
    assert_eq!(run(&f).home_tile().unwrap().note, "Old copies");
    // Steam changes in Game Mode cannot be finished here: shown, no tile.
    let mut g = fine();
    g.game_mode = true;
    g.queue = Ok(vec![queued("soh", QueuedKind::Add)]);
    let r = run(&g);
    let p = problem_of(r.outcome(Check::SteamWaiting).unwrap().clone());
    assert!(p.advisory && p.desktop_only);
    assert!(r.home_tile().is_none());
    g.game_mode = false;
    assert!(run(&g).home_tile().is_some(), "in Desktop Mode the same changes are a fix to do now");
}

#[test]
fn save_backups_are_not_nagged_without_saves_to_back_up_or_with_the_cloud_doing_it() {
    let mut f = fine();
    f.backup_days = None;
    assert_eq!(check(Check::Backups, &f), Outcome::Ok("No save folders known yet".into()), "no port has a known save folder: nothing a zip could hold");
    f.saves_known = 2;
    let p = problem_of(check(Check::Backups, &f));
    assert_eq!((p.summary.as_str(), p.advisory, p.fix.clone()), ("No save backup yet", true, Fix::BackUpNow));
    f.cloud_ok = true;
    assert_eq!(check(Check::Backups, &f), Outcome::Ok("Saves are copied to the cloud".into()), "signed in with a recent copy");
    f.cloud_problem = true;
    assert!(check(Check::Backups, &f).is_problem(), "a cloud that reports a problem is not a safe place");
    f.cloud_problem = false;
    f.cloud_ok = false;
    f.backup_days = Some(BACKUP_STALE_DAYS + 1);
    assert_eq!(problem_of(check(Check::Backups, &f)).summary, "Last save backup was 15 days ago");
    f.saves_known = 0;
    assert_eq!(check(Check::Backups, &f), Outcome::Ok("Backed up 15 days ago".into()), "an old zip is not nagged about when nothing is known to back up");
}

#[test]
fn an_icon_repair_waiting_for_steam_is_queued_not_fixed_and_the_page_says_so() {
    let mut f = fine();
    f.shortcuts = Ok(vec![finding("soh", IconState::Empty), ok_icon("dusk")]);
    f.queue = Ok(vec![queued("soh", QueuedKind::Icon)]);
    let o = check(Check::SteamIcons, &f);
    let Outcome::Queued { summary, fix_label, desktop_only } = &o else { panic!("{o:?}") };
    assert_eq!((summary.as_str(), fix_label.as_str(), *desktop_only), ("Queued: finishes when Steam closes (SOH)", "Close Steam and update shortcuts", false));
    assert_eq!(o.mark(), Mark::Waiting);
    assert!(!o.is_problem() && o.is_not_fine());
    let page = pages::after_page(Check::SteamIcons, &o, 1, 1, true, false);
    assert_eq!(page.title, "Queued, not finished", "never 'That is fixed'");
    assert_eq!((page.options[0].title.as_str(), page.options[0].action, page.options[0].recommended), ("Close Steam and update shortcuts", Act::TryAgain, true));
    assert_eq!(pages::line(Check::SteamIcons, &o), "Steam icons: queued: finishes when Steam closes (SOH)");
    // The Steam changes line counts it once; the icons line is not a second problem.
    let r = run(&f);
    assert_eq!(r.problem_count(), 1);
    // Game Mode: it cannot be finished here, so moving on leads.
    f.game_mode = true;
    let Outcome::Queued { desktop_only, fix_label, .. } = check(Check::SteamIcons, &f) else { panic!() };
    assert!(desktop_only && fix_label == "See how to finish this");
    let gm = pages::after_page(Check::SteamIcons, &check(Check::SteamIcons, &f), 1, 2, false, false);
    assert_eq!((gm.options[0].action, gm.options[0].recommended), (Act::Next, true));
}

#[test]
fn after_the_desktop_mode_page_next_problem_leads_and_one_of_several_done_chains_to_the_next() {
    let mut f = fine();
    f.game_mode = true;
    f.queue = Ok(vec![queued("soh", QueuedKind::Add)]);
    let o = check(Check::SteamWaiting, &f);
    let page = pages::after_page(Check::SteamWaiting, &o, 1, 3, false, false);
    assert_eq!(page.title, "Not fixed yet");
    assert_eq!(page.options.iter().map(|o| (o.action, o.recommended)).collect::<Vec<_>>(), [(Act::Next, true), (Act::TryAgain, false), (Act::CheckAgain, false)], "Game Mode cannot finish it: no Try again on top");
    // Several ports: the next one is named on the card, and the page says one is done.
    let mut g = fine();
    g.saves = vec![named("dusk"), named("abuse")];
    let o = check(Check::Saves, &g);
    let p = problem_of(o.clone());
    assert_eq!((p.fix.clone(), p.fix_label.as_str()), (Fix::FindSaves("dusk".into()), "Find saves for DUSK"), "the first port that still lacks its saves");
    let chained = pages::after_page(Check::Saves, &o, 1, 1, true, true);
    assert_eq!(chained.title, "One done, more to go");
    assert_eq!((chained.options[0].title.as_str(), chained.options[0].detail.as_str(), chained.options[0].action), ("Fix the next one", "Find saves for DUSK", Act::TryAgain));
    let again = pages::after_page(Check::Saves, &o, 1, 1, true, false);
    assert_eq!((again.title.as_str(), again.options[0].title.as_str()), ("Not fixed yet", "Try again"));
    let mut h = fine();
    h.programs = vec![named("severed"), named("soh")];
    assert_eq!(problem_of(check(Check::Programs, &h)).fix_label, "Choose the program for SEVERED");
}

#[test]
fn the_last_page_counts_what_is_left_whatever_kind() {
    let ok = Outcome::Ok("fine".into());
    let done = pages::finished_page(&[(Check::Updates, ok.clone())]);
    assert_eq!((done.title.as_str(), done.paragraphs[0].as_str()), ("All done", "Everything is fixed."));
    let mut f = fine();
    f.updates = vec![named("soh")];
    let problem = check(Check::Updates, &f);
    let queued_o = Outcome::Queued { summary: "Queued: finishes when Steam closes (SOH)".into(), fix_label: "x".into(), desktop_only: false };
    let items = [
        (Check::SteamWaiting, Outcome::Unknown("Steam's change list cannot be read".into())),
        (Check::SteamIcons, queued_o),
        (Check::Updates, problem.clone()),
        (Check::Saves, Outcome::Skipped(problem_of(problem))),
        (Check::Backups, ok),
    ];
    let page = pages::finished_page(&items);
    assert_eq!(page.title, "Done for now");
    assert_eq!(
        page.paragraphs[0],
        "Still left: 1 to fix, 1 waiting for Steam to close, 1 check could not be checked, 1 skipped. You can come back to it from Settings, Fix problems."
    );
    assert_eq!(page.facts.len(), 5);
    let unknown_only = pages::finished_page(&[(Check::Saves, Outcome::Unknown("the ports folder is not there".into()))]);
    assert_eq!(unknown_only.title, "Done for now", "a card pulled mid-run is not 'everything is fixed'");
}

#[test]
fn a_shortcut_that_still_starts_an_old_copy_is_in_steam_but_wrong_and_never_offered_as_a_duplicate_add() {
    let mut f = fine();
    let mut old = ok_icon("dusk");
    old.at_old_copy = true;
    f.shortcuts = Ok(vec![ok_icon("soh"), old]);
    let p = problem_of(check(Check::SteamShortcuts, &f));
    assert_eq!(p.summary, "Steam still starts the old copy of DUSK");
    assert_eq!((p.fix.clone(), p.fix_label.as_str()), (Fix::Repoint, "Point Steam at the new copy"), "a repoint, never an Add that would duplicate it");
    assert_eq!(p.ports, vec!["dusk"]);
    // Another port is not in Steam at all: the line says both, the repoint comes first, and only the old copy's port is its.
    let mut both = f.clone();
    both.installed.push(named("abuse"));
    let p = problem_of(check(Check::SteamShortcuts, &both));
    assert_eq!(p.summary, "Steam still starts the old copy of DUSK; Not in Steam: ABUSE");
    assert_eq!((p.fix, p.ports), (Fix::Repoint, vec!["dusk".to_string()]));
    // Its repoint is queued: that is a Steam change, not this line's problem.
    f.queue = Ok(vec![queued("dusk", QueuedKind::Repoint)]);
    assert_eq!(check(Check::SteamShortcuts, &f), Outcome::Ok("Every port is in Steam".into()));
    // A port whose only shortcut is at its old copy is not "Not in Steam" (no Add that would duplicate it).
    let mut g = fine();
    let mut only = ok_icon("dusk");
    only.at_old_copy = true;
    g.shortcuts = Ok(vec![only]);
    g.installed = vec![named("dusk")];
    assert!(!problem_of(check(Check::SteamShortcuts, &g)).summary.contains("Not in Steam"));
    // Any queued kind for a port counts as in Steam.
    for kind in [QueuedKind::Add, QueuedKind::Repoint, QueuedKind::Icon, QueuedKind::Update] {
        let mut h = fine();
        h.installed.push(named("abuse"));
        h.queue = Ok(vec![queued("abuse", kind)]);
        assert_eq!(check(Check::SteamShortcuts, &h), Outcome::Ok("Every port is in Steam".into()), "{kind:?}");
    }
    // A Deck with no shortcuts yet and nothing queued is not nagged.
    let mut k = fine();
    k.shortcuts = Ok(Vec::new());
    assert_eq!(check(Check::SteamShortcuts, &k), Outcome::Ok("No ports in Steam yet".into()));
    // The icon of a shortcut at an old copy is repaired like any other.
    let mut bad = finding("dusk", IconState::InOldCopy("/old/i.png".into()));
    bad.at_old_copy = true;
    let mut m = fine();
    m.shortcuts = Ok(vec![ok_icon("soh"), bad]);
    assert!(check(Check::SteamIcons, &m).is_problem());
}

#[test]
fn only_the_menus_the_chosen_fix_returns_to_hand_control_back() {
    use crate::fork::settings_menu::Group;
    let ports = Menu::SettingsGroup(Group::Ports);
    assert!(Fix::GameFiles.returns_to(ports) && Fix::OldCopies.returns_to(ports) && Fix::PortsFolderMenu.returns_to(ports));
    assert!(Fix::Cloud.returns_to(Menu::SettingsGroup(Group::Saves)));
    assert!(!Fix::Cloud.returns_to(ports), "the cloud flow's own menus are not the Ports page");
    assert!(Fix::OldCopies.returns_to(Menu::PortsFolder), "the old copies list's Back row opens the ports folder menu: that is leaving");
    assert!(!Fix::GameFiles.returns_to(Menu::PortsFolder), "the Game files flow opening the ports folder menu is part of the flow");
    assert!(!Fix::PortsFolderMenu.returns_to(Menu::PortsFolder), "inside the ports folder menu its own screens open it again: not a way out");
    for fix in [Fix::CloseSteam, Fix::SteamIcons, Fix::AddToSteam, Fix::Repoint, Fix::BackUpNow, Fix::Update("a".into()), Fix::FindSaves("a".into()), Fix::PickProgram("a".into())] {
        assert!(!fix.returns_to(ports) && !fix.returns_to(Menu::PortsFolder) && !fix.returns_to(Menu::SettingsGroup(Group::Steam)), "{fix:?}");
    }
}

#[test]
fn the_overview_keeps_to_the_lines_that_need_a_look_and_keeps_check_again_in_the_footer() {
    let mut f = fine();
    f.updates = vec![named("soh")];
    f.saves = vec![named("dusk")];
    let page = pages::overview_page(&run(&f));
    assert_eq!(page.facts.len(), 3, "two lines to look at and one for all the rest");
    assert_eq!(page.facts[2], "8 other checks are fine");
    assert_eq!(page.mark_of(2), Mark::Done);
    assert_eq!(page.buttons[crate::fork::page::LEFT].as_ref().map(|b| b.action), Some(Act::CheckAgain));
    let fine_page = pages::overview_page(&run(&fine()));
    assert_eq!(fine_page.facts.len(), 10, "all fine: every line, one card, Check again below");
    assert_eq!(fine_page.options.iter().map(|o| o.action).collect::<Vec<_>>(), [Act::Done]);
    assert_eq!(fine_page.buttons[crate::fork::page::LEFT].as_ref().map(|b| b.action), Some(Act::CheckAgain));
    assert_eq!(fine_page.facts[0], "Ports folder: found");
}

#[test]
fn wording_of_the_game_files_and_the_settings_line() {
    let mut f = fine();
    f.files = vec![(named("dusk"), FilesState::Unverifiable)];
    assert_eq!(problem_of(check(Check::GameFiles, &f)).summary, "Not connected yet: DUSK");
    f.files.push((named("soh"), FilesState::Missing));
    assert_eq!(problem_of(check(Check::GameFiles, &f)).summary, "Missing for SOH; not connected yet: DUSK");
    f.files.clear();
    f.updates = vec![named("soh")];
    assert_eq!(run(&f).summary(), "1 thing to fix");
    f.queue = Ok(vec![queued("soh", QueuedKind::Add)]);
    assert_eq!(run(&f).summary(), "2 things to fix", "two fixes that work now");
    f.saves = vec![named("soh")];
    assert_eq!(run(&f).summary(), "2 to do now, 1 reminder");
    assert_eq!(Check::ALL.iter().map(|c| c.key()).collect::<HashSet<_>>().len(), 10, "every check has its own key");
}

#[test]
fn a_lone_reminder_reads_like_settings_and_a_fix_that_left_a_shortcut_alone_says_why() {
    let mut f = fine();
    f.saves = vec![named("soh")];
    let r = run(&f);
    assert_eq!(r.summary(), "1 reminder");
    assert_eq!(pages::overview_page(&r).paragraphs, vec!["Nothing to do right now: 1 reminder."]);
    f.updates = vec![named("soh")];
    assert_eq!(pages::overview_page(&run(&f)).paragraphs, vec!["I found 2 things to fix: 1 to do now, 1 reminder."]);
    f.saves.clear();
    assert_eq!(pages::overview_page(&run(&f)).paragraphs, vec!["I found 1 thing to fix."]);
    let page = pages::with_notes(pages::after_page(Check::SteamShortcuts, &Outcome::Ok("x".into()), 1, 1, true, false), &["Steam has not finished setting up My Abuse; open and quit Steam once, then try again.".to_string()]);
    assert_eq!(page.paragraphs, vec!["Steam has not finished setting up My Abuse; open and quit Steam once, then try again."]);
}

#[test]
fn an_unreadable_waiting_list_is_could_not_check() {
    let mut f = fine();
    f.queue = Err(QUEUE_UNREADABLE.into());
    assert_eq!(check(Check::SteamWaiting, &f), Outcome::Unknown("Steam's change list cannot be read".into()));
    assert_eq!(pages::line(Check::SteamWaiting, &check(Check::SteamWaiting, &f)), "Steam changes: could not check (Steam's change list cannot be read)");
}

// -- One port (the Port page's Fix problems) ------------------------------------------------------------------

/// Two ports with something wrong each, and a stale all-ports backup.
fn two_with_problems() -> Facts {
    Facts {
        installed: vec![named("soh"), named("dusk")],
        shortcuts: Ok(vec![finding("soh", IconState::Empty), ok_icon("dusk")]),
        programs: vec![named("dusk")],
        files: vec![(named("soh"), FilesState::Missing)],
        saves: vec![named("dusk")],
        updates: vec![named("soh"), named("dusk")],
        queue: Ok(vec![queued("dusk", QueuedKind::Add)]),
        old_copies: Ok(vec![CopyFact { name: "DUSK".into(), state: CopyState::Ready }]),
        backup_days: Some(90),
        saves_known: 2,
        ..Facts::default()
    }
}

#[test]
fn the_port_filter_keeps_only_that_ports_facts() {
    let f = two_with_problems().only_port("dusk");
    assert_eq!(f.installed, vec![named("dusk")]);
    assert_eq!(f.programs, vec![named("dusk")]);
    assert!(f.files.is_empty(), "soh's missing file is not dusk's");
    assert_eq!(f.saves, vec![named("dusk")]);
    assert_eq!(f.updates, vec![named("dusk")]);
    assert_eq!(f.queue.as_ref().unwrap().len(), 1);
    assert_eq!(f.shortcuts.as_ref().unwrap().iter().map(|s| s.port_key.as_str()).collect::<Vec<_>>(), ["dusk"]);
    assert_eq!(f.old_copies.as_ref().unwrap().len(), 1, "old copies match by the port's name");
    assert_eq!((f.backup_days, f.saves_known), (None, 0), "the all-ports backup is not about one port");
    let soh = two_with_problems().only_port("soh");
    assert!(soh.old_copies.as_ref().unwrap().is_empty() && soh.queue.as_ref().unwrap().is_empty());
}

#[test]
fn the_checkup_for_one_port_runs_only_the_port_checks_and_names_only_that_port() {
    let r = run_for_port(&two_with_problems(), "soh");
    assert_eq!(r.items.iter().map(|(c, _)| *c).collect::<Vec<_>>(), Check::PORT.to_vec());
    assert!(r.outcome(Check::PortsFolder).is_none() && r.outcome(Check::Backups).is_none());
    let problems: Vec<Check> = r.problems().iter().map(|(c, _)| *c).collect();
    assert_eq!(problems, [Check::SteamIcons, Check::GameFiles, Check::Updates]);
    for (_, p) in r.problems() {
        assert!(p.ports.iter().all(|k| k == "soh"), "{p:?}");
        assert!(!p.summary.contains("DUSK"), "{}", p.summary);
    }
    assert_eq!(r.summary(), "3 things to fix");
    let dusk = run_for_port(&two_with_problems(), "dusk");
    let problems: Vec<Check> = dusk.problems().iter().map(|(c, _)| *c).collect();
    assert_eq!(problems, [Check::SteamWaiting, Check::Programs, Check::Saves, Check::OldCopies, Check::Updates]);
    assert!(matches!(dusk.problems()[1].1.fix, Fix::PickProgram(ref k) if k == "dusk"));
}

#[test]
fn a_port_with_nothing_wrong_reads_everything_looks_fine() {
    let r = run_for_port(&fine(), "soh");
    assert_eq!(r.problem_count(), 0);
    assert_eq!(r.summary(), "Everything looks fine");
    let gone = Facts { folder: Folder::Gone, ..fine() };
    assert!(matches!(run_for_port(&gone, "soh").outcome(Check::GameFiles), Some(Outcome::Unknown(_))), "no ports folder: could not check");
}

#[test]
fn one_ports_checkup_speaks_of_that_port_not_every_port() {
    let all = fine();
    assert_eq!(check(Check::SteamShortcuts, &all), Outcome::Ok("Every port is in Steam".to_string()));
    let one = run_for_port(&all, "soh");
    assert_eq!(one.outcome(Check::SteamShortcuts), Some(&Outcome::Ok("In Steam".to_string())));
    assert_eq!(one.outcome(Check::GameFiles), Some(&Outcome::Ok("Its game files are there".to_string())));
    assert_eq!(one.outcome(Check::Saves), Some(&Outcome::Ok("Its saves are found".to_string())));
    let none_in_steam = Facts { shortcuts: Ok(Vec::new()), ..fine() };
    assert_eq!(run_for_port(&none_in_steam, "soh").outcome(Check::SteamShortcuts), Some(&Outcome::Ok("Not in Steam yet".to_string())));
}
