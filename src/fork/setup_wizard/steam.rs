//! The setup wizard's "Add your ports to Steam?": one button for all of it. The same rules as Settings > Add to
//! Steam, with the choices taken out: every port that is not in Steam yet is added, and when Steam is running
//! (always in Game Mode) nothing is written: the changes are queued for Desktop Mode, exactly as the Add to
//! Steam screen would. `shortcuts.vdf` is touched only by `screens::apply_selected` (which backs it up and
//! refuses while Steam runs); a queued run never opens it for writing.

use super::flow::SteamOutcome;
use crate::core::models::Port;
use crate::fork::screens::{apply_selected, SteamScreen};
use crate::fork::settings::ForkSettings;
use crate::fork::steam_queue::{self, changes_from_screen, queue_changes};
use crate::fork::steam_user::SteamAccount;
use crate::fork::store::SteamProcessCheck;
use std::path::Path;
use std::time::SystemTime;

/// Where Steam stands, as the page needs it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Info {
    /// Ports that would be added or brought up to date.
    pub to_add: usize,
    /// Of those, the ones already in the queue for Desktop Mode (a second visit to the page after queueing).
    pub queued: usize,
    /// Several accounts: their names, to choose from before adding. Empty with one account.
    pub accounts: Vec<String>,
    /// Why nothing can be done (no Steam account folder, a shortcuts file that cannot be read), in plain words.
    pub problem: Option<String>,
    /// Steam is closed outside Game Mode, so the ports are added now; otherwise they are queued.
    pub immediate: bool,
    /// Every Steam change waiting in the queue (adds, and shortcuts to point at moved ports).
    pub waiting: usize,
    pub game_mode: bool,
}

pub fn info(screen: &SteamScreen, steam: &dyn SteamProcessCheck) -> Info {
    let problem = screen.load_error.clone();
    let immediate = !screen.game_mode && steam.running_steam_processes().is_ok_and(|p| p.is_empty());
    let accounts = if screen.accounts.len() > 1 { screen.accounts.iter().map(SteamAccount::label).collect() } else { Vec::new() };
    let queued = match screen.active_account() {
        Some(a) if problem.is_none() => screen.rows.iter().filter(|r| r.can_change() && !r.by_name() && screen.queue.contains(a.account_id, &r.key)).count(),
        _ => 0,
    };
    Info { to_add: if problem.is_some() { 0 } else { screen.missing_count() }, queued, accounts, problem, immediate, waiting: screen.queue.len(), game_mode: screen.game_mode }
}

/// Adds every missing port to the shortcut list, or queues them while Steam runs.
pub fn add_all(screen: &mut SteamScreen, steam: &dyn SteamProcessCheck, settings: &ForkSettings, queue_path: &Path, now: SystemTime, stamp: &str) -> SteamOutcome {
    if let Some(problem) = &screen.load_error {
        return SteamOutcome::Problem(problem.clone());
    }
    if screen.account.is_none() {
        return SteamOutcome::Problem("Choose your Steam account first.".to_string());
    }
    screen.select_all_missing();
    let count = screen.selected_count();
    if count == 0 {
        return SteamOutcome::Nothing;
    }
    // Writing now while other changes wait (a moved port's shortcut that still points at its old copy) would add a
    // second shortcut for that port: those are made first, through the close-Steam flow.
    if !screen.can_queue(steam) && !screen.queue.is_empty() {
        return SteamOutcome::Problem("Some Steam changes are still waiting. Update those first; then the rest can be added.".to_string());
    }
    if screen.can_queue(steam) {
        let changes = changes_from_screen(screen, stamp);
        return match queue_changes(queue_path, changes, screen.game_mode) {
            Ok(_) => SteamOutcome::Queued { count, game_mode: screen.game_mode },
            Err(text) => SteamOutcome::Problem(text),
        };
    }
    if let Some(problem) = screen.review_problem(steam) {
        return SteamOutcome::Problem(problem);
    }
    match apply_selected(screen, steam, settings, now) {
        Ok(out) if out.wrote => SteamOutcome::Added(count),
        Ok(_) => SteamOutcome::Nothing,
        Err(text) => SteamOutcome::Problem(text),
    }
}

/// What the screen opens for the wizard: the Add to Steam screen's own load, with the queue read as well.
pub fn load_screen(home: &Path, library_dir: &Path, catalog: &[Port], settings: &ForkSettings, favorite_exe: &dyn Fn(&str) -> Option<String>, cache_dir: &Path, queue_path: &Path, game_mode: bool) -> SteamScreen {
    let mut screen = SteamScreen::load(home, library_dir, catalog, settings, favorite_exe);
    screen.cache_dir = Some(cache_dir.to_path_buf());
    let loaded = steam_queue::load(queue_path);
    screen.queue = loaded.queue;
    screen.queue_note = loaded.problem;
    screen.game_mode = game_mode;
    screen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::port_from_value;
    use crate::fork::backups;
    use crate::fork::vdf::fixtures::{file_of, steam_entry};
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("dpm_wizard_steam_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn catalog() -> Vec<Port> {
        [
            json!({"name": "Dusklight", "folder": "dusklight", "source": "https://github.com/a/dusk", "executable": "dusk"}),
            json!({"name": "Blocks", "folder": "blocks", "source": "https://github.com/a/blocks", "executable": "blocks"}),
            json!({"name": "Not Installed", "folder": "nope", "source": "https://github.com/a/nope"}),
        ]
        .iter()
        .map(|v| port_from_value(v).unwrap())
        .collect()
    }

    fn put_exe(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// A home with a Steam folder holding `accounts`, a shortcuts file for the first one with a hand-made entry,
    /// and a library with two installed ports.
    fn fake_home(name: &str, accounts: &[u32]) -> (PathBuf, PathBuf, PathBuf) {
        let home = temp_dir(name);
        let steam = home.join(".local/share/Steam");
        fs::create_dir_all(steam.join("config")).unwrap();
        let mut shortcuts = PathBuf::new();
        for (i, id) in accounts.iter().enumerate() {
            let user = steam.join("userdata").join(id.to_string());
            fs::create_dir_all(user.join("config")).unwrap();
            let entry = steam_entry(0, 0xAAAA_0001, "Something else", "\"/x/other\"", "\"/x/\"", &[]);
            let path = user.join("config").join("shortcuts.vdf");
            fs::write(&path, file_of(&[entry])).unwrap();
            if i == 0 {
                shortcuts = path;
            }
        }
        let library = home.join("Ports");
        put_exe(&library.join("dusklight").join("dusk"));
        put_exe(&library.join("blocks").join("blocks"));
        (home, library, shortcuts)
    }

    fn screen(home: &Path, library: &Path, game_mode: bool) -> SteamScreen {
        let settings = ForkSettings::load(&home.join("s.json"));
        load_screen(home, library, &catalog(), &settings, &|_| None, &home.join("cache"), &home.join("steam-queue.json"), game_mode)
    }

    fn closed() -> std::io::Result<Vec<String>> {
        Ok(Vec::new())
    }

    fn running() -> std::io::Result<Vec<String>> {
        Ok(vec!["99 steam".into()])
    }

    fn broken() -> std::io::Result<Vec<String>> {
        Err(std::io::Error::other("no /proc"))
    }

    fn settings(home: &Path) -> ForkSettings {
        ForkSettings::load(&home.join("s.json"))
    }

    #[test]
    fn the_page_counts_the_installed_ports_that_are_not_in_steam() {
        let (home, library, _) = fake_home("count", &[1234]);
        let s = screen(&home, &library, false);
        let i = info(&s, &closed);
        assert_eq!((i.to_add, i.immediate, i.problem.clone(), i.accounts.clone()), (2, true, None, vec![]));
        assert!(!info(&s, &running).immediate, "with Steam running the ports are queued, not added");
        assert!(!info(&screen(&home, &library, true), &closed).immediate, "never in Game Mode");
        assert!(!info(&s, &broken).immediate, "when Steam cannot be checked nothing is written");
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn several_accounts_are_named_and_one_is_not_asked_about() {
        let (home, library, _) = fake_home("accounts", &[1234, 5678]);
        let i = info(&screen(&home, &library, false), &closed);
        assert_eq!(i.accounts.len(), 2);
        let (home1, library1, _) = fake_home("one_account", &[1234]);
        assert!(info(&screen(&home1, &library1, false), &closed).accounts.is_empty());
        let _ = fs::remove_dir_all(home);
        let _ = fs::remove_dir_all(home1);
    }

    #[test]
    fn no_steam_folder_is_a_plain_problem_and_nothing_is_added() {
        let home = temp_dir("nosteam");
        let library = home.join("Ports");
        put_exe(&library.join("dusklight").join("dusk"));
        let mut s = screen(&home, &library, false);
        let i = info(&s, &closed);
        assert!(i.problem.as_deref().is_some_and(|p| p.contains("Start Steam once")), "{:?}", i.problem);
        assert_eq!(i.to_add, 0);
        let out = add_all(&mut s, &closed, &settings(&home), &home.join("steam-queue.json"), SystemTime::now(), "20261023-100000");
        assert!(matches!(out, SteamOutcome::Problem(_)), "{out:?}");
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn with_steam_closed_every_missing_port_is_added_after_a_backup() {
        let (home, library, path) = fake_home("add", &[1234]);
        let before = fs::read(&path).unwrap();
        let mut s = screen(&home, &library, false);
        let out = add_all(&mut s, &closed, &settings(&home), &home.join("steam-queue.json"), SystemTime::now(), "20261023-100000");
        assert_eq!(out, SteamOutcome::Added(2));
        let backups = backups::list_backups(&path).unwrap();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(&backups[0].path).unwrap(), before, "the old list is kept byte for byte");
        let names: Vec<String> = crate::fork::shortcuts::list(&crate::fork::store::ShortcutsFile::load(&path).unwrap().doc).iter().map(|e| e.app_name.clone()).collect();
        assert!(names.contains(&"Dusklight".to_string()) && names.contains(&"Blocks".to_string()) && names.contains(&"Something else".to_string()), "{names:?}");
        assert!(!home.join("steam-queue.json").exists(), "nothing is queued when it was written");
        // A second run finds nothing to do.
        let mut again = screen(&home, &library, false);
        assert_eq!(add_all(&mut again, &closed, &settings(&home), &home.join("steam-queue.json"), SystemTime::now(), "20261023-100100"), SteamOutcome::Nothing);
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn with_steam_running_the_ports_are_queued_and_shortcuts_vdf_is_byte_identical() {
        let (home, library, path) = fake_home("queue", &[1234]);
        let before = fs::read(&path).unwrap();
        let queue = home.join("steam-queue.json");
        let mut s = screen(&home, &library, false);
        let out = add_all(&mut s, &running, &settings(&home), &queue, SystemTime::now(), "20261023-100000");
        assert_eq!(out, SteamOutcome::Queued { count: 2, game_mode: false });
        assert_eq!(fs::read(&path).unwrap(), before, "shortcuts.vdf was not touched");
        assert!(backups::list_backups(&path).unwrap().is_empty(), "no backup, because nothing was written");
        let loaded = steam_queue::load(&queue);
        assert_eq!(loaded.queue.len(), 2);
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn in_game_mode_the_ports_are_queued_even_when_the_check_says_steam_is_closed() {
        let (home, library, path) = fake_home("gamemode", &[1234]);
        let before = fs::read(&path).unwrap();
        let queue = home.join("steam-queue.json");
        let mut s = screen(&home, &library, true);
        let out = add_all(&mut s, &closed, &settings(&home), &queue, SystemTime::now(), "20261023-100000");
        assert_eq!(out, SteamOutcome::Queued { count: 2, game_mode: true });
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(steam_queue::load(&queue).queue.len() == 2);
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn when_steam_cannot_be_checked_nothing_is_written_or_queued() {
        let (home, library, path) = fake_home("broken", &[1234]);
        let before = fs::read(&path).unwrap();
        let queue = home.join("steam-queue.json");
        let mut s = screen(&home, &library, false);
        let out = add_all(&mut s, &broken, &settings(&home), &queue, SystemTime::now(), "20261023-100000");
        assert!(matches!(&out, SteamOutcome::Problem(t) if t.contains("whether Steam is running")), "{out:?}");
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!queue.exists());
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn a_shortcut_with_the_same_name_that_points_elsewhere_is_never_repointed_by_add_all() {
        use crate::fork::vdf::fixtures::steam_entry;
        let (home, library, path) = fake_home("byname", &[1234]);
        let entry = steam_entry(0, 0xAAAA_0002, "Dusklight", "\"/old/dusk\"", "\"/old/\"", &[]);
        fs::write(&path, file_of(&[entry])).unwrap();
        let mut s = screen(&home, &library, false);
        assert_eq!(s.missing_count(), 1, "only Blocks");
        assert_eq!(add_all(&mut s, &closed, &settings(&home), &home.join("steam-queue.json"), SystemTime::now(), "20261023-100000"), SteamOutcome::Added(1));
        let list = crate::fork::shortcuts::list(&crate::fork::store::ShortcutsFile::load(&path).unwrap().doc);
        let dusk = list.iter().find(|e| e.app_name == "Dusklight").unwrap();
        assert_eq!(dusk.exe_path(), "/old/dusk", "left for the user to repoint in Add to Steam");
        let _ = fs::remove_dir_all(home);
    }
    #[test]
    fn with_changes_waiting_and_steam_closed_nothing_is_added_before_them() {
        let (home, library, path) = fake_home("waiting_first", &[1234]);
        let queue = home.join("steam-queue.json");
        let mut first = screen(&home, &library, false);
        assert!(matches!(add_all(&mut first, &running, &settings(&home), &queue, SystemTime::now(), "20261023-100000"), SteamOutcome::Queued { .. }));
        let before = fs::read(&path).unwrap();
        let mut now_closed = screen(&home, &library, false);
        let out = add_all(&mut now_closed, &closed, &settings(&home), &queue, SystemTime::now(), "20261023-100100");
        assert!(matches!(&out, SteamOutcome::Problem(t) if t.contains("still waiting")), "{out:?}");
        assert_eq!(fs::read(&path).unwrap(), before, "shortcuts.vdf untouched");
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn ports_already_queued_are_counted_so_the_page_can_say_so() {
        let (home, library, _) = fake_home("alreadyqueued", &[1234]);
        let queue = home.join("steam-queue.json");
        let mut s = screen(&home, &library, false);
        assert_eq!(add_all(&mut s, &running, &settings(&home), &queue, SystemTime::now(), "20261023-100000"), SteamOutcome::Queued { count: 2, game_mode: false });
        let again = screen(&home, &library, false);
        let i = info(&again, &running);
        assert_eq!((i.to_add, i.queued, i.waiting, i.game_mode), (2, 2, 2, false));
        let _ = fs::remove_dir_all(home);
    }

}
