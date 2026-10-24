//! Folders that hold many apps' files, never one game's saves: another port's folder in the ports library,
//! the home folder's containers (`~/.local/share`, `~/.config`, Steam's folders, Documents), Proton prefix
//! levels (`compatdata/<n>`, `pfx`, `drive_c`, `users/<x>`, `AppData/*`, Saved Games) and the roots of mounted
//! drives. Restore never replaces them and the save finder never stores them (a hand-edited override gets the
//! same refusal), since "Back up now" would copy all of Steam and a restore could swap it.

use crate::core::path_safety::is_within;
use std::path::{Component, Path};

/// The refusal text `resolve_target` gives; `save_finder::check_choice` turns it into a sentence for the user.
pub const BROAD_REASON: &str = "holds many apps' files, not this game's";

const NAMES: [&str; 19] = [
    "steamapps", "compatdata", "pfx", "drive_c", "users", "appdata", "saved games", "my games", "steam", ".steam", ".local", ".config", ".cache", ".var", "documents", "my documents", "desktop", "downloads", "userdata",
];

fn parts(path: &Path) -> Vec<String> {
    path.components().filter_map(|c| if let Component::Normal(n) = c { Some(n.to_string_lossy().to_lowercase()) } else { None }).collect()
}

/// `Some(BROAD_REASON)` when `path` is a folder like those above. `game_dir` is the port's own folder and `library`
/// the ports library that holds it (and every other port's folder).
pub fn container_reason(path: &Path, game_dir: &Path, library: Option<&Path>) -> Option<&'static str> {
    if let Some(library) = library.filter(|l| l.components().count() > 1) {
        if is_within(path, library) && !is_within(path, game_dir) {
            return Some(BROAD_REASON);
        }
    }
    let c = parts(path);
    let n = c.len();
    let last = c.last().map(String::as_str).unwrap_or("");
    let before = if n >= 2 { c[n - 2].as_str() } else { "" };
    // Flatpak: `.var/app`, `.var/app/<id>` and its `data`, `config` and `cache` hold one app's files, not a game's saves.
    let flatpak = c.windows(2).position(|w| w[0] == ".var" && w[1] == "app").is_some_and(|i| {
        let rest = n - (i + 2);
        rest <= 1 || (rest == 2 && matches!(last, "data" | "config" | "cache"))
    });
    let named = flatpak
        || NAMES.contains(&last)
        || before == "compatdata"
        || before == "users"
        || (before == "appdata" && matches!(last, "roaming" | "local" | "locallow"))
        || (before == ".local" && last == "share")
        || (before == "steamapps" && matches!(last, "common" | "shadercache" | "workshop"));
    let top = |root: &[&str], depth: usize| n >= root.len() && c.iter().zip(root).all(|(a, b)| a == b) && n <= root.len() + depth;
    let mount_root = top(&["run", "media"], 2) || top(&["media"], 2) || top(&["mnt"], 1);
    (named || mount_root).then_some(BROAD_REASON)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(p: &str) -> bool {
        container_reason(Path::new(p), Path::new("/home/deck/Ports/dusk"), Some(Path::new("/home/deck/Ports"))).is_some()
    }

    #[test]
    fn containers_are_refused() {
        for p in [
            "/home/deck/.local/share", "/home/deck/.local", "/home/deck/.config", "/home/deck/.steam", "/home/deck/.var", "/home/deck/.cache", "/home/deck/Documents", "/home/deck/Desktop",
            "/home/deck/Downloads", "/home/deck/.local/share/Steam", "/home/deck/.steam/steam/steamapps", "/home/deck/.steam/steam/steamapps/compatdata", "/home/deck/.steam/steam/steamapps/compatdata/123",
            "/home/deck/.steam/steam/steamapps/common", "/x/compatdata/123/pfx", "/x/pfx/drive_c", "/x/drive_c/users", "/x/drive_c/users/steamuser", "/x/steamuser/AppData", "/x/AppData/Roaming",
            "/x/AppData/Local", "/x/AppData/LocalLow", "/x/steamuser/Saved Games", "/x/steamuser/My Games", "/run/media/deck", "/run/media/deck/SD", "/media/sd", "/media/deck/SD", "/mnt/data", "/mnt", "/home/deck/.steam/steam/userdata", "/home/deck/.local/share/Steam/userdata", "/home/deck/.var/app", "/home/deck/.var/app/com.valvesoftware.Steam",
            "/home/deck/.var/app/com.valvesoftware.Steam/data", "/home/deck/.var/app/com.valvesoftware.Steam/config", "/home/deck/.var/app/com.valvesoftware.Steam/cache",
        ] {
            assert!(refused(p), "{p}");
        }
    }

    #[test]
    fn a_games_own_folder_inside_a_container_is_fine() {
        for p in [
            "/home/deck/.local/share/Dusklight", "/home/deck/.config/dusk", "/home/deck/Documents/My Games/Dusk", "/x/AppData/Roaming/Dusklight", "/run/media/deck/SD/Saves/dusk", "/mnt/data/saves/dusk",
            "/home/deck/.local/share/Steam/steamapps/compatdata/123/pfx/drive_c/users/steamuser/AppData/Roaming/Dusklight", "/home/deck/Ports/dusk/Save", "/home/deck/.dusk", "/home/deck/.var/app/com.valvesoftware.Steam/data/Steam/steamapps/compatdata/1/pfx/drive_c/users/steamuser/AppData/Roaming/Dusk",
            "/home/deck/.var/app/org.dusk.Game/data/saves",
        ] {
            assert!(!refused(p), "{p}");
        }
    }

    #[test]
    fn another_ports_folder_and_the_library_are_refused_but_not_this_ones() {
        let (game, lib) = (Path::new("/home/deck/Ports/dusk"), Some(Path::new("/home/deck/Ports")));
        assert!(container_reason(Path::new("/home/deck/Ports"), game, lib).is_some());
        assert!(container_reason(Path::new("/home/deck/Ports/soh"), game, lib).is_some());
        assert!(container_reason(Path::new("/home/deck/Ports/soh/Save"), game, lib).is_some());
        assert!(container_reason(Path::new("/home/deck/Ports/dusk"), game, lib).is_none());
        assert!(container_reason(Path::new("/home/deck/Ports/dusk/Save"), game, lib).is_none());
        assert!(container_reason(Path::new("/home/deck/Ports/soh"), game, None).is_none(), "without a library there is nothing to compare with");
    }
}
