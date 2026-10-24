//! Choosing the game files folder. Browsing is the shared folder picker (`crate::fork::folder_picker`,
//! drawn by `src/app/folder_picker_screen.rs`); this module only adds the places game files usually
//! live (an SD card, an EmuDeck folder) as extra quick places.

use crate::fork::folder_picker::mounts;
use std::path::{Path, PathBuf};

/// Folder names that commonly hold ROMs, tried under the home folder and under each SD card.
const ROM_FOLDER_NAMES: [&str; 8] = ["ROMs", "Roms", "roms", "Emulation/roms", "Games/ROMs", "Games", "retro", "Downloads"];

/// The EmuDeck ROM folder (`Emulation/roms`), the place the wizard recommends.
const EMUDECK: &str = "Emulation/roms";

/// The EmuDeck ROM folder when there is one: on a removable drive first, then in the home folder.
pub fn emudeck_folder(home: Option<&Path>, drives: &[mounts::Drive]) -> Option<PathBuf> {
    drives.iter().map(|d| d.mount_point.join(EMUDECK)).chain(home.map(|h| h.join(EMUDECK))).find(|p| p.is_dir())
}

/// Places worth offering, as (label, path), labelled by place, never by a card's ID: "SD card: Emulation/roms"
/// (the EmuDeck folder first, marked Recommended), then other usual ROM folders, the whole card, the home folder.
/// Only folders that exist are returned.
pub fn start_points(home: Option<&Path>, drives: &[mounts::Drive]) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut add = |label: String, path: PathBuf| {
        if path.is_dir() && !out.iter().any(|(_, p)| *p == path) {
            out.push((label, path));
        }
    };
    if let Some(rec) = emudeck_folder(home, drives) {
        let place = drives.iter().find(|d| rec.starts_with(&d.mount_point)).map_or_else(|| "Home".to_string(), |d| d.title());
        add(format!("{place}: {EMUDECK} (Recommended)"), rec);
    }
    // By the drive's title (its name; a card named only by its ID is just "SD card"), so two drives differ.
    for d in drives {
        for sub in ROM_FOLDER_NAMES {
            add(format!("{}: {sub}", d.title()), d.mount_point.join(sub));
        }
        add(format!("{} (whole drive)", d.title()), d.mount_point.clone());
    }
    if let Some(home) = home {
        for sub in ROM_FOLDER_NAMES {
            add(format!("Home: {sub}"), home.join(sub));
        }
        add("Home folder".into(), home.to_path_buf());
    }
    out
}

/// Mounted removable drives (SD card, USB), from the kernel's mounts list.
pub fn system_drives() -> Vec<mounts::Drive> {
    mounts::removable_drives(&mounts::system_mounts(), &mounts::default_media_roots())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::rom_library::testutil::*;
    use std::fs;

    fn make_dirs(root: &Path, names: &[&str]) {
        for n in names {
            fs::create_dir_all(root.join(n)).unwrap();
        }
    }

    fn sd(path: &Path) -> mounts::Drive {
        mounts::Drive { mount_point: path.to_path_buf(), kind: "SD card", label: "c14f9023-6854-443f-9348-735551234567".into(), fs_type: "ext4".into() }
    }

    #[test]
    fn the_emudeck_folder_comes_first_as_recommended_and_no_label_shows_the_cards_id() {
        let tmp = temp_dir();
        make_dirs(tmp.path(), &["sd/Emulation/roms", "sd/ROMs", "home/Emulation/roms"]);
        let card = tmp.path().join("sd");
        let points = start_points(Some(&tmp.path().join("home")), &[sd(&card)]);
        assert_eq!(points[0], ("SD card: Emulation/roms (Recommended)".to_string(), card.join("Emulation/roms")));
        assert_eq!(points[1], ("SD card: ROMs".to_string(), card.join("ROMs")));
        assert!(points.iter().any(|(l, _)| l == "SD card (whole drive)"));
        // Two USB sticks are told apart by their names.
        make_dirs(tmp.path(), &["a/ROMs", "b/ROMs"]);
        let stick = |p: &str, label: &str| mounts::Drive { mount_point: tmp.path().join(p), kind: "USB drive", label: label.into(), fs_type: "ext4".into() };
        let two = start_points(None, &[stick("a", "RED"), stick("b", "BLUE")]);
        assert!(two.iter().any(|(l, _)| l == "USB drive (RED): ROMs") && two.iter().any(|(l, _)| l == "USB drive (BLUE): ROMs"), "{two:?}");
        assert!(points.iter().any(|(l, p)| l == "Home: Emulation/roms" && *p == tmp.path().join("home/Emulation/roms")));
        assert!(points.iter().all(|(l, _)| !l.contains("c14f9023")), "{points:?}");
        assert_eq!(emudeck_folder(Some(&tmp.path().join("home")), &[sd(&card)]), Some(card.join("Emulation/roms")));
        // No card: the home folder's EmuDeck folder is the recommended one.
        let points = start_points(Some(&tmp.path().join("home")), &[]);
        assert_eq!(points[0].0, "Home: Emulation/roms (Recommended)");
    }

    #[test]
    fn start_points_list_only_folders_that_exist() {
        let tmp = temp_dir();
        make_dirs(tmp.path(), &["home/Emulation/roms", "home/Documents", "sd/ROMs"]);
        let points = start_points(Some(&tmp.path().join("home")), &[sd(&tmp.path().join("sd"))]);
        let paths: Vec<_> = points.iter().map(|(_, p)| p.clone()).collect();
        assert!(paths.contains(&tmp.path().join("sd/ROMs")));
        assert!(paths.contains(&tmp.path().join("home/Emulation/roms")));
        assert!(paths.contains(&tmp.path().join("home")));
        assert!(!paths.contains(&tmp.path().join("home/Documents")));
        assert!(!paths.contains(&tmp.path().join("home/ROMs")));
    }
}
