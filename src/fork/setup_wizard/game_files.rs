//! What the wizard's game files page says after the folder was scanned: for the ports that are installed and name
//! a game file they need, how many have it ("Found game files for 3 of 4 ports") and which do not ("Still missing:
//! Zelda needs Ocarina of Time (USA).z64"). It is read from the same report as Settings > Game files, so the two
//! can never disagree; nothing is connected or copied here (that stays a choice in Settings > Game files).

use crate::fork::rom_library::{PortStatus, SlotResult, SlotStatus};
use crate::fork::rom_screens::report::{PortRow, Report};

/// How many missing ports are named on the page before "and N more".
pub const MAX_LISTED: usize = 3;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Results {
    /// Installed ports that need a game file.
    pub total: usize,
    /// Of those, the ones whose files were found in the folder (or are already connected).
    pub found: usize,
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

impl Results {
    pub fn from_report(report: &Report) -> Results {
        let mut out = Results::default();
        for row in report.groups.iter().flat_map(|g| g.ports.iter()).filter(|r| r.installed) {
            out.total += 1;
            if row.connected || matches!(row.status, PortStatus::Found | PortStatus::Ambiguous) {
                out.found += 1;
            } else {
                out.missing.push((row.port.display_name().to_string(), first_missing(row)));
            }
        }
        out.missing.sort();
        out
    }

    /// "Found game files for 3 of 4 ports."
    pub fn headline(&self) -> String {
        match (self.found, self.total) {
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
    use std::collections::{HashMap, HashSet};

    fn crc_text(data: &[u8]) -> String {
        format!("{:08X}", digests_of(data).crc32)
    }

    #[test]
    fn it_counts_installed_ports_that_have_their_files_and_names_the_rest() {
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
        let report = build_report(&ReportInput { catalog, installed, local_entries: HashMap::new(), mappings: vec![] }, &scan_dir(&roms));
        let r = Results::from_report(&report);
        assert_eq!((r.total, r.found), (3, 1), "{r:?}");
        let names: Vec<&str> = r.missing.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(names, ["Crash Port", "Kart Port"], "a port that needs nothing and one that is not installed are not counted");
        assert!(r.missing[0].1.contains("Crash Bandicoot"), "{:?}", r.missing);
        assert_eq!(r.headline(), "Found game files for 1 of 3 ports.");
        assert_eq!(r.missing_lines().len(), 2);
        assert!(r.missing_lines()[0].starts_with("Crash Port needs "));
    }

    #[test]
    fn the_headline_reads_for_every_case() {
        let r = |found, total| Results { found, total, missing: Vec::new() };
        assert_eq!(r(0, 0).headline(), "None of your ports need game files.");
        assert_eq!(r(0, 1).headline(), "No game files were found for your port.");
        assert_eq!(r(0, 3).headline(), "No game files were found for your 3 ports.");
        assert_eq!(r(1, 1).headline(), "Found game files for your port.");
        assert_eq!(r(4, 4).headline(), "Found game files for all 4 ports.");
        assert_eq!(r(3, 4).headline(), "Found game files for 3 of 4 ports.");
    }

    #[test]
    fn many_missing_ports_are_cut_with_a_count() {
        let missing: Vec<(String, String)> = (1..=5).map(|i| (format!("Port {i}"), format!("Game {i}"))).collect();
        let r = Results { found: 0, total: 5, missing };
        let lines = r.missing_lines();
        assert_eq!(lines, ["Port 1 needs Game 1.", "Port 2 needs Game 2.", "Port 3 needs Game 3.", "And 2 more."]);
    }
}
