//! Choosing the game files folder without a native file dialog: the same list the controller already
//! drives. Start points are the places game files usually live (an SD card, an EmuDeck folder), then
//! the user walks down folder by folder and says "use this folder".

use super::util::ellipsize;
use std::fs;
use std::path::{Path, PathBuf};

/// More than this many sub-folders in one folder are cut off; the list is for walking down, not for
/// listing a whole drive.
const MAX_ENTRIES: usize = 400;

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

/// Removable drives and mounts, newest layout first. On SteamOS an SD card appears under
/// `/run/media/<name>` (or `/run/media/<user>/<label>` on other systems).
pub fn system_mounts() -> Vec<PathBuf> {
    let mut found = Vec::new();
    for base in ["/run/media", "/media"] {
        let Ok(entries) = fs::read_dir(base) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            // `/run/media/<user>/<label>` holds the mounts one level down.
            let nested: Vec<PathBuf> = fs::read_dir(&path)
                .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir() && !is_hidden(p)).collect())
                .unwrap_or_default();
            let is_user_dir = path.file_name().is_some_and(|n| n.to_string_lossy() == std::env::var("USER").unwrap_or_default());
            if is_user_dir {
                found.extend(nested);
            } else {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

fn is_hidden(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    UseThis,
    Up,
    Folder(usize),
    More,
}

pub struct FolderBrowser {
    pub current: PathBuf,
    folders: Vec<PathBuf>,
    /// Sub-folders left out because there were too many.
    pub hidden_count: usize,
    pub error: Option<String>,
}

impl FolderBrowser {
    pub fn open(dir: &Path) -> FolderBrowser {
        let mut browser = FolderBrowser { current: dir.to_path_buf(), folders: Vec::new(), hidden_count: 0, error: None };
        browser.reload();
        browser
    }

    fn reload(&mut self) {
        self.folders.clear();
        self.hidden_count = 0;
        self.error = None;
        match fs::read_dir(&self.current) {
            Ok(entries) => {
                let mut dirs: Vec<PathBuf> = entries
                    .flatten()
                    .map(|e| e.path())
                    // `is_dir` follows links, so a linked folder (an SD card linked into home) counts.
                    .filter(|p| p.is_dir() && !is_hidden(p))
                    .collect();
                dirs.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default());
                if dirs.len() > MAX_ENTRIES {
                    self.hidden_count = dirs.len() - MAX_ENTRIES;
                    dirs.truncate(MAX_ENTRIES);
                }
                self.folders = dirs;
            }
            Err(e) => self.error = Some(format!("Could not open this folder: {e}")),
        }
    }

    pub fn lines(&self) -> Vec<Line> {
        let mut lines = vec![Line::UseThis];
        if self.current.parent().is_some() {
            lines.push(Line::Up);
        }
        lines.extend((0..self.folders.len()).map(Line::Folder));
        if self.hidden_count > 0 {
            lines.push(Line::More);
        }
        lines
    }

    pub fn labels(&self) -> Vec<String> {
        self.lines()
            .into_iter()
            .map(|line| match line {
                Line::UseThis => format!("Use this folder: {}", ellipsize(&self.current.to_string_lossy(), 60)),
                Line::Up => ".. (up one level)".to_string(),
                Line::Folder(i) => format!("{}/", self.folders[i].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
                Line::More => format!("... and {} more folders not shown", self.hidden_count),
            })
            .chain(self.error.clone())
            .collect()
    }

    pub fn enter(&mut self, index: usize) {
        if let Some(folder) = self.folders.get(index).cloned() {
            self.current = folder;
            self.reload();
        }
    }

    pub fn up(&mut self) {
        if let Some(parent) = self.current.parent().map(Path::to_path_buf) {
            self.current = parent;
            self.reload();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::rom_library::testutil::*;

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

    #[test]
    fn the_browser_walks_down_and_up_and_hides_hidden_folders_and_files() {
        let tmp = temp_dir();
        make_dirs(tmp.path(), &["Zeta", "alpha", ".git", "Beta"]);
        write(&tmp.path().join("a-file.txt"), b"x");
        let mut b = FolderBrowser::open(tmp.path());
        assert_eq!(b.lines()[0], Line::UseThis);
        assert_eq!(b.lines()[1], Line::Up);
        let labels = b.labels();
        assert_eq!(&labels[2..], ["alpha/", "Beta/", "Zeta/"]);
        b.enter(1);
        assert_eq!(b.current, tmp.path().join("Beta"));
        b.up();
        assert_eq!(b.current, tmp.path());
    }

    #[test]
    fn an_unreadable_folder_says_so_instead_of_crashing() {
        let b = FolderBrowser::open(Path::new("/definitely/not/here"));
        assert!(b.error.is_some());
        assert!(b.labels().last().unwrap().starts_with("Could not open"));
    }

    #[test]
    fn a_huge_folder_is_cut_off_with_a_note() {
        let tmp = temp_dir();
        for i in 0..(MAX_ENTRIES + 5) {
            fs::create_dir(tmp.path().join(format!("d{i:04}"))).unwrap();
        }
        let b = FolderBrowser::open(tmp.path());
        assert_eq!(b.hidden_count, 5);
        assert!(b.labels().iter().any(|l| l.contains("5 more folders")));
    }
}
