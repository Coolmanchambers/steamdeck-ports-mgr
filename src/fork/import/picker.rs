//! The folder picker Import ports uses, in one place.
//!
//! STUB until the shared folder picker of the ports-folder work is on `main`: this adapts the game-files
//! folder browser (`rom_screens::browse`) to ports folders (other start points, "Cancel" that does not
//! say "Game files"). Switching to the shared picker means changing this file and the one call in
//! `src/app/import_screen.rs`.

use crate::fork::rom_screens::browse::{system_mounts, FolderBrowser, Line};
use std::path::{Path, PathBuf};

/// Folder names that ports commonly live in, tried under the home folder and under each SD card.
const PORT_FOLDER_NAMES: [&str; 5] = ["Applications", "Games", "Ports", "Emulation/tools", "Downloads"];

/// Places worth offering, as (label, path); only folders that exist, each once.
pub fn start_points(home: Option<&Path>, mounts: &[PathBuf]) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut add = |label: String, path: PathBuf| {
        if path.is_dir() && !out.iter().any(|(_, p)| *p == path) {
            out.push((label, path));
        }
    };
    for mount in mounts {
        let name = mount.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Drive".into());
        for sub in PORT_FOLDER_NAMES {
            add(format!("{name}/{sub}"), mount.join(sub));
        }
        add(format!("{name} (whole card)"), mount.clone());
    }
    if let Some(home) = home {
        for sub in PORT_FOLDER_NAMES {
            add(format!("Home/{sub}"), home.join(sub));
        }
        add("Home folder".into(), home.to_path_buf());
    }
    out
}

/// The start points of this device (its SD card and home folder).
pub fn device_start_points(home: Option<&Path>) -> Vec<(String, PathBuf)> {
    start_points(home, &system_mounts())
}

/// The browser's rows with its "Cancel (back to Game files)" row reworded for Import.
pub fn browser_labels(browser: &FolderBrowser) -> Vec<String> {
    let lines = browser.lines();
    let mut labels = browser.labels();
    for (i, line) in lines.iter().enumerate() {
        match line {
            Line::Cancel => labels[i] = "Cancel".to_string(),
            Line::UseThis => labels[i] = "Add this folder".to_string(),
            _ => {}
        }
    }
    labels
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("dpm_import_picker_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn start_points_are_the_places_ports_live_and_only_those_that_exist() {
        let dir = temp_dir("points");
        for sub in ["home/Games", "home/Documents", "sd/Ports", "sd/Roms"] {
            fs::create_dir_all(dir.join(sub)).unwrap();
        }
        let points = start_points(Some(&dir.join("home")), &[dir.join("sd")]);
        let paths: Vec<_> = points.iter().map(|(_, p)| p.clone()).collect();
        assert!(paths.contains(&dir.join("sd/Ports")) && paths.contains(&dir.join("home/Games")));
        assert!(paths.contains(&dir.join("sd")) && paths.contains(&dir.join("home")));
        assert!(!paths.contains(&dir.join("home/Documents")) && !paths.contains(&dir.join("sd/Roms")));
        assert_eq!(points[0].0, "sd/Ports", "the SD card comes first");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_browser_rows_say_add_and_cancel_not_game_files() {
        let dir = temp_dir("labels");
        fs::create_dir_all(dir.join("a")).unwrap();
        let labels = browser_labels(&FolderBrowser::open(&dir));
        assert_eq!(labels[0], "Add this folder");
        assert_eq!(labels.last().unwrap(), "Cancel");
        assert!(labels.contains(&"a/".to_string()) && labels.contains(&".. (up one level)".to_string()));
        assert!(!labels.join("\n").contains("Game files"));
        let _ = fs::remove_dir_all(dir);
    }
}
