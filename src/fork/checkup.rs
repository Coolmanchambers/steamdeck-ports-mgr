//! The "Fix problems" checkup: one look over the whole setup, in plain words, and which existing screen fixes each thing.
//!
//! The facts are gathered once into a plain [`Facts`] (`src/app/checkup_screen.rs` reads the disk, Steam's files and the
//! caches Home already keeps), and each check is then a pure function over them returning an [`Outcome`]: fine, a
//! problem (with its mark, one line, which ports and which fix), or "could not check" with the reason. Home's tile,
//! Settings' card and the checkup pages all read the same [`Results`], so they never disagree. Looking changes nothing:
//! this module has no way to write, and every fix is an existing screen that asks on its own page.
//!
//! The checks, in the order the pages visit them (see `docs/design/checkup-wizard.md`): [`Check::ALL`].

pub mod pages;

use crate::fork::page::Mark;
use crate::fork::port_mover::old_copies::Readiness;
use crate::fork::setup_wizard::files_state::FilesState;
use crate::fork::steam_icons::{IconFinding, IconState};
use crate::fork::steam_queue::QueuedKind;
use std::collections::HashSet;

/// A save backup older than this many days is a problem (the Saves card and the checkup agree).
pub const BACKUP_STALE_DAYS: u64 = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Check {
    PortsFolder,
    SteamWaiting,
    SteamIcons,
    SteamShortcuts,
    Programs,
    GameFiles,
    Saves,
    OldCopies,
    Updates,
    Backups,
}

impl Check {
    /// The order the pages visit them in.
    pub const ALL: [Check; 10] = [
        Check::PortsFolder,
        Check::SteamWaiting,
        Check::SteamIcons,
        Check::SteamShortcuts,
        Check::Programs,
        Check::GameFiles,
        Check::Saves,
        Check::OldCopies,
        Check::Updates,
        Check::Backups,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Check::PortsFolder => "Ports folder",
            Check::SteamWaiting => "Steam changes",
            Check::SteamIcons => "Steam icons",
            Check::SteamShortcuts => "Steam shortcuts",
            Check::Programs => "Program to start",
            Check::GameFiles => "Game files",
            Check::Saves => "Saves",
            Check::OldCopies => "Old copies",
            Check::Updates => "Updates",
            Check::Backups => "Save backups",
        }
    }

    /// Needs the ports folder to be there to say anything (its icons, programs, game files and saves live in it).
    fn needs_ports_folder(self) -> bool {
        matches!(self, Check::SteamIcons | Check::Programs | Check::GameFiles | Check::Saves | Check::OldCopies)
    }
}

/// Which existing screen fixes a problem. The screen layer opens it; `String` is the port's key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fix {
    /// The "ports folder is not available" screen (try again, choose another).
    PortsFolderGone,
    /// Settings > Ports folder (finish a switch).
    PortsFolderMenu,
    /// "Close Steam and update shortcuts" (Desktop Mode); Game Mode says how to finish.
    CloseSteam,
    SteamIcons,
    AddToSteam,
    PickProgram(String),
    GameFiles,
    FindSaves(String),
    OldCopies,
    /// Asks first on a page of its own, then the usual update.
    Update(String),
    /// Asks first on a page of its own, then the usual "back up saves now".
    BackUpNow,
    Cloud,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// `Mark::Problem` (red) only when a game will not start; the rest are things to do or wait for.
    pub mark: Mark,
    /// One line, no file paths: "Missing for Dusklight and 2 more".
    pub summary: String,
    /// Longer lines for the step page, with the real values behind it (an icon path, say).
    pub detail: Vec<String>,
    /// The keys of the ports it is about.
    pub ports: Vec<String>,
    pub fix: Fix,
    /// What the fix card says: "Close Steam and update shortcuts".
    pub fix_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Fine, with a few words ("Nothing waiting").
    Ok(String),
    Problem(Problem),
    /// Could not look, and why ("The ports folder is not there").
    Unknown(String),
}

impl Outcome {
    pub fn is_problem(&self) -> bool {
        matches!(self, Outcome::Problem(_))
    }

    /// The mark before the line: green check, the problem's own, or a dash for "could not check".
    pub fn mark(&self) -> Mark {
        match self {
            Outcome::Ok(_) => Mark::Done,
            Outcome::Problem(p) => p.mark,
            Outcome::Unknown(_) => Mark::Skipped,
        }
    }

    /// The words after "Title: ".
    pub fn words(&self) -> String {
        match self {
            Outcome::Ok(w) => w.clone(),
            Outcome::Problem(p) => p.summary.clone(),
            Outcome::Unknown(why) => format!("could not check ({why})"),
        }
    }
}

/// An installed port: its key and the name shown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Named {
    pub key: String,
    pub name: String,
}

impl Named {
    pub fn new(key: &str, name: &str) -> Named {
        Named { key: key.to_string(), name: name.to_string() }
    }
}

/// Whether the ports folder is there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Folder {
    #[default]
    Here,
    /// On a drive that is not mounted (the SD card is out).
    DriveOut,
    Gone,
    NotAFolder,
}

impl Folder {
    pub fn from_missing(m: Option<&crate::fork::ports_folder::Missing>) -> Folder {
        use crate::fork::ports_folder::Missing;
        match m {
            None => Folder::Here,
            Some(Missing::DriveNotMounted(_)) => Folder::DriveOut,
            Some(Missing::FolderGone) => Folder::Gone,
            Some(Missing::NotAFolder) => Folder::NotAFolder,
        }
    }
}

/// A Steam change waiting for Steam to close.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedFact {
    pub port_key: String,
    pub name: String,
    pub kind: QueuedKind,
}

/// What an old copy still needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyState {
    Ready,
    StartOnce,
    WaitingForSteam,
    /// The folder is already gone: only the record is left.
    Gone,
}

impl CopyState {
    pub fn of(readiness: Readiness) -> CopyState {
        match readiness {
            Readiness::Ready(_) => CopyState::Ready,
            Readiness::StartOnce => CopyState::StartOnce,
            Readiness::WaitingForSteam => CopyState::WaitingForSteam,
        }
    }

    fn line(self) -> &'static str {
        match self {
            CopyState::Ready => "Ready to remove",
            CopyState::StartOnce => "Start it once from its new place",
            CopyState::WaitingForSteam => "Waiting for Steam to update",
            CopyState::Gone => "Already gone, forget the record",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyFact {
    pub name: String,
    pub state: CopyState,
}

/// Everything the checks read, gathered once.
#[derive(Debug, Clone)]
pub struct Facts {
    /// Game Mode: the Steam fix says how to finish in Desktop Mode instead of doing it.
    pub game_mode: bool,
    /// The ports that are installed.
    pub installed: Vec<Named>,
    pub folder: Folder,
    /// Ports whose switch to a new folder was cut short (their names).
    pub unfinished_moves: Vec<String>,
    /// A Steam account folder was found on this device.
    pub steam_found: bool,
    pub queue: Result<Vec<QueuedFact>, String>,
    /// Every Steam shortcut that belongs to an installed port, with what its icon is.
    pub shortcuts: Result<Vec<IconFinding>, String>,
    /// Ports with more than one program to start and none remembered.
    pub programs: Vec<Named>,
    /// Ports whose game files are missing (red) or only named by title (not red).
    pub files: Vec<(Named, FilesState)>,
    /// Ports whose saves the app could not find.
    pub saves: Vec<Named>,
    pub old_copies: Result<Vec<CopyFact>, String>,
    /// Ports with an update waiting.
    pub updates: Vec<Named>,
    /// Whole days since the last "Back up saves now" zip; `None` when there is none.
    pub backup_days: Option<u64>,
    /// Cloud backup is signed in and reports a problem.
    pub cloud_problem: bool,
}

impl Default for Facts {
    fn default() -> Facts {
        Facts {
            game_mode: false,
            installed: Vec::new(),
            folder: Folder::Here,
            unfinished_moves: Vec::new(),
            steam_found: true,
            queue: Ok(Vec::new()),
            shortcuts: Ok(Vec::new()),
            programs: Vec::new(),
            files: Vec::new(),
            saves: Vec::new(),
            old_copies: Ok(Vec::new()),
            updates: Vec::new(),
            backup_days: None,
            cloud_problem: false,
        }
    }
}

/// "A", "A and B", "A, B and C", "A, B and 2 more".
pub fn names(list: &[String]) -> String {
    match list {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} and {b}"),
        [a, b, c] => format!("{a}, {b} and {c}"),
        [a, b, rest @ ..] => format!("{a}, {b} and {} more", rest.len()),
    }
}

fn named_list(list: &[Named]) -> String {
    names(&list.iter().map(|n| n.name.clone()).collect::<Vec<_>>())
}

fn keys(list: &[Named]) -> Vec<String> {
    list.iter().map(|n| n.key.clone()).collect()
}

fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

fn problem(mark: Mark, summary: String, ports: Vec<String>, fix: Fix, fix_label: &str) -> Outcome {
    Outcome::Problem(Problem { mark, summary, detail: Vec::new(), ports, fix, fix_label: fix_label.to_string() })
}

/// The result of every check, in [`Check::ALL`] order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Results {
    pub items: Vec<(Check, Outcome)>,
}

pub fn run(f: &Facts) -> Results {
    Results { items: Check::ALL.into_iter().map(|c| (c, check(c, f))).collect() }
}

impl Results {
    pub fn outcome(&self, check: Check) -> Option<&Outcome> {
        self.items.iter().find(|(c, _)| *c == check).map(|(_, o)| o)
    }

    /// The checks with something to fix, in order.
    pub fn problems(&self) -> Vec<(Check, &Problem)> {
        self.items.iter().filter_map(|(c, o)| if let Outcome::Problem(p) = o { Some((*c, p)) } else { None }).collect()
    }

    pub fn problem_count(&self) -> usize {
        self.items.iter().filter(|(_, o)| o.is_problem()).count()
    }

    /// A game will not start (red): the ports folder is missing, or a game file the catalog names is.
    pub fn any_red(&self) -> bool {
        self.problems().iter().any(|(_, p)| p.mark == Mark::Problem)
    }

    fn unknown_count(&self) -> usize {
        self.items.iter().filter(|(_, o)| matches!(o, Outcome::Unknown(_))).count()
    }

    /// The line Settings shows on its card: "2 problems", "Everything looks fine".
    pub fn summary(&self) -> String {
        match self.problem_count() {
            0 if self.unknown_count() > 0 => format!("Looks fine ({} could not be checked)", self.unknown_count()),
            0 => "Everything looks fine".to_string(),
            1 => "1 problem".to_string(),
            n => format!("{n} problems"),
        }
    }

    /// The tile Home shows: "Fix 2 problems" and what is first; `None` when there is nothing to fix.
    pub fn home_tile(&self) -> Option<HomeTile> {
        let problems = self.problems();
        let first = problems.first()?;
        let n = problems.len();
        let title = if n == 1 { "Fix 1 problem".to_string() } else { format!("Fix {n} problems") };
        let note = if n == 1 { first.0.title().to_string() } else { format!("{} and {} more", first.0.title(), n - 1) };
        Some(HomeTile { title, note, mark: if self.any_red() { Mark::Problem } else { Mark::ToDo } })
    }
}

/// What Home's checkup tile says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeTile {
    pub title: String,
    /// The badge line: which check, "Game files and 2 more".
    pub note: String,
    /// Red only when a game will not start.
    pub mark: Mark,
}

pub fn check(c: Check, f: &Facts) -> Outcome {
    if c.needs_ports_folder() && f.folder != Folder::Here {
        return Outcome::Unknown("the ports folder is not there".to_string());
    }
    match c {
        Check::PortsFolder => ports_folder(f),
        Check::SteamWaiting => steam_waiting(f),
        Check::SteamIcons => steam_icons(f),
        Check::SteamShortcuts => steam_shortcuts(f),
        Check::Programs => programs(f),
        Check::GameFiles => game_files(f),
        Check::Saves => saves(f),
        Check::OldCopies => old_copies(f),
        Check::Updates => updates(f),
        Check::Backups => backups(f),
    }
}

fn ports_folder(f: &Facts) -> Outcome {
    let all: Vec<String> = keys(&f.installed);
    match f.folder {
        Folder::DriveOut => problem(Mark::Problem, "The drive it is on is not there. Is the SD card in?".into(), all, Fix::PortsFolderGone, "Find the ports folder"),
        Folder::Gone => problem(Mark::Problem, "The folder is gone. It may have been moved, renamed or deleted.".into(), all, Fix::PortsFolderGone, "Find the ports folder"),
        Folder::NotAFolder => problem(Mark::Problem, "Something that is not a folder has its name.".into(), all, Fix::PortsFolderGone, "Find the ports folder"),
        Folder::Here if !f.unfinished_moves.is_empty() => {
            problem(Mark::ToDo, format!("A move did not finish: {}", names(&f.unfinished_moves)), Vec::new(), Fix::PortsFolderMenu, "Finish the move")
        }
        Folder::Here => Outcome::Ok("It is there".to_string()),
    }
}

fn steam_waiting(f: &Facts) -> Outcome {
    let queue = match &f.queue {
        Ok(q) => q,
        Err(why) => return Outcome::Unknown(why.clone()),
    };
    if queue.is_empty() {
        return Outcome::Ok("Nothing waiting".to_string());
    }
    let mut ports: Vec<String> = Vec::new();
    for q in queue {
        if !ports.contains(&q.name) {
            ports.push(q.name.clone());
        }
    }
    let label = if f.game_mode { "See how to finish this" } else { "Close Steam and update shortcuts" };
    let summary = format!("{} waiting for Steam to close: {}", count(queue.len(), "change", "changes"), names(&ports));
    let mut keys: Vec<String> = Vec::new();
    for q in queue {
        if !keys.contains(&q.port_key) {
            keys.push(q.port_key.clone());
        }
    }
    problem(Mark::Waiting, summary, keys, Fix::CloseSteam, label)
}

/// A few words for what is wrong with an icon (no path: the step page has it).
pub fn icon_words(state: &IconState) -> &'static str {
    match state {
        IconState::Ok(_) | IconState::Outside(_) => "icon is fine",
        IconState::Empty => "has no icon",
        IconState::Missing(_) => "icon file is missing",
        IconState::NotAnImage(_) => "icon file is not a picture",
        IconState::Quoted { .. } => "icon is stored the wrong way",
        IconState::InOldCopy(_) => "icon is stored in an old copy",
    }
}

fn steam_icons(f: &Facts) -> Outcome {
    let shortcuts = match &f.shortcuts {
        Ok(s) => s,
        Err(why) => return Outcome::Unknown(why.clone()),
    };
    if !f.steam_found {
        return Outcome::Unknown("Steam's folder was not found".to_string());
    }
    // A repair already waiting for Steam to close is counted under "Steam changes".
    let repair_queued: HashSet<&str> = match &f.queue {
        Ok(q) => q.iter().filter(|q| q.kind == QueuedKind::Icon).map(|q| q.port_key.as_str()).collect(),
        Err(_) => HashSet::new(),
    };
    let bad: Vec<&IconFinding> = shortcuts.iter().filter(|s| s.state.needs_fix() && !repair_queued.contains(s.port_key.as_str())).collect();
    if bad.is_empty() {
        return Outcome::Ok(if shortcuts.is_empty() { "No shortcuts yet".to_string() } else { "All fine".to_string() });
    }
    let mut ports: Vec<String> = Vec::new();
    let mut port_keys: Vec<String> = Vec::new();
    for b in &bad {
        if !port_keys.contains(&b.port_key) {
            port_keys.push(b.port_key.clone());
            ports.push(b.port_name.clone());
        }
    }
    let summary = match bad.as_slice() {
        [one] => format!("{}: {}", one.port_name, icon_words(&one.state)),
        many => format!("{} show nothing: {}", count(many.len(), "icon", "icons"), names(&ports)),
    };
    let detail = bad.iter().take(4).map(|b| b.describe()).collect();
    Outcome::Problem(Problem { mark: Mark::ToDo, summary, detail, ports: port_keys, fix: Fix::SteamIcons, fix_label: "Fix the icons".to_string() })
}

fn steam_shortcuts(f: &Facts) -> Outcome {
    let shortcuts = match &f.shortcuts {
        Ok(s) => s,
        Err(why) => return Outcome::Unknown(why.clone()),
    };
    if !f.steam_found {
        return Outcome::Unknown("Steam's folder was not found".to_string());
    }
    let in_steam: HashSet<&str> = shortcuts.iter().map(|s| s.port_key.as_str()).collect();
    // A port still waiting to be added is counted under "Steam changes".
    let adding: HashSet<&str> = match &f.queue {
        Ok(q) => q.iter().filter(|q| q.kind == QueuedKind::Add).map(|q| q.port_key.as_str()).collect(),
        Err(_) => HashSet::new(),
    };
    if in_steam.is_empty() {
        // Nobody asked for Steam shortcuts yet: not a thing to nag about.
        return Outcome::Ok("No ports in Steam yet".to_string());
    }
    let missing: Vec<Named> = f.installed.iter().filter(|p| !in_steam.contains(p.key.as_str()) && !adding.contains(p.key.as_str())).cloned().collect();
    if missing.is_empty() {
        return Outcome::Ok("Every port is in Steam".to_string());
    }
    problem(Mark::ToDo, format!("Not in Steam: {}", named_list(&missing)), keys(&missing), Fix::AddToSteam, "Add them to Steam")
}

fn programs(f: &Facts) -> Outcome {
    if f.programs.is_empty() {
        return Outcome::Ok("Nothing to choose".to_string());
    }
    let first = f.programs[0].clone();
    problem(Mark::ToDo, format!("More than one program to start: {}", named_list(&f.programs)), keys(&f.programs), Fix::PickProgram(first.key), "Choose the program")
}

fn game_files(f: &Facts) -> Outcome {
    if f.files.is_empty() {
        return Outcome::Ok("All there".to_string());
    }
    let missing: Vec<Named> = f.files.iter().filter(|(_, s)| *s == FilesState::Missing).map(|(n, _)| n.clone()).collect();
    let to_add: Vec<Named> = f.files.iter().filter(|(_, s)| *s == FilesState::Unverifiable).map(|(n, _)| n.clone()).collect();
    let summary = match (missing.is_empty(), to_add.is_empty()) {
        (false, true) => format!("Missing for {}", named_list(&missing)),
        (false, false) => format!("Missing for {}; {} to add", named_list(&missing), named_list(&to_add)),
        _ => format!("Tell me where they are: {}", named_list(&to_add)),
    };
    let all: Vec<Named> = f.files.iter().map(|(n, _)| n.clone()).collect();
    problem(if missing.is_empty() { Mark::ToDo } else { Mark::Problem }, summary, keys(&all), Fix::GameFiles, "Find the game files")
}

fn saves(f: &Facts) -> Outcome {
    if f.saves.is_empty() {
        return Outcome::Ok("All found".to_string());
    }
    problem(Mark::ToDo, format!("Not found for {}", named_list(&f.saves)), keys(&f.saves), Fix::FindSaves(f.saves[0].key.clone()), "Find the saves")
}

fn old_copies(f: &Facts) -> Outcome {
    let copies = match &f.old_copies {
        Ok(c) => c,
        Err(why) => return Outcome::Unknown(why.clone()),
    };
    if copies.is_empty() {
        return Outcome::Ok("None kept".to_string());
    }
    let n = |s: CopyState| copies.iter().filter(|c| c.state == s).count();
    let summary = match copies.as_slice() {
        [one] => format!("{}: {}", one.name, one.state.line()),
        many => {
            let mut parts: Vec<String> = Vec::new();
            for (s, words) in [(CopyState::Ready, "ready to remove"), (CopyState::StartOnce, "to start once"), (CopyState::WaitingForSteam, "waiting for Steam"), (CopyState::Gone, "already gone")] {
                if n(s) > 0 {
                    parts.push(format!("{} {words}", n(s)));
                }
            }
            format!("{} kept: {}", count(many.len(), "old copy", "old copies"), parts.join(", "))
        }
    };
    let can_act = n(CopyState::Ready) + n(CopyState::Gone) > 0;
    let detail = copies.iter().take(4).map(|c| format!("{}: {}", c.name, c.state.line())).collect();
    Outcome::Problem(Problem { mark: if can_act { Mark::ToDo } else { Mark::Waiting }, summary, detail, ports: Vec::new(), fix: Fix::OldCopies, fix_label: "Look at the old copies".to_string() })
}

fn updates(f: &Facts) -> Outcome {
    if f.updates.is_empty() {
        return Outcome::Ok("Up to date".to_string());
    }
    let first = &f.updates[0];
    problem(Mark::ToDo, format!("Update ready: {}", named_list(&f.updates)), keys(&f.updates), Fix::Update(first.key.clone()), &format!("Update {}", first.name))
}

fn backups(f: &Facts) -> Outcome {
    if f.installed.is_empty() {
        return Outcome::Ok("No ports yet".to_string());
    }
    let stale = match f.backup_days {
        None => Some("No save backup yet".to_string()),
        Some(d) if d > BACKUP_STALE_DAYS => Some(format!("Last save backup was {d} days ago")),
        Some(_) => None,
    };
    match (stale, f.cloud_problem) {
        (Some(words), cloud) => {
            let summary = if cloud { format!("{words}; cloud backup has a problem") } else { words };
            problem(Mark::ToDo, summary, Vec::new(), Fix::BackUpNow, "Back up saves now")
        }
        (None, true) => problem(Mark::ToDo, "Cloud backup has a problem".to_string(), Vec::new(), Fix::Cloud, "Look at cloud backup"),
        (None, false) => Outcome::Ok(match f.backup_days {
            Some(0) => "Backed up today".to_string(),
            Some(1) => "Backed up yesterday".to_string(),
            Some(d) => format!("Backed up {d} days ago"),
            None => "Backed up".to_string(),
        }),
    }
}

/// The program choice question for one port's folder: how many launch candidates it has at its shallowest depth. A walk
/// that stops after `MAX_WALK` entries and `MAX_DEPTH` folders, so a huge game folder (discs, bundled Java) never makes
/// the checkup slow. `Err` when the folder cannot be read.
pub fn shallow_programs(dir: &std::path::Path) -> Result<Vec<std::path::PathBuf>, String> {
    const MAX_WALK: usize = 4000;
    const MAX_DEPTH: usize = 5;
    std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    let mut found: Vec<std::path::PathBuf> = Vec::new();
    let mut seen = 0usize;
    let mut stack: Vec<(std::path::PathBuf, usize)> = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_WALK {
                break;
            }
            let path = entry.path();
            let Ok(t) = entry.file_type() else { continue };
            if t.is_dir() {
                if depth + 1 < MAX_DEPTH && entry.file_name() != ".previous" {
                    stack.push((path, depth + 1));
                }
            } else if t.is_file() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if is_program(&path) && !name.contains("unins") {
                    found.push(path);
                }
            }
        }
    }
    Ok(crate::fork::program_detect::shallowest(dir, found))
}

#[cfg(target_os = "linux")]
fn is_program(path: &std::path::Path) -> bool {
    crate::fork::program_detect::is_linux_program(path)
}

#[cfg(not(target_os = "linux"))]
fn is_program(path: &std::path::Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["exe", "lnk", "bat", "url"].iter().any(|c| e.eq_ignore_ascii_case(c)))
}

#[cfg(test)]
mod tests;
