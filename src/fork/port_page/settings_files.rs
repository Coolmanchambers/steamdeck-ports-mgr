//! A port's own settings files (the Port page's "Port settings" card): found read-only and cheaply, never edited.
//!
//! **Rules.** A local entry (`ports.local.json`) may name them exactly with a `settings` field, the same shape as `save`
//! (a path, or one per system, relative to the port's folder or with `~` / `$HOME`; a list of those is fine too): then only
//! those are shown (a folder named there shows the files directly in it). Otherwise they are looked for in the port's
//! folder and in each save folder the app knows, two levels deep (the folder and the folders right inside it), skipping
//! hidden folders (`.previous` and the like). A file counts when it is under [`MAX_BYTES`] and has a settings extension
//! (`json`, `ini`, `toml`, `cfg`, `conf`, `yaml`, `yml`, `xml`; `txt` only when its name says settings), and is not a log,
//! not a game file the catalog names, and not one of the app's own files. Nothing is opened beyond a directory listing and
//! a file's size and date.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Files bigger than this are not settings (and are never read whole).
pub const MAX_BYTES: u64 = 1024 * 1024;
/// At most this many files are listed (a port folder full of JSON data stays a short page).
pub const MAX_FILES: usize = 24;
/// The View page shows at most this much of a file.
pub const VIEW_BYTES: usize = 48 * 1024;
/// And at most this many lines.
pub const VIEW_LINES: usize = 400;

const EXTENSIONS: [&str; 8] = ["json", "ini", "toml", "cfg", "conf", "yaml", "yml", "xml"];
/// A `.txt` counts only when its name has one of these.
const TXT_WORDS: [&str; 9] = ["config", "setting", "option", "pref", "cfg", "control", "keybind", "input", "video"];
/// The app's own files, wherever they turn up.
const OWN_FILES: [&str; 7] = ["ports.json", "ports.local.json", "state.json", "themes.json", "fork-settings.json", "port-moves.json", "steam-queue.json"];

/// One settings file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsFile {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// What the search reads from.
pub struct Input<'a> {
    pub game_dir: &'a Path,
    /// The save folders the app knows for the port (catalog, save finder).
    pub save_dirs: &'a [PathBuf],
    /// The local entry's `settings` field, when it has one.
    pub named: Option<&'a Value>,
    /// File names the catalog names as the port's game files (never settings).
    pub game_data: &'a [String],
}

fn lower(s: &str) -> String {
    s.to_ascii_lowercase()
}

/// A log or crash report, by its name.
fn is_log(stem: &str) -> bool {
    let s = lower(stem);
    let log = s.contains("log") && !s.contains("catalog") && !s.contains("dialog") && !s.contains("blog");
    log || s.contains("crash")
}

/// Whether a file name looks like a settings file (extension, and for `.txt` the name).
pub fn is_settings_name(name: &str) -> bool {
    let path = Path::new(name);
    let Some(ext) = path.extension().and_then(|e| e.to_str()).map(lower) else { return false };
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if stem.is_empty() || name.starts_with('.') || is_log(stem) || OWN_FILES.iter().any(|o| o.eq_ignore_ascii_case(name)) {
        return false;
    }
    if ext == "txt" {
        let s = lower(stem);
        return TXT_WORDS.iter().any(|w| s.contains(w));
    }
    EXTENSIONS.contains(&ext.as_str())
}

fn file_of(path: &Path) -> Option<SettingsFile> {
    let meta = std::fs::metadata(path).ok().filter(std::fs::Metadata::is_file)?;
    Some(SettingsFile { path: path.to_path_buf(), size: meta.len(), modified: meta.modified().ok() })
}

/// The files in `dir` and in the folders right inside it that pass `keep`, in name order, folders' files after the top's.
fn two_levels(dir: &Path, keep: &dyn Fn(&Path, &str) -> bool, out: &mut Vec<SettingsFile>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<(PathBuf, String, bool)> = read
        .flatten()
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            (e.path(), name, is_dir)
        })
        .collect();
    entries.sort_by_key(|(_, n, _)| lower(n));
    for (path, name, is_dir) in &entries {
        if !is_dir && keep(path, name) {
            out.extend(file_of(path));
        }
    }
    for (path, name, is_dir) in &entries {
        if *is_dir && !name.starts_with('.') {
            let Ok(read) = std::fs::read_dir(path) else { continue };
            let mut inner: Vec<(PathBuf, String)> = read.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_file())).map(|e| (e.path(), e.file_name().to_string_lossy().into_owned())).collect();
            inner.sort_by_key(|(_, n)| lower(n));
            for (p, n) in inner {
                if keep(&p, &n) {
                    out.extend(file_of(&p));
                }
            }
        }
    }
}

/// The paths a `settings` field names: one string, one per system (`{"linux": ..., "windows": ...}`), or a list of those.
fn named_paths(value: &Value, game_dir: &Path) -> Vec<PathBuf> {
    match value {
        Value::Array(items) => items.iter().flat_map(|v| named_paths(v, game_dir)).collect(),
        other => crate::core::platform_resolve::resolve_save_folder(other, game_dir).into_iter().collect(),
    }
}

/// The port's settings files, by the rules above.
pub fn discover(input: &Input) -> Vec<SettingsFile> {
    let mut out: Vec<SettingsFile> = Vec::new();
    if let Some(named) = input.named {
        for path in named_paths(named, input.game_dir) {
            if path.is_dir() {
                let Ok(read) = std::fs::read_dir(&path) else { continue };
                let mut files: Vec<PathBuf> = read.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_file())).map(|e| e.path()).collect();
                files.sort();
                out.extend(files.iter().filter_map(|p| file_of(p)).filter(|f| f.size <= MAX_BYTES));
            } else {
                out.extend(file_of(&path));
            }
        }
    } else {
        let game_data: Vec<String> = input.game_data.iter().map(|n| lower(n.trim())).collect();
        let keep = |path: &Path, name: &str| {
            is_settings_name(name) && !game_data.contains(&lower(name)) && std::fs::metadata(path).is_ok_and(|m| m.len() <= MAX_BYTES)
        };
        two_levels(input.game_dir, &keep, &mut out);
        for dir in input.save_dirs {
            two_levels(dir, &keep, &mut out);
        }
    }
    let mut seen: Vec<PathBuf> = Vec::new();
    out.retain(|f| {
        let fresh = !seen.contains(&f.path);
        seen.push(f.path.clone());
        fresh
    });
    out.truncate(MAX_FILES);
    out
}

/// What the View page shows of a file: its text, at most [`VIEW_BYTES`] and [`VIEW_LINES`], and whether it was cut.
/// `Err` with a plain reason for a file that is not text or cannot be read.
pub fn view_text(path: &Path) -> Result<(String, bool), String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("It could not be opened ({e})."))?;
    let mut bytes = Vec::new();
    file.take(VIEW_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|e| format!("It could not be read ({e})."))?;
    let mut cut = bytes.len() > VIEW_BYTES;
    bytes.truncate(VIEW_BYTES);
    if bytes.contains(&0) {
        return Err("It is not plain text, so it is not shown here.".to_string());
    }
    let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n").replace('\t', "    ");
    let mut lines: Vec<&str> = text.lines().collect();
    if lines.len() > VIEW_LINES {
        lines.truncate(VIEW_LINES);
        cut = true;
    }
    Ok((lines.join("\n"), cut))
}
