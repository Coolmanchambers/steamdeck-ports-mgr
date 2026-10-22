//! The fork's own small settings file, `fork-settings.json` next to `state.json`.
//!
//! Upstream's `state.json` is rewritten whole by upstream's code from a fixed set of fields, so extra
//! keys placed there would be lost. This file is ours. Unknown keys are kept when saving, so other
//! fork features can add their own without trampling each other.
//!
//! A file that exists but is not a JSON object is never overwritten: it loads as empty settings and
//! saving is refused until the user fixes or removes it.

use serde_json::{Map, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "fork-settings.json";
const KEY_STEAM_ACCOUNT: &str = "steam_account_id";
const KEY_ADOPT_FOLDERS: &str = "adopt_folders";
const KEY_BACKUPS_KEEP: &str = "steam_backups_keep";
const KEY_ROM_PROMPTED: &str = "rom_prompted_ports";

#[derive(Debug)]
pub struct ForkSettings {
    path: PathBuf,
    map: Map<String, Value>,
    /// False when the file on disk could not be read as an object; saving is then refused.
    writable: bool,
}

impl ForkSettings {
    pub fn load(path: &Path) -> ForkSettings {
        let (map, writable) = match fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Value>(text.strip_prefix('\u{FEFF}').unwrap_or(&text)) {
                Ok(Value::Object(map)) => (map, true),
                _ => (Map::new(), false),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => (Map::new(), true),
            Err(_) => (Map::new(), false),
        };
        ForkSettings { path: path.to_path_buf(), map, writable }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The Steam account the user picked (the folder name under `userdata/`).
    pub fn steam_account(&self) -> Option<u32> {
        self.map.get(KEY_STEAM_ACCOUNT).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok())
    }

    pub fn set_steam_account(&mut self, account_id: u32) -> io::Result<()> {
        self.map.insert(KEY_STEAM_ACCOUNT.into(), Value::from(account_id));
        self.save()
    }

    /// Extra folders to scan for ports installed outside the library, as the user wrote them (a leading
    /// `~` is expanded by the scanner).
    pub fn adopt_folders(&self) -> Vec<String> {
        self.map
            .get(KEY_ADOPT_FOLDERS)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// How many `shortcuts.vdf` backups to keep (at least 1).
    pub fn backups_keep(&self) -> usize {
        self.map
            .get(KEY_BACKUPS_KEEP)
            .and_then(Value::as_u64)
            .map_or(super::backups::DEFAULT_KEEP, |n| usize::try_from(n).unwrap_or(usize::MAX).max(1))
    }

    /// Ports the "connect your game files" offer was already made for, so it is made once per port.
    pub fn rom_prompted(&self, port_key: &str) -> bool {
        self.map.get(KEY_ROM_PROMPTED).and_then(Value::as_array).is_some_and(|a| a.iter().any(|v| v.as_str() == Some(port_key)))
    }

    pub fn mark_rom_prompted(&mut self, port_key: &str) -> io::Result<()> {
        if self.rom_prompted(port_key) {
            return Ok(());
        }
        let entry = self.map.entry(KEY_ROM_PROMPTED).or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(list) = entry {
            list.push(Value::from(port_key));
        }
        self.save()
    }

    /// A key owned by another fork feature (it documents its own format).
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.map.get(key)
    }

    pub fn set(&mut self, key: &str, value: Value) -> io::Result<()> {
        self.map.insert(key.to_string(), value);
        self.save()
    }

    /// Writes the file with the keys it has, plus an empty `adopt_folders` list so the user finds the
    /// place to add folders.
    pub fn save(&mut self) -> io::Result<()> {
        if !self.writable {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{} is not a JSON object; fix or remove it first", self.path.display())));
        }
        self.map.entry(KEY_ADOPT_FOLDERS).or_insert_with(|| Value::Array(Vec::new()));
        let text = serde_json::to_string_pretty(&self.map).map_err(io::Error::other)?;
        crate::core::files::write_atomic(&self.path, text.as_bytes())
    }

    /// Creates the file if it is missing so "edit the folder list" has something to open.
    pub fn ensure_file(&mut self) -> io::Result<()> {
        if self.path.exists() {
            return Ok(());
        }
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("dpm_fork_settings_{}_{}.json", std::process::id(), name));
        let _ = fs::remove_file(&p);
        p
    }

    #[test]
    fn a_missing_file_gives_defaults() {
        let s = ForkSettings::load(&temp_path("missing"));
        assert_eq!(s.steam_account(), None);
        assert!(s.adopt_folders().is_empty());
        assert_eq!(s.backups_keep(), super::super::backups::DEFAULT_KEEP);
    }

    #[test]
    fn saved_choices_round_trip_and_unknown_keys_survive() {
        let path = temp_path("roundtrip");
        fs::write(&path, r#"{"rom_folder": "/roms", "adopt_folders": [" ~/Games ", "", "/mnt/sd/ports"], "steam_backups_keep": 3}"#).unwrap();
        let mut s = ForkSettings::load(&path);
        assert_eq!(s.adopt_folders(), ["~/Games", "/mnt/sd/ports"]);
        assert_eq!(s.backups_keep(), 3);
        s.set_steam_account(39_734_274).unwrap();
        let again = ForkSettings::load(&path);
        assert_eq!(again.steam_account(), Some(39_734_274));
        assert_eq!(again.adopt_folders().len(), 2);
        assert!(fs::read_to_string(&path).unwrap().contains("rom_folder"));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn the_game_files_offer_is_remembered_per_port_and_other_keys_survive() {
        let path = temp_path("romprompt");
        fs::write(&path, r#"{"steam_backups_keep": 4}"#).unwrap();
        let mut s = ForkSettings::load(&path);
        assert!(!s.rom_prompted("a/b"));
        s.mark_rom_prompted("a/b").unwrap();
        s.mark_rom_prompted("a/b").unwrap();
        let again = ForkSettings::load(&path);
        assert!(again.rom_prompted("a/b") && !again.rom_prompted("c"));
        assert_eq!(again.backups_keep(), 4);
        assert_eq!(fs::read_to_string(&path).unwrap().matches("a/b").count(), 1);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn keep_is_at_least_one_and_wrong_types_fall_back() {
        let path = temp_path("keep");
        fs::write(&path, r#"{"steam_backups_keep": 0, "steam_account_id": "x", "adopt_folders": "nope"}"#).unwrap();
        let s = ForkSettings::load(&path);
        assert_eq!(s.backups_keep(), 1);
        assert_eq!(s.steam_account(), None);
        assert!(s.adopt_folders().is_empty());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_file_that_is_not_an_object_is_never_overwritten() {
        let path = temp_path("garbage");
        fs::write(&path, "[1, 2").unwrap();
        let mut s = ForkSettings::load(&path);
        assert!(s.set_steam_account(5).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "[1, 2");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn ensure_file_creates_a_starter_with_an_empty_folder_list_and_leaves_an_existing_one() {
        let path = temp_path("ensure");
        let mut s = ForkSettings::load(&path);
        s.ensure_file().unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("adopt_folders"));
        fs::write(&path, r#"{"adopt_folders": ["/x"]}"#).unwrap();
        let mut s = ForkSettings::load(&path);
        s.ensure_file().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), r#"{"adopt_folders": ["/x"]}"#);
        let _ = fs::remove_file(&path);
    }
}
