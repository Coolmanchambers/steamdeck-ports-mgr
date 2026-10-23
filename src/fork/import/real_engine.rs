//! The engine Import ports uses: the port mover (`fork::port_mover`) behind the `MoveEngine` door.
//!
//! For each port, on the worker thread: `prepare` (checks, copy into a hidden staging folder, verify,
//! game file check before and after, rename into place), then `switch_over` (Steam shortcuts repointed
//! with their app IDs kept, or queued while Steam runs; Game files records follow; the old folder is
//! recorded as an old copy). Nothing is ever deleted: the old folder stays, and the screen asks about
//! each one afterwards (`ports_folder_screen::prompt_old_copies`, Keep for now preselected).
//!
//! A game file problem found before the copy is put to the user through `Prompter` (Fix in Game files /
//! Move anyway / Cancel). Problems found after the copy always stop that port (the copy is removed).

use super::engine::{GameFileChoice, MoveContext, MoveEngine, PortOutcome, SteamResult};
use super::plan::MoveRequest;
use crate::core::models::Port;
use crate::fork::port_mover::game_files::{required_not_found, GameFiles};
use crate::fork::port_mover::{prepare, switch_over, MoveEnv, MoveError, MoveOptions, PortRef, Stage};
use crate::fork::rom_screens::util::human_bytes;
use crate::fork::settings::{ForkSettings, FILE_NAME};
use crate::fork::store::{SteamProcessCheck, SystemSteamCheck};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct RealEngine {
    pub home: PathBuf,
    /// The app folder (`steam-queue.json`, `port-moves.json`, `rom_library.json`, `fork-settings.json`).
    pub config_dir: PathBuf,
    pub catalog: Vec<Port>,
    pub game_mode: bool,
    pub steam: Arc<dyn SteamProcessCheck + Send + Sync>,
    pub running: fn(&Path) -> Vec<String>,
    pub free_space: fn(&Path) -> Option<u64>,
}

fn running_in(dir: &Path) -> Vec<String> {
    crate::fork::save_restore::running::processes_under(&[dir.to_path_buf()]).iter().map(|f| f.describe()).collect()
}

impl RealEngine {
    /// The engine for the app as it runs: the real `/proc` checks and `df`.
    pub fn system(home: PathBuf, config_dir: PathBuf, catalog: Vec<Port>, game_mode: bool) -> RealEngine {
        RealEngine { home, config_dir, catalog, game_mode, steam: Arc::new(SystemSteamCheck::new()), running: running_in, free_space: crate::fork::rom_screens::util::free_bytes }
    }
}

/// The engine the Import screen uses. Switching engines is this function.
pub fn engine_for_import(home: PathBuf, config_dir: PathBuf, catalog: Vec<Port>, game_mode: bool) -> Box<dyn MoveEngine> {
    Box::new(RealEngine::system(home, config_dir, catalog, game_mode))
}

fn refused(error: &MoveError, space: bool) -> PortOutcome {
    PortOutcome::Refused { reason: error.to_string(), space }
}

fn steam_result(report: &crate::fork::port_mover::steam::SteamReport, request: &MoveRequest) -> SteamResult {
    if !report.failed.is_empty() {
        return SteamResult::Failed(report.failed.join("; "));
    }
    if !report.queued.is_empty() {
        return SteamResult::Queued(request.steam.len().max(1));
    }
    match report.changed_count() {
        0 => SteamResult::NoShortcut,
        n => SteamResult::Repointed(n),
    }
}

impl MoveEngine for RealEngine {
    fn is_dry_run(&self) -> bool {
        false
    }

    fn move_port(&self, ctx: &MoveContext) -> PortOutcome {
        let request = &ctx.plan.request;
        if let Some(block) = &ctx.plan.block {
            return PortOutcome::Refused { reason: block.text(Path::new("")), space: block.is_space() };
        }
        let port = self.catalog.iter().find(|p| p.key() == request.port_key);
        let game_files = port.map(|p| GameFiles::for_port(p, &self.config_dir)).unwrap_or_default();
        let settings = ForkSettings::load(&self.config_dir.join(FILE_NAME));
        let env = MoveEnv {
            home: &self.home,
            config_dir: &self.config_dir,
            settings: &settings,
            steam: self.steam.as_ref(),
            game_mode: self.game_mode,
            running: &self.running,
            free_space: &self.free_space,
        };
        let port_ref = PortRef { key: request.port_key.clone(), name: request.port_name.clone() };

        let mut accept = false;
        let last_percent = std::cell::Cell::new(u64::MAX);
        let prepared = loop {
            (ctx.progress)("checking".into());
            let options = MoveOptions { game_files: game_files.clone(), accept_issues_before: accept, ..Default::default() };
            let mut progress = |done: u64, total: u64, _file: &Path| {
                let percent = if total == 0 { 100 } else { done * 100 / total };
                if percent != last_percent.get() {
                    last_percent.set(percent);
                    (ctx.progress)(format!("copying, {} of {}", human_bytes(done), human_bytes(total)));
                }
            };
            match prepare(&port_ref, &request.from, &request.to, &env, &options, &mut progress, ctx.cancel) {
                Ok(done) => break done,
                Err(MoveError::GameFiles { stage: Stage::BeforeCopy, issues }) if !accept => match ctx.prompts.game_file_problem(&request.port_name, &issues) {
                    GameFileChoice::MoveAnyway => accept = true,
                    GameFileChoice::FixInGameFiles => {
                        return PortOutcome::Stopped {
                            reason: "some game files it needs are missing or broken. Fix them in Settings > Game files, then import it again. Nothing was copied.".into(),
                            fix_in_game_files: true,
                        };
                    }
                    GameFileChoice::Cancel => {
                        ctx.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                        return PortOutcome::Stopped { reason: "you cancelled at the game file question. Nothing was copied.".into(), fix_in_game_files: false };
                    }
                },
                Err(MoveError::Cancelled) => {
                    return PortOutcome::Stopped { reason: "cancelled while copying. The partial copy was removed; the old folder is untouched.".into(), fix_in_game_files: false };
                }
                Err(e @ MoveError::NotEnoughSpace { .. }) => return refused(&e, true),
                Err(e @ (MoveError::Running(_) | MoveError::SourceMissing(_) | MoveError::TargetExists(_) | MoveError::TargetParentMissing(_) | MoveError::Nested)) => return refused(&e, false),
                Err(e) => return PortOutcome::Failed(e.to_string()),
            }
        };

        (ctx.progress)("switching over".into());
        let report = switch_over(&prepared, &env);
        let mut notes: Vec<String> = Vec::new();
        let text = report.text();
        if !text.is_empty() {
            notes.push(text);
        }
        if let Some(port) = port {
            let missing = required_not_found(&prepared.to, &GameFiles::for_port(port, &self.config_dir));
            if !missing.is_empty() {
                notes.push(format!("For information, its entry names {} which is neither in its folder nor connected in Game files (fine if it was only needed when installing).", missing.join(", ")));
            }
        }
        if !prepared.accepted_issues.is_empty() {
            notes.push(format!("Moved although the game file check found: {}.", prepared.accepted_issues.iter().map(|i| i.file.clone()).collect::<Vec<_>>().join(", ")));
        }
        for link in &prepared.rewritten_links {
            notes.push(format!("The link {} was adjusted so it keeps working at the new place.", link.display()));
        }
        PortOutcome::Moved { steam: steam_result(&report.steam, request), notes }
    }
}

#[cfg(test)]
mod tests {
    use super::super::engine::{move_selected, ImportSummary, NoPrompts, Prompter};
    use super::super::folders::ScanFolders;
    use super::super::plan::{build_requests, plan_all, PortPlan, SteamWrite};
    use super::super::scan::fixtures::*;
    use super::super::scan::{read_shortcuts, run_scan, Found, ScanInput};
    use super::*;
    use crate::fork::port_mover::game_files::Issue;
    use crate::fork::shortcuts;
    use crate::fork::store::ShortcutsFile;
    use std::fs;
    use std::sync::atomic::AtomicBool;
    use std::sync::Mutex;

    const SOH_ID: u32 = 0xAAAA_0001;

    fn engine(h: &Home, steam_running: bool) -> RealEngine {
        let config_dir = h.home.join("appdir");
        fs::create_dir_all(&config_dir).unwrap();
        RealEngine {
            home: h.home.clone(),
            config_dir,
            catalog: catalog(),
            game_mode: false,
            steam: Arc::new(move || -> std::io::Result<Vec<String>> { Ok(if steam_running { vec!["steam".to_string()] } else { Vec::new() }) }),
            running: |_| Vec::new(),
            free_space: |_| None,
        }
    }

    fn plans_for(h: &Home, names: &[&str], write: SteamWrite) -> Vec<PortPlan> {
        let input = ScanInput {
            roots: ScanFolders::default().roots(&h.home, &h.library),
            catalog: catalog(),
            library_dir: h.library.clone(),
            deeper: false,
            shortcuts: read_shortcuts(&h.home, &ForkSettings::load(&h.home.join("s.json"))),
        };
        let scan = run_scan(&input, &|| false, &|_| {});
        let refs: Vec<&Found> = names.iter().map(|n| scan.found.iter().find(|f| f.port_name == *n).unwrap()).collect();
        plan_all(&build_requests(&refs, &h.library), None, write)
    }

    fn run(engine: &RealEngine, plans: Vec<PortPlan>, prompts: &dyn Prompter) -> ImportSummary {
        move_selected(engine, plans, &AtomicBool::new(false), &|_| {}, prompts)
    }

    fn tree(root: &Path) -> Vec<(String, Vec<u8>)> {
        fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
            for e in fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                if p.is_dir() {
                    out.push((format!("{rel}/"), Vec::new()));
                    walk(root, &p, out);
                } else {
                    out.push((rel, fs::read(&p).unwrap()));
                }
            }
        }
        let mut out = Vec::new();
        walk(root, root, &mut out);
        out.sort();
        out
    }

    fn shortcut_of(h: &Home, name: &str) -> shortcuts::Shortcut {
        let file = ShortcutsFile::load(&h.shortcuts_path).unwrap();
        shortcuts::list(&file.doc).into_iter().find(|s| s.app_name == name).unwrap()
    }

    #[test]
    fn importing_a_port_copies_it_repoints_its_shortcut_keeping_the_app_id_and_keeps_the_old_copy() {
        let h = fake_home("real_e2e");
        let old = h.home.join("Applications/SoH");
        let old_tree = tree(&old);
        let before_other = shortcut_of(&h, "Something Else");
        let summary = run(&engine(&h, false), plans_for(&h, &["Ship of Harkinian"], SteamWrite::Now), &NoPrompts);

        assert!(!summary.dry_run && !summary.cancelled);
        let PortOutcome::Moved { steam, .. } = &summary.items[0].outcome else { panic!("{:?}", summary.items[0].outcome) };
        assert_eq!(*steam, SteamResult::Repointed(1));
        // The copy is in the ports folder, byte for byte (the saves too), and the port counts as installed.
        assert_eq!(tree(&h.library.join("SoH")), old_tree);
        assert!(crate::core::installer::is_installed(&catalog()[0], &h.library));
        // The old copy is untouched and still there.
        assert_eq!(tree(&old), old_tree);
        // The shortcut now points into the ports folder, app ID kept; the others did not change.
        let sc = shortcut_of(&h, "Ship of Harkinian");
        assert_eq!(sc.app_id, Some(SOH_ID));
        assert_eq!(Path::new(sc.exe_path()), h.library.join("SoH/soh.appimage"));
        assert!(Path::new(sc.start_dir_path()).starts_with(h.library.join("SoH")));
        assert_eq!(shortcut_of(&h, "Something Else"), before_other);
        // shortcuts.vdf was backed up first.
        let backups = fs::read_dir(h.shortcuts_path.parent().unwrap()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("shortcuts.vdf.bak-")).count();
        assert_eq!(backups, 1);
        // The old folder is recorded as an old copy, switch finished, so the prompt can ask about it.
        let records = crate::fork::port_mover::old_copies::load(&h.home.join("appdir")).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].old_path, old);
        assert_eq!(records[0].new_path, h.library.join("SoH"));
        assert!(records[0].switched);
        // The summary says the old copy is still there.
        assert!(summary.text(&h.home).contains("the old copy at ~/Applications/SoH (8 KB) is still there; 1 Steam shortcut repointed"), "{}", summary.text(&h.home));
    }

    #[test]
    fn with_steam_running_the_shortcut_change_is_queued_and_the_file_is_not_written() {
        let h = fake_home("real_queue");
        let before = fs::read(&h.shortcuts_path).unwrap();
        let summary = run(&engine(&h, true), plans_for(&h, &["Ship of Harkinian"], SteamWrite::Queued), &NoPrompts);
        let PortOutcome::Moved { steam, notes } = &summary.items[0].outcome else { panic!("{:?}", summary.items[0].outcome) };
        assert_eq!(*steam, SteamResult::Queued(1));
        assert!(notes.iter().any(|n| n.contains("queued because Steam is running")), "{notes:?}");
        assert_eq!(fs::read(&h.shortcuts_path).unwrap(), before, "shortcuts.vdf is never written while Steam runs");
        let queue = fs::read_to_string(h.home.join("appdir/steam-queue.json")).unwrap();
        assert!(queue.contains("repoint") && queue.contains("Applications"), "{queue}");
        assert!(h.library.join("SoH/soh.appimage").exists() && h.home.join("Applications/SoH/soh.appimage").exists());
    }

    #[test]
    fn a_port_with_no_shortcut_just_moves() {
        let h = fake_home("real_noshortcut");
        let summary = run(&engine(&h, false), plans_for(&h, &["Dusklight"], SteamWrite::Now), &NoPrompts);
        assert!(matches!(&summary.items[0].outcome, PortOutcome::Moved { steam: SteamResult::NoShortcut, .. }), "{:?}", summary.items[0].outcome);
        assert!(h.library.join("dusklight/dusk").exists() && h.home.join("Games/dusk-old/dusk").exists());
    }

    #[test]
    fn a_port_that_does_not_fit_is_refused_for_space_and_nothing_is_copied() {
        let h = fake_home("real_space");
        let mut engine = engine(&h, false);
        engine.free_space = |_| Some(1000);
        let input_plans = plans_for(&h, &["Dusklight"], SteamWrite::Now);
        // The plan itself knew nothing about the space; the engine's own check refuses.
        let summary = run(&engine, input_plans, &NoPrompts);
        assert!(matches!(&summary.items[0].outcome, PortOutcome::Refused { space: true, .. }), "{:?}", summary.items[0].outcome);
        assert_eq!(summary.link_candidates(), [0]);
        assert!(!h.library.join("dusklight").exists());
        assert!(fs::read_dir(&h.library).unwrap().count() == 0, "not even a staging folder is left");
    }

    #[cfg(unix)]
    mod game_files {
        use super::*;

        /// A link inside SoH to a file that is not there: a game file problem found before the copy.
        fn with_broken_game_file(h: &Home) {
            std::os::unix::fs::symlink("/definitely/not/here/oot.o2r", h.home.join("Applications/SoH/oot.o2r")).unwrap();
        }

        struct Answer {
            choice: GameFileChoice,
            asked: Mutex<Vec<String>>,
        }

        impl Prompter for Answer {
            fn game_file_problem(&self, port: &str, issues: &[Issue]) -> GameFileChoice {
                self.asked.lock().unwrap().push(format!("{port}: {}", issues.iter().map(|i| i.file.clone()).collect::<Vec<_>>().join(",")));
                self.choice
            }
        }

        fn ask(choice: GameFileChoice) -> Answer {
            Answer { choice, asked: Mutex::new(Vec::new()) }
        }

        #[test]
        fn fix_in_game_files_skips_that_port_and_copies_nothing_but_the_others_go_on() {
            let h = fake_home("real_gf_fix");
            with_broken_game_file(&h);
            let answer = ask(GameFileChoice::FixInGameFiles);
            let summary = run(&engine(&h, false), plans_for(&h, &["Dusklight", "Ship of Harkinian"], SteamWrite::Now), &answer);
            assert_eq!(*answer.asked.lock().unwrap(), ["Ship of Harkinian: oot.o2r"]);
            assert!(matches!(summary.items[0].outcome, PortOutcome::Moved { .. }));
            assert!(matches!(summary.items[1].outcome, PortOutcome::Stopped { fix_in_game_files: true, .. }));
            assert!(summary.wants_game_files());
            assert!(!h.library.join("SoH").exists(), "nothing was copied for the port with the problem");
            assert!(shortcut_of(&h, "Ship of Harkinian").exe_path().contains("Applications"), "its shortcut was not touched");
        }

        #[test]
        fn move_anyway_copies_it_and_the_summary_names_the_accepted_problem() {
            let h = fake_home("real_gf_anyway");
            with_broken_game_file(&h);
            let answer = ask(GameFileChoice::MoveAnyway);
            let summary = run(&engine(&h, false), plans_for(&h, &["Ship of Harkinian"], SteamWrite::Now), &answer);
            let PortOutcome::Moved { notes, .. } = &summary.items[0].outcome else { panic!("{:?}", summary.items[0].outcome) };
            assert!(notes.iter().any(|n| n.contains("Moved although the game file check found: oot.o2r")), "{notes:?}");
            assert!(h.library.join("SoH/soh.appimage").exists());
            assert_eq!(answer.asked.lock().unwrap().len(), 1, "asked once, not again after the answer");
        }

        #[test]
        fn cancel_stops_the_whole_import_there() {
            let h = fake_home("real_gf_cancel");
            with_broken_game_file(&h);
            let answer = ask(GameFileChoice::Cancel);
            // Ship of Harkinian is asked about first here because Dusklight sorts first, so put it first by name order.
            let summary = run(&engine(&h, false), plans_for(&h, &["Ship of Harkinian", "Dusklight"], SteamWrite::Now), &answer);
            assert!(summary.cancelled);
            assert!(matches!(summary.items[0].outcome, PortOutcome::Stopped { fix_in_game_files: false, .. }));
            assert_eq!(summary.items[1].outcome, PortOutcome::NotStarted);
            assert!(!h.library.join("SoH").exists() && !h.library.join("dusklight").exists());
        }
    }

    #[test]
    fn a_cancel_before_the_first_port_starts_copies_nothing() {
        let h = fake_home("real_precancel");
        let cancel = AtomicBool::new(true);
        let summary = move_selected(&engine(&h, false), plans_for(&h, &["Dusklight"], SteamWrite::Now), &cancel, &|_| {}, &NoPrompts);
        assert!(summary.cancelled && summary.items[0].outcome == PortOutcome::NotStarted);
        assert!(!h.library.join("dusklight").exists());
    }

    #[test]
    fn the_destination_appearing_before_the_copy_is_refused_not_overwritten() {
        let h = fake_home("real_exists");
        let plans = plans_for(&h, &["Dusklight"], SteamWrite::Now);
        put_file(&plans[0].request.to.join("keep.txt"), b"mine");
        // The plan was made before the folder appeared, so the engine's own check has to refuse it.
        let summary = run(&engine(&h, false), plans, &NoPrompts);
        assert!(matches!(&summary.items[0].outcome, PortOutcome::Refused { space: false, .. }), "{:?}", summary.items[0].outcome);
        assert_eq!(fs::read(h.library.join("dusklight/keep.txt")).unwrap(), b"mine");
        assert!(!h.library.join("dusklight/dusk").exists());
    }

    #[test]
    fn the_engine_is_not_a_dry_run_and_the_summary_title_says_finished() {
        let h = fake_home("real_title");
        let summary = run(&engine(&h, false), plans_for(&h, &["Dusklight"], SteamWrite::Now), &NoPrompts);
        assert_eq!(summary.title(), "Import finished");
    }
}
