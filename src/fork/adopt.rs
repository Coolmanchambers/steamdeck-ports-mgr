//! Adopting ports that are already installed somewhere outside `Library/`.
//!
//! Finding: look one level into each scan folder (well-known ones plus the user's own) for a folder
//! that matches a catalog entry, by folder name first and by the program's file name second.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    /// The install folder is named like the catalog entry's folder or name.
    FolderName,
    /// A program inside it is named like the catalog entry.
    ExecutableName,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ambiguous {
    pub path: PathBuf,
    pub dir_name: String,
    pub matches: Vec<String>,
}

#[derive(Debug, Default)]
pub struct ScanResult {
    pub candidates: Vec<Candidate>,
    /// Folders that matched more than one catalog entry equally well; never adopted automatically.
    pub ambiguous: Vec<Ambiguous>,
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

/// Names a catalog entry answers to: its folder, its name and its name without a trailing "(note)".
pub(crate) fn port_names(port: &Port) -> Vec<String> {
    let mut names = vec![norm(&port.folder), norm(&port.name), norm(port.display_name())];
    names.retain(|n| !n.is_empty());
    names.dedup();
    names
}

/// The program file name a catalog entry names for this platform, without extension, normalised.
pub(crate) fn declared_exe_stem(port: &Port) -> Option<String> {
    let exe = port.executable.as_ref().filter(|v| is_truthy(v))?;
    let value = resolve_per_platform(exe)?;
    let name = Path::new(value.as_str()?).file_stem()?.to_str()?.to_string();
    Some(norm(&name))
}

/// Normalised file stems of regular files in a folder, two levels deep, capped.
pub(crate) fn program_stems(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, depth: usize, budget: &mut usize, out: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() {
                if depth < MAX_EXE_SCAN_DEPTH {
                    walk(&path, depth + 1, budget, out);
                }
            } else if kind.is_file() {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    out.push(norm(stem));
                }
            }
        }
    }
    let mut out = Vec::new();
    let mut budget = MAX_EXE_SCAN_ENTRIES;
    walk(dir, 0, &mut budget, &mut out);
    out
}

fn match_dir(dir: &Path, dir_name: &str, catalog: &[Port]) -> (MatchKind, Vec<usize>) {
    let n = norm(dir_name);
    let by_folder: Vec<usize> = catalog
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.is_android_only() && !n.is_empty() && port_names(p).contains(&n))
        .map(|(i, _)| i)
        .collect();
    if !by_folder.is_empty() {
        return (MatchKind::FolderName, by_folder);
    }
    let stems = program_stems(dir);
    let by_exe = catalog
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            if p.is_android_only() {
                return false;
            }
            let wanted = declared_exe_stem(p).into_iter().chain(port_names(p));
            wanted.filter(|w| w.len() >= MIN_EXE_NAME_LEN).any(|w| stems.contains(&w))
        })
        .map(|(i, _)| i)
        .collect();
    (MatchKind::ExecutableName, by_exe)
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
    let mut result = ScanResult::default();
    let library = fs::canonicalize(library_dir).unwrap_or_else(|_| library_dir.to_path_buf());
    let mut deep_budget = MAX_DEEP_FOLDERS;
    for root in roots {
        let Some(top) = child_dirs(root) else { continue };
        result.roots_scanned.push(root.clone());
        // (folder, may we look inside it when it matches nothing)
        let mut queue: std::collections::VecDeque<(PathBuf, bool)> = top.into_iter().map(|d| (d, deeper)).collect();
        while let Some((dir, may_descend)) = queue.pop_front() {
            if cancelled() {
                return result;
            }
            let Some(dir_name) = dir.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
            // Never the library itself, anything in it, or a folder that contains it (the app's own
            // folder, usually): its `Library/<port>` folders would all look like installs to adopt.
            if fs::canonicalize(&dir).is_ok_and(|real| real.starts_with(&library) || library.starts_with(&real)) {
                continue;
            }
            let (how, hits) = match_dir(&dir, &dir_name, catalog);
            match hits.as_slice() {
                [] => {
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
                            queue.push_back((child, false));
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
                        });
                    }
                }
                many => result.ambiguous.push(Ambiguous {
                    path: dir,
                    dir_name,
                    matches: many.iter().map(|&i| catalog[i].display_name().to_string()).collect(),
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
        touch(&games.join("misc").join("bin").join("othergame-launcher"));
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
