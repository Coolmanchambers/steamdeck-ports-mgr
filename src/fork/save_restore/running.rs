//! Finds programs running from a folder, whoever started them (the app, Steam, a terminal).
//!
//! Linux only: a process counts when its program file (`/proc/<pid>/exe`) or its working folder
//! (`/proc/<pid>/cwd`) is inside one of the folders. Processes of other users cannot be read and are
//! skipped. Elsewhere nothing is found, and only the app's own launch record is checked.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub pid: u32,
    pub name: String,
}

impl Found {
    pub fn describe(&self) -> String {
        format!("{} (process {})", self.name, self.pid)
    }
}

pub fn processes_under(folders: &[PathBuf]) -> Vec<Found> {
    #[cfg(target_os = "linux")]
    {
        scan(std::path::Path::new("/proc"), folders, std::process::id())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = folders;
        Vec::new()
    }
}

#[cfg(target_os = "linux")]
fn scan(proc_root: &std::path::Path, folders: &[PathBuf], own_pid: u32) -> Vec<Found> {
    use crate::core::path_safety::is_within;
    use std::fs;
    // Both the path as written and its real location: /proc shows real paths, and a port folder can be a link.
    let roots: Vec<PathBuf> = folders.iter().flat_map(|f| [Some(f.clone()), fs::canonicalize(f).ok()]).flatten().collect();
    let Ok(entries) = fs::read_dir(proc_root) else { return Vec::new() };
    let mut found = Vec::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        if pid == own_pid {
            continue;
        }
        let dir = entry.path();
        let hit = ["exe", "cwd"].iter().any(|link| {
            fs::read_link(dir.join(link)).is_ok_and(|target| {
                let text = target.to_string_lossy();
                let target = std::path::Path::new(text.strip_suffix(" (deleted)").unwrap_or(&text)).to_path_buf();
                roots.iter().any(|root| is_within(&target, root))
            })
        });
        if hit {
            let name = fs::read_to_string(dir.join("comm")).map(|s| s.trim().to_string()).unwrap_or_else(|_| "a program".into());
            found.push(Found { pid, name });
        }
    }
    found.sort_by_key(|f| f.pid);
    found
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_program_whose_working_folder_is_in_the_game_folder_is_found() {
        #[cfg(target_os = "linux")]
        {
            use super::*;
            let dir = tempfile::tempdir().unwrap();
            let game = dir.path().join("Library/Game");
            std::fs::create_dir_all(game.join("bin")).unwrap();
            let mut child = std::process::Command::new("sleep").arg("30").current_dir(game.join("bin")).spawn().unwrap();
            let found = processes_under(std::slice::from_ref(&game));
            let elsewhere = processes_under(&[dir.path().join("Library/Other")]);
            let _ = child.kill();
            let _ = child.wait();
            assert!(found.iter().any(|f| f.pid == child.id() && f.name == "sleep"), "{found:?}");
            assert!(!elsewhere.iter().any(|f| f.pid == child.id()));
            assert!(found[0].describe().contains("(process "));
        }
    }
}
