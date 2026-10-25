//! The pages of "Steam icons" as plain data: the one the user decides on, the wait while icons are fetched, and the
//! result. `src/app/steam_icons_screen.rs` shows them.

use super::{ApplyReport, IconFinding, IconState, Plan, PlanItem, Resolved};
use crate::fork::page::{Focus, Mark, Page, PageOption};

pub const HEADER: &str = "Deck Ports Manager";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconAction {
    /// Make the changes (or queue them when Steam is running).
    Fix,
    NotNow,
    Done,
}

pub type IconPage = Page<IconAction>;

fn base(title: &str) -> IconPage {
    Page { window_title: "Steam icons".to_string(), large: true, header: HEADER.to_string(), title: title.to_string(), ..Page::default() }
}

fn option(title: &str, detail: &str, action: IconAction, recommended: bool) -> PageOption<IconAction> {
    PageOption { title: title.to_string(), detail: detail.to_string(), choice: None, enabled: true, recommended, action }
}

fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// While the catalog icons are fetched.
pub fn fetching_page() -> IconPage {
    let mut p = base("Steam icons");
    p.paragraphs = vec!["Getting the catalog icons for the shortcuts that need one...".to_string()];
    p.options = vec![option("Cancel", "Nothing is changed.", IconAction::NotNow, false)];
    p.initial = Focus::Option(0);
    p.with_progress(None).b_means("Cancel")
}

/// Why the scan could not be made.
pub fn problem_page(text: &str) -> IconPage {
    let mut p = base("Steam icons");
    p.paragraphs = vec![text.to_string()];
    p.options = vec![option("OK", "", IconAction::Done, true)];
    p.initial = Focus::Option(0);
    p
}

/// What was found and what will be done. `steam_running`: Steam has to be closed to change the list, so the fixes are
/// queued and the "Close Steam and update shortcuts" question follows; `game_mode`: Steam cannot be closed here.
pub fn overview_page(findings: &[IconFinding], plan: &Plan, steam_running: bool, game_mode: bool) -> IconPage {
    let mut p = base("Steam icons");
    if findings.is_empty() {
        p.paragraphs = vec!["No Steam shortcut points into your ports folder, so there is no icon to fix. Add your ports from Add to Steam first.".to_string()];
        p.options = vec![option("OK", "", IconAction::Done, true)];
        p.initial = Focus::Option(0);
        return p;
    }
    let fine: Vec<&IconFinding> = findings.iter().filter(|f| matches!(f.state, IconState::Ok(_))).collect();
    let outside: Vec<&IconFinding> = findings.iter().filter(|f| matches!(f.state, IconState::Outside(_))).collect();
    let broken = plan.items.len();
    p.paragraphs = vec![format!(
        "Looked at {} for your ports. {}",
        count(findings.len(), "Steam shortcut", "Steam shortcuts"),
        if broken == 0 { "Every icon shows.".to_string() } else { format!("{} will not show.", count(broken, "icon", "icons")) }
    )];
    for item in &plan.items {
        p.fact(if item.can_fix() { Mark::ToDo } else { Mark::Problem }, item.line());
    }
    for f in outside {
        p.fact(Mark::Skipped, f.describe());
    }
    // One line per shortcut that looks fine, with the path, so a cause this check does not catch is still visible.
    for f in fine {
        p.fact(Mark::Done, f.describe());
    }
    let fixable = plan.fixable();
    if fixable == 0 {
        p.options = vec![option("OK", "", IconAction::Done, true)];
        p.initial = Focus::Option(0);
        return p;
    }
    p.paragraphs.push("Only the icon of each shortcut changes (not its program, folder, launch options or name). The shortcut list is backed up first.".to_string());
    let detail = if game_mode {
        "Queued now; Steam can't be closed in Game Mode, so finish in Desktop Mode"
    } else if steam_running {
        "Steam is running, so this asks to close Steam next"
    } else {
        "Steam is closed, so the change is made now"
    };
    p.options = vec![option(&format!("Fix {}", count(fixable, "icon", "icons")), detail, IconAction::Fix, true), option("Not now", "Nothing is changed.", IconAction::NotNow, false)];
    p.initial = Focus::Option(0);
    p.b_means("Not now")
}

/// What happened. `resolved.left` holds what could not be placed, `report` what was written or queued.
pub fn result_page(report: &ApplyReport, resolved: &Resolved) -> IconPage {
    let mut p = base("Steam icons");
    let written = report.written.len();
    let queued = report.queued.len();
    for (name, _, _, to) in &report.written {
        p.fact(Mark::Done, format!("{name}: icon set to {to}"));
    }
    for fix in &report.queued {
        p.fact(Mark::Waiting, format!("{}: waiting for Steam to close (icon will be {})", fix.name, fix.to));
    }
    for (name, why) in report.skipped.iter().chain(resolved.left.iter()) {
        p.fact(Mark::Problem, format!("{name}: left as it was ({why})"));
    }
    for f in &report.failed {
        p.fact(Mark::Problem, f.clone());
    }
    let mut lines = Vec::new();
    if written > 0 {
        lines.push(format!("{} fixed. Steam shows them the next time it starts.", count(written, "icon", "icons")));
    }
    if queued > 0 {
        lines.push(format!("{} queued: they are made once Steam is closed (Close Steam and update shortcuts).", count(queued, "icon", "icons")));
    }
    if lines.is_empty() {
        lines.push("No icon was changed.".to_string());
    }
    p.paragraphs = lines;
    p.options = vec![option("OK", "", IconAction::Done, true)];
    p.initial = Focus::Option(0);
    p
}

/// Whether a plan item is one the page offers to fix (used by the screen to count without building a page).
pub fn is_fixable(item: &PlanItem) -> bool {
    item.can_fix()
}

#[cfg(test)]
mod tests {
    use super::super::{IconFix, Replacement, Source};
    use super::*;
    use std::path::PathBuf;

    fn finding(name: &str, state: IconState) -> IconFinding {
        IconFinding {
            port_key: name.into(),
            port_name: name.into(),
            port_folder: name.into(),
            account_id: 1,
            account_label: "Tim".into(),
            shortcuts_file: PathBuf::from("/s/shortcuts.vdf"),
            shortcut_name: format!("{name} shortcut"),
            app_id: Some(7),
            stored: String::new(),
            state,
            at_old_copy: false,
        }
    }

    fn item(name: &str, state: IconState, replacement: Replacement) -> PlanItem {
        PlanItem { finding: finding(name, state), replacement }
    }

    #[test]
    fn the_overview_names_each_icon_what_is_wrong_and_what_will_be_done() {
        let missing = item("Twilight Princess", IconState::Missing("/run/media/x/tp.png".into()), Replacement::Use { path: "/lib/tp.png".into(), source: Source::BackupMoved("b".into()) });
        let stuck = item("Zelda", IconState::Empty, Replacement::Nothing("the catalog has none for this port".into()));
        let ok = finding("Doom", IconState::Ok("/lib/Doom/i.png".into()));
        let outside = finding("Quake", IconState::Outside("/home/deck/q.png".into()));
        let findings = vec![missing.finding.clone(), stuck.finding.clone(), ok, outside];
        let plan = Plan { items: vec![missing, stuck] };
        let page = overview_page(&findings, &plan, false, false);
        assert_eq!(page.facts[0], "Twilight Princess: icon file missing (/run/media/x/tp.png); will use the icon from before the move (found in the port's new folder)");
        assert!(page.facts[1].starts_with("Zelda: no icon is set; cannot be fixed: "));
        assert_eq!((page.mark_of(0), page.mark_of(1), page.mark_of(2), page.mark_of(3)), (Mark::ToDo, Mark::Problem, Mark::Skipped, Mark::Done));
        assert_eq!(page.facts[3], "Doom: icon is fine (/lib/Doom/i.png)", "a fine icon shows its path too");
        assert!(page.paragraphs[0].contains("4 Steam shortcuts") && page.paragraphs[0].contains("2 icons will not show"));
        assert_eq!(page.options[0].title, "Fix 1 icon");
        assert!(page.options[0].recommended);
        assert_eq!(page.options[1].action, IconAction::NotNow);
    }

    #[test]
    fn the_detail_says_what_the_fix_does_in_each_mode() {
        let plan = Plan { items: vec![item("A", IconState::Empty, Replacement::Use { path: "/x.png".into(), source: Source::Grid })] };
        let findings = vec![plan.items[0].finding.clone()];
        assert!(overview_page(&findings, &plan, false, false).options[0].detail.contains("made now"));
        assert!(overview_page(&findings, &plan, true, false).options[0].detail.contains("close Steam"));
        assert!(overview_page(&findings, &plan, true, true).options[0].detail.contains("Desktop Mode"));
    }

    #[test]
    fn nothing_to_fix_and_nothing_found_have_only_ok() {
        let page = overview_page(&[], &Plan::default(), false, false);
        assert_eq!(page.options.len(), 1);
        assert!(page.paragraphs[0].contains("points into your ports folder"));
        let findings = vec![finding("A", IconState::Ok("/lib/A/i.png".into()))];
        let page = overview_page(&findings, &Plan::default(), false, false);
        assert!(page.paragraphs[0].contains("Every icon shows"));
        assert_eq!(page.options.len(), 1);
        let stuck = Plan { items: vec![item("B", IconState::Empty, Replacement::Nothing("x".into()))] };
        let page = overview_page(&[stuck.items[0].finding.clone()], &stuck, false, false);
        assert_eq!(page.options.len(), 1, "no Fix button when nothing can be fixed");
    }

    #[test]
    fn the_result_lists_written_waiting_and_left_alone() {
        let report = ApplyReport {
            written: vec![("A shortcut".into(), 1, String::new(), "/a.png".into())],
            queued: vec![IconFix { account_id: 1, account_label: String::new(), port_key: "b".into(), name: "B shortcut".into(), app_id: 2, from: String::new(), to: "/b.png".into() }],
            skipped: vec![("C shortcut".into(), "its icon changed since the problem was found".into())],
            failed: vec!["could not update the shortcuts of Tim: x".into()],
        };
        let resolved = Resolved { fixes: Vec::new(), left: vec![("D".into(), "no stored app ID".into())] };
        let page = result_page(&report, &resolved);
        assert_eq!(page.facts.len(), 5);
        assert_eq!((page.mark_of(0), page.mark_of(1), page.mark_of(2), page.mark_of(4)), (Mark::Done, Mark::Waiting, Mark::Problem, Mark::Problem));
        assert!(page.paragraphs.iter().any(|t| t.contains("1 icon fixed")) && page.paragraphs.iter().any(|t| t.contains("1 icon queued")));
        assert_eq!(result_page(&ApplyReport::default(), &Resolved::default()).paragraphs, ["No icon was changed."]);
    }
}
