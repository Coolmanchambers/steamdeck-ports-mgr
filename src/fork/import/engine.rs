//! The one door between the Import screen and whatever actually moves a port.
//!
//! `move_selected` runs a batch port by port: it publishes a status line per port, checks for a cancel
//! between ports (never in the middle of one), and gathers what each engine reports into an
//! `ImportSummary`. The screen knows nothing about copying: it builds plans, calls `move_selected` on a
//! worker thread and shows the summary.
//!
//! # STUB (until the move engine of the ports-folder work is on `main`)
//! The only engine today is `DryRunEngine`. It plans and reports exactly what would be checked, copied,
//! verified and repointed and writes nothing. The real engine implements `MoveEngine` (copy, verify with
//! the port's game files before and after, switch the app's records, repoint or queue Steam shortcuts,
//! never delete the original) and asks the user through `Prompter` when a game file is missing or broken
//! and, after each verified move, whether to remove the old copy. `engine_for_import()` is the single
//! place that picks the engine.

use super::plan::{PortPlan, SteamWrite};
use super::scan::Size;
use crate::fork::rom_screens::util::human_bytes;
use crate::fork::screens::short_path;
use std::cell::RefCell;
use std::path::Path;

/// What the engine reports for one Steam shortcut change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SteamResult {
    NoShortcut,
    /// This many shortcuts were repointed, app IDs kept.
    Repointed(usize),
    /// This many changes were queued for Desktop Mode (Steam was running).
    Queued(usize),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortOutcome {
    /// Dry run: the plan was worked out and shown; nothing was written.
    Planned,
    /// Copied, verified, switched over. `old_copy_removed` is true only when the user chose "Remove old
    /// copy" in the prompt that follows each verified move.
    Moved { steam: SteamResult, old_copy_removed: bool },
    /// The engine would not start; nothing changed. `space` is true for "not enough free space", the one
    /// case where linking the folder in place instead is offered.
    Refused { reason: String, space: bool },
    /// The user stopped this port at a prompt (a game file was missing or broken); nothing changed.
    Stopped(String),
    /// Something went wrong part way; the text says what, and the old folder is untouched.
    Failed(String),
    /// A cancel came before this port started.
    NotStarted,
}

/// The user's answers to the prompts an engine may raise on the worker thread.
pub trait Prompter: Sync {
    /// A game file the port needs is missing or broken. `true` = carry on with this port anyway.
    fn game_file_problem(&self, port: &str, problems: &[String]) -> bool;
    /// A move was verified. `true` = remove the old copy now; the default of "keep" is `false`.
    fn remove_old_copy(&self, port: &str, old_copy: &Path, size: Size) -> bool;
}

/// Answers every prompt the safe way: stop on a problem, keep the old copy.
pub struct NoPrompts;

impl Prompter for NoPrompts {
    fn game_file_problem(&self, _: &str, _: &[String]) -> bool {
        false
    }

    fn remove_old_copy(&self, _: &str, _: &Path, _: Size) -> bool {
        false
    }
}

pub struct MoveContext<'a> {
    pub plan: &'a PortPlan,
    pub cancelled: &'a dyn Fn() -> bool,
    /// Says what the engine is doing now ("copying", "checking the copy"), shown after the port's name.
    pub progress: &'a dyn Fn(String),
    pub prompts: &'a dyn Prompter,
}

pub trait MoveEngine: Send + Sync {
    fn is_dry_run(&self) -> bool;
    fn move_port(&self, ctx: &MoveContext) -> PortOutcome;
}

/// STUB engine: reports the plan, writes nothing. See the module documentation.
pub struct DryRunEngine;

impl MoveEngine for DryRunEngine {
    fn is_dry_run(&self) -> bool {
        true
    }

    fn move_port(&self, ctx: &MoveContext) -> PortOutcome {
        match &ctx.plan.block {
            Some(block) => PortOutcome::Refused { reason: block.text(Path::new("")), space: block.is_space() },
            None => PortOutcome::Planned,
        }
    }
}

/// The engine the Import screen uses. Switching from the dry run to the real engine is this function.
pub fn engine_for_import() -> Box<dyn MoveEngine> {
    Box::new(DryRunEngine)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryItem {
    pub plan: PortPlan,
    pub outcome: PortOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportSummary {
    pub dry_run: bool,
    pub items: Vec<SummaryItem>,
    pub cancelled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
    Waiting,
    Working(String),
    Finished(String),
}

/// One status line per port, republished whenever one changes.
struct Board {
    names: Vec<String>,
    status: Vec<Status>,
}

impl Board {
    fn render(&self) -> String {
        self.names
            .iter()
            .zip(&self.status)
            .map(|(name, status)| match status {
                Status::Waiting => format!("{name}: waiting"),
                Status::Working(note) if note.is_empty() => format!("{name}: working..."),
                Status::Working(note) => format!("{name}: {note}..."),
                Status::Finished(text) => format!("{name}: {text}"),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn finished_text(outcome: &PortOutcome) -> &'static str {
    match outcome {
        PortOutcome::Planned => "planned",
        PortOutcome::Moved { .. } => "done",
        PortOutcome::Refused { .. } => "not moved",
        PortOutcome::Stopped(_) => "stopped",
        PortOutcome::Failed(_) => "problem",
        PortOutcome::NotStarted => "not started",
    }
}

/// Runs the plans in order. `publish` gets the whole status text (one line per port) after every change.
/// `cancelled` is looked at before each port, so a cancel never interrupts one that is part way through.
pub fn move_selected(engine: &dyn MoveEngine, plans: Vec<PortPlan>, cancelled: &dyn Fn() -> bool, publish: &dyn Fn(String), prompts: &dyn Prompter) -> ImportSummary {
    let board = RefCell::new(Board { names: plans.iter().map(|p| p.request.port_name.clone()).collect(), status: vec![Status::Waiting; plans.len()] });
    publish(board.borrow().render());
    let mut summary = ImportSummary { dry_run: engine.is_dry_run(), items: Vec::new(), cancelled: false };
    for (i, plan) in plans.into_iter().enumerate() {
        if summary.cancelled || cancelled() {
            summary.cancelled = true;
            board.borrow_mut().status[i] = Status::Finished(finished_text(&PortOutcome::NotStarted).into());
            summary.items.push(SummaryItem { plan, outcome: PortOutcome::NotStarted });
            continue;
        }
        board.borrow_mut().status[i] = Status::Working(String::new());
        publish(board.borrow().render());
        let progress = |note: String| {
            board.borrow_mut().status[i] = Status::Working(note);
            publish(board.borrow().render());
        };
        let outcome = engine.move_port(&MoveContext { plan: &plan, cancelled, progress: &progress, prompts });
        board.borrow_mut().status[i] = Status::Finished(finished_text(&outcome).into());
        summary.items.push(SummaryItem { plan, outcome });
    }
    publish(board.borrow().render());
    summary
}

fn steam_clause(steam: &SteamResult) -> String {
    let n = |c: usize| if c == 1 { "1 Steam shortcut".to_string() } else { format!("{c} Steam shortcuts") };
    match steam {
        SteamResult::NoShortcut => String::new(),
        SteamResult::Repointed(c) => format!("; {} repointed", n(*c)),
        SteamResult::Queued(c) => format!("; {} queued for Desktop Mode", n(*c)),
        SteamResult::Failed(e) => format!("; Steam shortcut problem: {e}"),
    }
}

fn would_steam_clause(plan: &PortPlan) -> String {
    match (plan.steam.len(), plan.steam_write) {
        (0, _) => String::new(),
        (n, SteamWrite::Now) => format!("; {n} Steam shortcut{} would be repointed", if n == 1 { "" } else { "s" }),
        (n, SteamWrite::Queued) => format!("; {n} Steam shortcut change{} would be queued", if n == 1 { "" } else { "s" }),
    }
}

impl ImportSummary {
    pub fn moved(&self) -> impl Iterator<Item = &SummaryItem> {
        self.items.iter().filter(|i| matches!(i.outcome, PortOutcome::Moved { .. }))
    }

    pub fn planned_count(&self) -> usize {
        self.items.iter().filter(|i| i.outcome == PortOutcome::Planned).count()
    }

    pub fn moved_count(&self) -> usize {
        self.moved().count()
    }

    /// Moved ports whose old folder is still on disk.
    pub fn kept_old_copies(&self) -> Vec<&SummaryItem> {
        self.moved().filter(|i| matches!(i.outcome, PortOutcome::Moved { old_copy_removed: false, .. })).collect()
    }

    pub fn problem_count(&self) -> usize {
        self.items.iter().filter(|i| matches!(i.outcome, PortOutcome::Failed(_) | PortOutcome::Stopped(_) | PortOutcome::Refused { .. })).count()
    }

    /// (repointed, queued, failed) over every moved port.
    pub fn steam_counts(&self) -> (usize, usize, usize) {
        self.moved().fold((0, 0, 0), |(r, q, f), item| match &item.outcome {
            PortOutcome::Moved { steam: SteamResult::Repointed(c), .. } => (r + c, q, f),
            PortOutcome::Moved { steam: SteamResult::Queued(c), .. } => (r, q + c, f),
            PortOutcome::Moved { steam: SteamResult::Failed(_), .. } => (r, q, f + 1),
            _ => (r, q, f),
        })
    }

    /// Items the engine refused for lack of space: the only ones the link fallback is offered for.
    pub fn link_candidates(&self) -> Vec<usize> {
        self.items.iter().enumerate().filter(|(_, i)| matches!(i.outcome, PortOutcome::Refused { space: true, .. })).map(|(n, _)| n).collect()
    }

    /// One line per port.
    pub fn item_label(&self, index: usize, home: &Path) -> String {
        let item = &self.items[index];
        let name = &item.plan.request.port_name;
        let size = human_bytes(item.plan.request.size.bytes);
        match &item.outcome {
            PortOutcome::Planned => format!("{name}: would move {size}{}", would_steam_clause(&item.plan)),
            PortOutcome::Moved { steam, old_copy_removed } => {
                let old = if *old_copy_removed {
                    "old copy removed at your request".to_string()
                } else {
                    format!("old copy kept at {} ({size})", short_path(&item.plan.request.from, home))
                };
                format!("{name}: moved, {old}{}", steam_clause(steam))
            }
            PortOutcome::Refused { .. } => {
                let reason = item.plan.block.as_ref().map_or_else(|| "the engine refused".to_string(), |b| b.text(home));
                format!("{name}: not moved, {reason}")
            }
            PortOutcome::Stopped(why) => format!("{name}: stopped, {why}"),
            PortOutcome::Failed(why) => format!("{name}: problem, {why}"),
            PortOutcome::NotStarted => format!("{name}: not started (cancelled)"),
        }
    }

    pub fn title(&self) -> &'static str {
        if self.dry_run {
            "Import preview"
        } else if self.cancelled {
            "Import stopped"
        } else {
            "Import finished"
        }
    }

    /// The totals under the title: what moved, what stayed behind, Steam, problems.
    pub fn totals(&self, home: &Path) -> Vec<String> {
        let mut out = Vec::new();
        if self.dry_run {
            out.push(format!("Preview only: {} port{} would be moved. Nothing was copied or changed.", self.planned_count(), if self.planned_count() == 1 { "" } else { "s" }));
        } else {
            out.push(format!("Moved {} of {} port{}.", self.moved_count(), self.items.len(), if self.items.len() == 1 { "" } else { "s" }));
        }
        if self.cancelled {
            out.push("Stopped before the rest: you cancelled between ports.".to_string());
        }
        let kept = self.kept_old_copies();
        if !kept.is_empty() {
            let bytes: u64 = kept.iter().map(|i| i.plan.request.size.bytes).sum();
            out.push(format!(
                "{} old cop{} kept ({}). Nothing was deleted: once you have checked the ports, remove {} yourself, for example in Desktop Mode's file manager.",
                kept.len(),
                if kept.len() == 1 { "y" } else { "ies" },
                human_bytes(bytes),
                if kept.len() == 1 { "it" } else { "them" }
            ));
            for item in kept {
                out.push(format!("  {}", short_path(&item.plan.request.from, home)));
            }
        }
        let (repointed, queued, failed) = self.steam_counts();
        if repointed + queued + failed > 0 {
            let mut parts = Vec::new();
            if repointed > 0 {
                parts.push(format!("{repointed} repointed"));
            }
            if queued > 0 {
                parts.push(format!("{queued} queued for Desktop Mode"));
            }
            if failed > 0 {
                parts.push(format!("{failed} failed"));
            }
            out.push(format!("Steam shortcuts: {}.", parts.join(", ")));
        }
        let problems = self.problem_count();
        if problems > 0 {
            out.push(format!("{problems} port{} had a problem (listed below).", if problems == 1 { "" } else { "s" }));
        }
        out
    }

    /// The whole summary as one text.
    pub fn text(&self, home: &Path) -> String {
        let mut lines = self.totals(home);
        lines.push(String::new());
        lines.extend((0..self.items.len()).map(|i| self.item_label(i, home)));
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::super::plan::{build_requests, plan_all, MoveRequest};
    use super::super::scan::fixtures::*;
    use super::super::scan::{Found, Size};
    use super::*;
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    fn plans(h: &Home, free: Option<u64>) -> Vec<PortPlan> {
        use crate::fork::import::folders::ScanFolders;
        use crate::fork::import::scan::{read_shortcuts, run_scan, ScanInput};
        use crate::fork::settings::ForkSettings;
        let input = ScanInput {
            roots: ScanFolders::default().roots(&h.home, &h.library),
            catalog: catalog(),
            library_dir: h.library.clone(),
            deeper: false,
            shortcuts: read_shortcuts(&h.home, &ForkSettings::load(&h.home.join("s.json"))),
        };
        let scan = run_scan(&input, &|| false, &|_| {});
        let mut refs: Vec<&Found> = scan.found.iter().collect();
        refs.sort_by(|a, b| a.port_name.cmp(&b.port_name));
        let reqs: Vec<MoveRequest> = build_requests(&refs, &h.library);
        plan_all(&reqs, free, SteamWrite::Now)
    }

    /// An engine for tests that "moves" every port and records what it was asked.
    struct Recorder {
        seen: Mutex<Vec<String>>,
        cancel_after_first: Option<Arc<AtomicBool>>,
    }

    impl MoveEngine for Recorder {
        fn is_dry_run(&self) -> bool {
            false
        }

        fn move_port(&self, ctx: &MoveContext) -> PortOutcome {
            self.seen.lock().unwrap().push(ctx.plan.request.port_name.clone());
            (ctx.progress)("copying".into());
            if let Some(flag) = &self.cancel_after_first {
                flag.store(true, Ordering::SeqCst);
            }
            let steam = if ctx.plan.steam.is_empty() { SteamResult::NoShortcut } else { SteamResult::Repointed(ctx.plan.steam.len()) };
            PortOutcome::Moved { steam, old_copy_removed: false }
        }
    }

    #[test]
    fn the_dry_run_plans_every_port_and_writes_nothing() {
        let h = fake_home("engine_dry");
        let before = snapshot(&h.home);
        let lines = RefCell::new(Vec::new());
        let summary = move_selected(&DryRunEngine, plans(&h, None), &|| false, &|t| lines.borrow_mut().push(t), &NoPrompts);
        assert!(summary.dry_run && !summary.cancelled);
        assert_eq!(summary.items.len(), 2);
        assert!(summary.items.iter().all(|i| i.outcome == PortOutcome::Planned));
        assert_eq!(snapshot(&h.home), before, "a dry run must not change a single file or folder");
        assert!(!h.library.join("SoH").exists() && !h.library.join("dusklight").exists());
        let last = lines.borrow().last().cloned().unwrap();
        assert_eq!(last, "Dusklight: planned\nShip of Harkinian: planned");
        assert_eq!(lines.borrow().first().unwrap(), "Dusklight: waiting\nShip of Harkinian: waiting");
    }

    fn snapshot(root: &Path) -> Vec<(String, u64)> {
        fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, u64)>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                let md = std::fs::symlink_metadata(&p).unwrap();
                out.push((p.strip_prefix(root).unwrap().to_string_lossy().into_owned(), if md.is_file() { md.len() } else { 0 }));
                if md.is_dir() {
                    walk(root, &p, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(root, root, &mut out);
        out.sort();
        out
    }

    #[test]
    fn the_preview_summary_says_what_would_happen_and_that_nothing_did() {
        let h = fake_home("engine_dry_text");
        let summary = move_selected(&DryRunEngine, plans(&h, None), &|| false, &|_| {}, &NoPrompts);
        let text = summary.text(&h.home);
        assert_eq!(summary.title(), "Import preview");
        assert!(text.starts_with("Preview only: 2 ports would be moved. Nothing was copied or changed."), "{text}");
        assert!(text.contains("Dusklight: would move 700 B\n"), "{text}");
        assert!(text.contains("Ship of Harkinian: would move 8 KB; 1 Steam shortcut would be repointed"), "{text}");
    }

    #[test]
    fn a_dry_run_refusal_for_space_makes_the_port_a_link_candidate() {
        let h = fake_home("engine_space");
        let summary = move_selected(&DryRunEngine, plans(&h, Some(100)), &|| false, &|_| {}, &NoPrompts);
        assert!(summary.items.iter().all(|i| matches!(i.outcome, PortOutcome::Refused { space: true, .. })));
        assert_eq!(summary.link_candidates(), [0, 1]);
        assert_eq!(summary.problem_count(), 2);
        assert!(summary.item_label(0, &h.home).starts_with("Dusklight: not moved, not enough free space: needs 700 B plus 1.0 GB spare, 100 B free"), "{}", summary.item_label(0, &h.home));
    }

    #[test]
    fn a_blocked_port_that_is_not_about_space_is_not_offered_the_link_fallback() {
        let h = fake_home("engine_exists");
        let mut plans = plans(&h, None);
        std::fs::create_dir_all(&plans[0].request.to).unwrap();
        plans[0] = super::super::plan::plan_port(&plans[0].request, None, SteamWrite::Now);
        let summary = move_selected(&DryRunEngine, plans, &|| false, &|_| {}, &NoPrompts);
        assert!(matches!(summary.items[0].outcome, PortOutcome::Refused { space: false, .. }));
        assert_eq!(summary.link_candidates(), Vec::<usize>::new());
    }

    #[test]
    fn a_cancel_is_honoured_between_ports_and_never_in_the_middle_of_one() {
        let flag = Arc::new(AtomicBool::new(false));
        let h = fake_home("engine_cancel");
        let engine = Recorder { seen: Mutex::new(Vec::new()), cancel_after_first: Some(flag.clone()) };
        let summary = move_selected(&engine, plans(&h, None), &|| flag.load(Ordering::SeqCst), &|_| {}, &NoPrompts);
        assert!(summary.cancelled);
        assert_eq!(*engine.seen.lock().unwrap(), ["Dusklight"], "the second port never started");
        assert!(matches!(summary.items[0].outcome, PortOutcome::Moved { .. }), "the first one ran to the end");
        assert_eq!(summary.items[1].outcome, PortOutcome::NotStarted);
        assert_eq!(summary.title(), "Import stopped");
        assert!(summary.text(&h.home).contains("Stopped before the rest: you cancelled between ports."));
        assert!(summary.item_label(1, &h.home).ends_with("not started (cancelled)"));
    }

    #[test]
    fn progress_shows_what_the_engine_says_it_is_doing() {
        let h = fake_home("engine_progress");
        let engine = Recorder { seen: Mutex::new(Vec::new()), cancel_after_first: None };
        let lines = RefCell::new(Vec::new());
        move_selected(&engine, plans(&h, None), &|| false, &|t| lines.borrow_mut().push(t), &NoPrompts);
        let all = lines.into_inner();
        assert!(all.iter().any(|t| t == "Dusklight: copying...\nShip of Harkinian: waiting"), "{all:#?}");
        assert_eq!(all.last().unwrap(), "Dusklight: done\nShip of Harkinian: done");
    }

    #[test]
    fn the_summary_names_moved_kept_steam_and_problem_counts() {
        let h = fake_home("engine_summary");
        let mut p = plans(&h, None);
        let items = vec![
            SummaryItem { plan: p.remove(1), outcome: PortOutcome::Moved { steam: SteamResult::Queued(1), old_copy_removed: false } },
            SummaryItem { plan: p.remove(0), outcome: PortOutcome::Failed("the copy failed a check".into()) },
        ];
        let summary = ImportSummary { dry_run: false, items, cancelled: false };
        let text = summary.text(&h.home);
        assert_eq!(summary.title(), "Import finished");
        assert!(text.starts_with("Moved 1 of 2 ports."), "{text}");
        assert!(text.contains("1 old copy kept (8 KB). Nothing was deleted: once you have checked the ports, remove it yourself"), "{text}");
        assert!(text.contains("  ~/Applications/SoH\n"), "{text}");
        assert!(text.contains("Steam shortcuts: 1 queued for Desktop Mode."), "{text}");
        assert!(text.contains("1 port had a problem (listed below)."), "{text}");
        assert!(text.contains("Ship of Harkinian: moved, old copy kept at ~/Applications/SoH (8 KB); 1 Steam shortcut queued for Desktop Mode"), "{text}");
        assert!(text.contains("Dusklight: problem, the copy failed a check"), "{text}");
    }

    #[test]
    fn a_removed_old_copy_is_reported_as_removed_at_the_users_request() {
        let h = fake_home("engine_removed");
        let mut p = plans(&h, None);
        let item = SummaryItem { plan: p.remove(1), outcome: PortOutcome::Moved { steam: SteamResult::Repointed(2), old_copy_removed: true } };
        let summary = ImportSummary { dry_run: false, items: vec![item], cancelled: false };
        assert!(summary.kept_old_copies().is_empty());
        let text = summary.text(&h.home);
        assert!(text.contains("old copy removed at your request; 2 Steam shortcuts repointed"), "{text}");
        assert!(!text.contains("kept"), "{text}");
        assert_eq!(summary.steam_counts(), (2, 0, 0));
    }

    #[test]
    fn the_safe_answers_stop_on_a_problem_and_keep_the_old_copy() {
        let size = Size::default();
        assert!(!NoPrompts.game_file_problem("x", &["oot.o2r is missing".into()]));
        assert!(!NoPrompts.remove_old_copy("x", Path::new("/a"), size));
    }
}
