//! A folder picker the controller can drive, shown inside the app (native file dialogs do not work
//! well in Game Mode). The logic lives here and is tested without a window; the screen is
//! `src/app/folder_picker_screen.rs` with `ui/fork/folder-picker.slint`.
//!
//! What it offers:
//! - Quick places at the top: each mounted removable drive (SD card, USB) by label with its free
//!   space, Home, and common folders (`~/Games`, `~/Applications`) that exist.
//! - Browsing: the folders inside the current one (hidden ones only on request), Up, "New folder
//!   here..." (name checked by [`new_folder_problem`]) and "Use this folder".
//! - A header with the current path, its free space, file system type and drive.
//!
//! API for other screens (stage 2 setup and import use this):
//! `app::folder_picker_screen::open_folder_picker(app, router, PickerRequest { title, start, purpose },
//! on_chosen)`; `on_chosen(app, router, PathBuf)` runs once with the chosen folder; Cancel or B
//! calls the optional `on_cancel`. The `purpose` only changes the hint line and whether the folder
//! must be writable to be chosen; checks specific to a use (the ports folder rules) belong to the caller.

pub mod browser;
pub mod mounts;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub use browser::{Picker, Row};

/// What the folder is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Where installed ports go: must be writable; the hint mentions ext4.
    PortsFolder,
    /// A folder to look in for ports or game files: only read.
    ScanFolder,
    /// Anything else: only has to exist.
    Any,
}

impl Purpose {
    pub fn hint(self) -> &'static str {
        match self {
            Purpose::PortsFolder => "Ports need the internal drive or a card formatted by SteamOS. exFAT cards can't store Linux programs.",
            Purpose::ScanFolder => "This folder is only read. Nothing in it is changed.",
            Purpose::Any => "",
        }
    }

    pub fn needs_write(self) -> bool {
        matches!(self, Purpose::PortsFolder)
    }
}

#[derive(Debug, Clone)]
pub struct PickerRequest {
    pub title: String,
    /// Where browsing starts; when it does not exist, its nearest existing parent.
    pub start: PathBuf,
    pub purpose: Purpose,
    /// Extra quick places (label, folder), listed after the drives and home folders when they exist.
    pub extra_places: Vec<(String, PathBuf)>,
    /// The cursor starts on the first sub-folder (or Cancel), never on "Use this folder": one press of A must not
    /// choose the folder the picker happens to open in (the save finder).
    pub cursor_off_use_this: bool,
    /// List `extra_places` before the drives and home folders (the game files folder: its likely places first).
    pub extra_first: bool,
}

impl PickerRequest {
    pub fn new(title: &str, start: &Path, purpose: Purpose) -> PickerRequest {
        PickerRequest { title: title.to_string(), start: start.to_path_buf(), purpose, extra_places: Vec::new(), cursor_off_use_this: false, extra_first: false }
    }
}

/// Everything the picker reads from the machine, so tests can pass a fake one.
pub struct Env {
    pub home: Option<PathBuf>,
    pub mounts: Vec<mounts::MountEntry>,
    pub media_roots: Vec<PathBuf>,
    pub free_space: fn(&Path) -> Option<u64>,
}

impl Env {
    pub fn system() -> Env {
        Env {
            home: std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute()),
            mounts: mounts::system_mounts(),
            media_roots: mounts::default_media_roots(),
            free_space: crate::fork::rom_screens::util::free_bytes,
        }
    }
}

/// Folder names longer than this (in bytes) are refused; Linux file systems allow 255.
pub const MAX_NAME_BYTES: usize = 255;

/// Why a new folder name cannot be used, or `None` when it can. `parent` is where it would go.
pub fn new_folder_problem(parent: &Path, name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("Type a name for the new folder.".into());
    }
    if name.trim() != name {
        return Some("Remove the spaces at the start or end of the name.".into());
    }
    if name == "." || name == ".." {
        return Some("That name is not allowed.".into());
    }
    if name.contains('/') || name.contains('\\') {
        return Some("A folder name cannot contain / or \\.".into());
    }
    if name.chars().any(char::is_control) {
        return Some("The name contains a character that cannot be used.".into());
    }
    if name.starts_with('.') {
        return Some("Names starting with a dot make hidden folders. Pick a name without the dot.".into());
    }
    if name.len() > MAX_NAME_BYTES {
        return Some(format!("The name is too long (at most {MAX_NAME_BYTES} characters)."));
    }
    if fs::symlink_metadata(parent.join(name)).is_ok() {
        return Some(format!("There is already something called \"{name}\" here."));
    }
    None
}

/// Creates `parent/name` (one level, never its parents) after checking the name.
pub fn create_folder(parent: &Path, name: &str) -> Result<PathBuf, String> {
    if let Some(problem) = new_folder_problem(parent, name) {
        return Err(problem);
    }
    let path = parent.join(name);
    fs::create_dir(&path).map_err(|e| match e.kind() {
        io::ErrorKind::PermissionDenied => format!("You can't create folders in {} (no permission).", parent.display()),
        _ => format!("Could not create {}: {e}", path.display()),
    })?;
    Ok(path)
}

/// The deepest existing folder on the way to `path` (the path itself when it is a folder).
pub fn nearest_existing_folder(path: &Path) -> Option<PathBuf> {
    let mut probe = Some(path);
    while let Some(p) = probe {
        if p.is_dir() {
            return Some(p.to_path_buf());
        }
        probe = p.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("dpm_picker_naming_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn new_folder_names_follow_the_rules() {
        let dir = temp("rules");
        fs::create_dir(dir.join("Taken")).unwrap();
        assert!(new_folder_problem(&dir, "Ports").is_none());
        assert!(new_folder_problem(&dir, "My Ports 2").is_none());
        for bad in ["", " Ports", "Ports ", ".", "..", "a/b", "a\\b", ".hidden", "tab\there", "Taken"] {
            assert!(new_folder_problem(&dir, bad).is_some(), "{bad:?} should be refused");
        }
        assert!(new_folder_problem(&dir, &"x".repeat(256)).is_some());
        assert!(new_folder_problem(&dir, &"x".repeat(255)).is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn create_folder_makes_one_level_and_refuses_bad_names() {
        let dir = temp("create");
        let made = create_folder(&dir, "Ports").unwrap();
        assert!(made.is_dir());
        assert!(create_folder(&dir, "Ports").is_err());
        assert!(create_folder(&dir.join("missing"), "Ports").is_err(), "parents are never created");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn nearest_existing_folder_walks_up() {
        let dir = temp("nearest");
        assert_eq!(nearest_existing_folder(&dir.join("a/b/c")), Some(dir.clone()));
        assert_eq!(nearest_existing_folder(&dir), Some(dir.clone()));
        let _ = fs::remove_dir_all(dir);
    }
}
