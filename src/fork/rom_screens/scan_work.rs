//! The slow half of the game-files screens, written so it can run on a worker thread: scan the folder
//! (with the saved scan so unchanged files are not read again) and turn the result into a report.

use super::job::Ctl;
use super::report::{build_report, Report, ReportInput};
use super::util::{file_name, human_bytes};
use crate::fork::rom_library::{scan_folder, RomLibraryStore, ScanOptions, ScanResult};
use std::path::Path;

pub struct ScanOutput {
    pub report: Report,
    pub scan: ScanResult,
    /// The scan worked but could not be saved, so a restart will scan again.
    pub cache_error: Option<String>,
}

/// Scan `root`. `Ok(None)` means the user cancelled; nothing is saved then, so a half scan never
/// stands in for a whole one.
pub fn run_scan(ctl: &Ctl, store: &RomLibraryStore, root: &Path, input: &ReportInput) -> Result<Option<ScanOutput>, String> {
    let prior = store.load_scan_cache().filter(|p| p.root == root);
    let scan = scan_folder(root, &ScanOptions::default(), prior.as_ref(), ctl.flag(), &mut |p| {
        ctl.set_progress(format!("Checked {} files, {} read\n{}", p.files_done, human_bytes(p.bytes_hashed), file_name(p.current)));
    })
    .map_err(|e| format!("Could not read {}: {e}", root.display()))?;
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
            move |ctl| run_scan(ctl, &store, &roms, &input)
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
                run_scan(ctl, &store, &roms, &input(b"x"))
            }
        });
        assert!(wait(&job).unwrap().is_none());
        assert!(!store.scan_cache_path().exists(), "a cancelled scan leaves no cache");
    }
}
