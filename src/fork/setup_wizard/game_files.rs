//! What the wizard's game files page says after the folder was scanned: for the ports that are installed and name
//! a game file they need, how many have it ("Found game files for 3 of 4 ports") and which do not ("Still missing:
//! Zelda needs Ocarina of Time (USA).z64"). It is read from the same report as Settings > Game files, so the two
//! can never disagree. A file that is already inside the port's own folder counts as found (the way the mover reads
//! it back, by name), a file found in the game files folder is "found, not connected yet" until the user says "Connect
//! them now", which uses the same connect code and the same per-port rules as Settings > Game files (originals are
//! never moved or changed; a copy only after its size was shown and approved).

use crate::core::models::Port;
use crate::fork::rom_library::{PortStatus, SlotResult, SlotStatus};
use crate::fork::rom_screens::report::{PortRow, Report};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// How many missing ports are named on the page before "and N more".
pub const MAX_LISTED: usize = 3;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Results {
    /// Installed ports that need a game file.
    pub total: usize,
    /// Of those, the ones that already have it in their own folder.
    pub in_folder: usize,
    /// The ones whose files were found in the game files folder (or are already connected).
    pub found: usize,
    /// Of those found, how many "Connect them now" can connect.
    pub connectable: usize,
    /// The ones that were not found: port name and the game (or file) it needs.
    pub missing: Vec<(String, String)>,
}

fn needs(slot: &SlotResult) -> String {
    slot.alternatives.first().map(|a| a.display.clone()).filter(|d| !d.is_empty()).unwrap_or_else(|| slot.label.clone())
}

fn first_missing(row: &PortRow) -> String {
    row.matched
        .slots
        .iter()
        .find(|s| !s.optional && matches!(s.status, SlotStatus::Missing | SlotStatus::WrongVersion(_)))
        .map(needs)
        .unwrap_or_else(|| "a game file".to_string())
}

/// The ports (by key) whose game files are all already in their own folder or connected, or that already made their
/// game archive from them (`requirements::generated_archives`): nothing to look for, no scan needed. A port whose entry
/// names only a game title counts when a game file in its folder carries that title, or a connection was recorded.
/// The rule is `files_state::files_state`, the one Home uses too.
pub fn in_folder_keys<'a>(ports: impl Iterator<Item = (&'a Port, PathBuf)>, config_dir: &Path) -> HashSet<String> {
    ports.filter(|(port, dir)| super::files_state::files_state(port, dir, config_dir) == super::files_state::FilesState::Have).map(|(port, _)| port.key().to_string()).collect()
}

impl Results {
    /// `in_folder`: ports whose files are already in their folder; `connectable`: ports "Connect them now" can do.
    pub fn from_report(report: &Report, in_folder: &HashSet<String>, connectable: &HashSet<String>) -> Results {
        let mut out = Results::default();
        for row in report.groups.iter().flat_map(|g| g.ports.iter()).filter(|r| r.installed) {
            out.total += 1;
            if in_folder.contains(row.port.key()) {
                out.in_folder += 1;
            } else if row.connected || matches!(row.status, PortStatus::Found | PortStatus::Ambiguous) {
                out.found += 1;
                if connectable.contains(row.port.key()) {
                    out.connectable += 1;
                }
            } else {
                out.missing.push((row.port.display_name().to_string(), first_missing(row)));
            }
        }
        out.missing.sort();
        out
    }

    /// Ports that have their files, wherever they are.
    pub fn have(&self) -> usize {
        self.in_folder + self.found
    }

    /// "Found game files for 3 of 4 ports."
    pub fn headline(&self) -> String {
        match (self.have(), self.total) {
            (_, 0) => "None of your ports need game files.".to_string(),
            (0, t) => format!("No game files were found for your {}.", if t == 1 { "port".to_string() } else { format!("{t} ports") }),
            (f, t) if f == t => format!("Found game files for {}.", if t == 1 { "your port".to_string() } else { format!("all {t} ports") }),
            (f, t) => format!("Found game files for {f} of {t} ports."),
        }
    }

    /// One line per missing port ("Zelda needs Ocarina of Time (USA).z64"), the first few, then how many more.
    pub fn missing_lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = self.missing.iter().take(MAX_LISTED).map(|(port, game)| format!("{port} needs {game}.")).collect();
        if self.missing.len() > MAX_LISTED {
            lines.push(format!("And {} more.", self.missing.len() - MAX_LISTED));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::rom_library::testutil::*;
    use crate::fork::rom_screens::report::tests_support::{port, scan_dir};
    use crate::fork::rom_screens::report::{build_report, ReportInput};
    use std::collections::HashMap;

    fn crc_text(data: &[u8]) -> String {
        format!("{:08X}", digests_of(data).crc32)
    }

    #[test]
    fn a_port_that_already_made_its_game_archive_has_its_game_without_a_scan() {
        let tmp = temp_dir();
        let soh = port("Ship of Harkinian", "Shipwright", &["n64"], "Required files: Legend of Zelda, The - Ocarina of Time (USA).z64 (CRC32: 12345678)");
        let mm = port("2 Ship 2 Harkinian", "2ship2harkinian", &["n64"], "Required files: Legend of Zelda, The - Majora's Mask (USA).z64 (CRC32: 9ABCDEF0)");
        let (soh_dir, mm_dir) = (tmp.path().join("Shipwright"), tmp.path().join("2ship2harkinian"));
        write(&soh_dir.join("oot.o2r"), b"archive");
        write(&mm_dir.join("soh.appimage"), b"x");
        let keys = in_folder_keys([(&soh, soh_dir.clone()), (&mm, mm_dir.clone())].into_iter(), tmp.path());
        assert!(keys.contains(soh.key()), "oot.o2r is there");
        assert!(!keys.contains(mm.key()), "no mm.o2r, no ROM: still missing");
        write(&mm_dir.join("mm.o2r"), b"archive");
        assert!(in_folder_keys([(&mm, mm_dir)].into_iter(), tmp.path()).contains(mm.key()));
    }

    fn none() -> HashSet<String> {
        HashSet::new()
    }

    fn world() -> (tempfile::TempDir, Report, Vec<Port>) {
        let tmp = temp_dir();
        let roms = tmp.path().join("roms");
        let zelda = n64_rom(1, 20_000);
        write(&roms.join("n64/Zelda renamed.z64"), &zelda);
        let catalog = vec![
            port("Zelda Port", "ZeldaPort", &["n64"], &format!("Required files: Zelda (USA).z64 (CRC32: {})", crc_text(&zelda))),
            port("Kart Port", "KartPort", &["n64"], &format!("Required files: Mario Kart 64 (USA).z64 (CRC32: {})", crc_text(&n64_rom(99, 20_000)))),
            port("Crash Port", "CrashPort", &["ps1"], "Required files: Crash Bandicoot (USA).bin/cue (CRC32: DEADBEEF)"),
            port("Free Port", "FreePort", &["pc"], "No game files needed."),
            port("Not Installed", "NotInstalled", &["gba"], "Required files: Other (USA).gba (CRC32: 0A0B0C0D)"),
        ];
        let installed: HashSet<String> = ["ZeldaPort", "KartPort", "CrashPort", "FreePort"].iter().map(|s| s.to_string()).collect();
        let report = build_report(&ReportInput { catalog: catalog.clone(), installed, local_entries: HashMap::new(), mappings: vec![] }, &scan_dir(&roms));
        (tmp, report, catalog)
    }

    #[test]
    fn it_counts_installed_ports_that_have_their_files_and_names_the_rest() {
        let (_tmp, report, _) = world();
        let r = Results::from_report(&report, &none(), &["ZeldaPort".to_string()].into_iter().collect());
        assert_eq!((r.total, r.found, r.in_folder, r.connectable), (3, 1, 0, 1), "{r:?}");
        let names: Vec<&str> = r.missing.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(names, ["Crash Port", "Kart Port"], "a port that needs nothing and one that is not installed are not counted");
        assert!(r.missing[0].1.contains("Crash Bandicoot"), "{:?}", r.missing);
        assert_eq!(r.headline(), "Found game files for 1 of 3 ports.");
        assert_eq!(r.missing_lines().len(), 2);
        assert!(r.missing_lines()[0].starts_with("Crash Port needs "));
    }

    #[test]
    fn a_rom_already_inside_the_port_folder_counts_as_found_and_not_as_missing() {
        // Tim's case: the port's own folder holds its game file, the game files folder does not.
        let (tmp, report, catalog) = world();
        let lib = tmp.path().join("lib");
        write(&lib.join("KartPort/Mario Kart 64 (USA).z64"), &n64_rom(7, 20_000));
        write(&lib.join("CrashPort/readme.txt"), b"nothing");
        let installed: Vec<(&Port, PathBuf)> = catalog.iter().filter(|p| ["KartPort", "CrashPort", "ZeldaPort"].contains(&p.folder.as_str())).map(|p| (p, lib.join(&p.folder))).collect();
        let keys = in_folder_keys(installed.into_iter(), tmp.path());
        assert_eq!(keys.iter().map(String::as_str).collect::<Vec<_>>(), ["KartPort"], "only the port whose required file is in its folder");
        let r = Results::from_report(&report, &keys, &none());
        assert_eq!((r.total, r.in_folder, r.found), (3, 1, 1));
        assert_eq!(r.have(), 2);
        assert_eq!(r.headline(), "Found game files for 2 of 3 ports.");
        assert_eq!(r.missing.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), ["Crash Port"], "Kart Port is not listed as missing");
    }

    #[test]
    fn the_headline_reads_for_every_case() {
        let r = |found, total| Results { found, total, ..Results::default() };
        assert_eq!(r(0, 0).headline(), "None of your ports need game files.");
        assert_eq!(r(0, 1).headline(), "No game files were found for your port.");
        assert_eq!(r(0, 3).headline(), "No game files were found for your 3 ports.");
        assert_eq!(r(1, 1).headline(), "Found game files for your port.");
        assert_eq!(r(4, 4).headline(), "Found game files for all 4 ports.");
        assert_eq!(r(3, 4).headline(), "Found game files for 3 of 4 ports.");
        let mixed = Results { total: 4, in_folder: 2, found: 1, ..Results::default() };
        assert_eq!(mixed.headline(), "Found game files for 3 of 4 ports.");
    }

    #[test]
    fn many_missing_ports_are_cut_with_a_count() {
        let missing: Vec<(String, String)> = (1..=5).map(|i| (format!("Port {i}"), format!("Game {i}"))).collect();
        let r = Results { total: 5, missing, ..Results::default() };
        let lines = r.missing_lines();
        assert_eq!(lines, ["Port 1 needs Game 1.", "Port 2 needs Game 2.", "Port 3 needs Game 3.", "And 2 more."]);
    }
}
