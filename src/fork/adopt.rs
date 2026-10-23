//! Adopting ports that are already installed somewhere outside `Library/`.
//!
//! Finding: look one level into each scan folder (well-known ones plus the user's own) for a folder
//! that matches a catalog entry: by folder name first, then by a release name (the folder named like
//! the project's download, `SoH-Ackbar-Delta-Linux`), then by the name of a program inside it. A folder
//! that fits two or three entries is offered for the user to choose; one that fits more, or holds a
//! `roms`/`bios` folder or a Steam library, is a collection of games and is skipped. A top-level folder
//! that fits nothing but holds a program is listed as "not recognised" so the user can say which port
//! it is. Hidden folders are never looked at.
//!
//! Registering changes two things, both ours, and nothing inside the install:
//! 1. `Library/<catalog folder>` becomes a symlink to the install, because the app finds, launches and
//!    opens every port as `Library/<folder>`;
//! 2. a `user_managed` entry for it is added to `ports.local.json`. The entry carries the catalog entry's
//!    name, image, tags, save paths and so on but NOT its `source`: with a source the app would install
//!    and update through the link, writing into the user's folder, which adoption promises never to do.
//!    The release link moves to `website`. The cost: the entry's state key becomes its folder name
//!    instead of `owner/repo`, and the app no longer checks it for updates.
//!
//! Nothing is moved, copied, renamed or deleted. If registering fails halfway, the symlink this call
//! made is removed again. Symlinks need Linux; elsewhere registering reports that it cannot.

use crate::core::models::{Port, SourceType};
use crate::core::path_safety::is_within;
use crate::core::platform_resolve::{is_truthy, resolve_per_platform};
use serde_json::{json, Map, Value};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Folders under the home folder that people commonly keep ports in.
pub const WELL_KNOWN: [&str; 3] = ["Applications", "Games", "Ports"];
/// "Look deeper" visits at most this many second-level folders per scan, so a big drive stays quick.
pub const MAX_DEEP_FOLDERS: usize = 400;
/// A program name this short ("run", "game") says nothing about which port it is.
const MIN_EXE_NAME_LEN: usize = 4;
const MAX_EXE_SCAN_DEPTH: usize = 2;
const MAX_EXE_SCAN_ENTRIES: usize = 2000;

/// A folder that fits more ports than this is a collection of games, not one port.
pub const MAX_FITS: usize = 3;
/// Files that count as programs: a port's launcher, never its data (configs, images, saves, ROMs).
const PROGRAM_EXTENSIONS: [&str; 8] = ["appimage", "x86_64", "x86", "sh", "run", "bin", "elf", "exe"];
/// Words in release file and folder names that say nothing about which port it is ("steam" counts only
/// right before "deck", see `release_tokens`).
const NOISE_TOKENS: [&str; 17] = [
    "linux", "linux64", "x64", "x86", "x8664", "amd64", "arm64", "aarch64", "appimage", "release", "portable", "steamdeck", "deck", "flatpak",
    "nightly", "latest", "build",
];
/// Short or common first words that never name a port on their own.
const WEAK_FIRST_WORDS: [&str; 14] = ["the", "and", "die", "new", "pro", "men", "wet", "neo", "all", "big", "ger", "bak", "ico", "yml"];
/// Archive and package endings stripped from a release file name before reading its words.
const ARCHIVE_ENDINGS: [&str; 12] = [".gz", ".xz", ".zst", ".bz2", ".tar", ".tgz", ".zip", ".7z", ".rar", ".appimage", ".deb", ".flatpak"];
/// Top-level folders that are never a port and are never listed (Steam's library and its caches, EmuDeck's
/// folder, the file system's own folder). Hidden folders are skipped as well.
const NEVER_PORTS: [&str; 6] = ["steamapps", "steamlibrary", "lost+found", "emulation", "compatdata", "shadercache"];
/// Release names the Deck-ready list cannot cover: the list only knows each project's latest download
/// names and may not be saved yet (first start offline), and the program inside a download is not listed
/// at all. First word of the folder or program -> catalog folder (checked against the projects' release
/// files on 2026-10-23).
const RELEASE_ALIASES: [(&str, &str); 3] = [
    // HarbourMasters/Shipwright: SoH-<name>-<letter>-Linux.zip / SoH-<name>-<letter>.appimage, soh.appimage.
    ("soh", "Shipwright"),
    // HarbourMasters/2ship2harkinian: 2Ship-<name>-<letter>-Linux.zip holding 2ship.appimage (2s2h.appimage before).
    ("2ship", "2ship2harkinian"),
    ("2s2h", "2ship2harkinian"),
];

/// What else a folder can be matched by: the release file names of each port's latest download, from the
/// Deck-ready list (`Port::key()` and the file's words).
#[derive(Debug, Clone, Default)]
pub struct Hints {
    assets: Vec<(String, Vec<String>)>,
}

impl Hints {
    /// Adds one release file name (`SoH-Dewey-Bravo.appimage`) for the port with `key`.
    pub fn add_asset(&mut self, key: &str, file_name: &str) {
        let words = asset_tokens(file_name);
        if !words.is_empty() && !self.assets.iter().any(|(k, w)| k == key && *w == words) {
            self.assets.push((key.to_string(), words));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.assets.is_empty()
    }
}

/// The words of a release file name, archive endings removed: `Severed_Chains_Steam_Deck.tar.gz` gives
/// `severed chains`.
pub(crate) fn asset_tokens(file_name: &str) -> Vec<String> {
    let mut stem = file_name.trim();
    loop {
        let lower = stem.to_lowercase();
        match ARCHIVE_ENDINGS.iter().find(|e| lower.ends_with(*e)) {
            Some(end) => stem = &stem[..stem.len() - end.len()],
            None => break,
        }
    }
    release_tokens(stem)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    /// The install folder is named like the catalog entry's folder, name or project.
    FolderName,
    /// The install folder is named like the project's download (`Severed_Chains_Steam_Deck`,
    /// `Dusklight-Linux`): its words, or the first few of them, are the port's name or download name.
    ReleaseName,
    /// Only the first word of the folder names the port (`SoH-Ackbar-Delta-Linux`,
    /// `Dusklight-twilight-princess`): trusted when a program is inside.
    ReleaseWord,
    /// A program inside it is named like the catalog entry.
    ExecutableName,
    /// The user said which port it is.
    Chosen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The install folder, as found.
    pub path: PathBuf,
    pub dir_name: String,
    /// `Port::key()` of the catalog entry it matched.
    pub port_key: String,
    pub port_name: String,
    pub catalog_folder: String,
    pub how: MatchKind,
    /// Why it matched, in plain words ("folder name", "program soh.appimage").
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ambiguous {
    pub path: PathBuf,
    pub dir_name: String,
    /// The catalog names (with their note, so two entries for one game can be told apart).
    pub matches: Vec<String>,
    /// `Port::key()` of each match, in the same order.
    pub keys: Vec<String>,
}

#[derive(Debug, Default)]
pub struct ScanResult {
    pub candidates: Vec<Candidate>,
    /// Folders that matched two or three catalog entries equally well; never adopted without the user choosing.
    pub ambiguous: Vec<Ambiguous>,
    /// Top-level folders that match nothing but hold a program: the user may say which port it is.
    pub unrecognised: Vec<PathBuf>,
    /// Folders that look like a collection of many games (they fit more than `MAX_FITS` ports, or hold a
    /// `roms`/`bios` folder or a Steam library); never offered.
    pub collections: Vec<PathBuf>,
    /// Matches whose catalog entry is already installed in `Library/`.
    pub already_in_library: Vec<String>,
    /// A second folder that fits a port another folder was already offered for: port name and folder.
    pub duplicates: Vec<(String, PathBuf)>,
    pub roots_scanned: Vec<PathBuf>,
    /// "Look deeper" stopped at `MAX_DEEP_FOLDERS` sub-folders.
    pub deep_limit_hit: bool,
}

/// Lowercase letters and digits only, so `Ship-of_Harkinian`, `ship of harkinian` and `shipofharkinian` agree.
pub(crate) fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// A leading `~` or `%HOME%` becomes the home folder.
pub fn expand_home(path: &str, home: &Path) -> PathBuf {
    let rest = path.strip_prefix("%HOME%").or_else(|| path.strip_prefix('~'));
    match rest {
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => home.join(rest.trim_start_matches(['/', '\\'])),
        _ => PathBuf::from(path),
    }
}

/// The folders to scan: the well-known ones, then the user's own, existing ones only, each once.
/// The library itself is never scanned.
pub fn scan_roots(home: &Path, user_folders: &[String], library_dir: &Path) -> Vec<PathBuf> {
    scan_roots_with(home, &[], user_folders, library_dir)
}

/// Like `scan_roots`, with some of the well-known folders (by name, like `Games`) switched off.
pub fn scan_roots_with(home: &Path, defaults_off: &[String], user_folders: &[String], library_dir: &Path) -> Vec<PathBuf> {
    let library = fs::canonicalize(library_dir).unwrap_or_else(|_| library_dir.to_path_buf());
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut roots = Vec::new();
    let wanted = WELL_KNOWN
        .iter()
        .filter(|n| !defaults_off.iter().any(|off| off == *n))
        .map(|n| home.join(n))
        .chain(user_folders.iter().map(|f| expand_home(f, home)));
    for root in wanted {
        let Ok(real) = fs::canonicalize(&root) else { continue };
        if !real.is_dir() || real == library || real.starts_with(&library) || seen.contains(&real) {
            continue;
        }
        seen.push(real);
        roots.push(root);
    }
    roots
}

/// Names a catalog entry answers to: its folder, its name, its name without a trailing "(note)", and the
/// project's name (the last part of its GitHub/GitLab source).
pub(crate) fn port_names(port: &Port) -> Vec<String> {
    let project = port.repo.as_deref().and_then(|r| r.rsplit('/').find(|s| !s.is_empty())).map(norm);
    let mut names: Vec<String> = Vec::new();
    for n in [norm(&port.folder), norm(&port.name), norm(port.display_name())].into_iter().chain(project) {
        if !n.is_empty() && !names.contains(&n) {
            names.push(n);
        }
    }
    names
}

/// Whether a (normalised) word is a version: `v2`, `1`, `2026`, `r3`.
fn version_like(token: &str) -> bool {
    let digits = token.strip_prefix('v').or_else(|| token.strip_prefix('r')).unwrap_or(token);
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

/// The words of a release-style name that can say which port it is: split on `-`, `_`, space and `.`,
/// normalised, without platform words (`linux`, `x86_64`, `appimage`...) and versions (`v1.2.3`).
/// `SoH-Ackbar-Delta-Linux` gives `soh ackbar delta`; `dusklight_v1.0_linux` gives `dusklight`.
pub(crate) fn release_tokens(name: &str) -> Vec<String> {
    let raw: Vec<String> = name.split(['-', '_', ' ', '.']).map(norm).filter(|t| !t.is_empty()).collect();
    raw.iter()
        .enumerate()
        .filter(|(i, t)| {
            let steam_deck = t.as_str() == "steam" && raw.get(i + 1).is_some_and(|n| n == "deck");
            !steam_deck && !version_like(t) && !NOISE_TOKENS.contains(&t.as_str())
        })
        .map(|(_, t)| t.clone())
        .collect()
}

/// The catalog folder a release word stands for (`soh` -> `Shipwright`), from `RELEASE_ALIASES`.
fn alias_folder(token: &str) -> Option<&'static str> {
    RELEASE_ALIASES.iter().find(|(alias, _)| *alias == token).map(|(_, folder)| *folder)
}

/// Whether a release word is a built-in alias for this catalog entry.
fn is_alias_of(token: &str, port: &Port) -> bool {
    alias_folder(token).is_some_and(|folder| norm(folder) == norm(&port.folder))
}

/// The entries (by index) that answer to `name` (normalised), Android-only ones left out.
fn entries_named(catalog: &[Port], name: &str) -> Vec<usize> {
    if name.is_empty() {
        return Vec::new();
    }
    catalog.iter().enumerate().filter(|(_, p)| !p.is_android_only() && port_names(p).iter().any(|n| n == name)).map(|(i, _)| i).collect()
}

/// How many words (three letters or more) a port shares with a folder name; a built-in alias at the start of
/// the folder counts as many. Ports sharing more are listed first when the user picks the port for a folder.
pub(crate) fn shared_words(dir_name: &str, port: &Port) -> usize {
    let words: Vec<String> = release_tokens(dir_name).into_iter().filter(|w| w.len() >= 3 && !WEAK_FIRST_WORDS.contains(&w.as_str())).collect();
    if words.first().is_some_and(|first| is_alias_of(first, port)) {
        return words.len().max(1) * 10;
    }
    let project = port.repo.as_deref().and_then(|r| r.rsplit('/').find(|s| !s.is_empty())).unwrap_or("");
    let port_words: Vec<String> = [port.name.as_str(), port.folder.as_str(), project].iter().flat_map(|s| release_tokens(s)).collect();
    let names = port_names(port);
    words.iter().filter(|w| port_words.contains(w) || names.contains(w)).count()
}

/// The entry (by index) a hint's key belongs to.
fn entry_with_key(catalog: &[Port], key: &str) -> Option<usize> {
    catalog.iter().position(|p| !p.is_android_only() && p.key() == key)
}

fn common_prefix(a: &[String], b: &[String]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

/// Matches by the release file names of the Deck-ready list. Strong: the folder's words are exactly a
/// download's words (`Severed_Chains_Steam_Deck`). Weak: the folder starts with the same words as a
/// download, the most of any port (`SoH-Ackbar-Delta-Linux` and `SoH-Dewey-Bravo.appimage` share `soh`);
/// one shared word counts only when no other port's downloads start with it.
fn asset_hits(words: &[String], catalog: &[Port], hints: &Hints) -> (Vec<usize>, Vec<usize>) {
    let mut strong: Vec<usize> = Vec::new();
    let mut best = 0usize;
    let mut tied: Vec<usize> = Vec::new();
    for (key, asset) in &hints.assets {
        let Some(i) = entry_with_key(catalog, key) else { continue };
        if asset.as_slice() == words && asset.concat().len() >= MIN_EXE_NAME_LEN && !strong.contains(&i) {
            strong.push(i);
        }
        let common = common_prefix(words, asset);
        if common == 0 || common < best {
            continue;
        }
        if common > best {
            best = common;
            tied.clear();
        }
        if !tied.contains(&i) {
            tied.push(i);
        }
    }
    if strong.len() > MAX_FITS {
        strong.clear();
    }
    let weak_ok = match best {
        0 => false,
        1 => tied.len() == 1 && words[0].len() >= 3 && !WEAK_FIRST_WORDS.contains(&words[0].as_str()),
        _ => tied.len() <= MAX_FITS,
    };
    (strong, if weak_ok { tied } else { Vec::new() })
}

/// Matches a folder named like a project's download, in this order:
/// 1. strong (`ReleaseName`): its words are a download's words, or the first k words joined (longest
///    first, k >= 2) are a port's name (`Severed_Chains_Steam_Deck` -> `severedchains`);
/// 2. weak (`ReleaseWord`): its first word is a port name of at least `MIN_EXE_NAME_LEN` letters
///    (`Dusklight-twilight-princess`), a built-in alias (`SoH-Ackbar-Delta-Linux`), or it starts like a
///    download of the Deck-ready list.
fn release_hits(dir_name: &str, catalog: &[Port], hints: &Hints) -> (MatchKind, Vec<usize>) {
    let words = release_tokens(dir_name);
    let Some(first) = words.first() else { return (MatchKind::ReleaseName, Vec::new()) };
    let (strong, weak) = asset_hits(&words, catalog, hints);
    if !strong.is_empty() {
        return (MatchKind::ReleaseName, strong);
    }
    // All the words joined (`Dusklight-Linux` -> `dusklight`), then fewer, down to two.
    let shortest = if words.len() == 1 { 1 } else { 2 };
    for k in (shortest..=words.len()).rev() {
        let hits = entries_named(catalog, &words[..k].concat());
        if !hits.is_empty() {
            return (MatchKind::ReleaseName, hits);
        }
    }
    let mut hits: Vec<usize> = if words.len() > 1 && first.len() >= MIN_EXE_NAME_LEN { entries_named(catalog, first) } else { Vec::new() };
    let aliases = catalog.iter().enumerate().filter(|(_, p)| !p.is_android_only() && is_alias_of(first, p)).map(|(i, _)| i);
    for i in aliases.chain(weak) {
        if !hits.contains(&i) {
            hits.push(i);
        }
    }
    hits.sort_unstable();
    (MatchKind::ReleaseWord, hits)
}

/// The program file name a catalog entry names for this platform, without extension, normalised.
pub(crate) fn declared_exe_stem(port: &Port) -> Option<String> {
    let exe = port.executable.as_ref().filter(|v| is_truthy(v))?;
    let value = resolve_per_platform(exe)?;
    let name = Path::new(value.as_str()?).file_stem()?.to_str()?.to_string();
    Some(norm(&name))
}

/// A program file found in a folder: its whole name and its release-style words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Program {
    /// The file name without extension, normalised (`soh`, `dusklightv203linuxx8664`).
    pub stem: String,
    /// The release words joined (`dusklight` for `Dusklight-v2.0.3-linux-x86_64.AppImage`).
    pub joined: String,
    /// The first release word (`soh` for `SoH-Dewey-Bravo.appimage`), for the alias table.
    pub first: String,
    /// The file name as found (`soh.appimage`), for "why it matched".
    pub file: String,
}

impl Program {
    /// Named like `name` (normalised, at least `MIN_EXE_NAME_LEN` letters), whole or by its release words.
    fn named(&self, name: &str) -> bool {
        name.len() >= MIN_EXE_NAME_LEN && (self.stem == name || self.joined == name)
    }

    /// The built-in alias of this entry (`2s2h.appimage` for 2 Ship 2 Harkinian).
    fn alias_of(&self, port: &Port) -> bool {
        is_alias_of(&self.stem, port) || is_alias_of(&self.first, port)
    }
}

/// Whether a file is a program: a known program extension (`.AppImage`, `.x86_64`, `.sh`, `.exe`...), or
/// no extension and executable. Config files, images, saves and ROMs never are.
fn is_program(path: &Path, entry: &fs::DirEntry) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => PROGRAM_EXTENSIONS.iter().any(|p| ext.eq_ignore_ascii_case(p)),
        None => is_executable(entry),
    }
}

#[cfg(unix)]
fn is_executable(entry: &fs::DirEntry) -> bool {
    use std::os::unix::fs::PermissionsExt;
    entry.metadata().is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(_entry: &fs::DirEntry) -> bool {
    false
}

/// The program files in a folder, two levels deep, capped (`MAX_EXE_SCAN_ENTRIES` entries looked at).
pub(crate) fn programs(dir: &Path) -> Vec<Program> {
    programs_within(dir, MAX_EXE_SCAN_DEPTH)
}

/// The program files in a folder and its sub-folders down to `max_depth` (0: the folder only), capped
/// (`MAX_EXE_SCAN_ENTRIES` entries looked at).
pub(crate) fn programs_within(dir: &Path, max_depth: usize) -> Vec<Program> {
    fn walk(dir: &Path, depth: usize, max_depth: usize, budget: &mut usize, out: &mut Vec<Program>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() {
                if depth < max_depth {
                    walk(&path, depth + 1, max_depth, budget, out);
                }
            } else if kind.is_file() && is_program(&path, &entry) {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    let words = release_tokens(stem);
                    let file = entry.file_name().to_string_lossy().into_owned();
                    out.push(Program { stem: norm(stem), joined: words.concat(), first: words.first().cloned().unwrap_or_default(), file });
                }
            }
        }
    }
    let mut out = Vec::new();
    let mut budget = MAX_EXE_SCAN_ENTRIES;
    walk(dir, 0, max_depth, &mut budget, &mut out);
    out
}

/// Whether one of `programs` is the one the catalog entry declares, or its built-in alias: the program
/// match that is trusted enough to be preselected.
pub(crate) fn declared_program_inside(port: &Port, programs: &[Program]) -> bool {
    let declared = declared_exe_stem(port);
    programs.iter().any(|p| declared.as_deref().is_some_and(|d| p.named(d)) || p.alias_of(port))
}

/// Holds a `roms` or `bios` folder (EmuDeck's `Emulation`, a ROM library) or is or holds a Steam library.
fn looks_like_game_library(dir: &Path, dir_name: &str) -> bool {
    if dir_name.eq_ignore_ascii_case("steamapps") || dir.join("libraryfolder.vdf").is_file() {
        return true;
    }
    let Ok(entries) = fs::read_dir(dir) else { return false };
    entries.flatten().take(MAX_EXE_SCAN_ENTRIES).any(|e| {
        let name = e.file_name().to_string_lossy().to_lowercase();
        ["roms", "bios", "steamapps"].contains(&name.as_str()) && e.path().is_dir()
    })
}

/// How a folder matched: the kind, the entries (by index) and why, in plain words for the user.
struct DirMatch {
    how: MatchKind,
    hits: Vec<usize>,
    why: String,
}

fn match_dir(dir: &Path, dir_name: &str, catalog: &[Port], hints: &Hints) -> DirMatch {
    let by_folder = entries_named(catalog, &norm(dir_name));
    if !by_folder.is_empty() {
        return DirMatch { how: MatchKind::FolderName, hits: by_folder, why: "folder name".to_string() };
    }
    let (how, by_release) = release_hits(dir_name, catalog, hints);
    if !by_release.is_empty() {
        let why = match how {
            MatchKind::ReleaseName => "folder named like its download".to_string(),
            _ => format!("folder name starts with \"{}\"", dir_name.split(['-', '_', ' ', '.']).find(|w| !w.is_empty()).unwrap_or(dir_name)),
        };
        return DirMatch { how, hits: by_release, why };
    }
    let found = programs(dir);
    let mut why = String::new();
    let by_exe = catalog
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            if p.is_android_only() {
                return false;
            }
            let wanted: Vec<String> = declared_exe_stem(p).into_iter().chain(port_names(p)).collect();
            match found.iter().find(|prog| prog.alias_of(p) || wanted.iter().any(|w| prog.named(w))) {
                Some(prog) => {
                    why = format!("program {}", prog.file);
                    true
                }
                None => false,
            }
        })
        .map(|(i, _)| i)
        .collect();
    DirMatch { how: MatchKind::ExecutableName, hits: by_exe, why }
}

/// Looks one level into each root. Reads only; nothing is created or changed.
pub fn scan(roots: &[PathBuf], catalog: &[Port], library_dir: &Path) -> ScanResult {
    scan_with(roots, catalog, library_dir, false, &|| false)
}

/// Sorted sub-folders of `dir` (links to folders count).
fn child_dirs(dir: &Path) -> Option<Vec<PathBuf>> {
    let entries = fs::read_dir(dir).ok()?;
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    Some(dirs)
}

/// `scan`, optionally one level deeper: a folder that matched nothing has its own sub-folders looked at
/// too (hidden ones skipped, at most `MAX_DEEP_FOLDERS` per scan; `deep_limit_hit` says when that cut it
/// short). `cancelled` is polled between folders; a cancelled scan returns what it found so far.
pub fn scan_with(roots: &[PathBuf], catalog: &[Port], library_dir: &Path, deeper: bool, cancelled: &dyn Fn() -> bool) -> ScanResult {
    scan_with_hints(roots, catalog, &Hints::default(), library_dir, deeper, cancelled)
}

/// `scan_with`, also matching by the release file names in `hints` (from the Deck-ready list).
pub fn scan_with_hints(roots: &[PathBuf], catalog: &[Port], hints: &Hints, library_dir: &Path, deeper: bool, cancelled: &dyn Fn() -> bool) -> ScanResult {
    let mut result = scan_roots_inner(roots, catalog, hints, library_dir, deeper, cancelled);
    // A top-level folder that holds a port found deeper is not itself an unrecognised port.
    let offered: Vec<PathBuf> = result
        .candidates
        .iter()
        .map(|c| c.path.clone())
        .chain(result.ambiguous.iter().map(|a| a.path.clone()))
        .chain(result.duplicates.iter().map(|(_, p)| p.clone()))
        .collect();
    result.unrecognised.retain(|u| !offered.iter().any(|p| p != u && p.starts_with(u)));
    result
}

fn scan_roots_inner(roots: &[PathBuf], catalog: &[Port], hints: &Hints, library_dir: &Path, deeper: bool, cancelled: &dyn Fn() -> bool) -> ScanResult {
    let mut result = ScanResult::default();
    let library = fs::canonicalize(library_dir).unwrap_or_else(|_| library_dir.to_path_buf());
    let mut deep_budget = MAX_DEEP_FOLDERS;
    for root in roots {
        let Some(top) = child_dirs(root) else { continue };
        result.roots_scanned.push(root.clone());
        // (folder, may we look inside it when it matches nothing, is it directly in the scan folder)
        let mut queue: std::collections::VecDeque<(PathBuf, bool, bool)> = top.into_iter().map(|d| (d, deeper, true)).collect();
        while let Some((dir, may_descend, top_level)) = queue.pop_front() {
            if cancelled() {
                return result;
            }
            let Some(dir_name) = dir.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
            // Hidden folders (`.Trash-1000`, `.cache`) and Steam's, EmuDeck's and the file system's own
            // folders are never ports and are not listed at all.
            if dir_name.starts_with('.') || NEVER_PORTS.contains(&dir_name.to_lowercase().as_str()) {
                continue;
            }
            // Never the library itself, anything in it, or a folder that contains it (the app's own
            // folder, usually): its `Library/<port>` folders would all look like installs to adopt.
            if fs::canonicalize(&dir).is_ok_and(|real| real.starts_with(&library) || library.starts_with(&real)) {
                continue;
            }
            let DirMatch { how, hits, why } = match_dir(&dir, &dir_name, catalog, hints);
            // Named like a port, it is that port even with a `roms` folder inside; otherwise a folder of
            // ROMs, emulators or Steam games is a collection, and so is one that fits many ports.
            let named = matches!(how, MatchKind::FolderName | MatchKind::ReleaseName | MatchKind::ReleaseWord) && !hits.is_empty();
            if hits.len() > MAX_FITS || (!named && looks_like_game_library(&dir, &dir_name)) {
                result.collections.push(dir);
                continue;
            }
            match hits.as_slice() {
                [] => {
                    // "Not recognised" needs a program in the folder or one level in, as a download unpacks.
                    if top_level && !programs_within(&dir, 1).is_empty() {
                        result.unrecognised.push(dir.clone());
                    }
                    if may_descend {
                        for child in child_dirs(&dir).unwrap_or_default() {
                            let hidden = child.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
                            if hidden {
                                continue;
                            }
                            if deep_budget == 0 {
                                result.deep_limit_hit = true;
                                break;
                            }
                            deep_budget -= 1;
                            queue.push_back((child, false, false));
                        }
                    }
                }
                [one] => {
                    let port = &catalog[*one];
                    if crate::core::installer::is_installed(port, library_dir) {
                        result.already_in_library.push(port.display_name().to_string());
                    } else if result.candidates.iter().any(|c| c.port_key == port.key()) {
                        result.duplicates.push((port.display_name().to_string(), dir));
                    } else {
                        result.candidates.push(Candidate {
                            path: dir,
                            dir_name,
                            port_key: port.key().to_string(),
                            port_name: port.display_name().to_string(),
                            catalog_folder: port.folder.clone(),
                            how,
                            why,
                        });
                    }
                }
                many => result.ambiguous.push(Ambiguous {
                    path: dir,
                    dir_name,
                    matches: many.iter().map(|&i| catalog[i].name.clone()).collect(),
                    keys: many.iter().map(|&i| catalog[i].key().to_string()).collect(),
                }),
            }
        }
    }
    result
}

#[derive(Debug)]
pub enum AdoptError {
    /// `Library/<folder>` is already taken.
    LibraryFolderTaken(PathBuf),
    /// The folder to adopt is missing, not a folder, or inside the library.
    BadInstallFolder(PathBuf),
    BadCatalogFolder(String),
    /// `ports.local.json` already has an entry for this folder.
    AlreadyRegistered(String),
    /// `ports.local.json` exists but is not the expected shape; it is left alone.
    LocalFileUnreadable(PathBuf),
    /// Symlinks are not available here.
    Unsupported,
    Io { action: &'static str, path: PathBuf, source: io::Error },
}

impl fmt::Display for AdoptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AdoptError::LibraryFolderTaken(p) => write!(f, "{} already exists, so nothing was changed", p.display()),
            AdoptError::BadInstallFolder(p) => write!(f, "{} is not a folder that can be adopted", p.display()),
            AdoptError::BadCatalogFolder(s) => write!(f, "the catalog folder name \"{s}\" is not usable"),
            AdoptError::AlreadyRegistered(s) => write!(f, "the app's port list already has an entry for \"{s}\""),
            AdoptError::LocalFileUnreadable(p) => write!(f, "{} is not in the expected format; it was not touched", p.display()),
            AdoptError::Unsupported => write!(f, "adopting needs Linux (it links the install into the library)"),
            AdoptError::Io { action, path, source } => write!(f, "could not {action} {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for AdoptError {}

fn io_err<'a>(action: &'static str, path: &'a Path) -> impl FnOnce(io::Error) -> AdoptError + 'a {
    move |source| AdoptError::Io { action, path: path.to_path_buf(), source }
}

/// The `ports.local.json` entry for an adopted port: catalog data, no `source` (see module doc).
pub fn local_entry(port: &Port) -> Value {
    let mut entry = Map::new();
    entry.insert("name".into(), json!(port.name));
    entry.insert("folder".into(), json!(port.folder));
    let mut put = |key: &str, value: Option<Value>| {
        if let Some(value) = value {
            entry.insert(key.into(), value);
        }
    };
    if !port.tags_lower.is_empty() {
        put("tags", Some(json!(port.tags_lower)));
    }
    let website = port.website.clone().or_else(|| match (port.source_type, port.source.as_str()) {
        (SourceType::Github | SourceType::Gitlab, Some(url)) => Some(url.to_string()),
        _ => None,
    });
    put("website", website.map(Value::from));
    put("instructions", (!port.instructions.is_empty()).then(|| json!(port.instructions)));
    put("mods", port.mods.clone().map(Value::from));
    put("image", port.image.clone().map(Value::from));
    put("icon", port.icon.clone().map(Value::from));
    put("executable", port.executable.clone().filter(is_truthy));
    put("save", port.save.clone());
    put("save2", port.save2.clone());
    Value::Object(entry)
}

#[derive(Debug, PartialEq, Eq)]
pub struct Adopted {
    pub link: PathBuf,
    pub backup_of_local_file: Option<PathBuf>,
}

#[cfg(unix)]
fn make_link(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn make_link(_target: &Path, _link: &Path) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "no symlinks"))
}

/// Reads `ports.local.json` (missing is an empty list). Anything that is not `{"ports": [...]}` is refused.
pub(crate) fn read_local_file(path: &Path) -> Result<(Map<String, Value>, Option<Vec<u8>>), AdoptError> {
    match fs::read(path) {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
            match serde_json::from_str::<Value>(text) {
                Ok(Value::Object(map)) if map.get("ports").is_none_or(Value::is_array) => Ok((map, Some(bytes))),
                _ => Err(AdoptError::LocalFileUnreadable(path.to_path_buf())),
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok((Map::new(), None)),
        Err(e) => Err(io_err("read", path)(e)),
    }
}

/// Copies the current bytes to `<file>.bak-<stamp>`, or `-1`, `-2`... when that name is taken, so several
/// adoptions in the same second each keep their own backup and none overwrites another.
pub(crate) fn write_backup(path: &Path, bytes: &[u8], stamp: &str) -> Result<PathBuf, AdoptError> {
    let base = path.file_name().unwrap_or_default().to_os_string();
    for attempt in 0..1000u32 {
        let mut name = base.clone();
        name.push(if attempt == 0 { format!(".bak-{stamp}") } else { format!(".bak-{stamp}-{attempt}") });
        let backup = path.with_file_name(name);
        match fs::OpenOptions::new().write(true).create_new(true).open(&backup) {
            Ok(mut file) => {
                io::Write::write_all(&mut file, bytes).map_err(io_err("back up", &backup))?;
                return Ok(backup);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io_err("back up", &backup)(e)),
        }
    }
    Err(io_err("back up", path)(io::Error::new(io::ErrorKind::AlreadyExists, "too many backups with this timestamp")))
}

/// Links `Library/<folder>` to the install and registers the entry. `stamp` names the backup of an
/// existing `ports.local.json` (`backup_stamp` style). On any failure the link made here is removed.
pub fn adopt(install: &Path, port: &Port, library_dir: &Path, local_json: &Path, stamp: &str) -> Result<Adopted, AdoptError> {
    let real = fs::canonicalize(install).map_err(|_| AdoptError::BadInstallFolder(install.to_path_buf()))?;
    let library = fs::canonicalize(library_dir).unwrap_or_else(|_| library_dir.to_path_buf());
    // Neither inside the library nor containing it (a link back up the tree would be a cycle).
    if !real.is_dir() || is_within(&real, &library) || is_within(&library, &real) {
        return Err(AdoptError::BadInstallFolder(install.to_path_buf()));
    }
    // The folder name must be one plain name: it becomes a direct child of the library.
    let mut parts = Path::new(&port.folder).components();
    if !matches!((parts.next(), parts.next()), (Some(Component::Normal(_)), None)) {
        return Err(AdoptError::BadCatalogFolder(port.folder.clone()));
    }
    let link = library_dir.join(&port.folder);
    if fs::symlink_metadata(&link).is_ok() {
        return Err(AdoptError::LibraryFolderTaken(link));
    }

    let (mut file, original) = read_local_file(local_json)?;
    let ports = file.entry("ports").or_insert_with(|| Value::Array(Vec::new()));
    let Value::Array(ports) = ports else { return Err(AdoptError::LocalFileUnreadable(local_json.to_path_buf())) };
    if ports.iter().any(|p| p.get("folder").and_then(Value::as_str).is_some_and(|f| f == port.folder)) {
        return Err(AdoptError::AlreadyRegistered(port.folder.clone()));
    }
    ports.push(local_entry(port));
    let text = serde_json::to_string_pretty(&file).map_err(|e| io_err("encode", local_json)(io::Error::other(e)))?;

    fs::create_dir_all(library_dir).map_err(io_err("create", library_dir))?;
    make_link(&real, &link).map_err(|e| match e.kind() {
        io::ErrorKind::Unsupported => AdoptError::Unsupported,
        _ => io_err("link", &link)(e),
    })?;

    let backup = match &original {
        Some(bytes) => {
            match write_backup(local_json, bytes, stamp) {
                Ok(backup) => Some(backup),
                Err(e) => {
                    let _ = fs::remove_file(&link);
                    return Err(e);
                }
            }
        }
        None => None,
    };
    if let Err(e) = crate::core::files::write_atomic(local_json, text.as_bytes()) {
        let _ = fs::remove_file(&link);
        return Err(io_err("write", local_json)(e));
    }
    Ok(Adopted { link, backup_of_local_file: backup })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::port_from_value;
    use serde_json::json;

    fn temp_dir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("dpm_adopt_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn catalog() -> Vec<Port> {
        [
            json!({"name": "Ocarina of Time (Ship of Harkinian)", "folder": "SoH", "source": "https://github.com/HarbourMasters/Shipwright",
                   "tags": ["N64"], "image": "https://example.com/soh.png", "executable": {"linux": "soh.appimage", "windows": "soh.exe"},
                   "save": "~/.local/share/soh", "extra": "https://example.com/extra.zip", "preferred_asset": "linux"}),
            json!({"name": "Majora's Mask (2 Ship 2 Harkinian)", "folder": "2s2h", "source": "https://github.com/HarbourMasters/2ship2harkinian"}),
            json!({"name": "Twilight Princess (dusklight)", "folder": "dusklight", "source": "https://github.com/TwilitRealm/dusklight"}),
            json!({"name": "Other Game", "folder": "othergame", "source": "https://example.com/other.zip", "executable": "bin/othergame-launcher"}),
            json!({"name": "Phone Thing", "folder": "phonething", "source": "https://example.com/p.zip", "tags": ["android_only"]}),
            json!({"name": "Fan Remake", "folder": "fanremake", "source": "https://github.com/x/fanremake"}),
            json!({"name": "Fan Remake Deluxe", "folder": "Fan-Remake", "source": "https://github.com/x/fanremake2"}),
        ]
        .iter()
        .map(|v| port_from_value(v).unwrap())
        .collect()
    }

    fn mkdirs(root: &Path, rel: &[&str]) {
        for r in rel {
            fs::create_dir_all(root.join(r)).unwrap();
        }
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"x").unwrap();
    }

    use super::test_fixtures::{tims_sd_card, touch_exe};

    /// Entries shaped like the real catalog's (public catalog data: names, folders, sources).
    fn real_catalog() -> Vec<Port> {
        [
            json!({"name": "The Legend of Zelda: Ocarina of Time (Shipwright)", "folder": "Shipwright", "source": "https://github.com/HarbourMasters/Shipwright"}),
            json!({"name": "The Legend of Zelda: Majora's Mask (2ship2harkinian)", "folder": "2ship2harkinian", "source": "https://github.com/HarbourMasters/2ship2harkinian"}),
            json!({"name": "The Legend of Zelda: Majora's Mask (Zelda64Recomp)", "folder": "Zelda64Recomp", "source": "https://github.com/Zelda64Recomp/Zelda64Recomp"}),
            json!({"name": "The Legend of Zelda: Twilight Princess (dusklight)", "folder": "dusklight", "source": "https://github.com/TwilitRealm/dusklight"}),
            json!({"name": "The Legend of Dragoon (Severed-Chains)", "folder": "Severed-Chains", "source": "https://github.com/Legend-of-Dragoon-Modding/Severed-Chains"}),
            json!({"name": "SkyRecomp", "folder": "SkyRecomp", "source": "https://github.com/Apolinario-coder/SkyRecomp"}),
            json!({"name": "Star Fox 64 (starship)", "folder": "starship", "source": "https://github.com/harbourmasters/starship"}),
            json!({"name": "Mario Kart 64 (SpaghettiKart)", "folder": "SpaghettiKart", "source": "https://github.com/HarbourMasters/SpaghettiKart"}),
            json!({"name": "Doom (uzdoom)", "folder": "uzdoom", "source": "https://github.com/UZDoom/uzdoom"}),
            json!({"name": "Driver 2 (REDRIVER2)", "folder": "REDRIVER2", "source": "https://github.com/OpenDriver2/REDRIVER2"}),
        ]
        .iter()
        .map(|v| port_from_value(v).unwrap())
        .collect()
    }

    /// Release file names as the Deck-ready list has them (latest release of each project, 2026-10-23).
    fn real_hints() -> Hints {
        let mut hints = Hints::default();
        for (key, file) in [
            ("HarbourMasters/Shipwright", "SoH-Dewey-Bravo.appimage"),
            ("HarbourMasters/2ship2harkinian", "2Ship-Battler-Bravo-Linux.zip"),
            ("TwilitRealm/dusklight", "Dusklight-v2.0.3-linux-x86_64.AppImage"),
            ("Legend-of-Dragoon-Modding/Severed-Chains", "Severed_Chains_Steam_Deck.tar.gz"),
            ("Legend-of-Dragoon-Modding/Severed-Chains", "Severed_Chains_Linux.tar.gz"),
            ("Zelda64Recomp/Zelda64Recomp", "Zelda64Recompiled-v1.2.2-Linux-X64.zip"),
            ("HarbourMasters/SpaghettiKart", "spaghetti-linux.zip"),
            ("harbourmasters/starship", "Starship-Barnard-Alfa-Linux.zip"),
        ] {
            hints.add_asset(key, file);
        }
        hints
    }

    fn found_folders(result: &ScanResult) -> Vec<(String, String, MatchKind)> {
        let mut found: Vec<_> = result.candidates.iter().map(|c| (c.dir_name.clone(), c.catalog_folder.clone(), c.how)).collect();
        found.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        found
    }

    fn row(dir: &str, folder: &str, how: MatchKind) -> (String, String, MatchKind) {
        (dir.to_string(), folder.to_string(), how)
    }

    #[test]
    fn release_words_drop_platforms_and_versions() {
        assert_eq!(release_tokens("SoH-Ackbar-Delta-Linux"), ["soh", "ackbar", "delta"]);
        assert_eq!(release_tokens("2Ship-Keiichi-Charlie-Linux"), ["2ship", "keiichi", "charlie"]);
        assert_eq!(release_tokens("dusklight_v1.0_linux"), ["dusklight"]);
        assert_eq!(release_tokens("Dusklight-v2.0.3-linux-x86_64"), ["dusklight"]);
        assert_eq!(release_tokens("Severed_Chains_Steam_Deck"), ["severed", "chains"]);
        assert_eq!(release_tokens("Steam-Workshop-Tools"), ["steam", "workshop", "tools"], "steam is noise only before deck");
        assert_eq!(release_tokens("Sky.Recomp-v1.0.6-linux-x86_64"), ["sky", "recomp"]);
        assert_eq!(release_tokens("Game 1.2.3 portable (nightly)"), ["game"]);
        assert!(release_tokens("Linux-x64-AppImage").is_empty());
        assert_eq!(asset_tokens("Severed_Chains_Steam_Deck.tar.gz"), ["severed", "chains"]);
        assert_eq!(asset_tokens("SoH-Dewey-Bravo.appimage"), ["soh", "dewey", "bravo"]);
    }

    #[test]
    fn folders_named_like_the_release_download_match_their_port() {
        let dir = temp_dir("release_names");
        let sd = dir.join("sd");
        for name in [
            "SoH-Ackbar-Delta-Linux",
            "2Ship-Keiichi-Charlie-Linux",
            "Dusklight-Linux",
            "Dusklight-twilight-princess",
            "Starship-Barnard-Alfa-Linux",
            "Severed_Chains_Steam_Deck",
            "Sky.Recomp-v1.0.6-linux-x86_64",
        ] {
            touch(&sd.join(name).join("readme.txt"));
        }
        let result = scan(&[sd], &real_catalog(), &dir.join("Ports"));
        assert_eq!(
            found_folders(&result),
            [
                row("2Ship-Keiichi-Charlie-Linux", "2ship2harkinian", MatchKind::ReleaseWord),
                row("Dusklight-Linux", "dusklight", MatchKind::ReleaseName),
                row("Severed_Chains_Steam_Deck", "Severed-Chains", MatchKind::ReleaseName),
                row("Sky.Recomp-v1.0.6-linux-x86_64", "SkyRecomp", MatchKind::ReleaseName),
                row("SoH-Ackbar-Delta-Linux", "Shipwright", MatchKind::ReleaseWord),
                row("Starship-Barnard-Alfa-Linux", "starship", MatchKind::ReleaseWord),
            ]
        );
        assert_eq!(result.duplicates.len(), 1, "the second Dusklight folder is reported: {:?}", result.duplicates);
        assert!(result.ambiguous.is_empty() && result.unrecognised.is_empty() && result.collections.is_empty());
        let why: Vec<&str> = result.candidates.iter().map(|c| c.why.as_str()).collect();
        assert!(why.contains(&"folder named like its download") && why.contains(&"folder name starts with \"SoH\""), "{why:?}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_lowercase_versioned_release_folder_matches_too() {
        let dir = temp_dir("release_lower");
        let sd = dir.join("sd");
        touch(&sd.join("dusklight_v1.0_linux").join("x.txt"));
        let result = scan(&[sd], &real_catalog(), &dir.join("Ports"));
        assert_eq!(found_folders(&result), [row("dusklight_v1.0_linux", "dusklight", MatchKind::ReleaseName)]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn deck_ready_download_names_match_folders_the_catalog_names_differently() {
        let dir = temp_dir("release_hints");
        let sd = dir.join("sd");
        for name in ["Zelda64Recompiled-v1.2.2-Linux-X64", "spaghetti-linux", "Severed_Chains_Linux", "SoH-Ackbar-Delta-Linux"] {
            touch(&sd.join(name).join("readme.txt"));
        }
        assert!(scan(&[sd.clone()], &real_catalog(), &dir.join("Ports")).candidates.iter().all(|c| c.dir_name.starts_with("S")), "without the list only the built-in names match");
        let result = scan_with_hints(&[sd], &real_catalog(), &real_hints(), &dir.join("Ports"), false, &|| false);
        assert_eq!(
            found_folders(&result),
            [
                row("Severed_Chains_Linux", "Severed-Chains", MatchKind::ReleaseName),
                row("SoH-Ackbar-Delta-Linux", "Shipwright", MatchKind::ReleaseWord),
                row("Zelda64Recompiled-v1.2.2-Linux-X64", "Zelda64Recomp", MatchKind::ReleaseName),
                row("spaghetti-linux", "SpaghettiKart", MatchKind::ReleaseName),
            ]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn one_shared_first_word_is_not_enough_when_several_downloads_start_with_it() {
        let ports: Vec<Port> = [
            json!({"name": "Tony Hawk's Pro Skater", "folder": "thps-a", "source": "https://github.com/a/thps-a"}),
            json!({"name": "Tony Hawk's Underground", "folder": "thug-b", "source": "https://github.com/b/thug-b"}),
        ]
        .iter()
        .map(|v| port_from_value(v).unwrap())
        .collect();
        let mut hints = Hints::default();
        hints.add_asset("a/thps-a", "Tony-Hawks-Pro-Skater-linux.zip");
        hints.add_asset("b/thug-b", "Tony-Hawks-Underground-linux.zip");
        let words = |s: &str| release_tokens(s);
        assert_eq!(asset_hits(&words("Tony-Photos"), &ports, &hints), (vec![], vec![]));
        assert_eq!(asset_hits(&words("Tony-Hawks-Pro-Skater-Remix-v2"), &ports, &hints), (vec![], vec![0]), "three words in common pick one");
        assert_eq!(asset_hits(&words("Tony-Hawks-Pro-Skater"), &ports, &hints), (vec![0], vec![0]), "every word in common is strong");
        assert_eq!(asset_hits(&words("Tony-Hawks"), &ports, &hints).1, vec![0, 1], "two words shared by both: choose");
        let mut the = Hints::default();
        the.add_asset("a/thps-a", "The-Game-linux.zip");
        assert_eq!(asset_hits(&words("The-Photos"), &ports, &the), (vec![], vec![]), "a weak first word never matches alone");
    }

    #[test]
    fn folders_that_only_look_a_bit_like_a_release_do_not_match() {
        let dir = temp_dir("release_negative");
        let sd = dir.join("sd");
        for name in ["Photos-2024", "SoHo-Trip", "Linux", "x86_64", "My-Saves", "Driver-Updates", "Sky-Photos", "Steam-Screenshots"] {
            touch(&sd.join(name).join("a.txt"));
        }
        let result = scan_with_hints(&[sd], &real_catalog(), &real_hints(), &dir.join("Ports"), false, &|| false);
        assert!(result.candidates.is_empty(), "{:?}", found_folders(&result));
        assert!(result.ambiguous.is_empty() && result.unrecognised.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_project_name_from_the_source_is_a_port_name() {
        let ports = real_catalog();
        assert!(port_names(&ports[0]).contains(&"shipwright".to_string()));
        let mut renamed = ports[0].clone();
        renamed.folder = "soh-pc".into();
        assert!(port_names(&renamed).contains(&"shipwright".to_string()), "{:?}", port_names(&renamed));
        let no_repo = port_from_value(&json!({"name": "Xonotic", "folder": "xonotic", "source": "https://dl.example.org/xonotic-0.8.6.zip"})).unwrap();
        assert_eq!(port_names(&no_repo), ["xonotic"]);
    }

    #[test]
    fn data_files_and_roms_named_like_a_port_are_not_programs_even_when_executable() {
        let dir = temp_dir("not_programs");
        let games = dir.join("Games");
        for file in ["starship.json", "dusklight.png", "Shipwright.txt", "uzdoom.sav", "SkyRecomp.o2r", "spaghettikart.otr", "severedchains.ini"] {
            touch(&games.join("stuff").join(file));
        }
        // ROMs on the Deck often carry the executable bit; the extension decides.
        touch_exe(&games.join("stuff").join("The Legend of Zelda - Majora's Mask.z64"));
        touch_exe(&games.join("stuff").join("2ship2harkinian.json"));
        let result = scan(&[games.clone()], &real_catalog(), &dir.join("Ports"));
        assert!(result.candidates.is_empty() && result.ambiguous.is_empty(), "{:?} {:?}", found_folders(&result), result.ambiguous);
        assert!(result.unrecognised.is_empty(), "no program inside, so nothing to choose");
        // The same name as a program does match, and says which program.
        touch(&games.join("launch").join("dusklight.sh"));
        let result = scan(&[games], &real_catalog(), &dir.join("Ports"));
        assert_eq!(found_folders(&result), [row("launch", "dusklight", MatchKind::ExecutableName)]);
        assert_eq!(result.candidates[0].why, "program dusklight.sh");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn program_files_are_known_extensions_or_executable_files_without_one() {
        let dir = temp_dir("program_kinds");
        for name in ["a.AppImage", "b.x86_64", "c.x86", "d.sh", "e.run", "f.bin", "g.elf", "h.EXE"] {
            touch(&dir.join(name));
        }
        touch(&dir.join("plain"));
        touch(&dir.join("notes.md"));
        let mut stems: Vec<String> = programs(&dir).into_iter().map(|p| p.stem).collect();
        stems.sort();
        assert_eq!(stems, ["a", "b", "c", "d", "e", "f", "g", "h"], "no extension and not executable is data");
        #[cfg(unix)]
        {
            touch_exe(&dir.join("plain"));
            touch_exe(&dir.join("rom.z64"));
            let stems: Vec<String> = programs(&dir).into_iter().map(|p| p.stem).collect();
            assert!(stems.contains(&"plain".to_string()) && !stems.contains(&"rom".to_string()), "{stems:?}");
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_release_named_program_matches_by_its_words_and_aliases_count_for_programs() {
        let dir = temp_dir("program_release");
        let games = dir.join("Games");
        touch(&games.join("zelda tp").join("Dusklight-v2.0.3-linux-x86_64.AppImage"));
        touch(&games.join("mm").join("2s2h.appimage"));
        let result = scan(&[games], &real_catalog(), &dir.join("Ports"));
        assert_eq!(found_folders(&result), [row("mm", "2ship2harkinian", MatchKind::ExecutableName), row("zelda tp", "dusklight", MatchKind::ExecutableName)]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn tims_sd_card_offers_his_four_ports_and_nothing_else() {
        let dir = temp_dir("tims_sd");
        let sd = tims_sd_card(&dir);
        for hints in [Hints::default(), real_hints()] {
            let result = scan_with_hints(&[sd.clone()], &real_catalog(), &hints, &sd.join("Ports"), false, &|| false);
            assert_eq!(
                found_folders(&result),
                [
                    row("2Ship-Keiichi-Charlie-Linux", "2ship2harkinian", MatchKind::ReleaseWord),
                    row("Dusklight-twilight-princess", "dusklight", MatchKind::ReleaseWord),
                    row("Severed_Chains_Steam_Deck", "Severed-Chains", MatchKind::ReleaseName),
                    row("SoH-Ackbar-Delta-Linux", "Shipwright", MatchKind::ReleaseWord),
                ],
                "with hints: {}",
                !hints.is_empty()
            );
            assert!(result.ambiguous.is_empty(), "{:?}", result.ambiguous);
            assert!(result.unrecognised.is_empty(), "{:?}", result.unrecognised);
            assert!(result.collections.is_empty(), "Emulation, steamapps and lost+found are skipped silently: {:?}", result.collections);
            assert!(result.duplicates.is_empty() && result.already_in_library.is_empty());
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_folder_with_a_program_the_app_does_not_know_can_be_chosen_but_data_folders_stay_silent() {
        let dir = temp_dir("unrecognised");
        let sd = tims_sd_card(&dir);
        touch_exe(&sd.join("Mystery Game").join("bin").join("game.x86_64"));
        touch_exe(&sd.join("Too Deep").join("a").join("b").join("game.x86_64"));
        touch(&sd.join("Photos").join("beach.jpg"));
        let result = scan(&[sd.clone()], &real_catalog(), &sd.join("Ports"));
        assert_eq!(result.unrecognised, [sd.join("Mystery Game")], "a program at most one folder in");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_folder_that_fits_more_than_three_ports_is_a_collection() {
        let dir = temp_dir("collection");
        let games = dir.join("Games");
        for exe in ["uzdoom.sh", "starship.sh", "dusklight.sh", "spaghettikart.sh"] {
            touch(&games.join("bundle").join(exe));
        }
        for exe in ["uzdoom.sh", "starship.sh", "dusklight.sh"] {
            touch(&games.join("three").join(exe));
        }
        let result = scan(&[games.clone()], &real_catalog(), &dir.join("Ports"));
        assert_eq!(result.collections, [games.join("bundle")]);
        assert_eq!(result.ambiguous.len(), 1, "three fits can still be chosen from");
        assert_eq!(result.ambiguous[0].keys.len(), 3);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_roms_folder_or_a_steam_library_makes_a_collection_unless_the_folder_is_named_like_a_port() {
        let dir = temp_dir("library_signs");
        let sd = dir.join("sd");
        touch_exe(&sd.join("RetroStuff").join("roms").join("x.z64"));
        touch(&sd.join("RetroStuff").join("retroarch.sh"));
        touch(&sd.join("Games2").join("steamapps").join("common").join("Game").join("game.exe"));
        touch(&sd.join("Shipwright").join("roms").join("readme.txt"));
        for skipped in ["steamapps/common/Game/game.exe", "SteamLibrary/x.sh", "lost+found/x.sh", "Emulation/tools/doom.sh", "compatdata/1/x.exe", "shadercache/1/x.sh"] {
            touch(&sd.join(skipped));
        }
        let result = scan(&[sd.clone()], &real_catalog(), &dir.join("Ports"));
        let mut collections = result.collections.clone();
        collections.sort();
        assert_eq!(collections, [sd.join("Games2"), sd.join("RetroStuff")]);
        assert_eq!(found_folders(&result), [row("Shipwright", "Shipwright", MatchKind::FolderName)]);
        assert!(result.unrecognised.is_empty(), "{:?}", result.unrecognised);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn two_entries_for_one_game_make_a_choosable_folder_with_both_keys() {
        let dir = temp_dir("choosable");
        let sd = dir.join("sd");
        touch(&sd.join("The Legend of Zelda Majora's Mask").join("mm.appimage"));
        let result = scan(&[sd], &real_catalog(), &dir.join("Ports"));
        assert_eq!(result.ambiguous.len(), 1);
        let a = &result.ambiguous[0];
        assert_eq!(a.keys, ["HarbourMasters/2ship2harkinian", "Zelda64Recomp/Zelda64Recomp"]);
        assert_eq!(a.matches, ["The Legend of Zelda: Majora's Mask (2ship2harkinian)", "The Legend of Zelda: Majora's Mask (Zelda64Recomp)"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_folder_holding_ports_found_deeper_is_not_listed_as_unrecognised() {
        let dir = temp_dir("unrecognised_parent");
        let sd = dir.join("sd");
        touch(&sd.join("Games").join("Dusklight-Linux").join("launch.sh"));
        let shallow = scan_with(&[sd.clone()], &real_catalog(), &dir.join("Ports"), false, &|| false);
        assert_eq!(shallow.unrecognised, [sd.join("Games")], "without looking deeper the user may still choose it");
        let deep = scan_with(&[sd.clone()], &real_catalog(), &dir.join("Ports"), true, &|| false);
        assert!(deep.unrecognised.is_empty(), "{:?}", deep.unrecognised);
        assert_eq!(deep.candidates.len(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn folders_match_by_catalog_folder_or_name_ignoring_case_and_punctuation() {
        let dir = temp_dir("by_folder");
        let apps = dir.join("Applications");
        mkdirs(&apps, &["soh", "2S2H", "Twilight-Princess", "unrelated"]);
        let result = scan(&[apps.clone()], &catalog(), &dir.join("Library"));
        let found: Vec<(&str, &str, MatchKind)> =
            result.candidates.iter().map(|c| (c.dir_name.as_str(), c.catalog_folder.as_str(), c.how)).collect();
        assert_eq!(
            found,
            [
                ("2S2H", "2s2h", MatchKind::FolderName),
                ("Twilight-Princess", "dusklight", MatchKind::FolderName),
                ("soh", "SoH", MatchKind::FolderName)
            ]
        );
        assert!(result.ambiguous.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_program_name_inside_matches_when_the_folder_name_does_not() {
        let dir = temp_dir("by_exe");
        let games = dir.join("Games");
        touch(&games.join("zelda64").join("dusklight.AppImage"));
        touch_exe(&games.join("misc").join("bin").join("othergame-launcher"));
        touch(&games.join("junk").join("run.sh"));
        let result = scan(&[games], &catalog(), &dir.join("Library"));
        let found: Vec<(&str, &str, MatchKind)> =
            result.candidates.iter().map(|c| (c.dir_name.as_str(), c.catalog_folder.as_str(), c.how)).collect();
        assert_eq!(found, [("misc", "othergame", MatchKind::ExecutableName), ("zelda64", "dusklight", MatchKind::ExecutableName)]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_folder_that_fits_two_entries_equally_is_ambiguous_and_not_offered() {
        let dir = temp_dir("ambiguous");
        let apps = dir.join("Applications");
        mkdirs(&apps, &["Fan Remake"]);
        let result = scan(&[apps], &catalog(), &dir.join("Library"));
        assert!(result.candidates.is_empty());
        assert_eq!(result.ambiguous.len(), 1);
        assert_eq!(result.ambiguous[0].matches, ["Fan Remake", "Fan Remake Deluxe"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn android_only_entries_and_already_installed_ports_are_not_offered() {
        let dir = temp_dir("skips");
        let apps = dir.join("Applications");
        let library = dir.join("Library");
        mkdirs(&apps, &["phonething", "SoH", "dusklight"]);
        mkdirs(&library, &["SoH"]);
        let result = scan(&[apps], &catalog(), &library);
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].catalog_folder, "dusklight");
        assert_eq!(result.already_in_library, ["Ocarina of Time"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_library_is_never_scanned_even_through_a_scan_folder_that_contains_it() {
        let dir = temp_dir("library_skip");
        let library = dir.join("Games").join("Library");
        mkdirs(&library, &["SoH"]);
        let roots = scan_roots(&dir, &[library.to_string_lossy().into_owned()], &library);
        assert_eq!(roots, [dir.join("Games")]);
        let result = scan(&roots, &catalog(), &library);
        assert!(result.candidates.is_empty() && result.already_in_library.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn scan_roots_are_well_known_plus_user_folders_existing_only_and_deduped() {
        let dir = temp_dir("roots");
        mkdirs(&dir, &["Applications", "Games", "elsewhere"]);
        let user = vec!["~/elsewhere".to_string(), "%HOME%/Games".to_string(), "/no/such/place".to_string(), "".to_string()];
        let roots = scan_roots(&dir, &user, &dir.join("Library"));
        assert_eq!(roots, [dir.join("Applications"), dir.join("Games"), dir.join("elsewhere")]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn expand_home_handles_tilde_and_percent_home_only_at_the_start() {
        let home = Path::new("/home/deck");
        assert_eq!(expand_home("~/Games", home), Path::new("/home/deck/Games"));
        assert_eq!(expand_home("~", home), Path::new("/home/deck"));
        assert_eq!(expand_home("%HOME%/x", home), Path::new("/home/deck/x"));
        assert_eq!(expand_home("~other/x", home), Path::new("~other/x"));
        assert_eq!(expand_home("/mnt/~/x", home), Path::new("/mnt/~/x"));
    }

    #[test]
    fn the_local_entry_keeps_catalog_data_but_never_the_source_or_install_fields() {
        let entry = local_entry(&catalog()[0]);
        let obj = entry.as_object().unwrap();
        assert_eq!(obj["folder"], "SoH");
        assert_eq!(obj["website"], "https://github.com/HarbourMasters/Shipwright");
        assert_eq!(obj["tags"], json!(["n64"]));
        assert_eq!(obj["image"], "https://example.com/soh.png");
        assert_eq!(obj["save"], "~/.local/share/soh");
        assert!(obj.contains_key("executable"));
        for gone in ["source", "extra", "preferred_asset", "exe_is_archive"] {
            assert!(!obj.contains_key(gone), "{gone}");
        }
        let reparsed = port_from_value(&entry).unwrap();
        assert_eq!((reparsed.source_type, reparsed.key()), (SourceType::None, "SoH"));
    }

    #[cfg(unix)]
    #[test]
    fn adopting_links_the_library_folder_and_registers_the_entry_without_touching_the_install() {
        let dir = temp_dir("adopt_ok");
        let install = dir.join("Applications").join("SoH");
        touch(&install.join("soh.appimage"));
        touch(&install.join("saves").join("file1.sav"));
        let before = fs::read(install.join("saves").join("file1.sav")).unwrap();
        let library = dir.join("Library");
        let local = dir.join("ports.local.json");
        fs::write(&local, r#"{"ports":[{"name":"Mine","folder":"mine"}]}"#).unwrap();

        let done = adopt(&install, &catalog()[0], &library, &local, "T1").unwrap();

        assert_eq!(done.link, library.join("SoH"));
        assert_eq!(fs::read_link(&done.link).unwrap(), fs::canonicalize(&install).unwrap());
        assert!(crate::core::installer::is_installed(&catalog()[0], &library));
        assert_eq!(fs::read(library.join("SoH").join("saves").join("file1.sav")).unwrap(), before);
        let backup = done.backup_of_local_file.unwrap();
        assert_eq!(fs::read_to_string(backup).unwrap(), r#"{"ports":[{"name":"Mine","folder":"mine"}]}"#);
        let ports = crate::core::config::load_local_config(&local);
        assert_eq!(ports.iter().map(|p| p.folder.as_str()).collect::<Vec<_>>(), ["mine", "SoH"]);
        assert!(ports.iter().all(|p| p.user_managed));
        let mut install_files: Vec<_> = walk(&install);
        install_files.sort();
        assert_eq!(install_files, ["saves", "saves/file1.sav", "soh.appimage"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    fn walk(root: &Path) -> Vec<String> {
        fn inner(root: &Path, dir: &Path, out: &mut Vec<String>) {
            for e in fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                out.push(p.strip_prefix(root).unwrap().to_string_lossy().into_owned());
                if p.is_dir() {
                    inner(root, &p, out);
                }
            }
        }
        let mut out = Vec::new();
        inner(root, root, &mut out);
        out
    }

    #[cfg(unix)]
    #[test]
    fn adopting_creates_the_local_file_when_there_is_none() {
        let dir = temp_dir("adopt_new_file");
        let install = dir.join("soh");
        fs::create_dir_all(&install).unwrap();
        let local = dir.join("ports.local.json");
        let done = adopt(&install, &catalog()[0], &dir.join("Library"), &local, "T").unwrap();
        assert!(done.backup_of_local_file.is_none());
        assert_eq!(crate::core::config::load_local_config(&local).len(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_taken_library_folder_refuses_before_changing_anything() {
        let dir = temp_dir("taken");
        let install = dir.join("soh");
        fs::create_dir_all(&install).unwrap();
        let library = dir.join("Library");
        mkdirs(&library, &["SoH"]);
        let local = dir.join("ports.local.json");
        assert!(matches!(adopt(&install, &catalog()[0], &library, &local, "T"), Err(AdoptError::LibraryFolderTaken(_))));
        assert!(!local.exists());
        assert!(fs::symlink_metadata(library.join("SoH")).unwrap().is_dir());
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_link_in_the_library_also_counts_as_taken() {
        let dir = temp_dir("dangling");
        let install = dir.join("soh");
        fs::create_dir_all(&install).unwrap();
        let library = dir.join("Library");
        fs::create_dir_all(&library).unwrap();
        std::os::unix::fs::symlink(dir.join("gone"), library.join("SoH")).unwrap();
        assert!(matches!(adopt(&install, &catalog()[0], &library, &dir.join("l.json"), "T"), Err(AdoptError::LibraryFolderTaken(_))));
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_local_file_stops_adoption_and_leaves_no_link() {
        let dir = temp_dir("badlocal");
        let install = dir.join("soh");
        fs::create_dir_all(&install).unwrap();
        let local = dir.join("ports.local.json");
        fs::write(&local, "{ not json").unwrap();
        let library = dir.join("Library");
        assert!(matches!(adopt(&install, &catalog()[0], &library, &local, "T"), Err(AdoptError::LocalFileUnreadable(_))));
        assert_eq!(fs::read_to_string(&local).unwrap(), "{ not json");
        assert!(fs::symlink_metadata(library.join("SoH")).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn an_existing_entry_for_the_folder_stops_adoption() {
        let dir = temp_dir("registered");
        let install = dir.join("soh");
        fs::create_dir_all(&install).unwrap();
        let local = dir.join("ports.local.json");
        fs::write(&local, r#"{"ports":[{"name":"x","folder":"SoH"}]}"#).unwrap();
        let library = dir.join("Library");
        assert!(matches!(adopt(&install, &catalog()[0], &library, &local, "T"), Err(AdoptError::AlreadyRegistered(_))));
        assert!(fs::symlink_metadata(library.join("SoH")).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_write_removes_the_link_that_was_made() {
        let dir = temp_dir("writefail");
        let install = dir.join("soh");
        fs::create_dir_all(&install).unwrap();
        let library = dir.join("Library");
        // The temp file name write_atomic wants is a folder, so the write fails after the link exists.
        let local = dir.join("ports.local.json");
        fs::create_dir(dir.join("ports.local.json.tmp")).unwrap();
        assert!(matches!(adopt(&install, &catalog()[0], &library, &local, "T"), Err(AdoptError::Io { action: "write", .. })));
        assert!(fs::symlink_metadata(library.join("SoH")).is_err());
        assert!(install.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_library_or_something_inside_it_cannot_be_adopted() {
        let dir = temp_dir("inside");
        let library = dir.join("Library");
        mkdirs(&library, &["dusklight"]);
        let local = dir.join("l.json");
        assert!(matches!(adopt(&library.join("dusklight"), &catalog()[0], &library, &local, "T"), Err(AdoptError::BadInstallFolder(_))));
        assert!(matches!(adopt(&dir.join("missing"), &catalog()[0], &library, &local, "T"), Err(AdoptError::BadInstallFolder(_))));
        touch(&dir.join("afile"));
        assert!(matches!(adopt(&dir.join("afile"), &catalog()[0], &library, &local, "T"), Err(AdoptError::BadInstallFolder(_))));
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_catalog_folder_that_escapes_the_library_is_refused() {
        let dir = temp_dir("escape");
        let install = dir.join("soh");
        fs::create_dir_all(&install).unwrap();
        for bad in ["../evil", "a/b", ".", "/abs"] {
            let mut port = catalog()[0].clone();
            port.folder = bad.to_string();
            assert!(matches!(adopt(&install, &port, &dir.join("Library"), &dir.join("l.json"), "T"), Err(AdoptError::BadCatalogFolder(_))), "{bad}");
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn an_adopted_port_can_be_removed_from_the_library_without_touching_the_install() {
        // What uninstall would do to the link: remove_dir_all on a symlink removes the link only.
        let dir = temp_dir("unlink");
        let install = dir.join("soh");
        touch(&install.join("keep.sav"));
        let done = adopt(&install, &catalog()[0], &dir.join("Library"), &dir.join("l.json"), "T").unwrap();
        fs::remove_dir_all(&done.link).unwrap();
        assert!(fs::symlink_metadata(&done.link).is_err());
        assert!(install.join("keep.sav").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn two_adoptions_with_the_same_stamp_each_keep_their_own_backup_of_the_local_file() {
        let dir = temp_dir("same_stamp");
        let local = dir.join("ports.local.json");
        fs::write(&local, r#"{"ports":[]}"#).unwrap();
        let library = dir.join("Library");
        for (i, port) in [0usize, 2].iter().enumerate() {
            let install = dir.join(format!("install{i}"));
            fs::create_dir_all(&install).unwrap();
            adopt(&install, &catalog()[*port], &library, &local, "SAME").unwrap();
        }
        assert_eq!(crate::core::config::load_local_config(&local).len(), 2);
        assert!(dir.join("ports.local.json.bak-SAME").exists() && dir.join("ports.local.json.bak-SAME-1").exists());
        // The second backup holds the file as the first adoption left it.
        assert_eq!(crate::core::config::load_local_config(&dir.join("ports.local.json.bak-SAME-1")).len(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_folder_that_contains_the_library_cannot_be_adopted_and_is_not_offered() {
        let dir = temp_dir("contains_library");
        let library = dir.join("Applications").join("steamdeck-ports-mgr").join("Library");
        mkdirs(&library, &["SoH"]);
        touch(&library.join("SoH").join("soh.appimage"));
        let local = dir.join("l.json");
        let outer = dir.join("Applications").join("steamdeck-ports-mgr");
        assert!(matches!(adopt(&outer, &catalog()[0], &library, &local, "T"), Err(AdoptError::BadInstallFolder(_))));
        let result = scan(&[dir.join("Applications")], &catalog(), &library);
        assert!(result.candidates.is_empty() && result.ambiguous.is_empty() && result.already_in_library.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_second_folder_for_the_same_port_is_reported_not_silently_dropped() {
        let dir = temp_dir("duplicates");
        let apps = dir.join("Applications");
        mkdirs(&apps, &["SoH", "soh-old"]);
        touch(&apps.join("soh-old").join("shipwright.appimage"));
        let mut catalog = catalog();
        catalog[0].executable = Some(json!("shipwright.appimage"));
        let result = scan(&[apps], &catalog, &dir.join("Library"));
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].dir_name, "SoH");
        assert_eq!(result.duplicates.len(), 1);
        assert!(result.duplicates[0].1.ends_with("soh-old"));
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_link_in_the_library_still_offers_the_port_and_adopting_refuses_it() {
        let dir = temp_dir("dangling_scan");
        let apps = dir.join("Applications");
        mkdirs(&apps, &["SoH"]);
        let library = dir.join("Library");
        fs::create_dir_all(&library).unwrap();
        std::os::unix::fs::symlink(dir.join("gone"), library.join("SoH")).unwrap();
        let result = scan(&[apps], &catalog(), &library);
        assert_eq!(result.candidates.len(), 1);
        assert!(matches!(adopt(&result.candidates[0].path, &catalog()[0], &library, &dir.join("l.json"), "T"), Err(AdoptError::LibraryFolderTaken(_))));
        let _ = fs::remove_dir_all(dir);
    }

}

#[cfg(test)]
pub(crate) mod test_fixtures {
    //! Shared by the matching and import tests.
    use std::fs;
    use std::path::{Path, PathBuf};

    pub fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"x").unwrap();
    }

    /// A file with the executable bit (on Linux; elsewhere a plain file).
    pub fn touch_exe(path: &Path) {
        touch(path);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// Tim's SD card as reported from his Deck (2026-10-23), the card's id replaced by `sd`: four ports in
    /// folders named after their downloads, the ports folder `Ports`, EmuDeck's `Emulation`, a Steam library
    /// (`steamapps`, `libraryfolder.vdf`), `lost+found` and the trash. The ROMs carry the executable bit, as
    /// on his card. Returns the card's top folder.
    pub fn tims_sd_card(dir: &Path) -> PathBuf {
        let sd = dir.join("sd");
        fs::create_dir_all(sd.join("Ports")).unwrap();
        fs::create_dir_all(sd.join("lost+found")).unwrap();
        touch(&sd.join("libraryfolder.vdf"));
        touch(&sd.join("steamapps").join("common").join("Some Game").join("game.exe"));
        touch(&sd.join("steamapps").join("appmanifest_1.acf"));
        for name in ["starship.appimage", "dusklight.sh", "uzdoom.sh"] {
            touch(&sd.join(".Trash-1000").join("files").join(name));
        }
        let emu = sd.join("Emulation");
        for f in ["roms/n64/Zelda64Recomp.z64", "roms/psx/Severed-Chains.bin", "bios/scph1001.bin", "saves/x.sav", "tools/launchers/retroarch.sh", "tools/launchers/uzdoom.sh"] {
            touch(&emu.join(f));
        }
        for i in 0..60 {
            touch(&emu.join("roms").join(format!("game{i}.zip")));
        }

        let two = sd.join("2Ship-Keiichi-Charlie-Linux");
        touch_exe(&two.join("2ship.appimage"));
        for f in [
            "2S2HTimeSplitData.json",
            "2ship2harkinian.json",
            "7f3e2a10-cover.png",
            "images.steamusercontent.jpg",
            "imgui.ini",
            "legend_of_zelda__majora_s_mask_wallpaper_by_chasecubed_d9k1c5r-fullview.jpg",
            "logs/2ship.log",
            "mm.o2r",
            "mods/.keep",
            "presets/.keep",
            "randomizer/.keep",
            "readme.txt",
            "saves/file1.sav",
        ] {
            touch(&two.join(f));
        }
        touch_exe(&two.join("The Legend of Zelda - Majora's Mask.z64"));

        let soh = sd.join("SoH-Ackbar-Delta-Linux");
        touch_exe(&soh.join("soh.appimage"));
        for f in ["imgui.ini", "logs/soh.log", "mods/.keep", "oot.o2r", "readme.txt", "Save/file1.sav", "shipofharkinian.json"] {
            touch(&soh.join(f));
        }
        for rom in ["The Legend of Zelda - Majora's Mask.z64", "The Legend of Zelda Ocarina of Time Master Quest.v64", "The Legend of Zelda-Ocarina of Time.z64"] {
            touch_exe(&soh.join(rom));
        }

        let dusk = sd.join("Dusklight-twilight-princess");
        touch_exe(&dusk.join("Dusklight-v2.0.3-linux-x86_64.AppImage"));
        touch(&dusk.join("config.toml"));

        let sev = sd.join("Severed_Chains_Steam_Deck");
        touch_exe(&sev.join("launch"));
        touch(&sev.join("lod-game.jar"));
        touch(&sev.join("isos").join(".keep"));
        sd
    }
}
