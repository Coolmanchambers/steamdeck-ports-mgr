//! "Close Steam and update shortcuts": the one flow that applies queued Steam changes (`steam_queue`) when
//! Steam is running in Desktop Mode, which on a Deck is nearly always (Steam sits in the tray).
//!
//! 1. [`begin`] looks at the queue, the mode and Steam. In Game Mode nothing is done (Steam can't be closed
//!    there). With Steam already closed the queue is applied at once. With Steam running it asks Steam to
//!    shut down the normal way (`steam -shutdown`, the same as the Steam menu's Exit); it never kills Steam.
//!    It refuses when this app was itself started by Steam, since Steam would close it too.
//! 2. A [`Closer`] is polled (every half second) until no Steam process has been seen for [`SETTLE`]
//!    seconds, or until [`TIMEOUT`]; a timeout writes nothing and leaves the queue as it was.
//! 3. [`apply`] needs the [`SteamClosed`] proof the first two steps hand out, and goes through
//!    `steam_queue::apply_queue_file`, which checks for Steam again, backs `shortcuts.vdf` up first and
//!    writes atomically.
//! 4. The result page offers to start Steam again (detached, Desktop Mode only).
//!
//! No UI here: the pages are plain [`Page`] data; `src/app/steam_close_screen.rs` shows them and runs the
//! timer. Steam itself is behind [`SteamControl`], so tests use a fake.

use super::page::{Focus, Page, PageButton, PageOption, LEFT};
use super::steam_queue::{self, ApplyInputs, QueueApplyOutcome};
use super::store::{SteamProcessCheck, SystemSteamCheck};
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How long the flow waits for Steam to close before giving up (writing nothing).
pub const TIMEOUT: Duration = Duration::from_secs(90);
/// How long no Steam process must be seen before Steam counts as closed: Steam writes its own files as it
/// exits, and its helper processes go one by one.
pub const SETTLE: Duration = Duration::from_secs(3);
/// How often the screen looks again while it waits.
pub const POLL: Duration = Duration::from_millis(500);

/// The flatpak build of Steam, when that is the one installed.
const FLATPAK_ID: &str = "com.valvesoftware.Steam";

/// How to run Steam on this device: its program and the arguments that come before Steam's own (flatpak's
/// `run com.valvesoftware.Steam`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamCommand {
    pub program: PathBuf,
    pub prefix: Vec<String>,
}

impl SteamCommand {
    pub fn shutdown_args(&self) -> Vec<String> {
        let mut args = self.prefix.clone();
        args.push("-shutdown".to_string());
        args
    }

    pub fn start_args(&self) -> Vec<String> {
        self.prefix.clone()
    }
}

/// Finds Steam: the system's own (`/usr/bin/steam` on SteamOS, `/usr/games/steam` on Debian-style systems)
/// first, so a `steam` earlier on `PATH` cannot stand in for it; then `steam` on `PATH`; then the flatpak build
/// when it is installed and `flatpak` is on `PATH`. `is_file` is injectable for tests.
pub fn find_steam(path_var: Option<&OsStr>, home: Option<&Path>, is_file: &dyn Fn(&Path) -> bool) -> Option<SteamCommand> {
    let dirs: Vec<PathBuf> = path_var.map(|p| std::env::split_paths(p).filter(|d| d.is_absolute()).collect()).unwrap_or_default();
    let on_path = |name: &str| dirs.iter().map(|d| d.join(name)).find(|p| is_file(p));
    if let Some(program) = ["/usr/bin/steam", "/usr/games/steam"].iter().map(PathBuf::from).find(|p| is_file(p)).or_else(|| on_path("steam")) {
        return Some(SteamCommand { program, prefix: Vec::new() });
    }
    let mut installs = vec![PathBuf::from("/var/lib/flatpak/app").join(FLATPAK_ID).join("current")];
    if let Some(home) = home {
        installs.insert(0, home.join(".local/share/flatpak/app").join(FLATPAK_ID).join("current"));
    }
    let flatpak = on_path("flatpak")?;
    installs.iter().any(|p| is_file(&p.join("active/metadata")) || is_file(&p.join("metadata"))).then(|| SteamCommand { program: flatpak, prefix: vec!["run".to_string(), FLATPAK_ID.to_string()] })
}

/// Names of processes that mean "started by Steam" when they are among this app's parents: Steam itself
/// and the reaper it starts games and non-Steam shortcuts under.
const STEAM_PARENTS: [&str; 4] = ["steam", "steam.sh", "reaper", "steamwebhelper"];

/// Whether `pid` was started by Steam (directly or further up): closing Steam would close this app too.
/// Reads `/proc/<pid>/stat` up the parent chain under a `/proc`-shaped folder; anything unreadable ends
/// the walk (and counts as not started by Steam).
pub fn started_by_steam(proc_root: &Path, pid: u32) -> bool {
    let mut current = pid;
    for _ in 0..64 {
        let Ok(stat) = std::fs::read_to_string(proc_root.join(current.to_string()).join("stat")) else { return false };
        // "<pid> (<comm>) <state> <ppid> ...": the name can hold spaces and brackets, so split at the last ')'.
        let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else { return false };
        let comm = &stat[open + 1..close];
        if current != pid && STEAM_PARENTS.contains(&comm) {
            return true;
        }
        let Some(ppid) = stat[close + 1..].split_whitespace().nth(1).and_then(|p| p.parse::<u32>().ok()) else { return false };
        if ppid <= 1 || ppid == current {
            return false;
        }
        current = ppid;
    }
    false
}

/// Steam, as the flow needs it.
pub trait SteamControl {
    /// `pid name` of every Steam process (empty: Steam is closed).
    fn running(&self) -> io::Result<Vec<String>>;
    /// Games Steam started that are running now (`pid name`): closing Steam would close them too.
    fn games_running(&self) -> io::Result<Vec<String>>;
    /// Ask Steam to shut down the normal way. Returns at once; Steam closes on its own time.
    fn request_shutdown(&self) -> Result<(), String>;
    /// Start Steam, detached from this app.
    fn start(&self) -> Result<(), String>;
}

/// The real Steam: `/proc` for the check, the found command for the rest.
pub struct SystemSteam {
    pub command: Option<SteamCommand>,
    check: SystemSteamCheck,
}

impl SystemSteam {
    pub fn new() -> SystemSteam {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let path = std::env::var_os("PATH");
        SystemSteam { command: find_steam(path.as_deref(), home.as_deref(), &|p| p.is_file()), check: SystemSteamCheck::new() }
    }
}

impl Default for SystemSteam {
    fn default() -> Self {
        Self::new()
    }
}

/// Runs `program args` in a new session with no terminal and its output thrown away, and returns once the
/// small shell that started it has gone (it does not wait for the program). While `steam -shutdown` itself runs
/// (a second or two) it is a process named `steam` too, so the process check sees "Steam" a little longer; the
/// settle time in [`Closer`] covers that.
fn spawn_detached(program: &Path, args: &[String]) -> Result<(), String> {
    let script = "if command -v setsid >/dev/null 2>&1; then setsid \"$0\" \"$@\" </dev/null >/dev/null 2>&1 & else nohup \"$0\" \"$@\" </dev/null >/dev/null 2>&1 & fi";
    let status = std::process::Command::new("sh").arg("-c").arg(script).arg(program).args(args).status().map_err(|e| format!("could not run sh: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("sh exited with {status}")) }
}

impl SteamControl for SystemSteam {
    fn running(&self) -> io::Result<Vec<String>> {
        self.check.running_steam_processes()
    }

    fn games_running(&self) -> io::Result<Vec<String>> {
        super::store::steam_game_processes_in(Path::new("/proc"))
    }

    fn request_shutdown(&self) -> Result<(), String> {
        let cmd = self.command.as_ref().ok_or("the steam program was not found")?;
        spawn_detached(&cmd.program, &cmd.shutdown_args())
    }

    fn start(&self) -> Result<(), String> {
        let cmd = self.command.as_ref().ok_or("the steam program was not found")?;
        spawn_detached(&cmd.program, &cmd.start_args())
    }
}

/// Proof that Steam was seen closed just now; [`apply`] takes it. Only this module makes one.
#[derive(Debug)]
pub struct SteamClosed(());

/// Why the flow stops before closing anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocked {
    /// Steam can't be closed in Game Mode.
    GameMode,
    NothingQueued,
    /// This app was started by Steam, so closing Steam would close it too.
    StartedBySteam,
    /// A game Steam started is running (closing Steam would close it): its names.
    GameRunning(Vec<String>),
    /// Whether Steam runs could not be told (no `/proc`).
    CheckFailed(String),
    /// Steam could not be asked to close (no `steam` program found, say).
    ShutdownFailed(String),
    /// The queue file could not be read (what `steam_queue::load` said).
    QueueProblem(String),
}

#[derive(Debug)]
pub enum Start {
    /// Steam is closed already: apply now.
    ApplyNow(SteamClosed),
    /// Steam was asked to close: poll the closer.
    Closing(Closer),
    Blocked(Blocked),
}

/// The first look. `now` is the screen's clock (time since the flow opened, say).
pub fn begin(queue_len: usize, game_mode: bool, started_by_steam: bool, steam: &dyn SteamControl, now: Duration) -> Start {
    if game_mode {
        return Start::Blocked(Blocked::GameMode);
    }
    if queue_len == 0 {
        return Start::Blocked(Blocked::NothingQueued);
    }
    match steam.running() {
        Err(e) => Start::Blocked(Blocked::CheckFailed(e.to_string())),
        Ok(procs) if procs.is_empty() => Start::ApplyNow(SteamClosed(())),
        Ok(_) if started_by_steam => Start::Blocked(Blocked::StartedBySteam),
        Ok(_) => match steam.games_running() {
            Ok(games) if !games.is_empty() => Start::Blocked(Blocked::GameRunning(games)),
            Err(e) => Start::Blocked(Blocked::CheckFailed(e.to_string())),
            Ok(_) => match steam.request_shutdown() {
                Ok(()) => Start::Closing(Closer::new(now)),
                Err(e) => Start::Blocked(Blocked::ShutdownFailed(e)),
            },
        },
    }
}

/// Waits for Steam to be gone.
#[derive(Debug)]
pub struct Closer {
    asked_at: Duration,
    clear_since: Option<Duration>,
}

#[derive(Debug)]
pub enum Poll {
    /// Still waiting: seconds since Steam was asked to close.
    Waiting { seconds: u64 },
    Closed(SteamClosed),
    /// Steam is still up after [`TIMEOUT`]: nothing is written. What is still running.
    TimedOut { still: Vec<String> },
    CheckFailed(String),
}

impl Closer {
    pub fn new(now: Duration) -> Closer {
        Closer { asked_at: now, clear_since: None }
    }

    /// One look: `running` is what the process check says now.
    pub fn poll(&mut self, running: io::Result<Vec<String>>, now: Duration) -> Poll {
        let waited = now.saturating_sub(self.asked_at);
        match running {
            Err(e) => Poll::CheckFailed(e.to_string()),
            Ok(procs) if procs.is_empty() => {
                let since = *self.clear_since.get_or_insert(now);
                if now.saturating_sub(since) >= SETTLE {
                    Poll::Closed(SteamClosed(()))
                } else {
                    Poll::Waiting { seconds: waited.as_secs() }
                }
            }
            Ok(procs) => {
                self.clear_since = None;
                if waited >= TIMEOUT {
                    Poll::TimedOut { still: procs }
                } else {
                    Poll::Waiting { seconds: waited.as_secs() }
                }
            }
        }
    }
}

/// Applies the queue now that Steam is closed (the write path checks for Steam once more before writing).
pub fn apply(_closed: SteamClosed, queue_path: &Path, inputs: &ApplyInputs, steam: &dyn SteamProcessCheck, now: SystemTime) -> QueueApplyOutcome {
    steam_queue::apply_queue_file(queue_path, inputs, steam, now)
}

// ---------------------------------------------------------------------------------------------
// Pages
// ---------------------------------------------------------------------------------------------

/// What each item on the flow's pages does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    CloseSteam,
    /// Leave the changes waiting.
    NotNow,
    StopWaiting,
    TryAgain,
    StartSteam,
    /// Leave Steam closed / OK.
    Done,
    Details,
    /// Back from the details to the page before.
    Back,
}

pub type ClosePage = Page<CloseAction>;

const HEADER: &str = "Deck Ports Manager";

fn base(window: &str, title: &str) -> ClosePage {
    Page { window_title: format!("Update Steam: {window}"), large: true, header: HEADER.to_string(), title: title.to_string(), ..Page::default() }
}

fn option(title: &str, detail: &str, action: CloseAction, recommended: bool) -> PageOption<CloseAction> {
    PageOption { title: title.to_string(), detail: detail.to_string(), choice: None, enabled: true, recommended, action }
}

fn changes(n: usize) -> String {
    if n == 1 { "1 Steam change".to_string() } else { format!("{n} Steam changes") }
}

fn are(n: usize) -> &'static str {
    if n == 1 { "is" } else { "are" }
}

pub const CLOSE_LABEL: &str = "Close Steam and update shortcuts";
pub const GAME_MODE_TEXT: &str = "Switch to Desktop Mode and open Deck Ports Manager to finish updating Steam.";

pub const UPDATE_NOW_LABEL: &str = "Update Steam shortcuts now";
const ONLY_THE_LIST: &str = "Only Steam's shortcut list changes; your games, saves and play time are not touched. Steam has to be closed because it only reads that list when it starts.";

/// "Update your Steam shortcuts?" `lead` comes first (what just happened, "Queued 2 changes."). `stray_safe`:
/// the page came up by itself, so the cursor starts on Not now and a stray A does not close Steam.
/// `steam_closed`: Steam is closed already, so nothing has to be closed.
pub fn ask_page(waiting: usize, lead: Option<&str>, stray_safe: bool, steam_closed: bool) -> ClosePage {
    let mut p = base(if steam_closed { "update now?" } else { "close Steam?" }, "Update your Steam shortcuts?");
    p.paragraphs = lead.map(str::to_string).into_iter().collect();
    if steam_closed {
        p.paragraphs.push(format!("{} {} waiting. Steam is closed, so {} can be made now.", changes(waiting), are(waiting), if waiting == 1 { "it" } else { "they" }));
    } else {
        p.paragraphs.push(format!("{} {} waiting. Steam has to be closed to make {}.", changes(waiting), are(waiting), if waiting == 1 { "it" } else { "them" }));
    }
    p.paragraphs.push(ONLY_THE_LIST.to_string());
    let (label, detail) = if steam_closed { (UPDATE_NOW_LABEL, "Your shortcut list is backed up first") } else { (CLOSE_LABEL, "Takes a few seconds; Steam can start again after") };
    p.options = vec![
        option(label, detail, CloseAction::CloseSteam, true),
        option("Not now", "They keep waiting. Settings > Add to Steam has them.", CloseAction::NotNow, false),
    ];
    p.initial = Focus::Option(if stray_safe { 1 } else { 0 });
    p
}

/// Game Mode: nothing can be done here.
pub fn game_mode_page(waiting: usize) -> ClosePage {
    let mut p = base("Game Mode", "Finish in Desktop Mode");
    p.paragraphs = vec![GAME_MODE_TEXT.to_string(), format!("{} {} waiting. Steam can't be closed in Game Mode.", changes(waiting), are(waiting))];
    p.options = vec![option("OK", "", CloseAction::Done, true)];
    p.initial = Focus::Option(0);
    p
}

/// While Steam closes.
pub fn closing_page(seconds: u64) -> ClosePage {
    let mut p = base("closing Steam", "Closing Steam...");
    p.paragraphs = vec!["Waiting for Steam to close. This usually takes a few seconds.".to_string()];
    p.facts = vec![format!("Waited {seconds} s of up to {} s.", TIMEOUT.as_secs())];
    p.options = vec![option("Stop waiting", "Steam still closes on its own. Nothing is written; the app asks again once Steam is gone.", CloseAction::StopWaiting, false)];
    p.initial = Focus::Option(0);
    p
}

/// Steam did not close in time.
pub fn timed_out_page(waiting: usize) -> ClosePage {
    let mut p = base("Steam did not close", "Steam did not close");
    p.paragraphs = vec![
        format!("Steam is still running after {} seconds, so nothing was changed. {} {} still waiting.", TIMEOUT.as_secs(), changes(waiting), are(waiting)),
        "If Steam shows a question or is downloading, finish that first, or close Steam yourself (tray icon > Exit Steam), then try again.".to_string(),
    ];
    p.options = vec![option("Try again", "", CloseAction::TryAgain, true), option("Not now", "They keep waiting", CloseAction::NotNow, false)];
    p.buttons[LEFT] = Some(PageButton { label: "Details".to_string(), action: CloseAction::Details });
    p.initial = Focus::Option(0);
    p
}

/// The flow stopped before closing Steam.
pub fn blocked_page(blocked: &Blocked, waiting: usize) -> ClosePage {
    if *blocked == Blocked::GameMode {
        return game_mode_page(waiting);
    }
    let (window, title, text) = match blocked {
        Blocked::NothingQueued => ("nothing waiting", "Nothing is waiting", "There are no Steam changes waiting.".to_string()),
        Blocked::StartedBySteam => (
            "started from Steam",
            "Open this app from the desktop",
            "Deck Ports Manager was started from Steam, so closing Steam would close it too. Open it from the desktop's app menu, then try again.".to_string(),
        ),
        Blocked::CheckFailed(e) => ("could not check", "Steam was not closed", format!("This app could not tell whether Steam is running ({e}), so nothing was changed.")),
        Blocked::ShutdownFailed(e) => ("could not close", "Steam was not closed", format!("Steam could not be asked to close ({e}). Close it from the Steam menu (Exit), then try again.")),
        Blocked::QueueProblem(e) => ("could not read", "The Steam changes could not be read", e.clone()),
        Blocked::GameRunning(games) => (
            "game running",
            "Close the game first",
            format!("A game started from Steam is running ({}). Closing Steam would close it too. Close it first, then try again.", games.iter().map(|g| g.split_once(' ').map_or(g.as_str(), |(_, name)| name)).collect::<Vec<_>>().join(", ")),
        ),
        Blocked::GameMode => unreachable!("handled above"),
    };
    let mut p = base(window, title);
    p.paragraphs = vec![text];
    if waiting > 0 && *blocked != Blocked::NothingQueued {
        p.paragraphs.push(format!("{} {} still waiting.", changes(waiting), are(waiting)));
    }
    p.options = if matches!(blocked, Blocked::GameRunning(_)) {
        vec![option("Try again", "After closing the game", CloseAction::TryAgain, true), option("Not now", "They keep waiting", CloseAction::NotNow, false)]
    } else {
        vec![option("OK", "", CloseAction::Done, true)]
    };
    p.initial = Focus::Option(0);
    p
}

/// What applying did, in a few plain lines. `closed_by_us`: this flow closed Steam, so it offers to start it
/// again (Desktop Mode only, which is the only place it runs), unless `steam_up` says it is running again
/// (it came back before the write, say).
pub fn result_page(outcome: &QueueApplyOutcome, closed_by_us: bool, steam_up: bool) -> ClosePage {
    let mut p;
    if outcome.refused {
        p = base("not finished", if outcome.done == 0 { "Nothing was changed" } else { "Some changes are still waiting" });
        let waiting = outcome.remaining.len();
        p.paragraphs = vec![
            if waiting > 0 { format!("{} {} still waiting: Details says why.", changes(waiting), are(waiting)) } else { "The Steam changes could not be read: Details says why.".to_string() },
            "Your Steam shortcut list was not changed by this.".to_string(),
        ];
        if outcome.done > 0 {
            p.paragraphs[1] = format!("{} made before that.", if outcome.done == 1 { "1 change was".to_string() } else { format!("{} changes were", outcome.done) });
        }
        p.options = vec![option("Try again", "", CloseAction::TryAgain, true)];
    } else {
        p = base("done", "Steam shortcuts updated");
        p.facts = vec![if outcome.done == 1 { "1 Steam change made.".to_string() } else { format!("{} Steam changes made.", outcome.done) }];
        if outcome.already > 0 {
            p.facts.push(format!("{} already up to date.", if outcome.already == 1 { "1 was".to_string() } else { format!("{} were", outcome.already) }));
        }
        if outcome.wrote {
            p.facts.push("Your old shortcut list was backed up first.".to_string());
        }
        if outcome.done > 0 {
            p.paragraphs = vec!["Steam will start these ports from their new place when it runs again.".to_string()];
        }
        if outcome.dropped > 0 {
            p.facts.push(format!("{} could not be made any more: Details says why.", if outcome.dropped == 1 { "1 change".to_string() } else { format!("{} changes", outcome.dropped) }));
        }
    }
    if closed_by_us && !steam_up {
        p.options.push(option("Start Steam again", "", CloseAction::StartSteam, !outcome.refused));
        p.options.push(option("Leave Steam closed", "", CloseAction::Done, false));
    } else if !outcome.refused {
        p.options.push(option("Done", "", CloseAction::Done, true));
    } else {
        p.options.push(option("Not now", "They keep waiting", CloseAction::NotNow, false));
    }
    p.buttons[LEFT] = Some(PageButton { label: "Details".to_string(), action: CloseAction::Details });
    p.initial = Focus::Option(0);
    p
}

/// "Start Steam again" did not work.
pub fn start_failed_page(error: &str) -> ClosePage {
    let mut p = base("Steam did not start", "Steam did not start");
    p.paragraphs = vec![format!("Steam could not be started ({error}). Start it from the desktop's app menu.")];
    p.options = vec![option("OK", "", CloseAction::Done, true)];
    p.initial = Focus::Option(0);
    p
}

/// The full text behind Details, with one Back.
pub fn details_page(text: &str) -> ClosePage {
    let mut p = base("details", "Details");
    p.paragraphs = text.split("\n\n").map(str::to_string).collect();
    p.buttons[LEFT] = Some(PageButton { label: "Back".to_string(), action: CloseAction::Back });
    p.initial = Focus::Button(LEFT);
    p
}

/// What B does on each page: the safe way out (never closes Steam, never starts it).
pub fn back_action(page: &ClosePage) -> CloseAction {
    let has = |a: CloseAction| page.options.iter().any(|o| o.action == a) || page.buttons.iter().flatten().any(|b| b.action == a);
    [CloseAction::Back, CloseAction::StopWaiting, CloseAction::NotNow, CloseAction::Done].into_iter().find(|a| has(*a)).unwrap_or(CloseAction::Done)
}

#[cfg(test)]
mod tests;
