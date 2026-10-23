//! The folders Import ports looks in: the well-known ones (switchable), the ones the user added with the
//! folder picker, and the "look deeper" switch. All of it lives in `fork-settings.json`; the user never
//! sees or edits that file.
//!
//! Settings keys (documented here because the file is shared with other fork features):
//! - `adopt_folders`: the user's own folders (`ForkSettings::set_adopt_folders`), as absolute paths (the key name predates Import ports and
//!   is kept so an existing list still works);
//! - `import_defaults_off`: names of well-known folders switched off (`"Games"`);
//! - `import_look_deeper`: true to look one level further into folders that matched nothing.

use crate::fork::adopt::{expand_home, scan_roots_with, WELL_KNOWN};
use crate::fork::settings::ForkSettings;
use serde_json::Value;
use std::io;
use std::path::{Path, PathBuf};

const KEY_DEFAULTS_OFF: &str = "import_defaults_off";
const KEY_DEEPER: &str = "import_look_deeper";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScanFolders {
    /// Names (under the home folder) of the well-known folders that are switched off.
    pub defaults_off: Vec<String>,
    /// The user's own folders, as stored.
    pub user: Vec<String>,
    pub look_deeper: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddOutcome {
    Added,
    /// It is a well-known folder that was switched off; it is switched back on.
    DefaultTurnedOn(String),
    AlreadyListed,
    /// The picker gave something that is not a folder (gone since, or a file).
    NotAFolder,
}

impl ScanFolders {
    pub fn load(settings: &ForkSettings) -> ScanFolders {
        let defaults_off = settings
            .get(KEY_DEFAULTS_OFF)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).filter(|n| WELL_KNOWN.contains(n)).map(str::to_string).collect())
            .unwrap_or_default();
        ScanFolders {
            defaults_off,
            user: settings.adopt_folders(),
            look_deeper: settings.get(KEY_DEEPER).and_then(Value::as_bool).unwrap_or(false),
        }
    }

    /// Writes all three keys; the first failure is returned.
    pub fn save(&self, settings: &mut ForkSettings) -> io::Result<()> {
        settings.set_adopt_folders(&self.user)?;
        settings.set(KEY_DEFAULTS_OFF, Value::from(self.defaults_off.clone()))?;
        settings.set(KEY_DEEPER, Value::from(self.look_deeper))
    }

    pub fn default_is_on(&self, name: &str) -> bool {
        !self.defaults_off.iter().any(|n| n == name)
    }

    pub fn toggle_default(&mut self, name: &str) {
        if !WELL_KNOWN.contains(&name) {
            return;
        }
        if self.default_is_on(name) {
            self.defaults_off.push(name.to_string());
        } else {
            self.defaults_off.retain(|n| n != name);
        }
    }

    /// Adds a folder the user picked. A folder that is already covered (listed, or a well-known one that
    /// is on) is not listed twice.
    pub fn add_user(&mut self, picked: &Path, home: &Path) -> AddOutcome {
        if !picked.is_dir() {
            return AddOutcome::NotAFolder;
        }
        let same = |a: &Path, b: &Path| a == b || matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y);
        for name in WELL_KNOWN {
            if same(picked, &home.join(name)) {
                return if self.default_is_on(name) {
                    AddOutcome::AlreadyListed
                } else {
                    self.toggle_default(name);
                    AddOutcome::DefaultTurnedOn(name.to_string())
                };
            }
        }
        if self.user.iter().any(|u| same(&expand_home(u, home), picked)) {
            return AddOutcome::AlreadyListed;
        }
        self.user.push(picked.to_string_lossy().into_owned());
        AddOutcome::Added
    }

    pub fn remove_user(&mut self, index: usize) -> Option<String> {
        (index < self.user.len()).then(|| self.user.remove(index))
    }

    pub fn toggle_deeper(&mut self) {
        self.look_deeper = !self.look_deeper;
    }

    /// The folders that exist and are switched on, each once, never the library.
    pub fn roots(&self, home: &Path, library_dir: &Path) -> Vec<PathBuf> {
        scan_roots_with(home, &self.defaults_off, &self.user, library_dir)
    }

    /// How many folders are switched on (well-known ones plus the user's), whether or not they exist.
    pub fn on_count(&self) -> usize {
        WELL_KNOWN.iter().filter(|n| self.default_is_on(n)).count() + self.user.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("dpm_import_folders_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn nothing_saved_means_the_three_standard_folders_on_and_no_deeper_look() {
        let dir = temp_dir("fresh");
        let f = ScanFolders::load(&ForkSettings::load(&dir.join("s.json")));
        assert_eq!(f, ScanFolders::default());
        assert!(WELL_KNOWN.iter().all(|n| f.default_is_on(n)));
        assert_eq!(f.on_count(), 3);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn adding_removing_and_toggling_persist_through_the_settings_file() {
        let dir = temp_dir("persist");
        let home = dir.join("home");
        let sd = dir.join("sdcard").join("Ports");
        fs::create_dir_all(&sd).unwrap();
        fs::create_dir_all(home.join("Games")).unwrap();
        let path = dir.join("fork-settings.json");
        fs::write(&path, r#"{"steam_account_id": 5}"#).unwrap();

        let mut settings = ForkSettings::load(&path);
        let mut f = ScanFolders::load(&settings);
        assert_eq!(f.add_user(&sd, &home), AddOutcome::Added);
        f.toggle_default("Games");
        f.toggle_deeper();
        f.save(&mut settings).unwrap();

        let again = ScanFolders::load(&ForkSettings::load(&path));
        assert_eq!(again.user, [sd.to_string_lossy().into_owned()]);
        assert_eq!(again.defaults_off, ["Games"]);
        assert!(again.look_deeper && !again.default_is_on("Games") && again.default_is_on("Ports"));
        assert!(fs::read_to_string(&path).unwrap().contains("steam_account_id"), "other keys survive");

        let mut f = again;
        assert_eq!(f.remove_user(0), Some(sd.to_string_lossy().into_owned()));
        assert_eq!(f.remove_user(0), None);
        f.toggle_default("Games");
        f.toggle_deeper();
        let mut settings = ForkSettings::load(&path);
        f.save(&mut settings).unwrap();
        assert_eq!(ScanFolders::load(&ForkSettings::load(&path)), ScanFolders::default());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_folder_is_never_listed_twice_and_a_switched_off_standard_one_comes_back_on() {
        let dir = temp_dir("dupes");
        let home = dir.join("home");
        let extra = dir.join("extra");
        fs::create_dir_all(home.join("Applications")).unwrap();
        fs::create_dir_all(&extra).unwrap();
        let mut f = ScanFolders::default();
        assert_eq!(f.add_user(&extra, &home), AddOutcome::Added);
        assert_eq!(f.add_user(&extra, &home), AddOutcome::AlreadyListed);
        assert_eq!(f.add_user(&home.join("Applications"), &home), AddOutcome::AlreadyListed);
        f.toggle_default("Applications");
        assert_eq!(f.add_user(&home.join("Applications"), &home), AddOutcome::DefaultTurnedOn("Applications".into()));
        assert!(f.default_is_on("Applications"));
        assert_eq!(f.user.len(), 1);
        assert_eq!(f.add_user(&dir.join("missing"), &home), AddOutcome::NotAFolder);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn toggling_something_that_is_not_a_standard_folder_does_nothing() {
        let mut f = ScanFolders::default();
        f.toggle_default("Documents");
        assert!(f.defaults_off.is_empty());
    }

    #[test]
    fn roots_follow_the_switches_and_skip_folders_that_do_not_exist() {
        let dir = temp_dir("roots");
        let home = dir.join("home");
        for n in ["Applications", "Games", "Ports", "mine"] {
            fs::create_dir_all(home.join(n)).unwrap();
        }
        let mut f = ScanFolders { user: vec![home.join("mine").to_string_lossy().into_owned(), "/no/such/place".into()], ..Default::default() };
        let library = dir.join("Library");
        assert_eq!(f.roots(&home, &library), [home.join("Applications"), home.join("Games"), home.join("Ports"), home.join("mine")]);
        f.toggle_default("Games");
        f.toggle_default("Ports");
        assert_eq!(f.roots(&home, &library), [home.join("Applications"), home.join("mine")]);
        assert_eq!(f.on_count(), 3, "one standard folder plus two listed ones, even though one does not exist");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn junk_in_the_settings_file_falls_back_to_defaults() {
        let dir = temp_dir("junk");
        let path = dir.join("s.json");
        fs::write(&path, r#"{"import_defaults_off": ["Games", "Documents", 7], "import_look_deeper": "yes", "adopt_folders": "no"}"#).unwrap();
        let f = ScanFolders::load(&ForkSettings::load(&path));
        assert_eq!(f.defaults_off, ["Games"]);
        assert!(!f.look_deeper && f.user.is_empty());
        let _ = fs::remove_dir_all(dir);
    }
}
