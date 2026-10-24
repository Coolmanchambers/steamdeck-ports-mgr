//! The slow half of the game-files screens, written so it can run on a worker thread: scan the folder
//! (with the saved scan so unchanged files are not read again) and turn the result into a report.

use super::job::Ctl;
use super::report::{build_report, Report, ReportInput};
use super::util::{file_name, human_bytes};
use crate::fork::rom_library::scan::{Progress, Wanted};
use crate::fork::rom_library::{scan_folder, RomLibraryStore, RomSpec, ScanOptions, ScanResult};
use std::path::Path;
use std::time::Duration;

/// Three lines: "Looking at 3 of 12 files: 1.2 GB of 4.0 GB, about 2 min left", the file being read, and how many
/// other files are not needed. `elapsed` is the time since the scan started; the estimate appears once enough has
/// been read to judge the speed. (Lists that show two lines show the first two.)
pub fn progress_text(p: &Progress<'_>, elapsed: Duration) -> String {
    let n = (p.files_read + 1).min(p.to_read.max(1));
    let mut line = format!("Looking at {n} of {} {}: {} of {}", p.to_read, if p.to_read == 1 { "file" } else { "files" }, human_bytes(p.bytes_hashed), human_bytes(p.bytes_to_read));
    let secs = elapsed.as_secs_f64();
    if secs >= 3.0 && p.bytes_hashed > 0 && p.bytes_to_read > p.bytes_hashed {
        let left = (p.bytes_to_read - p.bytes_hashed) as f64 / (p.bytes_hashed as f64 / secs);
        line.push_str(&if left < 60.0 { ", under a minute left".to_string() } else { format!(", about {} min left", (left / 60.0).ceil() as u64) });
    }
    let others = p.seen.saturating_sub(p.to_read);
    let rest = if others > 0 { format!("\n{others} other game {} not needed, so not read.", if others == 1 { "file is" } else { "files are" }) } else { String::new() };
    format!("{line}\n{}{rest}", file_name(p.current))
}

/// What the installed ports of `input` need: the files worth reading.
pub fn wanted_for(input: &ReportInput) -> Wanted {
    let specs: Vec<RomSpec> = input.catalog.iter().filter(|p| input.installed.contains(p.key())).map(|p| RomSpec::for_port(p, input.local_entries.get(&p.folder))).collect();
    Wanted::from_specs(specs.iter())
}

pub struct ScanOutput {
    pub report: Report,
    pub scan: ScanResult,
    /// The scan worked but could not be saved, so a restart will scan again.
    pub cache_error: Option<String>,
}

/// Scan `root`. `Ok(None)` means the user cancelled; nothing is saved then, so a half scan never
/// stands in for a whole one. `only_wanted`: read only the files the installed ports could need (the setup
/// wizard); otherwise every game file (Settings > Game files, whose lists show everything).
pub fn run_scan(ctl: &Ctl, store: &RomLibraryStore, root: &Path, input: &ReportInput, only_wanted: bool) -> Result<Option<ScanOutput>, String> {
    let prior = store.load_scan_cache().filter(|p| p.root == root);
    let options = ScanOptions { wanted: only_wanted.then(|| wanted_for(input)), ..ScanOptions::default() };
    let started = std::time::Instant::now();
    let scan = scan_folder(root, &options, prior.as_ref(), ctl.flag(), &mut |p| {
        ctl.set_progress(progress_text(&p, started.elapsed()));
    })
    .map_err(|e| format!("Could not read {}: {e}\n\nIs the drive or card plugged in? Plug it in, or choose another folder in Settings > Game files.", root.display()))?;
    if scan.cancelled || ctl.cancelled() {
        return Ok(None);
    }
    let cache_error = store.save_scan_cache(&scan).err().map(|e| e.to_string());
    ctl.set_progress("Matching files to ports".into());
    let report = build_report(input, &scan);
    Ok(Some(ScanOutput { report, scan, cache_error }))
}

/// The report for the saved scan, without touching the game folder. `Err` when there is no saved scan
/// for this folder yet.
pub fn run_report(store: &RomLibraryStore, root: &Path, input: &ReportInput) -> Result<ScanOutput, String> {
    let scan = store.load_scan_cache().filter(|p| p.root == root).ok_or_else(|| "There is no saved scan of this folder yet. Scan it first.".to_string())?;
    let report = build_report(input, &scan);
    Ok(ScanOutput { report, scan, cache_error: None })
}

#[cfg(test)]
mod progress_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_progress_says_how_far_how_many_are_skipped_and_how_long_is_left() {
        let p = Progress { current: Path::new("/r/n64/Zelda.z64"), files_done: 0, bytes_hashed: 1_000_000_000, to_read: 3, files_read: 1, bytes_to_read: 4_000_000_000, seen: 63 };
        let t = progress_text(&p, Duration::from_secs(10));
        assert_eq!(t, "Looking at 2 of 3 files: 1.0 GB of 4.0 GB, under a minute left\nZelda.z64\n60 other game files are not needed, so not read.");
        let slow = progress_text(&p, Duration::from_secs(100));
        assert!(slow.starts_with("Looking at 2 of 3 files: 1.0 GB of 4.0 GB, about 5 min left"), "{slow}");
        let early = progress_text(&Progress { seen: 1, to_read: 1, files_read: 0, ..p }, Duration::from_secs(1));
        assert_eq!(early, "Looking at 1 of 1 file: 1.0 GB of 4.0 GB\nZelda.z64");
    }
}

#[cfg(test)]
mod tests {
    use super::super::job::Background;
    use super::super::report::tests_support::*;
    use super::*;
    use crate::fork::rom_library::testutil::*;
    use std::time::{Duration, Instant};

    fn input(zelda: &[u8]) -> ReportInput {
        let catalog = vec![port("Zelda Port", "ZeldaPort", &["n64"], &format!("Required files: Zelda (USA).z64 (CRC32: {:08X})", digests_of(zelda).crc32))];
        ReportInput { catalog, installed: Default::default(), local_entries: Default::default(), mappings: vec![] }
    }

    fn wait<T: Send + 'static>(job: &Background<T>) -> T {
        let start = Instant::now();
        loop {
            if let Some(r) = job.take() {
                return r.unwrap();
            }
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_scan_saves_its_result_and_a_later_report_reads_it_without_the_folder() {
        let tmp = temp_dir();
        let roms = tmp.path().join("roms");
        let zelda = n64_rom(1, 30_000);
        write(&roms.join("n64/z.z64"), &zelda);
        let store = RomLibraryStore::load(&tmp.path().join("rom_library.json")).unwrap();
        let job = Background::spawn({
            let (store, roms, input) = (store.clone(), roms.clone(), input(&zelda));
            move |ctl| run_scan(ctl, &store, &roms, &input, false)
        });
        let out = wait(&job).unwrap().unwrap();
        assert_eq!(out.report.total_files, 1);
        assert!(out.cache_error.is_none());
        // "Restart": the folder is gone, the saved scan is not.
        std::fs::remove_dir_all(&roms).unwrap();
        let again = run_report(&RomLibraryStore::load(&tmp.path().join("rom_library.json")).unwrap(), &roms, &input(&zelda)).unwrap();
        assert_eq!(again.report.total_files, 1);
        assert_eq!(again.report.groups.iter().map(|g| g.ports.len()).sum::<usize>(), 1);
    }

    #[test]
    fn a_report_for_a_different_folder_or_no_scan_is_an_error() {
        let tmp = temp_dir();
        let store = RomLibraryStore::load(&tmp.path().join("rom_library.json")).unwrap();
        assert!(run_report(&store, tmp.path(), &input(b"x")).err().unwrap().contains("no saved scan"));
    }

    #[test]
    fn cancelling_stops_the_scan_and_saves_nothing() {
        let tmp = temp_dir();
        let roms = tmp.path().join("roms");
        for i in 0..20 {
            write(&roms.join(format!("n64/g{i}.z64")), &n64_rom(i, 50_000));
        }
        let store = RomLibraryStore::load(&tmp.path().join("rom_library.json")).unwrap();
        let job = Background::spawn({
            let (store, roms) = (store.clone(), roms.clone());
            move |ctl| {
                ctl.flag().store(true, std::sync::atomic::Ordering::Relaxed);
                run_scan(ctl, &store, &roms, &input(b"x"), false)
            }
        });
        assert!(wait(&job).unwrap().is_none());
        assert!(!store.scan_cache_path().exists(), "a cancelled scan leaves no cache");
    }
}
