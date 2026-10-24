//! The folders Import ports looks in: the well-known ones (switchable), the ones the user added with the
//! folder picker, and the "look deeper" switch. All of it lives in `fork-settings.json`; the user never
//! sees or edits that file.
//!
//! Settings keys (documented here because the file is shared with other fork features):
//! - `adopt_folders`: the user's own folders (`ForkSettings::set_adopt_folders`), as absolute paths (the key name predates Import ports and
//!   is kept so an existing list still works);
//! - `import_defaults_off`: names of well-known folders switched off (`"Games"`);
//! - `import_look_deeper`: true to look one level further into folders that matched nothing.

use crate::fork::adopt::{expand_home, scan_roots_full, WELL_KNOWN};
use crate::fork::folder_picker::mounts::{default_media_roots, removable_drives, system_mounts, MountEntry};
use crate::fork::settings::ForkSettings;
use serde_json::Value;
use std::io;
use std::path::{Path, PathBuf};

const KEY_DEFAULTS_OFF: &str = "import_defaults_off";
const KEY_DEEPER: &str = "import_look_deeper";
const KEY_DRIVES_OFF: &str = "import_drives_off";

/// A removable drive (an SD card, a USB drive) that is plugged in: its top is a place Import ports looks in,
/// switched on unless the user turned it off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveRoot {
    pub mount: PathBuf,
    /// "SD card (SD512)".
    pub title: String,
}

/// The drives that are plugged in, from the mount list and the places the system mounts removable media.
pub fn drive_roots_from(mounts: &[MountEntry], media_roots: &[PathBuf]) -> Vec<DriveRoot> {
    removable_drives(mounts, media_roots).into_iter().map(|d| DriveRoot { title: d.title(), mount: d.mount_point }).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScanFolders {
    /// Names (under the home folder) of the well-known folders that are switched off.
    pub defaults_off: Vec<String>,
    /// The user's own folders, as stored.
    pub user: Vec<String>,
    pub look_deeper: bool,
    /// The removable drives plugged in now (not saved: they come and go).
    pub drives: Vec<DriveRoot>,
    /// Mount points of the drives switched off (saved).
    pub drives_off: Vec<String>,
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
            drives: Vec::new(),
            drives_off: settings.get(KEY_DRIVES_OFF).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default(),
        }
    }

    /// [`load`], with the removable drives that are plugged in on this device.
    pub fn on_this_device(settings: &ForkSettings) -> ScanFolders {
        ScanFolders::load(settings).with_drives(drive_roots_from(&system_mounts(), &default_media_roots()))
    }

    pub fn with_drives(mut self, drives: Vec<DriveRoot>) -> ScanFolders {
        self.drives = drives;
        self
    }

    pub fn drive_is_on(&self, index: usize) -> bool {
        self.drives.get(index).is_some_and(|d| !self.drives_off.iter().any(|m| Path::new(m) == d.mount))
    }

    pub fn toggle_drive(&mut self, index: usize) {
        let Some(drive) = self.drives.get(index) else { return };
        let mount = drive.mount.to_string_lossy().into_owned();
        if self.drive_is_on(index) {
            self.drives_off.push(mount);
        } else {
            self.drives_off.retain(|m| *m != mount);
        }
    }

    /// Writes all three keys; the first failure is returned.
    pub fn save(&self, settings: &mut ForkSettings) -> io::Result<()> {
        settings.set_adopt_folders(&self.user)?;
        settings.set(KEY_DEFAULTS_OFF, Value::from(self.defaults_off.clone()))?;
        settings.set(KEY_DRIVES_OFF, Value::from(self.drives_off.clone()))?;
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
        let tops: Vec<PathBuf> = (0..self.drives.len()).filter(|i| self.drive_is_on(*i)).map(|i| self.drives[i].mount.clone()).collect();
        scan_roots_full(home, &self.defaults_off, &self.user, &tops, library_dir)
    }

    /// How many folders are switched on (well-known ones plus the user's), whether or not they exist.
    pub fn on_count(&self) -> usize {
        WELL_KNOWN.iter().filter(|n| self.default_is_on(n)).count() + (0..self.drives.len()).filter(|i| self.drive_is_on(*i)).count() + self.user.len()
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
    fn a_plugged_in_drive_is_a_place_that_is_looked_in_until_it_is_switched_off() {
        let dir = temp_dir("drives");
        let home = dir.join("home");
        let sd = dir.join("run/media/deck/SD512");
        fs::create_dir_all(&sd).unwrap();
        fs::create_dir_all(&home).unwrap();
        let mounts = [MountEntry { device: "/dev/mmcblk0p1".into(), mount_point: sd.clone(), fs_type: "ext4".into() }];
        let drives = drive_roots_from(&mounts, &[dir.join("run/media/deck")]);
        assert_eq!(drives, [DriveRoot { mount: sd.clone(), title: "SD card (SD512)".into() }]);
        let mut f = ScanFolders::default().with_drives(drives);
        let library = sd.join("Ports");
        assert_eq!(f.roots(&home, &library), [sd.clone()], "found with no folder added by the user");
        assert!(f.drive_is_on(0) && f.on_count() == 4);
        f.toggle_drive(0);
        assert!(f.roots(&home, &library).is_empty() && f.on_count() == 3);
        // The switch is remembered, and a drive that is not plugged in changes nothing.
        let path = dir.join("s.json");
        let mut settings = ForkSettings::load(&path);
        f.save(&mut settings).unwrap();
        let again = ScanFolders::load(&ForkSettings::load(&path)).with_drives(f.drives.clone());
        assert!(!again.drive_is_on(0));
        assert!(ScanFolders::load(&ForkSettings::load(&path)).roots(&home, &library).is_empty());
        f.toggle_drive(0);
        assert!(f.drive_is_on(0));
        f.toggle_drive(7);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_ports_on_an_sd_card_top_are_found_with_no_folder_added() {
        use crate::fork::import::scan::{run_scan, tests as scan_tests};
        let dir = temp_dir("sd_top_scan");
        let sd = crate::fork::adopt::test_fixtures::tims_sd_card(&dir);
        let mounts = [MountEntry { device: "/dev/mmcblk0p1".into(), mount_point: sd.clone(), fs_type: "ext4".into() }];
        let folders = ScanFolders::default().with_drives(drive_roots_from(&mounts, &[sd.parent().unwrap().to_path_buf()]));
        let home = dir.join("home");
        fs::create_dir_all(&home).unwrap();
        let mut input = scan_tests::tims_input(&sd, Default::default());
        input.roots = folders.roots(&home, &sd.join("Ports"));
        assert_eq!(input.roots, [sd.clone()]);
        let scan = run_scan(&input, &|| false, &|_| {});
        let folders: Vec<&str> = scan.found.iter().map(|f| f.catalog_folder.as_str()).collect();
        assert_eq!(folders, ["Severed-Chains", "2ship2harkinian", "Shipwright", "dusklight"], "Tim's four ports, from the card's top");
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
