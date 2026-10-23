//! "What's in your game folders": the scan and the catalog turned into rows grouped by system.
//!
//! Pure data and text, no UI and no disk writes, so the decisions (which ports show, what each result
//! says, what counts as connected) are tested without a window. The app draws the lines it returns.

use super::util::{ellipsize, file_name};
use crate::core::models::Port;
use crate::fork::rom_library::{
    check_mapping, load_local_entries, match_all, Health, Mapping, PortMatch, PortStatus, RomLibraryStore, RomSpec, ScanResult, SlotKind, SlotResult,
    SlotStatus, System,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Everything the report needs that lives in the app, copied so the work can run on a thread.
#[derive(Clone)]
pub struct ReportInput {
    pub catalog: Vec<Port>,
    /// `Port::key()` of every port installed in the library.
    pub installed: HashSet<String>,
    pub local_entries: HashMap<String, Value>,
    pub mappings: Vec<Mapping>,
}

impl ReportInput {
    pub fn new(catalog: Vec<Port>, installed: HashSet<String>, ports_local_json: &Path, mappings: Vec<Mapping>) -> ReportInput {
        ReportInput { catalog, installed, local_entries: load_local_entries(ports_local_json), mappings }
    }
}

#[derive(Debug, Clone)]
pub struct PortRow {
    pub port: Port,
    pub installed: bool,
    pub status: PortStatus,
    pub system: System,
    pub spec: RomSpec,
    pub matched: PortMatch,
    /// Every required file has a recorded connection that still checks out on disk.
    pub connected: bool,
    /// A file was matched by name alone, so the user should look at it.
    pub low_confidence: bool,
}

impl PortRow {
    /// The short result shown beside the port name.
    pub fn chip(&self) -> String {
        let mut chip = if self.connected {
            "Connected".to_string()
        } else {
            match self.status {
                PortStatus::Found => "Found".into(),
                PortStatus::Ambiguous => "Several matches, pick one".into(),
                PortStatus::Partial => "Partly found".into(),
                PortStatus::WrongVersion => "Wrong version".into(),
                PortStatus::Missing => "Missing".into(),
                PortStatus::Unverifiable => "Cannot be checked".into(),
                PortStatus::Unspecified => "No files needed".into(),
            }
        };
        if !self.connected && self.low_confidence && matches!(self.status, PortStatus::Found | PortStatus::Ambiguous | PortStatus::Partial) {
            chip.push_str(" (check this)");
        }
        chip
    }

    /// Can the Connect step do anything for this port: it is installed and at least one required
    /// file was found.
    pub fn can_connect(&self) -> bool {
        self.installed
            && self.matched.slots.iter().any(|s| matches!(s.status, SlotStatus::Found(_) | SlotStatus::Ambiguous(_)))
    }

    /// One line per required file for the port's detail screen.
    pub fn detail_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for slot in &self.matched.slots {
            lines.push(slot_line(slot));
        }
        lines
    }
}

fn expected(slot: &SlotResult) -> String {
    slot.alternatives.first().map(|a| a.display.clone()).filter(|d| !d.is_empty()).unwrap_or_else(|| slot.label.clone())
}

fn short_name(path: &Path) -> String {
    ellipsize(&file_name(path), 36)
}

fn slot_line(slot: &SlotResult) -> String {
    let optional = if slot.optional { " (optional)" } else { "" };
    let bios = if slot.kind == SlotKind::Bios { "BIOS " } else { "" };
    let text = match &slot.status {
        SlotStatus::Found(f) => {
            let how = if f.is_low_confidence() { "matched by file name only, check this" } else { "checked by checksum" };
            let order = if f.needs_byte_order_conversion() { ", N64 byte order differs" } else { "" };
            format!("{bios}Found: {} ({how}{order})", short_name(&f.path))
        }
        SlotStatus::Ambiguous(c) => {
            let names: Vec<String> = c.iter().take(3).map(|f| short_name(&f.path)).collect();
            let more = if c.len() > 3 { format!(" and {} more", c.len() - 3) } else { String::new() };
            format!("{bios}{} files match, pick one: {}{more}", c.len(), names.join(", "))
        }
        SlotStatus::WrongVersion(c) => {
            let names: Vec<String> = c.iter().take(2).map(|f| short_name(&f.path)).collect();
            format!("{bios}Wrong version: you have {}, this port wants {}", names.join(", "), ellipsize(&expected(slot), 40))
        }
        SlotStatus::Missing => format!("{bios}Missing: {}", ellipsize(&expected(slot), 40)),
    };
    ellipsize(&format!("{text}{optional}"), 120)
}

#[derive(Debug, Clone)]
pub struct SystemGroup {
    pub system: System,
    pub files: usize,
    pub ports: Vec<PortRow>,
}

impl SystemGroup {
    /// "12 files; 2 found, 1 missing", or "12 files; no port uses them".
    pub fn summary(&self) -> String {
        let files = if self.files == 1 { "1 file".to_string() } else { format!("{} files", self.files) };
        if self.ports.is_empty() {
            return format!("{files}, no port here needs them");
        }
        let count = |f: &dyn Fn(&PortRow) -> bool| self.ports.iter().filter(|r| f(r)).count();
        let mut parts = Vec::new();
        let connected = count(&|r| r.connected);
        if connected > 0 {
            parts.push(format!("{connected} connected"));
        }
        let found = count(&|r| !r.connected && r.status == PortStatus::Found);
        if found > 0 {
            parts.push(format!("{found} found"));
        }
        let ambiguous = count(&|r| !r.connected && r.status == PortStatus::Ambiguous);
        if ambiguous > 0 {
            parts.push(format!("{ambiguous} to pick"));
        }
        let problem = count(&|r| !r.connected && matches!(r.status, PortStatus::Partial | PortStatus::WrongVersion));
        if problem > 0 {
            parts.push(format!("{problem} wrong or partial"));
        }
        let missing = count(&|r| !r.connected && matches!(r.status, PortStatus::Missing | PortStatus::Unverifiable));
        if missing > 0 {
            parts.push(format!("{missing} missing"));
        }
        format!("{files}; {}", parts.join(", "))
    }
}

#[derive(Debug, Clone)]
pub struct Report {
    pub root: PathBuf,
    pub total_files: usize,
    pub ignored_files: usize,
    /// Folders that could not be read or were skipped.
    pub problems: usize,
    /// Files no port in the list uses.
    pub unused_files: usize,
    /// Something the user should know next to the results (the scan could not be saved).
    pub note: Option<String>,
    pub groups: Vec<SystemGroup>,
}

fn system_from_tags(port: &Port) -> Option<System> {
    use crate::fork::rom_library::system::system_from_folder_name;
    port.tags_lower.iter().find_map(|t| if t == "360" { Some(System::Xbox360) } else { system_from_folder_name(t) })
}

/// A port with nothing to say about its files is left out; a port that is not installed is only
/// listed when something real was found for it (so hundreds of uninstalled catalog entries do not
/// bury the result).
pub fn row_visible(status: PortStatus, installed: bool) -> bool {
    match status {
        PortStatus::Unspecified => false,
        PortStatus::Found | PortStatus::Ambiguous | PortStatus::Partial | PortStatus::WrongVersion => true,
        PortStatus::Missing | PortStatus::Unverifiable => installed,
    }
}

pub fn all_slots_connected(matched: &PortMatch, mappings: &[Mapping]) -> bool {
    let required: Vec<&SlotResult> = matched.slots.iter().filter(|s| !s.optional).collect();
    !required.is_empty()
        && required.iter().all(|s| {
            let ours: Vec<&Mapping> = mappings.iter().filter(|m| m.port_key == matched.port_key && m.slot == s.slot_id).collect();
            !ours.is_empty() && ours.iter().all(|m| check_mapping(m) == Health::Ok)
        })
}

pub fn build_report(input: &ReportInput, scan: &ScanResult) -> Report {
    let matches = match_all(&input.catalog, &input.local_entries, &scan.files);
    let system_of: HashMap<&Path, System> = scan.files.iter().map(|f| (f.path.as_path(), f.system)).collect();
    let mut used: HashSet<PathBuf> = HashSet::new();
    let mut by_system: HashMap<System, Vec<PortRow>> = HashMap::new();

    for (port, matched) in input.catalog.iter().zip(matches) {
        if port.is_android_only() {
            continue;
        }
        let installed = input.installed.contains(port.key());
        if !row_visible(matched.status, installed) {
            continue;
        }
        let mut found_system = None;
        let mut low_confidence = false;
        for slot in &matched.slots {
            let candidates = match &slot.status {
                SlotStatus::Found(f) => std::slice::from_ref(f),
                SlotStatus::Ambiguous(c) => c.as_slice(),
                _ => continue,
            };
            for f in candidates {
                low_confidence |= f.is_low_confidence();
                for p in f.all_paths().into_iter().chain(f.also_at.iter().cloned()) {
                    used.insert(p);
                }
                if slot.kind == SlotKind::Game && found_system.is_none() {
                    found_system = system_of.get(f.path.as_path()).copied().filter(|s| *s != System::Unknown);
                }
            }
        }
        let system = found_system.or_else(|| system_from_tags(port)).unwrap_or(System::Unknown);
        let spec = RomSpec::for_port(port, input.local_entries.get(&port.folder));
        let connected = all_slots_connected(&matched, &input.mappings);
        by_system.entry(system).or_default().push(PortRow { port: port.clone(), installed, status: matched.status, system, spec, matched, connected, low_confidence });
    }

    let counts: HashMap<System, usize> = scan.counts_by_system().into_iter().collect();
    let mut systems: Vec<System> = by_system.keys().copied().chain(counts.keys().copied()).collect();
    systems.sort();
    systems.dedup();
    let groups = systems
        .into_iter()
        .map(|system| {
            let mut ports = by_system.remove(&system).unwrap_or_default();
            ports.sort_by_key(|r| (!r.installed, r.port.name.to_lowercase()));
            SystemGroup { system, files: counts.get(&system).copied().unwrap_or(0), ports }
        })
        .collect();
    Report {
        root: scan.root.clone(),
        total_files: scan.files.len(),
        ignored_files: scan.ignored_files,
        problems: scan.issues.len(),
        unused_files: scan.files.iter().filter(|f| !used.contains(&f.path)).count(),
        note: None,
        groups,
    }
}

/// When the saved scan was written, for "last scanned".
pub fn scan_time(store: &RomLibraryStore) -> Option<SystemTime> {
    // Only a scan of the folder that is set now counts, not a leftover from an earlier choice.
    store.load_scan_cache().filter(|c| Some(c.root.as_path()) == store.rom_dir())?;
    std::fs::metadata(store.scan_cache_path()).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use crate::fork::rom_library::{scan_folder, ScanOptions};
    use std::sync::atomic::AtomicBool;

    pub(crate) fn port(name: &str, folder: &str, tags: &[&str], instructions: &str) -> Port {
        let tags: Vec<Value> = tags.iter().map(|t| Value::from(*t)).collect();
        crate::core::models::port_from_value(&serde_json::json!({"name": name, "folder": folder, "tags": tags, "instructions": instructions, "source": "https://example.com/x.zip"}))
            .unwrap()
    }

    pub(crate) fn scan_dir(root: &Path) -> ScanResult {
        scan_folder(root, &ScanOptions::default(), None, &AtomicBool::new(false), &mut |_| {}).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::*;
    use super::*;
    use crate::fork::rom_library::testutil::*;
    use crate::fork::rom_library::{Access, Approval, ConnectRequest, Method};

    fn crc_text(data: &[u8]) -> String {
        format!("{:08X}", digests_of(data).crc32)
    }

    struct World {
        _tmp: tempfile::TempDir,
        roms: PathBuf,
        lib: PathBuf,
        zelda: Vec<u8>,
    }

    fn world() -> World {
        let tmp = temp_dir();
        let roms = tmp.path().join("roms");
        let lib = tmp.path().join("lib");
        let zelda = n64_rom(1, 20_000);
        write(&roms.join("n64/Zelda renamed.z64"), &zelda);
        write(&roms.join("n64/Mario Kart 64 (USA).z64"), &n64_rom(2, 20_000));
        write(&roms.join("snes/Unused (USA).sfc"), &random_bytes(3, 4_000));
        fs_create(&lib.join("ZeldaPort"));
        World { _tmp: tmp, roms, lib, zelda }
    }

    fn fs_create(p: &Path) {
        std::fs::create_dir_all(p).unwrap();
    }

    fn input(w: &World, installed: &[&str], mappings: Vec<Mapping>) -> ReportInput {
        let catalog = vec![
            port("Zelda Port", "ZeldaPort", &["n64"], &format!("Required files: Zelda (USA).z64 (CRC32: {})", crc_text(&w.zelda))),
            port("Kart Port", "KartPort", &["n64"], &format!("Required files: Mario Kart 64 (USA).z64 (CRC32: {})", crc_text(&n64_rom(99, 20_000)))),
            port("Missing Port", "MissingPort", &["ps1"], "Required files: Some Game (USA).bin/cue (CRC32: DEADBEEF)"),
            port("Free Port", "FreePort", &["pc"], "No game files needed."),
            port("Hidden Missing", "HiddenMissing", &["gba"], "Required files: Other (USA).gba (CRC32: 0A0B0C0D)"),
        ];
        ReportInput { catalog, installed: installed.iter().map(|s| s.to_string()).collect(), local_entries: HashMap::new(), mappings }
    }

    #[test]
    fn rows_group_by_the_system_of_the_matched_file_and_hide_noise() {
        let w = world();
        let report = build_report(&input(&w, &["ZeldaPort", "MissingPort"], vec![]), &scan_dir(&w.roms));
        let n64 = report.groups.iter().find(|g| g.system == System::N64).unwrap();
        let names: Vec<&str> = n64.ports.iter().map(|r| r.port.name.as_str()).collect();
        assert!(names.contains(&"Zelda Port"), "{names:?}");
        let zelda = n64.ports.iter().find(|r| r.port.name == "Zelda Port").unwrap();
        assert_eq!(zelda.status, PortStatus::Found);
        assert_eq!(zelda.chip(), "Found");
        let kart = n64.ports.iter().find(|r| r.port.name == "Kart Port").unwrap();
        assert_eq!(kart.chip(), "Wrong version", "the wrong dump is shown, not hidden, even though Kart Port is not installed");
        assert!(!kart.installed);
        // An installed port that is missing its files shows up, under the system its tags name.
        let ps1 = report.groups.iter().find(|g| g.system == System::Ps1).unwrap();
        assert_eq!(ps1.ports[0].chip(), "Missing");
        // Not installed and nothing found: left out. A port with no requirements: left out.
        let all: Vec<&str> = report.groups.iter().flat_map(|g| g.ports.iter().map(|r| r.port.name.as_str())).collect();
        assert!(!all.contains(&"Hidden Missing") && !all.contains(&"Free Port"));
        // The SNES file has no port: its group exists with a note.
        let snes = report.groups.iter().find(|g| g.system == System::Snes).unwrap();
        assert!(snes.ports.is_empty() && snes.summary().contains("no port here needs them"));
        assert_eq!(report.total_files, 3);
        assert_eq!(report.unused_files, 1 + 1, "the wrong-version Kart dump is not used by any port; the SNES file is unused");
    }

    #[test]
    fn summaries_count_each_outcome() {
        let w = world();
        let report = build_report(&input(&w, &["ZeldaPort"], vec![]), &scan_dir(&w.roms));
        let n64 = report.groups.iter().find(|g| g.system == System::N64).unwrap();
        assert_eq!(n64.summary(), "2 files; 1 found, 1 wrong or partial");
    }

    #[cfg(unix)]
    #[test]
    fn a_healthy_recorded_connection_reads_connected_and_a_broken_one_does_not() {
        let w = world();
        let scan = scan_dir(&w.roms);
        let first = build_report(&input(&w, &["ZeldaPort"], vec![]), &scan);
        let row = first.groups.iter().flat_map(|g| &g.ports).find(|r| r.port.name == "Zelda Port").unwrap().clone();
        let slot = &row.matched.slots[0];
        let SlotStatus::Found(found) = &slot.status else { panic!() };
        let plan = crate::fork::rom_library::plan_connect(&ConnectRequest {
            port_key: "ZeldaPort".into(),
            slot: slot.slot_id.clone(),
            source: found.path.clone(),
            port_dir: w.lib.join("ZeldaPort"),
            dest_subdir: String::new(),
            dest_name: None,
            method: Method::Symlink,
            access: Access::Link,
            byte_order: None,
            known_crc32: Some(found.crc32),
        })
        .unwrap();
        let mapping = crate::fork::rom_library::apply(&plan, Approval::None).unwrap();
        let connected = build_report(&input(&w, &["ZeldaPort"], vec![mapping.clone()]), &scan);
        let row = connected.groups.iter().flat_map(|g| &g.ports).find(|r| r.port.name == "Zelda Port").unwrap();
        assert!(row.connected && row.chip() == "Connected");
        // The original disappears (drive unplugged): no longer "connected".
        std::fs::remove_file(&found.path).unwrap();
        let broken = build_report(&input(&w, &["ZeldaPort"], vec![mapping]), &scan);
        assert!(!broken.groups.iter().flat_map(|g| &g.ports).find(|r| r.port.name == "Zelda Port").unwrap().connected);
    }

    #[test]
    fn a_name_only_match_is_flagged_check_this() {
        let tmp = temp_dir();
        write(&tmp.path().join("n64/Real Name (USA).z64"), &n64_rom(5, 9_000));
        let catalog = vec![port("Named", "Named", &["n64"], "Required files: Real Name (USA).z64")];
        let input = ReportInput { catalog, installed: HashSet::from(["Named".to_string()]), local_entries: HashMap::new(), mappings: vec![] };
        let report = build_report(&input, &scan_dir(tmp.path()));
        let row = &report.groups.iter().find(|g| g.system == System::N64).unwrap().ports[0];
        assert_eq!(row.chip(), "Found (check this)");
        assert!(row.detail_lines()[0].contains("file name only"));
    }

    #[test]
    fn detail_lines_say_what_was_found_and_what_was_wanted() {
        let w = world();
        let report = build_report(&input(&w, &["ZeldaPort", "MissingPort"], vec![]), &scan_dir(&w.roms));
        let rows: Vec<&PortRow> = report.groups.iter().flat_map(|g| &g.ports).collect();
        let zelda = rows.iter().find(|r| r.port.name == "Zelda Port").unwrap();
        assert!(zelda.detail_lines()[0].starts_with("Found: Zelda renamed.z64 (checked by checksum"));
        let kart = rows.iter().find(|r| r.port.name == "Kart Port").unwrap();
        assert!(kart.detail_lines()[0].starts_with("Wrong version: you have Mario Kart 64 (USA).z64"), "{:?}", kart.detail_lines());
        let missing = rows.iter().find(|r| r.port.name == "Missing Port").unwrap();
        assert!(missing.detail_lines()[0].starts_with("Missing:"));
        assert!(zelda.can_connect() && !kart.can_connect() && !missing.can_connect());
    }

    #[test]
    fn visibility_rules() {
        assert!(!row_visible(PortStatus::Unspecified, true));
        assert!(row_visible(PortStatus::Found, false));
        assert!(!row_visible(PortStatus::Missing, false));
        assert!(row_visible(PortStatus::Missing, true));
        assert!(!row_visible(PortStatus::Unverifiable, false));
    }
}
