//! "Connect by hand...": walk folders and pick one file, with the same list the controller drives.
//!
//! The folder browser (`browse.rs`) only picks folders; this lists the files of the current folder too.
//! Game files (ROM and disc extensions) are listed first. Nothing is opened for writing; choosing a
//! file only hands its path back.

use super::util::{ellipsize, human_bytes};
use crate::fork::rom_library::system::is_rom_extension;
use std::fs;
use std::path::{Path, PathBuf};

/// More entries than this in one folder are cut off (folders and files counted separately).
const MAX_ENTRIES: usize = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    Where,
    Up,
    Folder(usize),
    File(usize),
    More,
    Cancel,
}

pub struct FileBrowser {
    pub current: PathBuf,
    folders: Vec<PathBuf>,
    files: Vec<(PathBuf, u64)>,
    pub hidden_count: usize,
    pub error: Option<String>,
}

fn is_hidden(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn is_game_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(is_rom_extension)
}

impl FileBrowser {
    pub fn open(dir: &Path) -> FileBrowser {
        let mut b = FileBrowser { current: dir.to_path_buf(), folders: Vec::new(), files: Vec::new(), hidden_count: 0, error: None };
        b.reload();
        b
    }

    fn reload(&mut self) {
        self.folders.clear();
        self.files.clear();
        self.hidden_count = 0;
        self.error = None;
        let entries = match fs::read_dir(&self.current) {
            Ok(rd) => rd,
            Err(e) => {
                self.error = Some(format!("Could not open this folder: {e}"));
                return;
            }
        };
        for path in entries.flatten().map(|e| e.path()).filter(|p| !is_hidden(p)) {
            // `metadata` follows links, so a linked file or folder counts as what it points to.
            match fs::metadata(&path) {
                Ok(m) if m.is_dir() => self.folders.push(path),
                Ok(m) if m.is_file() => self.files.push((path, m.len())),
                _ => {}
            }
        }
        self.folders.sort_by_key(|p| name_of(p).to_lowercase());
        self.files.sort_by_key(|(p, _)| (!is_game_file(p), name_of(p).to_lowercase()));
        if self.folders.len() > MAX_ENTRIES {
            self.hidden_count += self.folders.len() - MAX_ENTRIES;
            self.folders.truncate(MAX_ENTRIES);
        }
        if self.files.len() > MAX_ENTRIES {
            self.hidden_count += self.files.len() - MAX_ENTRIES;
            self.files.truncate(MAX_ENTRIES);
        }
    }

    pub fn lines(&self) -> Vec<Line> {
        let mut lines = vec![Line::Where];
        if self.current.parent().is_some() {
            lines.push(Line::Up);
        }
        lines.extend((0..self.folders.len()).map(Line::Folder));
        lines.extend((0..self.files.len()).map(Line::File));
        if self.hidden_count > 0 {
            lines.push(Line::More);
        }
        lines.push(Line::Cancel);
        lines
    }

    pub fn labels(&self) -> Vec<String> {
        self.lines()
            .into_iter()
            .map(|line| match line {
                Line::Where => match &self.error {
                    Some(e) => ellipsize(e, 90),
                    None if self.folders.is_empty() && self.files.is_empty() => "This folder is empty. Pick a file to connect it.".to_string(),
                    None => "Pick the game file. It is only read; nothing is moved.".to_string(),
                },
                Line::Up => ".. (up one level)".to_string(),
                Line::Folder(i) => format!("{}/", ellipsize(&name_of(&self.folders[i]), 60)),
                Line::File(i) => {
                    let (path, size) = &self.files[i];
                    format!("{}  ({})", ellipsize(&name_of(path), 60), human_bytes(*size))
                }
                Line::More => format!("... and {} more not shown", self.hidden_count),
                Line::Cancel => "Cancel".to_string(),
            })
            .collect()
    }

    pub fn up(&mut self) {
        if let Some(parent) = self.current.parent() {
            self.current = parent.to_path_buf();
            self.reload();
        }
    }

    pub fn enter(&mut self, i: usize) {
        if let Some(dir) = self.folders.get(i).cloned() {
            self.current = dir;
            self.reload();
        }
    }

    pub fn file(&self, i: usize) -> Option<&Path> {
        self.files.get(i).map(|(p, _)| p.as_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::rom_library::testutil::*;

    #[test]
    fn folders_then_game_files_first_and_hidden_left_out() {
        let tmp = temp_dir();
        write(&tmp.path().join("notes.txt"), b"x");
        write(&tmp.path().join("b.z64"), &n64_rom(1, 2_000));
        write(&tmp.path().join("A.sfc"), b"rom");
        write(&tmp.path().join(".hidden.z64"), b"x");
        std::fs::create_dir_all(tmp.path().join("sub")).unwrap();
        let b = FileBrowser::open(tmp.path());
        let labels = b.labels();
        assert_eq!(labels[2], "sub/");
        assert!(labels[3].starts_with("A.sfc") && labels[4].starts_with("b.z64") && labels[5].starts_with("notes.txt"), "{labels:?}");
        assert!(!labels.iter().any(|l| l.contains(".hidden")));
        assert_eq!(b.lines()[3], Line::File(0));
        assert_eq!(b.file(1).unwrap(), tmp.path().join("b.z64"));
        assert_eq!(labels.last().unwrap(), "Cancel");
    }

    #[test]
    fn walking_down_and_up() {
        let tmp = temp_dir();
        write(&tmp.path().join("n64/game.z64"), &n64_rom(1, 2_000));
        let mut b = FileBrowser::open(tmp.path());
        b.enter(0);
        assert_eq!(b.current, tmp.path().join("n64"));
        assert_eq!(b.file(0).unwrap(), tmp.path().join("n64/game.z64"));
        b.up();
        assert_eq!(b.current, tmp.path());
    }

    #[test]
    fn a_missing_folder_says_so() {
        let tmp = temp_dir();
        let b = FileBrowser::open(&tmp.path().join("gone"));
        assert!(b.labels()[0].starts_with("Could not open this folder"));
    }
}
