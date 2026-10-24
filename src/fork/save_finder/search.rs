//! "Look for them for me": a read-only search of the places a game or its Proton prefix keeps saves, for
//! folders named like the port that hold save-like files.
//!
//! Places: the port's own folder (a few levels down, folders named like "Save"), `~/.local/share`, `~/.config`,
//! `~/Documents`, hidden folders in the home folder (`~/.soh`), and every Proton prefix under Steam's
//! `compatdata` (`drive_c/users/steamuser/{AppData/*, Documents, Saved Games}`), one and two levels down
//! (`Company/Game`). A folder is a candidate when its name fits this port (the same name words the import
//! scan uses) and it holds a save-like file. A folder that is another catalog port's own name never is, so
//! Ship of Harkinian does not offer 2 Ship 2 Harkinian's saves. Nothing is written, moved or followed through
//! a link; the search stops at its time and entry budgets, and a cancel stops it at the next folder.

use crate::core::models::Port;
use crate::core::path_safety::is_within;
use crate::fork::adopt::{norm, port_names, shared_words};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Longest the search runs; what it has found by then is shown.
pub const TIME_BUDGET: Duration = Duration::from_secs(25);
/// Folders looked into while judging one candidate, and how deep.
const EVAL_DEPTH: usize = 5;
const EVAL_ENTRIES: usize = 4000;
/// Whole-search budget of directory entries read (a huge tree cannot keep it busy).
const TOTAL_ENTRIES: usize = 60_000;
/// Entries read when listing one folder to look for a name in it.
const LIST_LIMIT: usize = 600;
/// How far down the port's own folder is searched, and how many entries of it.
const PORT_DEPTH: usize = 3;
const PORT_ENTRIES: usize = 4000;
/// Most candidates offered.
pub const MAX_CANDIDATES: usize = 6;

const SAVE_EXTENSIONS: [&str; 10] = ["sav", "srm", "sra", "eep", "mpk", "fla", "save", "sl2", "sgd", "dat"];
/// Hidden folders of the home folder that are never a game's.
const HOME_NOISE: [&str; 14] = [".cache", ".local", ".config", ".steam", ".var", ".mozilla", ".ssh", ".gnupg", ".cargo", ".rustup", ".npm", ".pki", ".thumbnails", ".trash"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    PortFolder,
    LocalShare,
    Config,
    Documents,
    HomeHidden,
    Proton,
}

impl Place {
    /// Short words for the card.
    pub fn label(self) -> &'static str {
        match self {
            Place::PortFolder => "inside the game folder",
            Place::LocalShare => "in your Linux app data",
            Place::Config => "in your Linux settings",
            Place::Documents => "in Documents",
            Place::HomeHidden => "in a hidden folder in your home folder",
            Place::Proton => "in Steam's Windows files",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    /// The folder's own name.
    pub name: String,
    pub files: usize,
    pub newest: SystemTime,
    pub place: Place,
}

#[derive(Debug, Default)]
pub struct Found {
    /// Newest first.
    pub candidates: Vec<Candidate>,
    /// The time or entry budget ran out, or the user cancelled, before every place was looked in.
    pub stopped_early: bool,
}

pub struct Input<'a> {
    pub port: &'a Port,
    /// Every catalog entry (this one included): another entry's own folder name is never offered.
    pub catalog: &'a [Port],
    pub game_dir: &'a Path,
    pub library_dir: &'a Path,
    pub saves_backup_dir: &'a Path,
    pub home: &'a Path,
    /// Folders that hold a `steamapps` folder, beyond the ones under the home folder (a card in the Deck).
    pub steam_roots: &'a [PathBuf],
}

struct Budget<'a> {
    deadline: Instant,
    entries: usize,
    cancelled: &'a dyn Fn() -> bool,
    stopped: bool,
}

impl Budget<'_> {
    fn spent(&mut self) -> bool {
        if self.stopped {
            return true;
        }
        if self.entries > TOTAL_ENTRIES || Instant::now() >= self.deadline || (self.cancelled)() {
            self.stopped = true;
        }
        self.stopped
    }
}

fn lower(name: &std::ffi::OsStr) -> String {
    name.to_string_lossy().to_lowercase()
}

/// Whether a folder named `name` fits `port` and no other catalog entry.
pub fn name_fits(name: &str, port: &Port, catalog: &[Port]) -> bool {
    let n = norm(name);
    if n.len() < 3 {
        return false;
    }
    let mine = port_names(port);
    let others_name_it = catalog.iter().filter(|p| p.key() != port.key() && !p.is_android_only()).any(|p| port_names(p).contains(&n));
    if others_name_it && !mine.contains(&n) {
        return false;
    }
    if mine.contains(&n) {
        return true;
    }
    if n.len() >= 5 && mine.iter().any(|m| m.len() >= 5 && (n.contains(m.as_str()) || m.contains(n.as_str()))) {
        return !others_name_it;
    }
    shared_words(name, port) >= 2
}

fn save_like(rel_components: &[String], file_name: &str, root_name: &str) -> bool {
    let name = file_name.to_lowercase();
    let ext = Path::new(&name).extension().and_then(|e| e.to_str()).unwrap_or("");
    SAVE_EXTENSIONS.contains(&ext)
        || name.contains("save")
        || name.contains("slot")
        || name.contains("profile")
        || rel_components.iter().any(|c| c.to_lowercase().contains("save"))
        || root_name.to_lowercase().contains("save")
}

#[derive(Default)]
struct Stats {
    files: usize,
    save_files: usize,
    /// Save-like files directly in the folder (not in a sub-folder).
    direct_save_files: usize,
    newest: Option<SystemTime>,
}

/// Counts what is under `dir` (no links followed), within the depth and entry caps.
fn evaluate(dir: &Path, budget: &mut Budget) -> Stats {
    let mut stats = Stats::default();
    let root_name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut seen = 0usize;
    let mut stack: Vec<(PathBuf, Vec<String>)> = vec![(dir.to_path_buf(), Vec::new())];
    while let Some((d, rel)) = stack.pop() {
        if budget.spent() {
            break;
        }
        let Ok(entries) = fs::read_dir(&d) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            budget.entries += 1;
            if seen > EVAL_ENTRIES {
                return stats;
            }
            let Ok(kind) = entry.file_type() else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            if kind.is_file() {
                stats.files += 1;
                let when = entry.metadata().and_then(|m| m.modified()).ok();
                if save_like(&rel, &name, &root_name) {
                    stats.save_files += 1;
                    if rel.is_empty() {
                        stats.direct_save_files += 1;
                    }
                    stats.newest = stats.newest.into_iter().chain(when).max();
                }
            } else if kind.is_dir() && rel.len() + 1 < EVAL_DEPTH {
                let mut next = rel.clone();
                next.push(name);
                stack.push((entry.path(), next));
            }
        }
    }
    stats
}

fn candidate(dir: &Path, place: Place, budget: &mut Budget) -> Option<Candidate> {
    let whole = evaluate(dir, budget);
    if whole.save_files == 0 {
        return None;
    }
    let (mut path, mut stats) = (dir.to_path_buf(), whole);
    // All the save files live in one "Save" sub-folder: offer that, so the logs and settings beside it stay out.
    if stats.direct_save_files == 0 {
        let saveish: Vec<PathBuf> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && lower(&e.file_name()).contains("save"))
            .map(|e| e.path())
            .collect();
        if let [only] = saveish.as_slice() {
            let inner = evaluate(only, budget);
            if inner.save_files == stats.save_files {
                path = only.clone();
                stats = inner;
            }
        }
    }
    Some(Candidate {
        name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        path,
        files: stats.files,
        newest: stats.newest.unwrap_or(SystemTime::UNIX_EPOCH),
        place,
    })
}

/// The folders of a directory (links to folders included, only listed, never entered by the walks above beyond
/// being judged), up to `LIST_LIMIT` entries.
fn child_dirs(dir: &Path, budget: &mut Budget) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else { return out };
    for entry in entries.flatten().take(LIST_LIMIT) {
        budget.entries += 1;
        if entry.path().is_dir() {
            out.push(entry.path());
        }
    }
    out
}

fn steam_prefix_roots(input: &Input) -> Vec<PathBuf> {
    let mut steams: Vec<PathBuf> = vec![
        input.home.join(".steam/steam"),
        input.home.join(".local/share/Steam"),
        input.home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ];
    steams.extend(input.steam_roots.iter().cloned());
    let mut out: Vec<PathBuf> = Vec::new();
    for steam in steams {
        let compat = steam.join("steamapps/compatdata");
        let Ok(entries) = fs::read_dir(&compat) else { continue };
        for entry in entries.flatten().take(LIST_LIMIT) {
            let user = entry.path().join("pfx/drive_c/users/steamuser");
            for sub in ["AppData/Roaming", "AppData/Local", "AppData/LocalLow", "Documents", "My Documents", "Saved Games"] {
                let dir = user.join(sub);
                if dir.is_dir() && !out.iter().any(|d| same_folder(d, &dir)) {
                    out.push(dir);
                }
            }
        }
    }
    out
}

fn same_folder(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Where a candidate may not be: Saves Backup, and any other port's folder in the ports library.
fn allowed(path: &Path, input: &Input) -> bool {
    if is_within(path, input.saves_backup_dir) || is_within(input.saves_backup_dir, path) {
        return false;
    }
    !is_within(path, input.library_dir) || is_within(path, input.game_dir)
}

fn search_port_folder(input: &Input, budget: &mut Budget, out: &mut Vec<Candidate>) {
    let mut seen = 0usize;
    let mut stack = vec![(input.game_dir.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if budget.spent() {
            return;
        }
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            budget.entries += 1;
            if seen > PORT_ENTRIES {
                return;
            }
            let Ok(kind) = entry.file_type() else { continue };
            if !kind.is_dir() {
                continue;
            }
            let path = entry.path();
            if lower(&entry.file_name()).contains("save") {
                if let Some(c) = candidate(&path, Place::PortFolder, budget) {
                    out.push(c);
                }
            } else if depth + 1 < PORT_DEPTH {
                stack.push((path, depth + 1));
            }
        }
    }
}

fn search_parent(parent: &Path, place: Place, input: &Input, budget: &mut Budget, out: &mut Vec<Candidate>) {
    for child in child_dirs(parent, budget) {
        if budget.spent() {
            return;
        }
        let name = child.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name_fits(&name, input.port, input.catalog) {
            if allowed(&child, input) {
                out.extend(candidate(&child, place, budget));
            }
            continue;
        }
        // One level further: `Company/Game`, `My Games/Game`. A folder that is another port's own is not entered.
        let norm_name = norm(&name);
        if input.catalog.iter().any(|p| p.key() != input.port.key() && port_names(p).contains(&norm_name)) {
            continue;
        }
        for grand in child_dirs(&child, budget) {
            if budget.spent() {
                return;
            }
            let inner = grand.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if name_fits(&inner, input.port, input.catalog) && allowed(&grand, input) {
                out.extend(candidate(&grand, place, budget));
            }
        }
    }
}

fn search_home_hidden(input: &Input, budget: &mut Budget, out: &mut Vec<Candidate>) {
    for child in child_dirs(input.home, budget) {
        let name = child.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !name.starts_with('.') || HOME_NOISE.contains(&name.to_lowercase().as_str()) {
            continue;
        }
        if budget.spent() {
            return;
        }
        if name_fits(name.trim_start_matches('.'), input.port, input.catalog) && allowed(&child, input) {
            out.extend(candidate(&child, Place::HomeHidden, budget));
        }
    }
}

/// Runs the search. `progress` is told which place is being looked in; `cancelled` is asked between folders.
pub fn find(input: &Input, cancelled: &dyn Fn() -> bool, progress: &mut dyn FnMut(&str)) -> Found {
    find_with_budget(input, cancelled, progress, TIME_BUDGET)
}

pub fn find_with_budget(input: &Input, cancelled: &dyn Fn() -> bool, progress: &mut dyn FnMut(&str), time: Duration) -> Found {
    let mut budget = Budget { deadline: Instant::now() + time, entries: 0, cancelled, stopped: false };
    let mut out: Vec<Candidate> = Vec::new();

    progress("Looking in the game's folder...");
    search_port_folder(input, &mut budget, &mut out);
    progress("Looking in your Linux app data...");
    search_parent(&input.home.join(".local/share"), Place::LocalShare, input, &mut budget, &mut out);
    search_parent(&input.home.join(".config"), Place::Config, input, &mut budget, &mut out);
    progress("Looking in your home folder...");
    search_home_hidden(input, &mut budget, &mut out);
    search_parent(&input.home.join("Documents"), Place::Documents, input, &mut budget, &mut out);
    progress("Looking in Steam's Windows files...");
    for dir in steam_prefix_roots(input) {
        if budget.spent() {
            break;
        }
        search_parent(&dir, Place::Proton, input, &mut budget, &mut out);
    }

    out.sort_by(|a, b| b.newest.cmp(&a.newest).then_with(|| a.path.cmp(&b.path)));
    out.dedup_by(|a, b| a.path == b.path);
    out.truncate(MAX_CANDIDATES);
    Found { candidates: out, stopped_early: budget.stopped }
}

/// "just now", "5 minutes ago", "2 days ago": how long ago a folder last changed.
pub fn ago(now: SystemTime, then: SystemTime) -> String {
    if then == SystemTime::UNIX_EPOCH {
        return "a long time ago".into();
    }
    let secs = now.duration_since(then).map_or(0, |d| d.as_secs());
    let plural = |n: u64, unit: &str| if n == 1 { format!("1 {unit} ago") } else { format!("{n} {unit}s ago") };
    match secs {
        0..=89 => "just now".into(),
        90..=3_599 => plural(secs / 60, "minute"),
        3_600..=86_399 => plural(secs / 3_600, "hour"),
        86_400..=172_799 => "yesterday".into(),
        172_800..=5_183_999 => plural(secs / 86_400, "day"),
        5_184_000..=63_071_999 => plural(secs / 2_592_000, "month"),
        _ => plural(secs / 31_536_000, "year"),
    }
}
