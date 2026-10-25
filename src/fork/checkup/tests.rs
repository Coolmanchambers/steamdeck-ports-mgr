use super::pages::{self, Act};
use super::*;
use crate::fork::page::{action_at, initial_focus, Mark};
use crate::fork::port_mover::old_copies::StartedBy;
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
    assert_eq!(r.summary(), "1 problem");
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
    assert_eq!(check(Check::SteamIcons, &f), Outcome::Ok("All fine".into()));
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
    assert_eq!((p.mark, p.summary.as_str()), (Mark::ToDo, "Tell me where they are: DUSK"), "a title only is not red");
    f.files = vec![(named("dusk"), FilesState::Unverifiable), (named("soh"), FilesState::Missing), (named("abuse"), FilesState::Missing), (named("q"), FilesState::Missing)];
    let p = problem_of(check(Check::GameFiles, &f));
    assert_eq!(p.mark, Mark::Problem, "a named file that is not there: the game will not start");
    assert_eq!(p.summary, "Missing for SOH, ABUSE and Q; DUSK to add");
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
    assert_eq!(p.summary, "Start Dusklight once from its new place");
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
    assert_eq!(r.summary(), "1 problem");
    let tile = r.home_tile().unwrap();
    assert_eq!((tile.title.as_str(), tile.note.as_str(), tile.mark), ("Fix 1 problem", "Updates", Mark::ToDo));
    f.saves = vec![named("soh")];
    f.queue = Ok(vec![queued("soh", QueuedKind::Add)]);
    let r = run(&f);
    assert_eq!(r.summary(), "3 problems");
    let tile = r.home_tile().unwrap();
    assert_eq!((tile.title.as_str(), tile.note.as_str()), ("Fix 3 problems", "Steam changes and 2 more"));
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
    assert_eq!(page.facts.len(), 10, "one line per check");
    assert_eq!(page.title, "Checkup");
    let updates = Check::ALL.iter().position(|c| *c == Check::Updates).unwrap();
    assert_eq!(page.facts[updates], "Updates: Update ready: SOH", "kept as it is: it reads as a title");
    assert_eq!(page.mark_of(updates), Mark::ToDo);
    let icons = Check::ALL.iter().position(|c| *c == Check::SteamIcons).unwrap();
    assert_eq!(page.facts[icons], "Steam icons: could not check (Could not read Steam)", "a check that cannot run says why on its own line");
    assert_eq!(page.mark_of(icons), Mark::Skipped);
    assert_eq!(page.paragraphs, vec!["I found 1 thing to fix."]);
    assert_eq!(page.options.len(), 2);
    assert_eq!((page.options[0].title.as_str(), page.options[0].recommended, page.options[0].action), ("Fix it", true, Act::FixAll));
    assert_eq!((page.options[1].title.as_str(), page.options[1].action), ("Not now", Act::NotNow));
    assert_eq!(action_at(&page, initial_focus(&page)), Some(Act::FixAll));
    assert_eq!(page.back_hint, "Not now");

    f.saves = vec![named("soh")];
    let page = pages::overview_page(&run(&f));
    assert_eq!(page.options[0].title, "Fix them one by one");
    assert_eq!(page.paragraphs, vec!["I found 2 things to fix."]);
}

#[test]
fn when_everything_is_fine_the_overview_says_so_and_offers_done() {
    let page = pages::overview_page(&run(&fine()));
    assert_eq!(page.paragraphs, vec!["Everything looks fine."]);
    assert_eq!(page.options.iter().map(|o| o.action).collect::<Vec<_>>(), [Act::Done, Act::CheckAgain]);
    assert!(page.options[0].recommended);
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
    let green = pages::after_page(Check::SteamIcons, &Outcome::Ok("All fine".into()), 1, 2, false);
    assert_eq!(green.title, "That is fixed");
    assert_eq!(green.mark_of(0), Mark::Done);
    assert_eq!(green.facts, vec!["Steam icons: all fine"]);
    assert_eq!(green.options.len(), 1);
    assert_eq!((green.options[0].title.as_str(), green.options[0].action), ("Next problem", Act::Next));
    let last = pages::after_page(Check::SteamIcons, &Outcome::Ok("All fine".into()), 2, 2, true);
    assert_eq!(last.options[0].title, "Finish");

    let mut f = fine();
    f.updates = vec![named("soh")];
    let still = pages::after_page(Check::Updates, &check(Check::Updates, &f), 1, 1, true);
    assert_eq!(still.title, "Not fixed yet");
    assert_eq!(still.mark_of(0), Mark::ToDo);
    assert_eq!(still.options.iter().map(|o| o.action).collect::<Vec<_>>(), [Act::TryAgain, Act::CheckAgain, Act::Next]);
    assert_eq!(still.options[0].detail, "Update SOH");

    let cannot = pages::after_page(Check::Saves, &Outcome::Unknown("the ports folder is not there".into()), 1, 1, true);
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
    assert!(left.paragraphs[0].starts_with("1 thing is still left"));
    assert_eq!((left.mark_of(0), left.mark_of(1)), (Mark::ToDo, Mark::Done));

    let ask = pages::confirm_page(Check::Updates, "Update SOH to the newest version?", vec!["Your saves are kept.".into()], "Update", 1, 1);
    assert_eq!(ask.paragraphs.len(), 2);
    assert_eq!((ask.options[0].action, ask.options[1].action), (Act::Yes, Act::Skip));
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
