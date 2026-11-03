//! Paths the human way: "SD card > Emulation > roms", "Internal storage > Emulation > roms", never a card's ID
//! (`/run/media/deck/c14f9023-.../Emulation/roms`). One formatter for every screen that names a place: the Port page and
//! its card pages, the game files pages, the game files search's lines.

use crate::fork::folder_picker::mounts::{self, Drive};
use std::path::{Path, PathBuf};

/// What a path is told against: the mounted removable drives, the home folder (internal storage on a Deck) and the
/// folders drives are mounted under (so a card that is not mounted right now is still not shown by its ID).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Places {
    pub drives: Vec<Drive>,
    pub home: Option<PathBuf>,
    pub media_roots: Vec<PathBuf>,
}

impl Places {
    /// This system's: the kernel's mount list, `$HOME`, `/run/media/<user>` and friends. Reads `/proc/mounts` once.
    pub fn system() -> Places {
        let home = ["HOME", "USERPROFILE"].iter().find_map(|v| std::env::var_os(v)).map(PathBuf::from).filter(|p| p.is_absolute());
        let media_roots = mounts::default_media_roots();
        Places { drives: mounts::removable_drives(&mounts::system_mounts(), &media_roots), home, media_roots }
    }

    /// The top places a person knows: internal storage (the home folder) and each drive. A folder browser stops going
    /// up at these; above them is only `/home`, `/run/media` and the like.
    pub fn tops(&self) -> Vec<PathBuf> {
        self.home.iter().cloned().chain(self.drives.iter().map(|d| d.mount_point.clone())).collect()
    }
}

/// The crumbs below a base: "SD card", "Emulation", "roms" joined with " > ".
fn crumbs(base: &str, rest: &Path) -> String {
    let mut parts = vec![base.to_string()];
    parts.extend(rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).filter(|p| !p.is_empty() && p != "/"));
    parts.join(" > ")
}

/// A path as a place a person knows:
/// - on a mounted drive: the drive's title and the folders below it ("SD card > Emulation > roms", "USB drive (STICK) > ROMs";
///   a card whose folder name is only its ID is just "SD card");
/// - under a media root but not mounted now: "SD card > ..." for a folder named by an ID, else the folder's name;
/// - in the home folder: "Internal storage > ..." ("Internal storage > Home" for the home folder itself);
/// - anything else: the path as it is.
pub fn human_path(path: &Path, places: &Places) -> String {
    if let Some(d) = places.drives.iter().filter(|d| path.starts_with(&d.mount_point)).max_by_key(|d| d.mount_point.as_os_str().len()) {
        return crumbs(&d.title(), path.strip_prefix(&d.mount_point).unwrap_or(Path::new("")));
    }
    // The most specific media root first (`/run/media/deck` before `/run/media`); a user folder is a root, not a drive.
    let mut roots: Vec<&PathBuf> = places.media_roots.iter().collect();
    roots.sort_by_key(|r| std::cmp::Reverse(r.components().count()));
    for root in roots {
        let Ok(rest) = path.strip_prefix(root) else { continue };
        let mut parts = rest.components();
        let Some(first) = parts.next() else { continue };
        let name = first.as_os_str().to_string_lossy().into_owned();
        if places.media_roots.contains(&root.join(&name)) {
            continue;
        }
        // A card's ID or a device name (`mmcblk0p1`: older SteamOS) is the kind of drive, never the name itself.
        let base = match mounts::device_name_kind(&name) {
            Some(kind) => kind.to_string(),
            None if mounts::looks_like_an_id(&name) => "SD card".to_string(),
            None => name,
        };
        return crumbs(&base, parts.as_path());
    }
    if let Some(rest) = places.home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        if rest.as_os_str().is_empty() {
            return "Internal storage > Home".to_string();
        }
        return crumbs("Internal storage", rest);
    }
    path.display().to_string()
}

/// `human_path` with this system's places (reads the mount list; for the few screens that name one path).
pub fn human_path_here(path: &Path) -> String {
    human_path(path, &Places::system())
}

/// A sentence from the engines (an error, a refusal) with every path in it told the human way: a path that starts at
/// the home folder or a drive's folder ("/home/deck/Games/Ports", "/run/media/deck/SD/Ports") becomes "Internal storage >
/// Games > Ports" (where a path ends: [`path_end`]). The engines' ` -- ` between a reason and its path becomes `: `. Text
/// without such a path comes back as it was.
pub fn plain_paths(text: &str, places: &Places) -> String {
    let mut roots: Vec<PathBuf> = places.home.iter().cloned().chain(places.media_roots.iter().cloned()).chain(places.drives.iter().map(|d| d.mount_point.clone())).collect();
    roots.retain(|r| r.is_absolute() && r.as_os_str().len() > 1);
    let text = text.replace(" -- ", ": ");
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    'scan: while let Some(at) = rest.find('/') {
        let (before, from) = rest.split_at(at);
        let boundary = before.chars().last().is_none_or(|c| c.is_whitespace() || "(\"'".contains(c));
        let root_hit = boundary && roots.iter().any(|r| {
            let r = r.to_string_lossy();
            from.starts_with(r.as_ref()) && from[r.len()..].chars().next().is_none_or(|c| c == '/' || !c.is_alphanumeric())
        });
        if !root_hit {
            out.push_str(before);
            out.push('/');
            rest = &from[1..];
            continue 'scan;
        }
        let path = &from[..path_end(from)];
        out.push_str(before);
        out.push_str(&human_path(Path::new(path), places));
        rest = &from[path.len()..];
    }
    out.push_str(rest);
    out
}

/// `plain_paths` with this system's places.
pub fn plain_paths_here(text: &str) -> String {
    plain_paths(text, &Places::system())
}

/// Where the path at the start of `from` ends: at `(`, `)`, `,`, `;`, `"`, a line break, `: `, a `.` before a space or
/// the end, or a space that is not followed by a capital letter or a digit (the app's own folders have names like
/// "Saves Backup" and "Update Backups"; a sentence goes on in small letters: "/home/deck/Ports is not there").
fn path_end(from: &str) -> usize {
    let chars: Vec<(usize, char)> = from.char_indices().collect();
    for (k, (i, c)) in chars.iter().enumerate() {
        let next = chars.get(k + 1).map(|(_, n)| *n);
        let stop = match c {
            '(' | ')' | ',' | ';' | '"' | '\n' => true,
            ':' | '.' => next.is_none_or(char::is_whitespace),
            ' ' => !next.is_some_and(|n| n.is_uppercase() || n.is_ascii_digit()),
            _ => false,
        };
        if stop {
            return *i;
        }
    }
    from.len()
}

/// Bytes as "1.2 GB", "340 MB", "12 KB" (decimal units, as file managers on the Deck show them).
pub fn size_text(bytes: u64) -> String {
    const UNITS: [(&str, f64); 3] = [("GB", 1e9), ("MB", 1e6), ("KB", 1e3)];
    for (unit, size) in UNITS {
        let v = bytes as f64 / size;
        if v >= 1.0 {
            return if v >= 10.0 { format!("{v:.0} {unit}") } else { format!("{v:.1} {unit}") };
        }
    }
    if bytes == 1 { "1 byte".to_string() } else { format!("{bytes} bytes") }
}
