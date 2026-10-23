//! Choosing the game files folder. Browsing is the shared folder picker (`crate::fork::folder_picker`,
//! drawn by `src/app/folder_picker_screen.rs`); this module only adds the places game files usually
//! live (an SD card, an EmuDeck folder) as extra quick places.

use crate::fork::folder_picker::mounts;
use std::path::{Path, PathBuf};

/// Folder names that commonly hold ROMs, tried under the home folder and under each SD card.
const ROM_FOLDER_NAMES: [&str; 8] = ["ROMs", "Roms", "roms", "Emulation/roms", "Games/ROMs", "Games", "retro", "Downloads"];

/// Places worth offering, as (label, path). Only folders that exist are returned.
pub fn start_points(home: Option<&Path>, mounts: &[PathBuf]) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut add = |label: String, path: PathBuf| {
        if path.is_dir() && !out.iter().any(|(_, p)| *p == path) {
            out.push((label, path));
        }
    };
    for mount in mounts {
        let name = mount.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Drive".into());
        for sub in ROM_FOLDER_NAMES {
            add(format!("{name}/{sub}"), mount.join(sub));
        }
        add(format!("{name} (whole card)"), mount.clone());
    }
    if let Some(home) = home {
        for sub in ROM_FOLDER_NAMES {
            add(format!("Home/{sub}"), home.join(sub));
        }
        add("Home folder".into(), home.to_path_buf());
    }
    out
}

/// Mounted removable drives (SD card, USB), from the kernel's mounts list.
pub fn system_mounts() -> Vec<PathBuf> {
    mounts::removable_drives(&mounts::system_mounts(), &mounts::default_media_roots()).into_iter().map(|d| d.mount_point).collect()
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

    #[test]
    fn start_points_list_only_folders_that_exist() {
        let tmp = temp_dir();
        make_dirs(tmp.path(), &["home/Emulation/roms", "home/Documents", "sd/ROMs"]);
        let points = start_points(Some(&tmp.path().join("home")), &[tmp.path().join("sd")]);
        let paths: Vec<_> = points.iter().map(|(_, p)| p.clone()).collect();
        assert!(paths.contains(&tmp.path().join("sd/ROMs")));
        assert!(paths.contains(&tmp.path().join("home/Emulation/roms")));
        assert!(paths.contains(&tmp.path().join("home")));
        assert!(!paths.contains(&tmp.path().join("home/Documents")));
        assert!(!paths.contains(&tmp.path().join("home/ROMs")));
    }
}
