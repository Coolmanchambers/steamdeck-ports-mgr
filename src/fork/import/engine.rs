//! The one door between the Import screen and whatever actually moves a port.
//!
//! `move_selected` runs a batch port by port: it publishes a status line per port, checks for a cancel
//! between ports (never in the middle of one), and gathers what each engine reports into an
//! `ImportSummary`. The screen knows nothing about copying: it builds plans, calls `move_selected` on a
//! worker thread and shows the summary.
//!
//! The real engine is `real_engine::RealEngine` (the port mover: copy, verify with the port's game files
//! before and after, switch over, Steam repoint or queue, old copy kept). `DryRunEngine` (plan and report,
//! write nothing) stays for tests. `Prompter` is how an engine asks the user, on the UI thread, while the
//! worker waits.

use super::plan::{PortPlan, SteamWrite};
use crate::fork::page::Mark;
use crate::fork::rom_screens::util::human_bytes;
use crate::fork::screens::short_path;
use crate::fork::port_mover::game_files::Issue;
use std::cell::RefCell;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

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
    /// Copied, verified, switched over. The old folder is still there: the screen asks about each one
    /// afterwards ("Remove the old copy now?", Keep for now preselected). `notes` are lines for the summary
    /// (Steam queued until Desktop Mode, game files named but not found, links adjusted).
    Moved { steam: SteamResult, notes: Vec<String> },
    /// The engine would not start; nothing changed. `space` is true for "not enough free space", the one
    /// case where linking the folder in place instead is offered.
    Refused { reason: String, space: bool },
    /// The user stopped this port at a prompt (a game file was missing or broken); nothing changed.
    /// `fix_in_game_files`: they chose to fix it in Settings > Game files first.
    Stopped { reason: String, fix_in_game_files: bool },
    /// Something went wrong part way; the text says what, and the old folder is untouched.
    Failed(String),
    /// A cancel came before this port started.
    NotStarted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameFileChoice {
    /// Skip this port; the user will fix it in Settings > Game files and import again.
    FixInGameFiles,
    /// Copy it although the files named are missing or broken.
    MoveAnyway,
    /// Stop the whole import here (the ports after this one are not started).
    Cancel,
}

/// The user's answers to the prompts an engine may raise on the worker thread.
pub trait Prompter: Sync {
    /// Game files the port needs are missing or broken (found before anything is copied).
    fn game_file_problem(&self, port: &str, issues: &[Issue]) -> GameFileChoice;
}

/// Answers the safe way: skip the port with the problem.
pub struct NoPrompts;

impl Prompter for NoPrompts {
    fn game_file_problem(&self, _: &str, _: &[Issue]) -> GameFileChoice {
        GameFileChoice::FixInGameFiles
    }
}

pub struct MoveContext<'a> {
    pub plan: &'a PortPlan,
    pub cancel: &'a AtomicBool,
    /// Says what the engine is doing now ("copying", "checking the copy"), shown after the port's name.
    pub progress: &'a dyn Fn(String),
    /// How far the copy of this port is, 0 to 1, when the engine knows (a progress bar).
    pub fraction: &'a dyn Fn(f32),
    pub prompts: &'a dyn Prompter,
}

pub trait MoveEngine: Send + Sync {
    fn is_dry_run(&self) -> bool;
    fn move_port(&self, ctx: &MoveContext) -> PortOutcome;
}

/// Reports the plan and writes nothing (tests).
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
        PortOutcome::Stopped { .. } => "stopped",
        PortOutcome::Failed(_) => "problem",
        PortOutcome::NotStarted => "not started",
    }
}

/// Runs the plans in order. `publish` gets the whole status text (one line per port) after every change.
/// `cancel` is looked at before each port; the engine may also watch it inside a port (the copy stops and
/// is removed). An engine can set it (the user chose Cancel at a prompt) to stop the ports after it.
pub fn move_selected(engine: &dyn MoveEngine, plans: Vec<PortPlan>, cancel: &AtomicBool, publish: &dyn Fn(String), prompts: &dyn Prompter) -> ImportSummary {
    move_selected_with(engine, plans, cancel, publish, &|_| {}, prompts)
}

/// [`move_selected`], also saying how far the whole import is (0 to 1) as each port is copied.
pub fn move_selected_with(engine: &dyn MoveEngine, plans: Vec<PortPlan>, cancel: &AtomicBool, publish: &dyn Fn(String), whole: &dyn Fn(f32), prompts: &dyn Prompter) -> ImportSummary {
    let count = plans.len().max(1) as f32;
    let board = RefCell::new(Board { names: plans.iter().map(|p| p.request.port_name.clone()).collect(), status: vec![Status::Waiting; plans.len()] });
    publish(board.borrow().render());
    let mut summary = ImportSummary { dry_run: engine.is_dry_run(), items: Vec::new(), cancelled: false };
    for (i, plan) in plans.into_iter().enumerate() {
        if summary.cancelled || cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            board.borrow_mut().status[i] = Status::Finished(finished_text(&PortOutcome::NotStarted).into());
            summary.items.push(SummaryItem { plan, outcome: PortOutcome::NotStarted });
            continue;
        }
        board.borrow_mut().status[i] = Status::Working(String::new());
        publish(board.borrow().render());
        whole(i as f32 / count);
        let fraction = |f: f32| whole((i as f32 + f.clamp(0.0, 1.0)) / count);
        let progress = |note: String| {
            board.borrow_mut().status[i] = Status::Working(note);
            publish(board.borrow().render());
        };
        let outcome = engine.move_port(&MoveContext { plan: &plan, cancel, progress: &progress, fraction: &fraction, prompts });
        board.borrow_mut().status[i] = Status::Finished(finished_text(&outcome).into());
        summary.items.push(SummaryItem { plan, outcome });
    }
    publish(board.borrow().render());
    whole(1.0);
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

    /// True when the user chose "Fix in Game files" for some port.
    pub fn wants_game_files(&self) -> bool {
        self.items.iter().any(|i| matches!(i.outcome, PortOutcome::Stopped { fix_in_game_files: true, .. }))
    }

    pub fn problem_count(&self) -> usize {
        self.items.iter().filter(|i| matches!(i.outcome, PortOutcome::Failed(_) | PortOutcome::Stopped { .. } | PortOutcome::Refused { .. })).count()
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
            PortOutcome::Moved { steam, .. } => format!("{name}: moved (from {}, {size}){}", short_path(&item.plan.request.from, home), steam_clause(steam)),
            PortOutcome::Refused { reason, .. } => {
                let reason = item.plan.block.as_ref().map_or_else(|| reason.clone(), |b| b.text(home));
                format!("{name}: not moved, {reason}")
            }
            PortOutcome::Stopped { reason, .. } => format!("{name}: stopped, {reason}"),
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

    /// One short line for each port that did not move, without paths (the long text, with them, is behind Details).
    pub fn short_problems(&self) -> Vec<String> {
        self.short_problem_lines().into_iter().map(|(_, l)| l).collect()
    }

    /// [`ImportSummary::short_problems`] with the mark each line is drawn with: a port that was not started because
    /// the user cancelled is skipped; every other one is a problem.
    pub fn short_problem_lines(&self) -> Vec<(Mark, String)> {
        self.items
            .iter()
            .filter_map(|item| {
                let name = &item.plan.request.port_name;
                match &item.outcome {
                    PortOutcome::Refused { space: true, .. } => Some((Mark::Problem, format!("{name} was not moved: not enough room."))),
                    PortOutcome::Refused { .. } => Some((Mark::Problem, format!("{name} was not moved. Details say why."))),
                    PortOutcome::Stopped { .. } => Some((Mark::Problem, format!("{name} was stopped: a game file needs a look."))),
                    PortOutcome::Failed(_) => Some((Mark::Problem, format!("{name} had a problem and was not moved. Details say why."))),
                    PortOutcome::NotStarted => Some((Mark::Skipped, format!("{name} was not started (you cancelled)."))),
                    PortOutcome::Planned | PortOutcome::Moved { .. } => None,
                }
            })
            .collect()
    }

    /// One short line about Steam shortcuts that followed the moved ports, when any did.
    pub fn short_steam(&self) -> Option<String> {
        self.short_steam_line().map(|(_, l)| l)
    }

    /// [`ImportSummary::short_steam`] with its mark: a shortcut that could not be changed is a problem, one still
    /// queued is waiting, else done.
    pub fn short_steam_line(&self) -> Option<(Mark, String)> {
        let (_, queued, failed) = self.steam_counts();
        let mark = if failed > 0 { Mark::Problem } else if queued > 0 { Mark::Waiting } else { Mark::Done };
        self.short_steam_text().map(|l| (mark, l))
    }

    fn short_steam_text(&self) -> Option<String> {
        let (repointed, queued, failed) = self.steam_counts();
        let mut parts = Vec::new();
        if repointed > 0 {
            parts.push(format!("{repointed} followed the move"));
        }
        if queued > 0 {
            parts.push(format!("{queued} will follow in Desktop Mode"));
        }
        if failed > 0 {
            parts.push(format!("{failed} could not be changed"));
        }
        (!parts.is_empty()).then(|| format!("Steam shortcuts: {}.", parts.join(", ")))
    }

    /// The totals under the title: what moved, what stayed behind, Steam, problems.
    pub fn totals(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.dry_run {
            out.push(format!("Preview only: {} port{} would be moved. Nothing was copied or changed.", self.planned_count(), if self.planned_count() == 1 { "" } else { "s" }));
        } else {
            out.push(format!("Moved {} of {} port{}.", self.moved_count(), self.items.len(), if self.items.len() == 1 { "" } else { "s" }));
        }
        if self.cancelled {
            out.push("Stopped: you cancelled. The copy in progress was removed; ports already moved stay moved.".to_string());
        }
        let moved: Vec<&SummaryItem> = self.moved().collect();
        if !moved.is_empty() {
            let bytes: u64 = moved.iter().map(|i| i.plan.request.size.bytes).sum();
            out.push(format!(
                "Old folder{} from this import: {} ({}). Nothing is removed unless you choose to remove it; whatever is kept stays listed under Settings > Ports folder > Old copies.",
                if moved.len() == 1 { "" } else { "s" },
                moved.len(),
                human_bytes(bytes)
            ));
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
        let mut lines = self.totals();
        lines.push(String::new());
        lines.extend((0..self.items.len()).map(|i| self.item_label(i, home)));
        for item in self.moved() {
            if let PortOutcome::Moved { notes, .. } = &item.outcome {
                if !notes.is_empty() {
                    lines.push(String::new());
                    lines.push(format!("{}:", item.plan.request.port_name));
                    lines.extend(notes.iter().cloned());
                }
            }
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::super::plan::{build_requests, plan_all, MoveRequest};
    use super::super::scan::fixtures::*;
    use super::super::scan::Found;
    use super::*;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::sync::Mutex;

    static NO_CANCEL: AtomicBool = AtomicBool::new(false);

    fn plans(h: &Home, free: Option<u64>) -> Vec<PortPlan> {
        use crate::fork::import::folders::ScanFolders;
        use crate::fork::import::scan::{read_shortcuts, run_scan, ScanInput};
        use crate::fork::settings::ForkSettings;
        let input = ScanInput {
            roots: ScanFolders::default().roots(&h.home, &h.library),
            catalog: catalog(),
            library_dir: h.library.clone(),
            home: h.home.clone(),
            deeper: false,
            shortcuts: read_shortcuts(&h.home, &ForkSettings::load(&h.home.join("s.json"))),
            ..Default::default()
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
        cancel_after_first: bool,
    }

    impl MoveEngine for Recorder {
        fn is_dry_run(&self) -> bool {
            false
        }

        fn move_port(&self, ctx: &MoveContext) -> PortOutcome {
            self.seen.lock().unwrap().push(ctx.plan.request.port_name.clone());
            (ctx.progress)("copying".into());
            if self.cancel_after_first {
                ctx.cancel.store(true, Ordering::SeqCst);
            }
            let steam = if ctx.plan.steam.is_empty() { SteamResult::NoShortcut } else { SteamResult::Repointed(ctx.plan.steam.len()) };
            PortOutcome::Moved { steam, notes: Vec::new() }
        }
    }

    #[test]
    fn the_whole_import_reports_how_far_it_is_from_0_to_1() {
        let h = fake_home("engine_fraction");
        let seen = RefCell::new(Vec::new());
        move_selected_with(&DryRunEngine, plans(&h, None), &NO_CANCEL, &|_| {}, &|f| seen.borrow_mut().push(f), &NoPrompts);
        let seen = seen.into_inner();
        assert_eq!(seen.first(), Some(&0.0));
        assert_eq!(seen.last(), Some(&1.0));
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "never goes back: {seen:?}");
        assert!(seen.contains(&0.5), "the second of two ports starts half way: {seen:?}");
    }

    #[test]
    fn the_dry_run_plans_every_port_and_writes_nothing() {
        let h = fake_home("engine_dry");
        let before = snapshot(&h.home);
        let lines = RefCell::new(Vec::new());
        let summary = move_selected(&DryRunEngine, plans(&h, None), &NO_CANCEL, &|t| lines.borrow_mut().push(t), &NoPrompts);
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
        let summary = move_selected(&DryRunEngine, plans(&h, None), &NO_CANCEL, &|_| {}, &NoPrompts);
        let text = summary.text(&h.home);
        assert_eq!(summary.title(), "Import preview");
        assert!(text.starts_with("Preview only: 2 ports would be moved. Nothing was copied or changed."), "{text}");
        assert!(text.contains("Dusklight: would move 700 B\n"), "{text}");
        assert!(text.contains("Ship of Harkinian: would move 8 KB; 1 Steam shortcut would be repointed"), "{text}");
    }

    #[test]
    fn a_dry_run_refusal_for_space_makes_the_port_a_link_candidate() {
        let h = fake_home("engine_space");
        let summary = move_selected(&DryRunEngine, plans(&h, Some(100)), &NO_CANCEL, &|_| {}, &NoPrompts);
        assert!(summary.items.iter().all(|i| matches!(i.outcome, PortOutcome::Refused { space: true, .. })));
        assert_eq!(summary.link_candidates(), [0, 1]);
        assert_eq!(summary.problem_count(), 2);
        assert!(summary.item_label(0, &h.home).starts_with(&format!("Dusklight: not moved, not enough free space: needs 700 B plus {} spare, 100 B free", human_bytes(crate::fork::import::plan::SPACE_MARGIN))), "{}", summary.item_label(0, &h.home));
    }

    #[test]
    fn a_blocked_port_that_is_not_about_space_is_not_offered_the_link_fallback() {
        let h = fake_home("engine_exists");
        let mut plans = plans(&h, None);
        std::fs::create_dir_all(&plans[0].request.to).unwrap();
        plans[0] = super::super::plan::plan_port(&plans[0].request, None, SteamWrite::Now);
        let summary = move_selected(&DryRunEngine, plans, &NO_CANCEL, &|_| {}, &NoPrompts);
        assert!(matches!(summary.items[0].outcome, PortOutcome::Refused { space: false, .. }));
        assert_eq!(summary.link_candidates(), Vec::<usize>::new());
    }

    #[test]
    fn a_cancel_is_honoured_between_ports_and_never_in_the_middle_of_one() {
        let flag = AtomicBool::new(false);
        let h = fake_home("engine_cancel");
        let engine = Recorder { seen: Mutex::new(Vec::new()), cancel_after_first: true };
        let summary = move_selected(&engine, plans(&h, None), &flag, &|_| {}, &NoPrompts);
        assert!(summary.cancelled);
        assert_eq!(*engine.seen.lock().unwrap(), ["Dusklight"], "the second port never started");
        assert!(matches!(summary.items[0].outcome, PortOutcome::Moved { .. }), "the first one ran to the end");
        assert_eq!(summary.items[1].outcome, PortOutcome::NotStarted);
        assert_eq!(summary.title(), "Import stopped");
        assert!(summary.text(&h.home).contains("Stopped: you cancelled. The copy in progress was removed; ports already moved stay moved."));
        assert!(summary.item_label(1, &h.home).ends_with("not started (cancelled)"));
    }

    #[test]
    fn progress_shows_what_the_engine_says_it_is_doing() {
        let h = fake_home("engine_progress");
        let engine = Recorder { seen: Mutex::new(Vec::new()), cancel_after_first: false };
        let lines = RefCell::new(Vec::new());
        move_selected(&engine, plans(&h, None), &NO_CANCEL, &|t| lines.borrow_mut().push(t), &NoPrompts);
        let all = lines.into_inner();
        assert!(all.iter().any(|t| t == "Dusklight: copying...\nShip of Harkinian: waiting"), "{all:#?}");
        assert_eq!(all.last().unwrap(), "Dusklight: done\nShip of Harkinian: done");
    }

    #[test]
    fn the_summary_names_moved_kept_steam_and_problem_counts() {
        let h = fake_home("engine_summary");
        let mut p = plans(&h, None);
        let items = vec![
            SummaryItem { plan: p.remove(1), outcome: PortOutcome::Moved { steam: SteamResult::Queued(1), notes: vec!["Steam: queued until Desktop Mode.".into()] } },
            SummaryItem { plan: p.remove(0), outcome: PortOutcome::Failed("the copy failed a check".into()) },
        ];
        let summary = ImportSummary { dry_run: false, items, cancelled: false };
        let text = summary.text(&h.home);
        assert_eq!(summary.title(), "Import finished");
        assert!(text.starts_with("Moved 1 of 2 ports."), "{text}");
        assert!(text.contains("Old folder from this import: 1 (8 KB). Nothing is removed unless you choose to remove it;"), "{text}");
        assert!(text.contains("Steam shortcuts: 1 queued for Desktop Mode."), "{text}");
        assert!(text.contains("1 port had a problem (listed below)."), "{text}");
        assert!(text.contains("Ship of Harkinian: moved (from ~/Applications/SoH, 8 KB); 1 Steam shortcut queued for Desktop Mode"), "{text}");
        assert!(text.contains("\nShip of Harkinian:\nSteam: queued until Desktop Mode."), "{text}");
        assert!(text.contains("Dusklight: problem, the copy failed a check"), "{text}");
    }

    #[test]
    fn the_safe_answer_skips_the_port_with_the_problem() {
        let issue = Issue { file: "oot.o2r".into(), expected_at: PathBuf::from("/x/oot.o2r"), problem: "is missing".into() };
        assert_eq!(NoPrompts.game_file_problem("x", &[issue]), GameFileChoice::FixInGameFiles);
    }

    #[test]
    fn a_fix_in_game_files_stop_is_a_problem_that_asks_for_the_game_files_screen() {
        let h = fake_home("engine_fix");
        let mut p = plans(&h, None);
        let items = vec![SummaryItem { plan: p.remove(0), outcome: PortOutcome::Stopped { reason: "fix its game files, then import again".into(), fix_in_game_files: true } }];
        let summary = ImportSummary { dry_run: false, items, cancelled: false };
        assert!(summary.wants_game_files());
        assert_eq!(summary.problem_count(), 1);
        assert!(summary.item_label(0, &h.home).starts_with("Dusklight: stopped, fix its game files"));
    }

    #[test]
    fn the_short_problem_lines_have_no_paths_and_the_steam_line_counts_what_followed() {
        let h = fake_home("engine_short");
        let summary = move_selected(&DryRunEngine, plans(&h, Some(100)), &NO_CANCEL, &|_| {}, &NoPrompts);
        let lines = summary.short_problems();
        assert_eq!(lines, ["Dusklight was not moved: not enough room.", "Ship of Harkinian was not moved: not enough room."]);
        assert!(lines.iter().all(|l| !l.contains('/') && !l.contains("~")));
        let mut blocked = plans(&h, None);
        std::fs::create_dir_all(&blocked[0].request.to).unwrap();
        blocked[0] = super::super::plan::plan_port(&blocked[0].request, None, SteamWrite::Now);
        let other = move_selected(&DryRunEngine, blocked, &NO_CANCEL, &|_| {}, &NoPrompts);
        assert_eq!(other.short_problems(), ["Dusklight was not moved. Details say why."]);
        assert_eq!(other.short_steam(), None, "a dry run moved nothing, so no shortcut followed");
        let cancelled = {
            let flag = AtomicBool::new(false);
            let engine = Recorder { seen: Mutex::new(Vec::new()), cancel_after_first: true };
            move_selected(&engine, plans(&fake_home("engine_short_cancel"), None), &flag, &|_| {}, &NoPrompts)
        };
        assert_eq!(cancelled.short_problems(), ["Ship of Harkinian was not started (you cancelled)."]);
        assert_eq!(cancelled.short_problem_lines()[0].0, Mark::Skipped, "cancelled is skipped, not a problem");
        assert!(other.short_problem_lines().iter().all(|(m, _)| *m == Mark::Problem));
    }
}
