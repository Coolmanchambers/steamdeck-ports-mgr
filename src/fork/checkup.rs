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

    /// A stable name for the settings file ("Skip this one" is remembered by it).
    pub fn key(self) -> &'static str {
        match self {
            Check::PortsFolder => "ports_folder",
            Check::SteamWaiting => "steam_waiting",
            Check::SteamIcons => "steam_icons",
            Check::SteamShortcuts => "steam_shortcuts",
            Check::Programs => "programs",
            Check::GameFiles => "game_files",
            Check::Saves => "saves",
            Check::OldCopies => "old_copies",
            Check::Updates => "updates",
            Check::Backups => "backups",
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
    /// Asks first on a page of its own, then points the shortcuts that still start an old copy at the new one (the port's
    /// ports are the problem's `ports`): written when Steam is closed, else queued and the Close Steam flow follows.
    Repoint,
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

/// A menu an existing screen leads back to when the user backs out of it (the checkup takes over from these).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    /// A group's page of Settings.
    SettingsGroup(crate::fork::settings_menu::Group),
    /// Settings > Ports folder.
    PortsFolder,
}

impl Fix {
    /// Whether the screen this fix opens leads back to `menu`, so opening that menu means "the user left the fix": come back
    /// to the checkup. Any other menu is part of the flow (the ports folder flow handing over to Game files, say) and opens
    /// as it always did. Like `setup_wizard::flow::menu_returns_to_wizard`.
    pub fn returns_to(&self, menu: Menu) -> bool {
        use crate::fork::settings_menu::Group;
        match (self, menu) {
            (Fix::PortsFolderGone | Fix::PortsFolderMenu | Fix::GameFiles | Fix::OldCopies, Menu::SettingsGroup(Group::Ports)) => true,
            (Fix::Cloud, Menu::SettingsGroup(Group::Saves)) => true,
            // The old copies list's Back row opens the Ports folder menu: that is leaving it.
            (Fix::OldCopies, Menu::PortsFolder) => true,
            _ => false,
        }
    }
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
    /// Shown in the checkup and on Settings' card, but not worth a tile on Home by itself: nothing can be done about it
    /// right now (Steam changes in Game Mode, an old copy still waiting) or it is a nudge (saves not found, no backup).
    /// Red problems always count.
    pub advisory: bool,
    /// Finishing it needs Desktop Mode (Steam cannot be closed in Game Mode): after its page, "Next problem" leads.
    pub desktop_only: bool,
}

impl Problem {
    /// What "Skip this one" remembers: the line and the ports it is about. When either changes (a new port, another
    /// state) the problem is new again.
    pub fn fingerprint(&self) -> String {
        format!("{}|{}", self.summary, self.ports.join(","))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Fine, with a few words ("Nothing waiting").
    Ok(String),
    Problem(Problem),
    /// The fix was made and is waiting for Steam to close (an icon repair queued while Steam runs): not fine yet, not a
    /// problem of its own (the Steam changes line counts it). Carries what the card to finish it says.
    Queued { summary: String, fix_label: String, desktop_only: bool },
    /// A problem the user chose to skip, as it stood when they did: shown as skipped, not counted.
    Skipped(Problem),
    /// Could not look, and why ("The ports folder is not there").
    Unknown(String),
}

impl Outcome {
    /// Something to fix that is not skipped.
    pub fn is_problem(&self) -> bool {
        matches!(self, Outcome::Problem(_))
    }

    /// Anything but fine: a problem, a fix still queued, a skipped problem, a check that could not run.
    pub fn is_not_fine(&self) -> bool {
        !matches!(self, Outcome::Ok(_))
    }

    /// The mark before the line: green check, the problem's own, a clock for a queued fix, a dash for "skipped" or
    /// "could not check".
    pub fn mark(&self) -> Mark {
        match self {
            Outcome::Ok(_) => Mark::Done,
            Outcome::Problem(p) => p.mark,
            Outcome::Queued { .. } => Mark::Waiting,
            Outcome::Skipped(_) | Outcome::Unknown(_) => Mark::Skipped,
        }
    }

    /// The words after "Title: ".
    pub fn words(&self) -> String {
        match self {
            Outcome::Ok(w) => w.clone(),
            Outcome::Problem(p) => p.summary.clone(),
            Outcome::Queued { summary, .. } => summary.clone(),
            Outcome::Skipped(p) => format!("skipped ({})", p.summary),
            Outcome::Unknown(why) => format!("could not check ({why})"),
        }
    }
}

/// What "Skip this one" remembered: the check's key and the problem's fingerprint when it was skipped.
pub type Skips = std::collections::BTreeMap<String, String>;

pub fn skips_from_json(v: Option<&serde_json::Value>) -> Skips {
    v.and_then(serde_json::Value::as_object).map(|o| o.iter().filter_map(|(k, f)| Some((k.clone(), f.as_str()?.to_string()))).collect()).unwrap_or_default()
}

pub fn skips_to_json(s: &Skips) -> serde_json::Value {
    serde_json::Value::Object(s.iter().map(|(k, f)| (k.clone(), serde_json::Value::String(f.clone()))).collect())
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

    /// One sentence about the old copy of `name` (the name comes first or last so a colon inside it reads fine).
    fn sentence(self, name: &str) -> String {
        match self {
            CopyState::Ready => format!("The old copy of {name} can be removed"),
            CopyState::StartOnce => format!("Start {name} once, from here or from Steam"),
            CopyState::WaitingForSteam => format!("Steam still has to update {name}"),
            CopyState::Gone => format!("The old copy of {name} is already gone"),
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
    /// Cloud backup is signed in and copied saves successfully recently: saves are safe without a zip here.
    pub cloud_ok: bool,
    /// How many installed ports have a save folder the app knows (with files in it): what "Back up saves now" can copy.
    pub saves_known: usize,
    /// Problems the user chose to skip, by check (what they were when skipped).
    pub skips: Skips,
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
            cloud_ok: false,
            saves_known: 0,
            skips: Skips::new(),
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
    Outcome::Problem(Problem { mark, summary, detail: Vec::new(), ports, fix, fix_label: fix_label.to_string(), advisory: false, desktop_only: false })
}

/// A problem that is a nudge, not something worth a tile on Home (it still counts in the checkup and Settings' line).
fn advisory(o: Outcome) -> Outcome {
    match o {
        Outcome::Problem(p) => Outcome::Problem(Problem { advisory: true, ..p }),
        other => other,
    }
}

/// The result of every check, in [`Check::ALL`] order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Results {
    pub items: Vec<(Check, Outcome)>,
}

pub fn run(f: &Facts) -> Results {
    Results { items: Check::ALL.into_iter().map(|c| (c, check(c, f))).collect() }.with_skips(&f.skips)
}

impl Results {
    pub fn outcome(&self, check: Check) -> Option<&Outcome> {
        self.items.iter().find(|(c, _)| *c == check).map(|(_, o)| o)
    }

    /// A problem the user skipped, as it stood then, shows as skipped for as long as it reads the same. Anything that
    /// changed it (another port, another state) makes it a problem again.
    pub fn with_skips(mut self, skips: &Skips) -> Results {
        for (c, o) in &mut self.items {
            if let Outcome::Problem(p) = o {
                if skips.get(c.key()).is_some_and(|f| *f == p.fingerprint()) {
                    *o = Outcome::Skipped(p.clone());
                }
            }
        }
        self
    }

    /// The checks with something to fix (not skipped), in order.
    pub fn problems(&self) -> Vec<(Check, &Problem)> {
        self.items.iter().filter_map(|(c, o)| if let Outcome::Problem(p) = o { Some((*c, p)) } else { None }).collect()
    }

    pub fn problem_count(&self) -> usize {
        self.items.iter().filter(|(_, o)| o.is_problem()).count()
    }

    pub fn skipped_count(&self) -> usize {
        self.items.iter().filter(|(_, o)| matches!(o, Outcome::Skipped(_))).count()
    }

    /// The problems that raise Home's tile: red ones, and those with a fix that works right now.
    pub fn home_problems(&self) -> Vec<(Check, &Problem)> {
        self.problems().into_iter().filter(|(_, p)| p.mark == Mark::Problem || !p.advisory).collect()
    }

    /// A game will not start (red): the ports folder is missing, or a game file the catalog names is.
    pub fn any_red(&self) -> bool {
        self.problems().iter().any(|(_, p)| p.mark == Mark::Problem)
    }

    fn unknown_count(&self) -> usize {
        self.items.iter().filter(|(_, o)| matches!(o, Outcome::Unknown(_))).count()
    }

    /// How many problems have a fix that works now (or are red): what the Home tile counts. The rest are reminders.
    pub fn to_do_now(&self) -> usize {
        self.home_problems().len()
    }

    /// How the problems split, for a line: "3 things to fix", or "2 to do now, 1 reminder" when some are only reminders.
    pub fn split_words(&self) -> String {
        let (n, now) = (self.problem_count(), self.to_do_now());
        let reminders = n - now;
        let r = if reminders == 1 { "1 reminder".to_string() } else { format!("{reminders} reminders") };
        match (n, now) {
            (1, 1) => "1 thing to fix".to_string(),
            (n, m) if n == m => format!("{n} things to fix"),
            (_, 0) => r,
            (_, m) => format!("{m} to do now, {r}"),
        }
    }

    /// The line Settings shows on its card: "6 things to fix", "5 to do now, 2 reminders", "Everything looks fine".
    pub fn summary(&self) -> String {
        let skipped = match self.skipped_count() {
            0 => String::new(),
            n => format!(", {n} skipped"),
        };
        match self.problem_count() {
            0 if self.unknown_count() > 0 => format!("Looks fine ({} could not be checked){skipped}", self.unknown_count()),
            0 => format!("Everything looks fine{skipped}"),
            _ => format!("{}{skipped}", self.split_words()),
        }
    }

    /// The tile Home shows: "Fix 2 problems" and which check comes first; `None` when no problem is red or has a fix that
    /// works now.
    pub fn home_tile(&self) -> Option<HomeTile> {
        let problems = self.home_problems();
        let first = problems.first()?;
        let n = problems.len();
        let title = if n == 1 { "Fix 1 problem".to_string() } else { format!("Fix {n} problems") };
        Some(HomeTile { title, note: first.0.title().to_string(), mark: if problems.iter().any(|(_, p)| p.mark == Mark::Problem) { Mark::Problem } else { Mark::ToDo } })
    }
}

/// What Home's checkup tile says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeTile {
    pub title: String,
    /// The badge line: the title of the first check to fix.
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
        Folder::Here => Outcome::Ok("Found".to_string()),
    }
}

/// What the Steam changes line says when the list of waiting changes cannot be read (never "nothing waiting").
pub const QUEUE_UNREADABLE: &str = "Steam's change list cannot be read";

fn steam_waiting(f: &Facts) -> Outcome {
    let queue = match &f.queue {
        Ok(q) => q,
        Err(why) => return Outcome::Unknown(why.clone()),
    };
    if queue.is_empty() {
        return Outcome::Ok("Nothing waiting".to_string());
    }
    let mut ports: Vec<String> = Vec::new();
    let mut port_keys: Vec<String> = Vec::new();
    for q in queue {
        if !ports.contains(&q.name) {
            ports.push(q.name.clone());
        }
        if !port_keys.contains(&q.port_key) {
            port_keys.push(q.port_key.clone());
        }
    }
    let label = if f.game_mode { "See how to finish this" } else { "Close Steam and update shortcuts" };
    let summary = format!("{} waiting for Steam to close: {}", count(queue.len(), "change", "changes"), names(&ports));
    match problem(Mark::Waiting, summary, port_keys, Fix::CloseSteam, label) {
        Outcome::Problem(p) if f.game_mode => Outcome::Problem(Problem { advisory: true, desktop_only: true, ..p }),
        other => other,
    }
}

/// A few words for what is wrong with an icon (no path: the step page has it); the port's name follows them.
pub fn icon_words(state: &IconState) -> &'static str {
    match state {
        IconState::Ok(_) | IconState::Outside(_) => "Icon is fine",
        IconState::Empty => "No icon is set",
        IconState::Missing(_) => "Icon file is missing",
        IconState::NotAnImage(_) => "Icon file is not a picture",
        IconState::Quoted { .. } => "Icon is stored the wrong way",
        IconState::InOldCopy(_) => "Icon is stored in an old copy",
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
    let all_bad: Vec<&IconFinding> = shortcuts.iter().filter(|s| s.state.needs_fix()).collect();
    let bad: Vec<&IconFinding> = all_bad.iter().copied().filter(|s| !repair_queued.contains(s.port_key.as_str())).collect();
    if bad.is_empty() {
        if all_bad.is_empty() {
            return Outcome::Ok(if shortcuts.is_empty() { "No shortcuts yet".to_string() } else { "All fine".to_string() });
        }
        // The repair is made and waits for Steam to close: not fixed yet, and not "all fine".
        let mut ports: Vec<String> = Vec::new();
        for b in &all_bad {
            if !ports.contains(&b.port_name) {
                ports.push(b.port_name.clone());
            }
        }
        return Outcome::Queued {
            summary: format!("Queued: finishes when Steam closes ({})", names(&ports)),
            fix_label: if f.game_mode { "See how to finish this" } else { "Close Steam and update shortcuts" }.to_string(),
            desktop_only: f.game_mode,
        };
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
        [one] => format!("{} for {}", icon_words(&one.state), one.port_name),
        many => format!("{} show nothing for {}", count(many.len(), "icon", "icons"), names(&ports)),
    };
    let detail = bad.iter().take(4).map(|b| b.describe()).collect();
    Outcome::Problem(Problem { mark: Mark::ToDo, summary, detail, ports: port_keys, fix: Fix::SteamIcons, fix_label: "Fix the icons".to_string(), advisory: false, desktop_only: false })
}

fn steam_shortcuts(f: &Facts) -> Outcome {
    let shortcuts = match &f.shortcuts {
        Ok(s) => s,
        Err(why) => return Outcome::Unknown(why.clone()),
    };
    if !f.steam_found {
        return Outcome::Unknown("Steam's folder was not found".to_string());
    }
    // A port with any change waiting (add, repoint, icon) is already being taken care of: counted under "Steam changes".
    let queued: HashSet<&str> = match &f.queue {
        Ok(q) => q.iter().map(|q| q.port_key.as_str()).collect(),
        Err(_) => HashSet::new(),
    };
    let in_steam: HashSet<&str> = shortcuts.iter().map(|s| s.port_key.as_str()).collect();
    // A shortcut that still starts an old copy of its port (the move's repoint never applied) is in Steam, but wrong.
    let mut at_old: Vec<Named> = Vec::new();
    for s in shortcuts.iter().filter(|s| s.at_old_copy && !queued.contains(s.port_key.as_str())) {
        if !at_old.iter().any(|n| n.key == s.port_key) {
            at_old.push(Named::new(&s.port_key, &s.port_name));
        }
    }
    // Nobody asked for Steam shortcuts yet (none in Steam): not a thing to nag about.
    if in_steam.is_empty() {
        return Outcome::Ok("No ports in Steam yet".to_string());
    }
    let missing: Vec<Named> = f.installed.iter().filter(|p| !in_steam.contains(p.key.as_str()) && !queued.contains(p.key.as_str())).cloned().collect();
    if at_old.is_empty() && missing.is_empty() {
        return Outcome::Ok("Every port is in Steam".to_string());
    }
    let mut parts = Vec::new();
    if !at_old.is_empty() {
        parts.push(format!("Steam still starts the old copy of {}", named_list(&at_old)));
    }
    if !missing.is_empty() {
        parts.push(format!("Not in Steam: {}", named_list(&missing)));
    }
    // A shortcut at an old copy is repointed first (its name, art, play time and launch options stay); a port that is not in
    // Steam at all is added after, which the next look offers.
    if !at_old.is_empty() {
        return problem(Mark::ToDo, parts.join("; "), keys(&at_old), Fix::Repoint, "Point Steam at the new copy");
    }
    problem(Mark::ToDo, parts.join("; "), keys(&missing), Fix::AddToSteam, "Add them to Steam")
}

fn programs(f: &Facts) -> Outcome {
    if f.programs.is_empty() {
        return Outcome::Ok("Nothing to choose".to_string());
    }
    let first = f.programs[0].clone();
    problem(Mark::ToDo, format!("Not chosen for {}", named_list(&f.programs)), keys(&f.programs), Fix::PickProgram(first.key), &format!("Choose the program for {}", first.name))
}

fn game_files(f: &Facts) -> Outcome {
    if f.files.is_empty() {
        return Outcome::Ok("All there".to_string());
    }
    let missing: Vec<Named> = f.files.iter().filter(|(_, s)| *s == FilesState::Missing).map(|(n, _)| n.clone()).collect();
    let to_add: Vec<Named> = f.files.iter().filter(|(_, s)| *s == FilesState::Unverifiable).map(|(n, _)| n.clone()).collect();
    let summary = match (missing.is_empty(), to_add.is_empty()) {
        (false, true) => format!("Missing for {}", named_list(&missing)),
        (false, false) => format!("Missing for {}; not connected yet: {}", named_list(&missing), named_list(&to_add)),
        _ => format!("Not connected yet: {}", named_list(&to_add)),
    };
    let all: Vec<Named> = f.files.iter().map(|(n, _)| n.clone()).collect();
    problem(if missing.is_empty() { Mark::ToDo } else { Mark::Problem }, summary, keys(&all), Fix::GameFiles, "Find the game files")
}

fn saves(f: &Facts) -> Outcome {
    if f.saves.is_empty() {
        return Outcome::Ok("All found".to_string());
    }
    let first = &f.saves[0];
    advisory(problem(Mark::ToDo, format!("Not found for {}", named_list(&f.saves)), keys(&f.saves), Fix::FindSaves(first.key.clone()), &format!("Find saves for {}", first.name)))
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
        [one] => one.state.sentence(&one.name),
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
    let detail = copies.iter().take(4).map(|c| c.state.sentence(&c.name)).collect();
    // Nothing to do yet (a first start, Steam still to update) is not worth a tile on Home.
    Outcome::Problem(Problem {
        mark: if can_act { Mark::ToDo } else { Mark::Waiting },
        summary,
        detail,
        ports: Vec::new(),
        fix: Fix::OldCopies,
        fix_label: "Look at the old copies".to_string(),
        advisory: !can_act,
        desktop_only: false,
    })
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
    // Signed in to cloud backup with a recent success: the saves are copied online, a zip here is not needed.
    if f.cloud_ok && !f.cloud_problem {
        return Outcome::Ok("Saves are copied to the cloud".to_string());
    }
    // A zip can only hold saves the app knows: with none known there is nothing to back up yet.
    let stale = match f.backup_days {
        None if f.saves_known == 0 => None,
        None => Some("No save backup yet".to_string()),
        Some(d) if d > BACKUP_STALE_DAYS && f.saves_known > 0 => Some(format!("Last save backup was {d} days ago")),
        Some(_) => None,
    };
    match (stale, f.cloud_problem) {
        (Some(words), cloud) => {
            let summary = if cloud { format!("{words}; cloud backup has a problem") } else { words };
            advisory(problem(Mark::ToDo, summary, Vec::new(), Fix::BackUpNow, "Back up saves now"))
        }
        (None, true) => advisory(problem(Mark::ToDo, "Cloud backup has a problem".to_string(), Vec::new(), Fix::Cloud, "Look at cloud backup")),
        (None, false) if f.saves_known == 0 && f.backup_days.is_none() => Outcome::Ok("No save folders known yet".to_string()),
        (None, false) => Outcome::Ok(match f.backup_days {
            Some(0) => "Backed up today".to_string(),
            Some(1) => "Backed up yesterday".to_string(),
            Some(d) => format!("Backed up {d} days ago"),
            None => "Backed up".to_string(),
        }),
    }
}


// The program choice question for one port's folder: how many launch candidates it has at its shallowest depth. A walk
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
