//! The wizard's own checks for a ports folder: the quick choices (SD cards first, then the internal
//! default), and whether a folder can be used. Used while the reusable folder picker and ports-folder
//! setting are built elsewhere; the screen's adapter can switch to that code's validation later.

use super::flow::FolderPlan;
use crate::fork::rom_screens::util::{ellipsize, file_name, free_bytes, human_bytes};
use std::fs;
use std::path::{Path, PathBuf};

/// The folder made on an SD card or drive when the user picks it from the quick choices.
pub const SUBFOLDER: &str = "Ports";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickChoice {
    /// What it is: "Internal storage", an SD card.
    pub title: String,
    /// The folder, and what is known about it ("new folder", free space).
    pub detail: String,
    pub path: PathBuf,
}

fn notes(path: &Path) -> String {
    let note = if !path.is_dir() {
        Some("new folder".to_string())
    } else {
        free_bytes(path).map(|free| format!("{} free", human_bytes(free)))
    };
    note.map(|n| format!(" - {n}")).unwrap_or_default()
}

/// SD cards and drives first (`<mount>/Ports`), then the internal default, then the current folder
/// when it is neither.
pub fn quick_choices(current: &Path, internal_default: &Path, mounts: &[PathBuf]) -> Vec<QuickChoice> {
    let mut out: Vec<QuickChoice> = Vec::new();
    let mut add = |title: String, path: PathBuf| {
        if out.iter().any(|c| c.path == path) {
            return;
        }
        let detail = format!("{}{}", ellipsize(&path.to_string_lossy(), 60), notes(&path));
        out.push(QuickChoice { title, detail, path });
    };
    for mount in mounts {
        add(format!("SD card or drive \"{}\"", ellipsize(&file_name(mount), 20)), mount.join(SUBFOLDER));
    }
    add("Internal storage".to_string(), internal_default.to_path_buf());
    add("Current folder".to_string(), current.to_path_buf());
    out
}

/// Can `path` be the ports folder? It must be a full path to a folder (not the whole drive) that exists
/// and can be written, or that can be created inside a folder that can be written.
pub fn plan(path: &Path) -> FolderPlan {
    if !path.is_absolute() {
        return FolderPlan::Refused("That is not a full folder path.".into());
    }
    if path.parent().is_none() {
        return FolderPlan::Refused("Choose a folder, not the whole drive. A folder like Ports inside it works well.".into());
    }
    match fs::metadata(path) {
        Ok(meta) if !meta.is_dir() => FolderPlan::Refused(format!("{} is a file, not a folder.", path.display())),
        Ok(_) => match writable(path) {
            Ok(()) => FolderPlan::Ready,
            Err(e) => FolderPlan::Refused(format!("Ports cannot be saved in {}: {e}", path.display())),
        },
        Err(_) => {
            // The nearest folder that exists must be writable for the new one to be made.
            let Some(parent) = path.ancestors().skip(1).find(|p| p.exists()) else {
                return FolderPlan::Refused(format!("{} does not exist and cannot be created.", path.display()));
            };
            if !parent.is_dir() {
                return FolderPlan::Refused(format!("{} is a file, so a folder cannot be made inside it.", parent.display()));
            }
            match writable(parent) {
                Ok(()) => FolderPlan::NeedsCreate,
                Err(e) => FolderPlan::Refused(format!("The folder cannot be created in {}: {e}", parent.display())),
            }
        }
    }
}

pub fn create(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes and removes a small file of this app's own, the only reliable test of "can I save here".
fn writable(dir: &Path) -> Result<(), String> {
    let probe = dir.join(format!(".deck-ports-write-test-{}", std::process::id()));
    fs::write(&probe, b"ok").map_err(|e| e.to_string())?;
    fs::remove_file(&probe).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn sd_cards_come_first_then_internal_then_a_different_current_folder() {
        let t = temp();
        let sd = t.path().join("SD512");
        let sd2 = t.path().join("USB");
        fs::create_dir_all(&sd).unwrap();
        fs::create_dir_all(&sd2).unwrap();
        let internal = t.path().join("app").join("Library");
        let current = t.path().join("elsewhere");
        let choices = quick_choices(&current, &internal, &[sd.clone(), sd2.clone()]);
        let paths: Vec<&Path> = choices.iter().map(|c| c.path.as_path()).collect();
        assert_eq!(paths, [sd.join("Ports"), sd2.join("Ports"), internal.clone(), current.clone()]);
        assert_eq!(choices[0].title, "SD card or drive \"SD512\"");
        assert!(choices[0].detail.contains("new folder"), "{}", choices[0].detail);
        assert_eq!(choices[2].title, "Internal storage");
        assert_eq!(choices[3].title, "Current folder");
    }

    #[test]
    fn a_folder_that_exists_shows_free_space_and_one_that_does_not_says_new() {
        let t = temp();
        let internal = t.path().join("Library");
        fs::create_dir_all(&internal).unwrap();
        let choices = quick_choices(&internal, &internal, &[]);
        assert_eq!(choices.len(), 1, "the current folder is not listed twice");
        assert!(!choices[0].detail.contains("new folder"), "{}", choices[0].detail);
        let sd = t.path().join("SD");
        fs::create_dir_all(&sd).unwrap();
        let choices = quick_choices(&internal, &internal, &[sd.clone()]);
        assert_eq!(choices.len(), 2);
        assert!(choices[0].detail.contains("new folder"));
        fs::create_dir_all(sd.join("Ports")).unwrap();
        let choices = quick_choices(&internal, &internal, &[sd]);
        assert!(!choices[0].detail.contains("new folder"));
    }

    #[test]
    fn with_no_cards_only_internal_storage_is_offered() {
        let t = temp();
        let internal = t.path().join("Library");
        let choices = quick_choices(&internal, &internal, &[]);
        assert_eq!(choices.iter().map(|c| &c.path).collect::<Vec<_>>(), [&internal]);
    }

    #[test]
    fn an_existing_writable_folder_is_ready_and_leaves_nothing_behind() {
        let t = temp();
        assert_eq!(plan(t.path()), FolderPlan::Ready);
        assert_eq!(fs::read_dir(t.path()).unwrap().count(), 0, "the write test file must be removed");
    }

    #[test]
    fn a_missing_folder_inside_a_writable_one_needs_creating_and_nothing_is_made_yet() {
        let t = temp();
        let target = t.path().join("a").join("b").join("Ports");
        assert_eq!(plan(&target), FolderPlan::NeedsCreate);
        assert!(!t.path().join("a").exists());
        create(&target).unwrap();
        assert!(target.is_dir());
        assert_eq!(plan(&target), FolderPlan::Ready);
    }

    #[test]
    fn a_file_a_relative_path_and_the_drive_root_are_refused() {
        let t = temp();
        let file = t.path().join("f.txt");
        fs::write(&file, "x").unwrap();
        for bad in [file.clone(), PathBuf::from("relative/folder"), PathBuf::from("/"), file.join("inside")] {
            assert!(matches!(plan(&bad), FolderPlan::Refused(_)), "{bad:?}");
        }
        let FolderPlan::Refused(text) = plan(&file) else { panic!() };
        assert!(text.contains("is a file"));
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_folder_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let t = temp();
        let dir = t.path().join("ro");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
        // Running as root ignores the mode, so only assert when the folder really is unwritable.
        if fs::write(dir.join("probe"), b"x").is_err() {
            assert!(matches!(plan(&dir), FolderPlan::Refused(_)));
            assert!(matches!(plan(&dir.join("new")), FolderPlan::Refused(_)));
        }
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    }
}
