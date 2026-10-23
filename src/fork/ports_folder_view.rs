//! The lists of the ports folder screens (labels, what each row does, which row starts selected),
//! built without any UI so they can be tested. `src/app/ports_folder_screen.rs` draws them.

use super::port_mover::game_files::Issue;
use super::port_mover::old_copies::OldCopy;
use super::ports_folder::{Missing, MovePlan};
use super::rom_screens::util::{ellipsize, human_bytes};
use std::path::Path;

/// What choosing a row does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// An information row: nothing.
    Info,
    /// Shows the whole text of item `n` (a problem row).
    Detail(usize),
    // Old copy prompt
    RemoveOldCopy,
    ExplainKept,
    KeepForNow,
    // Missing folder
    TryAgain,
    ChooseAnother,
    ContinueWithout,
    Quit,
    // Menu
    OpenFolder,
    OldCopies,
    UseDefault,
    Back,
    // Switch screen
    Move,
    ReplaceStaleAndMove,
    SwitchWithoutMoving,
    SwitchEmpty,
    NotEnoughSpace,
    // Game files and failures
    FixInGameFiles,
    MoveAnyway,
    SwitchAnyway,
    /// Cancel; after a copy it also removes the new copies.
    Cancel,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListModel {
    pub labels: Vec<String>,
    pub acts: Vec<Act>,
    /// The row selected when the list opens.
    pub select: usize,
}

impl ListModel {
    fn push(&mut self, label: impl Into<String>, act: Act) -> usize {
        self.labels.push(label.into());
        self.acts.push(act);
        self.acts.len() - 1
    }

    pub fn act(&self, index: usize) -> Act {
        self.acts.get(index).copied().unwrap_or(Act::Info)
    }

    pub fn row_of(&self, act: Act) -> Option<usize> {
        self.acts.iter().position(|a| *a == act)
    }
}

/// "Remove the old copy now?" for one old copy. Starts on Keep for now: removing is never the default.
pub fn old_copy_prompt(entry: &OldCopy, check: &Result<u64, String>) -> ListModel {
    let mut m = ListModel::default();
    let size = match check {
        Ok(bytes) if entry.link_only => format!("A link to another folder ({} stays)", human_bytes(*bytes)),
        Ok(bytes) => human_bytes(*bytes),
        Err(_) => human_bytes(entry.bytes),
    };
    m.push(entry.name.clone(), Act::Info);
    m.push(format!("Old copy: {}", ellipsize(&entry.old_path.to_string_lossy(), 70)), Act::Info);
    m.push(format!("Size: {size}"), Act::Info);
    m.push(format!("In use now: {}", ellipsize(&entry.new_path.to_string_lossy(), 66)), Act::Info);
    match check {
        Ok(_) if entry.link_only => m.push("Remove the old link (the folder it points to stays)", Act::RemoveOldCopy),
        Ok(_) => m.push("Remove old copy (its saves are zipped to Saves Backup/Update Backups first)", Act::RemoveOldCopy),
        Err(why) => m.push(format!("Can't remove it now: {}", ellipsize(why, 90)), Act::ExplainKept),
    };
    m.select = m.push("Keep for now", Act::KeepForNow);
    m
}

/// The list after a move in which some ports failed.
pub fn after_move(failed: &[(String, String, bool)], moved: usize) -> ListModel {
    let mut m = ListModel::default();
    m.push(format!("{} of {} ports could not be moved:", failed.len(), failed.len() + moved), Act::Info);
    for (i, (name, text, _)) in failed.iter().enumerate() {
        m.push(format!("  {name}: {}", ellipsize(&text.replace('\n', " "), 90)), Act::Detail(i));
    }
    let mut first = None;
    if failed.iter().any(|(_, _, game_files)| *game_files) {
        first = Some(m.push("Fix in Game files (then start the move again; the new copies are removed)", Act::FixInGameFiles));
    }
    if moved > 0 {
        let row = m.push(format!("Switch anyway ({moved} moved; the others stay in the old folder and show as not installed)"), Act::SwitchAnyway);
        first = first.or(Some(row));
    }
    let cancel = m.push("Cancel (nothing changes; the new copies are removed)", Act::Cancel);
    m.select = first.unwrap_or(cancel);
    m
}

/// "Switch the ports folder?"
pub fn switch_screen(new_dir: &Path, summary: &str, plan: &MovePlan, free: Option<u64>, needed: u64) -> ListModel {
    let mut m = ListModel::default();
    m.push(format!("New ports folder: {}", ellipsize(&new_dir.to_string_lossy(), 60)), Act::Info);
    m.push(format!("  {summary}"), Act::Info);
    if !plan.already_there.is_empty() {
        m.push(format!("Already in the new folder (used from there): {}", plan.already_there.join(", ")), Act::Info);
    }
    for s in &plan.stale {
        m.push(format!("{}: an old copy you kept is already there; the copy at {} is newer and is the one in use", s.name, ellipsize(&s.from.to_string_lossy(), 50)), Act::Info);
    }
    if !plan.other_folders.is_empty() {
        m.push(format!("Not ports, never moved: {}", ellipsize(&plan.other_folders.join(", "), 60)), Act::Info);
    }
    let n = plan.ports.len() + plan.stale.len();
    let fits = free.is_none_or(|f| f >= needed);
    let first = if n == 0 {
        m.push("Switch to this folder", Act::SwitchEmpty)
    } else if !fits {
        m.push(format!("Not enough space to move the ports: needs {}, {} free", human_bytes(needed), human_bytes(free.unwrap_or(0))), Act::NotEnoughSpace)
    } else if !plan.stale.is_empty() {
        m.push(
            format!("Replace the old {} with the current one{}, then move (saves zipped first)", if plan.stale.len() == 1 { "copy" } else { "copies" }, if plan.stale.len() == 1 { "" } else { "s" }),
            Act::ReplaceStaleAndMove,
        )
    } else {
        m.push(format!("Move {n} port{} here ({}), then switch (the old copies are kept)", if n == 1 { "" } else { "s" }, human_bytes(plan.bytes)), Act::Move)
    };
    // With a stale old copy in the way, switching without moving would use it: not offered.
    if n > 0 && plan.stale.is_empty() {
        m.push(format!("Switch without moving ({n} port{} stay in the old folder and show as not installed)", if n == 1 { "" } else { "s" }), Act::SwitchWithoutMoving);
    }
    if !plan.stale.is_empty() {
        m.push("Or remove those old copies first: Settings > Ports folder > Old copies", Act::Info);
    }
    m.push("Cancel", Act::Cancel);
    m.select = first;
    m
}

/// "Ports folder not available" at startup or after Try again.
pub fn missing(dir: &Path, why: &Missing, again: bool) -> ListModel {
    let mut m = ListModel::default();
    m.push(if again { "Still not there. Your ports folder:" } else { "Your ports folder is not available:" }, Act::Info);
    m.push(format!("  {}", ellipsize(&dir.to_string_lossy(), 70)), Act::Info);
    let (question, retry) = match why {
        Missing::DriveNotMounted(_) => ("Is the SD card in?", "Try again (the card is in now)"),
        Missing::FolderGone => ("Was it moved, renamed or deleted?", "Try again"),
        Missing::NotAFolder => ("Something that is not a folder has its name.", "Try again"),
    };
    m.push(question, Act::Info);
    m.select = m.push(retry, Act::TryAgain);
    m.push("Choose another folder...", Act::ChooseAnother);
    m.push("Continue without it (installs, adopting and restores wait until it is back)", Act::ContinueWithout);
    m.push("Quit", Act::Quit);
    m
}

/// Settings > Ports folder...
pub fn menu(dir: &Path, state: &str, is_default: bool, old: (usize, u64), default_dir: &Path, game_mode: bool) -> ListModel {
    let mut m = ListModel::default();
    m.push(format!("Ports folder: {}", ellipsize(&dir.to_string_lossy(), 64)), Act::Info);
    m.push(format!("  {state}{}", if is_default { " (default, next to the app)" } else { "" }), Act::Info);
    m.select = m.push("Choose another folder...", Act::ChooseAnother);
    // A file manager window cannot be used in Game Mode.
    if !game_mode {
        m.push("Open the ports folder (Desktop Mode)", Act::OpenFolder);
    }
    if old.0 > 0 {
        m.push(format!("Old copies kept after moves ({}, {})...", old.0, human_bytes(old.1)), Act::OldCopies);
    }
    if !is_default {
        m.push(format!("Use the default folder ({})", ellipsize(&default_dir.to_string_lossy(), 48)), Act::UseDefault);
    }
    m.push("Back", Act::Back);
    m
}

/// Problems found before copying, with Fix / Move anyway / Cancel.
pub fn game_file_issues(issues: &[(String, Issue)]) -> ListModel {
    let mut m = ListModel::default();
    m.push(format!("{} problem(s) with game files, found before copying:", issues.len()), Act::Info);
    for (i, (name, issue)) in issues.iter().enumerate().take(8) {
        m.push(format!("  {name}: {}: {}", issue.file, issue.problem.split(" (").next().unwrap_or(&issue.problem)), Act::Detail(i));
    }
    if issues.len() > 8 {
        m.push(format!("  ... and {} more", issues.len() - 8), Act::Info);
    }
    m.select = m.push("Fix in Game files (then start the move again)", Act::FixInGameFiles);
    m.push("Move anyway", Act::MoveAnyway);
    m.push("Cancel", Act::Cancel);
    m
}

/// The rows of the progress list during a move.
pub fn progress_labels(progress: &str, cancelling: bool) -> Vec<String> {
    let (a, b) = progress.split_once('\n').unwrap_or((progress, ""));
    vec![
        if a.is_empty() { "Starting...".into() } else { a.to_string() },
        if b.is_empty() { " ".into() } else { b.to_string() },
        if cancelling { "Cancelling... (stops after the current file)".into() } else { "Cancel (the copies made so far are removed)".into() },
    ]
}

/// Why an action that writes into the ports folder must wait, if it must. `unavailable` is the
/// "not available" text when the ports folder is missing.
pub fn blocked_reason(moving: bool, unavailable: Option<String>) -> Option<String> {
    if moving {
        return Some("Installed ports are being moved to a new ports folder. Wait for that to finish, then try again.".to_string());
    }
    unavailable.map(|text| format!("{text}\nInstalls, adopting and restores wait until it is back. To use another folder: Settings > Ports folder."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::ports_folder::PortToMove;
    use std::path::PathBuf;

    fn entry(link_only: bool) -> OldCopy {
        OldCopy {
            port_key: "k".into(),
            name: "Ship of Harkinian".into(),
            old_path: PathBuf::from("/home/deck/Applications/dpm/Library/SoH"),
            new_path: PathBuf::from("/run/media/deck/SD/Ports/SoH"),
            bytes: 1_000_000,
            moved_at: String::new(),
            link_only,
            switched: true,
        }
    }

    #[test]
    fn the_old_copy_prompt_starts_on_keep_for_now() {
        let can = old_copy_prompt(&entry(false), &Ok(2_000_000));
        assert_eq!(can.act(can.select), Act::KeepForNow, "removing is never the default");
        assert_eq!(can.labels[can.select], "Keep for now");
        let remove = can.row_of(Act::RemoveOldCopy).unwrap();
        assert!(can.labels[remove].contains("zipped to Saves Backup/Update Backups"));
        assert_eq!(can.labels[2], "Size: 2.0 MB");
        let cannot = old_copy_prompt(&entry(false), &Err("Steam shortcuts still point into it".into()));
        assert_eq!(cannot.act(cannot.select), Act::KeepForNow);
        assert!(cannot.row_of(Act::RemoveOldCopy).is_none());
        assert!(cannot.labels[cannot.row_of(Act::ExplainKept).unwrap()].starts_with("Can't remove it now: Steam"));
        let link = old_copy_prompt(&entry(true), &Ok(0));
        assert!(link.labels[link.row_of(Act::RemoveOldCopy).unwrap()].contains("folder it points to stays"));
    }

    #[test]
    fn after_a_partial_failure_the_choices_depend_on_what_failed() {
        let failed = vec![("Dusk".to_string(), "copy failed".to_string(), false), ("SoH".to_string(), "game files".to_string(), true)];
        let m = after_move(&failed, 2);
        assert_eq!(m.labels[0], "2 of 4 ports could not be moved:");
        assert_eq!((m.act(1), m.act(2)), (Act::Detail(0), Act::Detail(1)));
        assert_eq!(m.act(m.select), Act::FixInGameFiles);
        assert!(m.labels[m.select].contains("then start the move again"));
        assert!(m.row_of(Act::SwitchAnyway).is_some());
        assert_eq!(*m.acts.last().unwrap(), Act::Cancel);
        // Nothing moved and no game file problem: only Cancel.
        let m = after_move(&failed[..1], 0);
        assert_eq!(m.acts, [Act::Info, Act::Detail(0), Act::Cancel]);
        assert_eq!(m.act(m.select), Act::Cancel);
    }

    fn port(name: &str, from: &str, to: &str) -> PortToMove {
        PortToMove { key: name.into(), name: name.into(), from: from.into(), to: to.into(), link_only: false, bytes: 10 }
    }

    #[test]
    fn the_switch_screen_never_offers_to_use_a_stale_old_copy() {
        let plan = MovePlan { ports: vec![port("SoH", "/a/SoH", "/b/SoH")], bytes: 10, ..Default::default() };
        let m = switch_screen(Path::new("/b"), "5 GB free", &plan, Some(1 << 40), 10);
        assert_eq!(m.act(m.select), Act::Move);
        assert!(m.row_of(Act::SwitchWithoutMoving).is_some());

        let plan = MovePlan { ports: vec![port("SoH", "/a/SoH", "/b/SoH")], stale: vec![port("Dusk", "/a/Dusk", "/b/Dusk")], bytes: 20, ..Default::default() };
        let m = switch_screen(Path::new("/b"), "5 GB free", &plan, Some(1 << 40), 20);
        assert!(m.labels.iter().any(|l| l.starts_with("Dusk: an old copy you kept is already there; the copy at /a/Dusk is newer")));
        assert_eq!(m.act(m.select), Act::ReplaceStaleAndMove);
        assert!(m.row_of(Act::Move).is_none() && m.row_of(Act::SwitchWithoutMoving).is_none(), "no way to switch onto the stale copy");

        let m = switch_screen(Path::new("/b"), "", &plan, Some(5), 20);
        assert_eq!(m.act(m.select), Act::NotEnoughSpace);
        let empty = switch_screen(Path::new("/b"), "", &MovePlan::default(), None, 0);
        assert_eq!(empty.act(empty.select), Act::SwitchEmpty);
    }

    #[test]
    fn the_missing_folder_list_and_the_menu() {
        let m = missing(Path::new("/run/media/deck/SD/Ports"), &Missing::DriveNotMounted(PathBuf::from("/run/media/deck/SD")), false);
        assert_eq!(m.labels[m.select], "Try again (the card is in now)");
        assert_eq!(m.labels[2], "Is the SD card in?");
        assert_eq!(m.row_of(Act::Quit), Some(m.labels.len() - 1));
        let menu_desktop = menu(Path::new("/x"), "ok", true, (0, 0), Path::new("/d"), false);
        assert!(menu_desktop.row_of(Act::OpenFolder).is_some() && menu_desktop.row_of(Act::UseDefault).is_none());
        let menu_game = menu(Path::new("/x"), "ok", false, (2, 5_000_000), Path::new("/d"), true);
        assert!(menu_game.row_of(Act::OpenFolder).is_none(), "no file manager in Game Mode");
        assert!(menu_game.labels[menu_game.row_of(Act::OldCopies).unwrap()].contains("(2, 5.0 MB)"));
        assert_eq!(menu_game.act(menu_game.select), Act::ChooseAnother);
    }

    #[test]
    fn game_file_problems_are_short_rows_with_details() {
        let issue = Issue { file: "Zelda.z64".into(), expected_at: PathBuf::from("/x"), problem: "the original game file is missing (drive not plugged in)".into() };
        let issues: Vec<(String, Issue)> = (0..10).map(|i| (format!("P{i}"), issue.clone())).collect();
        let m = game_file_issues(&issues);
        assert_eq!(m.labels[1], "  P0: Zelda.z64: the original game file is missing");
        assert_eq!(m.act(8), Act::Detail(7));
        assert_eq!(m.labels[9], "  ... and 2 more");
        assert_eq!(m.act(m.select), Act::FixInGameFiles);
    }

    #[test]
    fn progress_rows_and_the_cancel_label() {
        assert_eq!(progress_labels("", false), ["Starting...", " ", "Cancel (the copies made so far are removed)"]);
        let rows = progress_labels("Copying 1 of 2: SoH\n1.0 MB of 2.0 MB", true);
        assert_eq!(rows[0], "Copying 1 of 2: SoH");
        assert_eq!(rows[2], "Cancelling... (stops after the current file)");
    }

    #[test]
    fn blocked_reasons() {
        assert!(blocked_reason(false, None).is_none());
        assert!(blocked_reason(true, None).unwrap().contains("being moved"));
        let r = blocked_reason(false, Some("Your ports folder /x is not available. Is the SD card in?".into())).unwrap();
        assert!(r.contains("adopting and restores wait"));
    }
}
