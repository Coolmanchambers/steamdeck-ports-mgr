//! The one setting (the ROM folder) and the record of what is connected to what.
//!
//! This lives in its own file (`rom_library.json`, beside `state.json`) and not in the app's
//! `state.json`: `StateManager::save` rewrites that file from a fixed list of fields, so any key this
//! fork added to it would be silently dropped the next time the app saved. A separate file also keeps
//! the upstream `state` module unedited.

use super::connect::{apply, apply_set, ApplyError, Approval, ConnectPlan, Mapping};
use super::scan::ScanResult;
use crate::core::files::write_atomic;
use serde_json::{json, Value};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const VERSION: u64 = 1;

#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    /// The file exists but is not a library file. It is left untouched so nothing is lost.
    Unreadable(PathBuf),
    NotAFolder(PathBuf),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Io(e) => write!(f, "{e}"),
            StoreError::Unreadable(p) => write!(f, "{} is not a readable ROM library file; it was left as it is", p.display()),
            StoreError::NotAFolder(p) => write!(f, "{} is not a folder", p.display()),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        StoreError::Io(e)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomLibraryStore {
    path: PathBuf,
    rom_dir: Option<PathBuf>,
    mappings: Vec<Mapping>,
}

impl RomLibraryStore {
    /// A missing file is an empty library. A file that does not parse is an error, not an empty
    /// library, so a damaged record is never overwritten by a fresh one.
    pub fn load(path: &Path) -> Result<RomLibraryStore, StoreError> {
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(RomLibraryStore { path: path.to_path_buf(), rom_dir: None, mappings: Vec::new() }),
            Err(e) => return Err(e.into()),
        };
        let v: Value = serde_json::from_str(text.trim_start_matches('\u{FEFF}')).map_err(|_| StoreError::Unreadable(path.to_path_buf()))?;
        let obj = v.as_object().ok_or_else(|| StoreError::Unreadable(path.to_path_buf()))?;
        Ok(RomLibraryStore {
            path: path.to_path_buf(),
            rom_dir: obj.get("rom_dir").and_then(Value::as_str).filter(|s| !s.is_empty()).map(PathBuf::from),
            mappings: obj.get("mappings").and_then(Value::as_array).map(|a| a.iter().filter_map(Mapping::from_json).collect()).unwrap_or_default(),
        })
    }

    pub fn save(&self) -> io::Result<()> {
        let data = json!({
            "version": VERSION,
            "rom_dir": self.rom_dir.as_ref().map(|p| p.to_string_lossy().into_owned()),
            "mappings": self.mappings.iter().map(Mapping::to_json).collect::<Vec<_>>(),
        });
        let text = serde_json::to_string_pretty(&data).map_err(io::Error::other)?;
        write_atomic(&self.path, text.as_bytes())
    }

    pub fn rom_dir(&self) -> Option<&Path> {
        self.rom_dir.as_deref()
    }

    /// The single setting. `None` clears it. The folder must exist; nothing in it is touched.
    pub fn set_rom_dir(&mut self, dir: Option<PathBuf>) -> Result<(), StoreError> {
        if let Some(d) = &dir {
            if !d.is_dir() {
                return Err(StoreError::NotAFolder(d.clone()));
            }
        }
        self.rom_dir = dir;
        self.save()?;
        Ok(())
    }

    pub fn mappings(&self) -> &[Mapping] {
        &self.mappings
    }

    pub fn mappings_for(&self, port_key: &str) -> Vec<&Mapping> {
        self.mappings.iter().filter(|m| m.port_key == port_key).collect()
    }

    /// A port folder moved from `from` to `to` (`port_mover`): connections whose link or copy lived in it
    /// now name the same place under `to`. Originals are not touched. Saves only when something changed;
    /// returns how many records changed.
    pub fn repoint_dests(&mut self, from: &Path, to: &Path) -> io::Result<usize> {
        let mut changed = 0;
        for m in &mut self.mappings {
            let Some(rest) = m.dest.as_ref().and_then(|d| d.strip_prefix(from).ok()).map(Path::to_path_buf) else { continue };
            m.dest = Some(if rest.as_os_str().is_empty() { to.to_path_buf() } else { to.join(rest) });
            changed += 1;
        }
        if changed > 0 {
            self.save()?;
        }
        Ok(changed)
    }

    /// Record a connection, replacing an earlier one for the same port and file slot.
    pub fn record(&mut self, mapping: Mapping) -> io::Result<()> {
        self.mappings.retain(|m| !(m.port_key == mapping.port_key && m.slot == mapping.slot));
        self.mappings.push(mapping);
        self.save()
    }

    /// Drop the record of a connection. Only the record: the link or copy and the original stay where
    /// they are, because this app never deletes game files. Returns whether there was one.
    pub fn forget(&mut self, port_key: &str, slot: &str) -> io::Result<bool> {
        let before = self.mappings.len();
        self.mappings.retain(|m| !(m.port_key == port_key && m.slot == slot));
        let removed = self.mappings.len() != before;
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    /// Apply a plan and record the result in one step. A failed apply records nothing.
    pub fn apply_and_record(&mut self, plan: &ConnectPlan, approval: Approval) -> Result<Mapping, ApplyOrStoreError> {
        let mapping = apply(plan, approval).map_err(ApplyOrStoreError::Apply)?;
        self.record(mapping.clone()).map_err(ApplyOrStoreError::Record)?;
        Ok(mapping)
    }

    /// Apply several plans (a disc: its `.cue` and tracks) all-or-nothing and record every mapping.
    pub fn apply_set_and_record(&mut self, plans: &[ConnectPlan], approval: Approval) -> Result<Vec<Mapping>, ApplyOrStoreError> {
        let mappings = apply_set(plans, approval).map_err(ApplyOrStoreError::Apply)?;
        self.mappings.retain(|old| !mappings.iter().any(|m| m.port_key == old.port_key && m.slot == old.slot));
        self.mappings.extend(mappings.iter().cloned());
        self.save().map_err(ApplyOrStoreError::Record)?;
        Ok(mappings)
    }

    /// Where the saved scan lives: beside the library file.
    pub fn scan_cache_path(&self) -> PathBuf {
        self.path.with_file_name("rom_scan_cache.json")
    }

    /// The last scan, to hand to `scan` so unchanged files are not hashed again. Any problem reading
    /// it gives `None`: the cache only saves time.
    pub fn load_scan_cache(&self) -> Option<ScanResult> {
        let text = fs::read_to_string(self.scan_cache_path()).ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        Some(ScanResult::from_json(&v))
    }

    pub fn save_scan_cache(&self, scan: &ScanResult) -> io::Result<()> {
        let text = serde_json::to_string(&scan.to_json()).map_err(io::Error::other)?;
        write_atomic(&self.scan_cache_path(), text.as_bytes())
    }
}

#[derive(Debug)]
pub enum ApplyOrStoreError {
    Apply(ApplyError),
    /// The connection was made but could not be written to the record.
    Record(io::Error),
}

impl fmt::Display for ApplyOrStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApplyOrStoreError::Apply(e) => write!(f, "{e}"),
            ApplyOrStoreError::Record(e) => write!(f, "connected, but the record could not be saved: {e}"),
        }
    }
}

impl std::error::Error for ApplyOrStoreError {}

#[cfg(test)]
mod tests {
    use super::super::connect::{plan_connect, Access, ConnectRequest, Method};
    use super::super::scan::{scan, ScanOptions};
    use super::super::testutil::*;
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn mapping(port: &str, slot: &str) -> Mapping {
        Mapping {
            port_key: port.into(),
            slot: slot.into(),
            method: Method::Symlink,
            source: PathBuf::from("/roms/a.z64"),
            dest: Some(PathBuf::from("/lib/p/a.z64")),
            size: 10,
            mtime_ms: 5,
            crc32: Some(1),
            created_by_us: true,
            connected_at: "2026-10-21T00:00:00Z".into(),
        }
    }

    #[test]
    fn a_missing_file_is_an_empty_library() {
        let dir = temp_dir();
        let store = RomLibraryStore::load(&dir.path().join("rom_library.json")).unwrap();
        assert!(store.rom_dir().is_none() && store.mappings().is_empty());
        assert!(!dir.path().join("rom_library.json").exists(), "loading does not write");
    }

    #[test]
    fn the_rom_folder_setting_survives_a_restart() {
        let dir = temp_dir();
        let roms = dir.path().join("roms");
        fs::create_dir_all(&roms).unwrap();
        let file = dir.path().join("rom_library.json");
        let mut store = RomLibraryStore::load(&file).unwrap();
        store.set_rom_dir(Some(roms.clone())).unwrap();
        assert_eq!(RomLibraryStore::load(&file).unwrap().rom_dir(), Some(roms.as_path()));
        store.set_rom_dir(None).unwrap();
        assert!(RomLibraryStore::load(&file).unwrap().rom_dir().is_none());
    }

    #[test]
    fn the_rom_folder_must_be_a_folder_and_a_bad_value_changes_nothing() {
        let dir = temp_dir();
        let file = dir.path().join("rom_library.json");
        let roms = dir.path().join("roms");
        fs::create_dir_all(&roms).unwrap();
        let mut store = RomLibraryStore::load(&file).unwrap();
        store.set_rom_dir(Some(roms.clone())).unwrap();
        assert!(matches!(store.set_rom_dir(Some(dir.path().join("nope"))), Err(StoreError::NotAFolder(_))));
        write(&dir.path().join("afile"), b"x");
        assert!(matches!(store.set_rom_dir(Some(dir.path().join("afile"))), Err(StoreError::NotAFolder(_))));
        assert_eq!(store.rom_dir(), Some(roms.as_path()));
    }

    #[test]
    fn mappings_survive_a_restart_and_the_same_slot_is_replaced_not_duplicated() {
        let dir = temp_dir();
        let file = dir.path().join("rom_library.json");
        let mut store = RomLibraryStore::load(&file).unwrap();
        store.record(mapping("p1", "game")).unwrap();
        store.record(mapping("p1", "bios")).unwrap();
        store.record(mapping("p2", "game")).unwrap();
        let mut again = mapping("p1", "game");
        again.size = 99;
        store.record(again.clone()).unwrap();

        let reloaded = RomLibraryStore::load(&file).unwrap();
        assert_eq!(reloaded.mappings().len(), 3);
        assert_eq!(reloaded.mappings_for("p1").len(), 2);
        assert!(reloaded.mappings_for("p1").contains(&&again));
        assert!(reloaded.mappings_for("nobody").is_empty());
    }

    #[test]
    fn forgetting_removes_only_the_record() {
        let dir = temp_dir();
        let tmp_rom = dir.path().join("roms/a.z64");
        let dest = dir.path().join("lib/a.z64");
        write(&tmp_rom, b"rom");
        write(&dest, b"rom");
        let file = dir.path().join("rom_library.json");
        let mut store = RomLibraryStore::load(&file).unwrap();
        let mut m = mapping("p", "game");
        m.source = tmp_rom.clone();
        m.dest = Some(dest.clone());
        store.record(m).unwrap();
        assert!(store.forget("p", "game").unwrap());
        assert!(!store.forget("p", "game").unwrap());
        assert!(RomLibraryStore::load(&file).unwrap().mappings().is_empty());
        assert!(tmp_rom.exists() && dest.exists(), "files are never deleted");
    }

    #[test]
    fn a_damaged_file_is_an_error_and_is_not_overwritten_by_loading() {
        let dir = temp_dir();
        let file = dir.path().join("rom_library.json");
        fs::write(&file, "{ not json").unwrap();
        assert!(matches!(RomLibraryStore::load(&file), Err(StoreError::Unreadable(_))));
        fs::write(&file, "[1,2]").unwrap();
        assert!(matches!(RomLibraryStore::load(&file), Err(StoreError::Unreadable(_))));
        assert_eq!(fs::read_to_string(&file).unwrap(), "[1,2]");
    }

    #[test]
    fn unreadable_mapping_entries_are_skipped_not_fatal() {
        let dir = temp_dir();
        let file = dir.path().join("rom_library.json");
        let good = mapping("p", "game").to_json();
        fs::write(&file, json!({"rom_dir": "", "mappings": [good, {"port_key": 1}, "junk"]}).to_string()).unwrap();
        let store = RomLibraryStore::load(&file).unwrap();
        assert_eq!(store.mappings().len(), 1);
        assert!(store.rom_dir().is_none());
    }

    #[test]
    fn apply_and_record_saves_the_mapping_and_a_refused_copy_records_nothing() {
        let dir = temp_dir();
        let rom = dir.path().join("roms/Game.z64");
        write(&rom, &n64_rom(1, 1000));
        let port = dir.path().join("lib/Port");
        fs::create_dir_all(&port).unwrap();
        let file = dir.path().join("rom_library.json");
        let mut store = RomLibraryStore::load(&file).unwrap();
        let plan = plan_connect(&ConnectRequest {
            port_key: "p".into(),
            slot: "game".into(),
            source: rom.clone(),
            port_dir: port.clone(),
            dest_subdir: String::new(),
            dest_name: None,
            method: Method::Copy,
            access: Access::Copy,
            byte_order: None,
            known_crc32: None,
        })
        .unwrap();
        assert!(matches!(store.apply_and_record(&plan, Approval::None), Err(ApplyOrStoreError::Apply(ApplyError::NeedsConfirmation))));
        assert!(store.mappings().is_empty() && !file.exists());
        let m = store.apply_and_record(&plan, Approval::CopyAllowed).unwrap();
        assert_eq!(RomLibraryStore::load(&file).unwrap().mappings(), &[m]);
    }

    #[test]
    fn the_scan_cache_round_trips_and_a_bad_cache_is_just_ignored() {
        let dir = temp_dir();
        let roms = dir.path().join("roms");
        write(&roms.join("a.gba"), &random_bytes(1, 500));
        let store = RomLibraryStore::load(&dir.path().join("rom_library.json")).unwrap();
        assert!(store.load_scan_cache().is_none());
        let result = scan(&roms, &ScanOptions::default(), None, &AtomicBool::new(false), &mut |_| {}).unwrap();
        store.save_scan_cache(&result).unwrap();
        assert_eq!(store.load_scan_cache().unwrap().files, result.files);
        fs::write(store.scan_cache_path(), "garbage").unwrap();
        assert!(store.load_scan_cache().is_none());
    }
}
