//! The quick choices of the wizard's ports folder step: removable drives first (`<mount>/Ports`), then
//! the internal default, then the current folder if it is neither, and a folder an earlier build of the
//! wizard recorded (offered once). Whether a folder can be used, creating it, and moving ports into it
//! belong to `fork::ports_folder` and `app::ports_folder_screen`; the wizard only offers the places.

use crate::fork::folder_picker::mounts::{fs_label, is_non_linux_fs, Drive};
use crate::fork::rom_screens::util::{ellipsize, free_bytes, human_bytes};
use std::path::{Path, PathBuf};

/// The folder suggested on a drive.
pub const SUBFOLDER: &str = "Ports";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickChoice {
    /// What it is: "Internal storage", an SD card.
    pub title: String,
    /// The folder, and what is known about it ("new folder", free space, a file system that cannot hold ports).
    pub detail: String,
    pub path: PathBuf,
}

fn notes(path: &Path, fs_type: Option<&str>) -> String {
    let note = match fs_type {
        Some(fs) if is_non_linux_fs(fs) => Some(format!("{} cannot hold ports", fs_label(fs))),
        _ if !path.is_dir() => Some("new folder".to_string()),
        _ => free_bytes(path).map(|free| format!("{} free", human_bytes(free))),
    };
    note.map(|n| format!(" - {n}")).unwrap_or_default()
}

/// Drives first (SD cards before USB drives, as `removable_drives` sorts them), then the internal default,
/// then the current folder when it is neither, then `earlier` (a folder chosen in a build before the real
/// ports folder setting, offered once).
pub fn quick_choices(current: &Path, internal_default: &Path, drives: &[Drive], earlier: Option<&Path>) -> Vec<QuickChoice> {
    let mut out: Vec<QuickChoice> = Vec::new();
    let mut add = |title: String, path: PathBuf, fs_type: Option<&str>| {
        if out.iter().any(|c| c.path == path) {
            return;
        }
        let detail = format!("{}{}", ellipsize(&path.to_string_lossy(), 60), notes(&path, fs_type));
        out.push(QuickChoice { title, detail, path });
    };
    for drive in drives {
        add(drive.title(), drive.mount_point.join(SUBFOLDER), Some(&drive.fs_type));
    }
    add("Internal storage".to_string(), internal_default.to_path_buf(), None);
    add("Current folder".to_string(), current.to_path_buf(), None);
    if let Some(path) = earlier {
        add("Folder you chose earlier".to_string(), path.to_path_buf(), None);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn drive(mount: &Path, kind: &'static str, label: &str, fs_type: &str) -> Drive {
        Drive { mount_point: mount.to_path_buf(), kind, label: label.into(), fs_type: fs_type.into() }
    }

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn drives_come_first_then_internal_then_a_different_current_folder_then_the_earlier_choice() {
        let t = temp();
        let sd = t.path().join("SD512");
        let usb = t.path().join("USB");
        fs::create_dir_all(&sd).unwrap();
        fs::create_dir_all(&usb).unwrap();
        let internal = t.path().join("app").join("Library");
        let current = t.path().join("elsewhere");
        let earlier = t.path().join("old-choice");
        let drives = [drive(&sd, "SD card", "SD512", "ext4"), drive(&usb, "USB drive", "USB", "ext4")];
        let choices = quick_choices(&current, &internal, &drives, Some(&earlier));
        let paths: Vec<&Path> = choices.iter().map(|c| c.path.as_path()).collect();
        assert_eq!(paths, [sd.join("Ports"), usb.join("Ports"), internal.clone(), current.clone(), earlier.clone()]);
        assert_eq!(choices[0].title, "SD card (SD512)");
        assert!(choices[0].detail.contains("new folder"), "{}", choices[0].detail);
        assert_eq!(choices[2].title, "Internal storage");
        assert_eq!(choices[3].title, "Current folder");
        assert_eq!(choices[4].title, "Folder you chose earlier");
    }

    #[test]
    fn a_drive_that_cannot_hold_ports_says_so() {
        let t = temp();
        let sd = t.path().join("SD");
        fs::create_dir_all(&sd).unwrap();
        let internal = t.path().join("Library");
        let choices = quick_choices(&internal, &internal, &[drive(&sd, "SD card", "SD", "exfat")], None);
        assert!(choices[0].detail.contains("exFAT cannot hold ports"), "{}", choices[0].detail);
        assert!(!choices[0].detail.contains("new folder"));
    }

    #[test]
    fn a_folder_that_exists_shows_free_space_and_one_that_does_not_says_new() {
        let t = temp();
        let internal = t.path().join("Library");
        fs::create_dir_all(&internal).unwrap();
        let choices = quick_choices(&internal, &internal, &[], None);
        assert_eq!(choices.len(), 1, "the current folder is not listed twice");
        assert!(!choices[0].detail.contains("new folder"), "{}", choices[0].detail);
        let sd = t.path().join("SD");
        fs::create_dir_all(&sd).unwrap();
        let drives = [drive(&sd, "SD card", "SD", "ext4")];
        let choices = quick_choices(&internal, &internal, &drives, None);
        assert_eq!(choices.len(), 2);
        assert!(choices[0].detail.contains("new folder"));
        fs::create_dir_all(sd.join("Ports")).unwrap();
        let choices = quick_choices(&internal, &internal, &drives, None);
        assert!(!choices[0].detail.contains("new folder"));
    }

    #[test]
    fn with_no_drives_only_internal_storage_is_offered_and_an_earlier_choice_equal_to_it_is_not_repeated() {
        let t = temp();
        let internal = t.path().join("Library");
        let choices = quick_choices(&internal, &internal, &[], Some(&internal));
        assert_eq!(choices.iter().map(|c| &c.path).collect::<Vec<_>>(), [&internal]);
    }
}
