//! The quick choices of the wizard's ports folder step: removable drives first (`<mount>/Ports`), then
//! the internal default, then the current folder if it is neither, and a folder an earlier build of the
//! wizard recorded (offered once). Whether a folder can be used, creating it, and moving ports into it
//! belong to `fork::ports_folder` and `app::ports_folder_screen`; the wizard only offers the places.

use crate::fork::folder_picker::mounts::{fs_label, is_non_linux_fs, Drive};
use crate::fork::rom_screens::util::{ellipsize, free_bytes, human_bytes};
use std::path::{Path, PathBuf};

/// The folder suggested on a drive.
pub const SUBFOLDER: &str = "Ports";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An SD card: what the step suggests first.
    SdCard,
    /// A USB drive or any other removable drive.
    OtherDrive,
    Internal,
    Current,
    Earlier,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickChoice {
    /// What it is: "Internal storage", an SD card.
    pub title: String,
    /// The folder, and what is known about it ("new folder", free space, a file system that cannot hold ports).
    pub detail: String,
    /// What is known about it in plain words, without the folder: "58 GB free", "a new folder will be made".
    pub note: String,
    pub path: PathBuf,
    pub kind: Kind,
    /// False for a drive whose file system cannot hold ports (exFAT, FAT, NTFS): shown, not selectable.
    pub usable: bool,
}

fn plain_note(path: &Path, fs_type: Option<&str>) -> Option<String> {
    match fs_type {
        Some(fs) if is_non_linux_fs(fs) => Some(format!("{} cannot hold ports", fs_label(fs))),
        _ if !path.is_dir() => Some("a new folder will be made".to_string()),
        _ => free_bytes(path).map(|free| format!("{} free", human_bytes(free))),
    }
}

/// The note of a drive's Ports folder: when the folder is there already, the wizard says it is used as it is.
fn drive_note(path: &Path, fs_type: &str) -> Option<String> {
    let note = plain_note(path, Some(fs_type));
    if path.is_dir() && !is_non_linux_fs(fs_type) {
        // Free space is not known on every system; the sentence stands without it.
        return Some(match note {
            Some(free) => format!("uses the Ports folder already there. {free}"),
            None => "uses the Ports folder already there".to_string(),
        });
    }
    note
}

fn notes(path: &Path, fs_type: Option<&str>) -> String {
    plain_note(path, fs_type).map(|n| format!(" - {}", n.replace("a new folder will be made", "new folder"))).unwrap_or_default()
}

/// Usable drives first (SD cards before USB drives, as `removable_drives` sorts them), then the internal
/// default, then the current folder when it is neither, then `earlier` (a folder chosen in a build before
/// the real ports folder setting, offered once), then the drives that cannot hold ports (listed, but not
/// selectable).
pub fn quick_choices(current: &Path, internal_default: &Path, drives: &[Drive], earlier: Option<&Path>) -> Vec<QuickChoice> {
    let mut usable: Vec<QuickChoice> = Vec::new();
    let mut unusable: Vec<QuickChoice> = Vec::new();
    let detail = |path: &Path, fs_type: Option<&str>| format!("{}{}", ellipsize(&path.to_string_lossy(), 60), notes(path, fs_type));
    for drive in drives {
        let path = drive.mount_point.join(SUBFOLDER);
        let can_hold = !is_non_linux_fs(&drive.fs_type);
        let choice = QuickChoice {
            title: if can_hold { drive.title() } else { format!("{} (can't be used)", drive.title()) },
            detail: detail(&path, Some(&drive.fs_type)),
            note: drive_note(&path, &drive.fs_type).unwrap_or_default(),
            path,
            kind: if drive.kind == "SD card" { Kind::SdCard } else { Kind::OtherDrive },
            usable: can_hold,
        };
        if can_hold { &mut usable } else { &mut unusable }.push(choice);
    }
    let mut add = |title: &str, path: &Path, kind: Kind| {
        if usable.iter().chain(unusable.iter()).any(|c| c.path == path) {
            return;
        }
        usable.push(QuickChoice { title: title.to_string(), detail: detail(path, None), note: plain_note(path, None).unwrap_or_default(), path: path.to_path_buf(), kind, usable: true });
    };
    add("Internal storage", internal_default, Kind::Internal);
    add("Current folder", current, Kind::Current);
    if let Some(path) = earlier {
        add("Folder you chose earlier", path, Kind::Earlier);
    }
    usable.extend(unusable);
    usable
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
    fn sd_cards_and_other_drives_are_told_apart() {
        let t = temp();
        let (sd, usb, other) = (t.path().join("SD"), t.path().join("USB"), t.path().join("OTHER"));
        for d in [&sd, &usb, &other] {
            fs::create_dir_all(d).unwrap();
        }
        let internal = t.path().join("Library");
        let drives = [drive(&sd, "SD card", "SD", "ext4"), drive(&usb, "USB drive", "USB", "ext4"), drive(&other, "Drive", "OTHER", "ext4")];
        let kinds: Vec<Kind> = quick_choices(&internal, &internal, &drives, None).iter().map(|c| c.kind).collect();
        assert_eq!(kinds, [Kind::SdCard, Kind::OtherDrive, Kind::OtherDrive, Kind::Internal]);
    }

    #[test]
    fn a_drive_that_cannot_hold_ports_is_listed_after_the_usable_choices_and_cannot_be_chosen() {
        let t = temp();
        let bad = t.path().join("EXFAT");
        let good = t.path().join("GOOD");
        fs::create_dir_all(&bad).unwrap();
        fs::create_dir_all(&good).unwrap();
        let internal = t.path().join("Library");
        // The exFAT card sorts first (SD cards come first), but it is listed after everything usable.
        let drives = [drive(&bad, "SD card", "EXFAT", "exfat"), drive(&good, "USB drive", "GOOD", "ext4")];
        let choices = quick_choices(&internal, &internal, &drives, None);
        let order: Vec<(&str, bool)> = choices.iter().map(|c| (c.title.as_str(), c.usable)).collect();
        assert_eq!(order, [("USB drive (GOOD)", true), ("Internal storage", true), ("SD card (EXFAT) (can't be used)", false)]);
        assert!(choices[2].detail.contains("exFAT cannot hold ports"), "{}", choices[2].detail);
        assert!(!choices[2].detail.contains("new folder"));
        assert_eq!(choices[2].kind, Kind::SdCard);
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

    #[test]
    fn each_choice_has_a_plain_note_without_the_folder() {
        let t = temp();
        let sd = t.path().join("SD");
        fs::create_dir_all(&sd).unwrap();
        let internal = t.path().join("Library");
        fs::create_dir_all(&internal).unwrap();
        let bad = t.path().join("EXFAT");
        fs::create_dir_all(&bad).unwrap();
        let drives = [drive(&sd, "SD card", "SD", "ext4"), drive(&bad, "SD card", "EXFAT", "exfat")];
        let choices = quick_choices(&internal, &internal, &drives, None);
        let notes: Vec<(&str, &str)> = choices.iter().map(|c| (c.title.as_str(), c.note.as_str())).collect();
        assert_eq!(notes[0], ("SD card (SD)", "a new folder will be made"));
        assert!(notes[1].1.is_empty() || notes[1].1.ends_with(" free"), "{notes:?}"); // free space is not known everywhere
        // A card whose Ports folder is there already says it is used as it is.
        fs::create_dir_all(sd.join("Ports")).unwrap();
        let again = quick_choices(&internal, &internal, &drives, None);
        assert!(again[0].note.starts_with("uses the Ports folder already there"), "{:?}", again[0].note);
        assert!(notes[2].1.contains("exFAT cannot hold ports"), "{notes:?}");
        assert!(choices.iter().all(|c| !c.note.contains('/')), "no path in a note");
    }
}
