//! The picker's state: where it is, what it lists, and which row is which.

use super::mounts::{self, Drive};
use super::{nearest_existing_folder, Env, Purpose};
use crate::fork::rom_screens::util::{ellipsize, human_bytes};
use std::fs;
use std::path::{Path, PathBuf};

/// More sub-folders than this in one folder are cut off with a note.
pub const MAX_FOLDERS: usize = 500;

/// Common folders offered under the home folder when they exist.
const COMMON_FOLDERS: [(&str, &str); 2] = [("Games", "Games"), ("Applications", "Applications")];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub label: String,
    pub path: PathBuf,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// A section title; not selectable.
    Heading(&'static str),
    Place(usize),
    UseThis,
    NewFolder,
    Up,
    Folder(usize),
    /// "Show 3 hidden folders" / "Hide hidden folders".
    ToggleHidden,
    /// "... and N more folders not shown"; not selectable.
    More,
    Cancel,
}

impl Row {
    pub fn selectable(self) -> bool {
        !matches!(self, Row::Heading(_) | Row::More)
    }
}

/// One line as the screen draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowView {
    pub label: String,
    pub detail: String,
    pub heading: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
struct Folder {
    name: String,
    link: bool,
}

pub struct Picker {
    pub purpose: Purpose,
    pub current: PathBuf,
    pub show_hidden: bool,
    places: Vec<Place>,
    folders: Vec<Folder>,
    hidden_count: usize,
    cut_off: usize,
    pub error: Option<String>,
    env: Env,
    /// Places the caller adds (the Game files screen offers ROM folders), shown after the built-in ones.
    extra: Vec<(String, PathBuf)>,
    /// Show `extra` before the built-in places.
    extra_first: bool,
    /// Whether a folder could be written to, so redrawing does not create a test file each time.
    write_cache: std::cell::RefCell<std::collections::HashMap<PathBuf, bool>>,
}

fn name_key(name: &str) -> (String, String) {
    (name.to_lowercase(), name.to_string())
}

impl Picker {
    pub fn open(start: &Path, purpose: Purpose, env: Env) -> Picker {
        Picker::open_with(start, purpose, env, Vec::new())
    }

    /// `extra` places (label, folder) are listed after the drives and home folders when they exist.
    pub fn open_with(start: &Path, purpose: Purpose, env: Env, extra: Vec<(String, PathBuf)>) -> Picker {
        let current = nearest_existing_folder(start)
            .or_else(|| env.home.clone())
            .unwrap_or_else(|| PathBuf::from("/"));
        let mut picker = Picker { purpose, current, show_hidden: false, places: Vec::new(), folders: Vec::new(), hidden_count: 0, cut_off: 0, error: None, env, extra, extra_first: false, write_cache: Default::default() };
        picker.places = picker.find_places();
        picker.reload();
        picker
    }

    fn drives(&self) -> Vec<Drive> {
        mounts::removable_drives(&self.env.mounts, &self.env.media_roots)
    }

    /// The caller's places go first (the game files folder: "SD card: Emulation/roms (Recommended)" on top).
    pub fn put_extra_first(&mut self) {
        self.extra_first = true;
        self.places = self.find_places();
        self.reload();
    }

    /// The folder in use as a place: "SD card > Emulation > roms", never a card's ID or a bare "~".
    pub fn breadcrumb(&self) -> String {
        crate::fork::rom_screens::util::ellipsize(&mounts::folder_label(&self.current, &self.drives(), self.env.home.as_deref()), 58)
    }

    fn free_text(&self, path: &Path) -> Option<String> {
        (self.env.free_space)(path).map(|b| format!("{} free", human_bytes(b)))
    }

    fn find_places(&self) -> Vec<Place> {
        let mut out: Vec<Place> = Vec::new();
        for drive in self.drives() {
            if drive.mount_point.is_dir() {
                let mut detail = self.free_text(&drive.mount_point).unwrap_or_default();
                if mounts::is_non_linux_fs(&drive.fs_type) {
                    let note = if self.purpose.needs_write() { format!("{}: can't hold ports", drive.fs_type) } else { drive.fs_type.clone() };
                    detail = if detail.is_empty() { note } else { format!("{detail}, {note}") };
                }
                out.push(Place { label: drive.title(), path: drive.mount_point.clone(), detail });
            }
        }
        if let Some(home) = self.env.home.clone().filter(|h| h.is_dir()) {
            out.push(Place { label: "Home".into(), detail: self.free_text(&home).unwrap_or_default(), path: home.clone() });
            for (label, sub) in COMMON_FOLDERS {
                let path = home.join(sub);
                if path.is_dir() {
                    out.push(Place { label: label.into(), detail: format!("~/{sub}"), path });
                }
            }
        }
        let mut extra: Vec<Place> = Vec::new();
        for (label, path) in &self.extra {
            if path.is_dir() && !out.iter().chain(extra.iter()).any(|p| p.path == *path) {
                // "... (Recommended)" moves to the detail column, so a narrow window never cuts it off.
                let (label, detail) = label.strip_suffix(" (Recommended)").map_or((label.clone(), String::new()), |l| (l.to_string(), "Recommended".to_string()));
                extra.push(Place { label, detail, path: path.clone() });
            }
        }
        if self.extra_first {
            // The caller's places first; a built-in place they repeat (a whole card) is not listed twice.
            out.retain(|p| !extra.iter().any(|e| e.path == p.path));
            extra.extend(out);
            return extra;
        }
        out.extend(extra);
        out
    }

    pub fn places(&self) -> &[Place] {
        &self.places
    }

    /// Lists the current folder again (after a change on disk, or a new folder).
    pub fn reload(&mut self) {
        self.folders.clear();
        self.hidden_count = 0;
        self.cut_off = 0;
        self.error = None;
        let entries = match fs::read_dir(&self.current) {
            Ok(entries) => entries,
            Err(e) => {
                self.error = Some(format!("Could not open this folder: {e}"));
                return;
            }
        };
        let mut folders: Vec<Folder> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            // `is_dir` follows links, so a linked folder counts.
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                self.hidden_count += 1;
                if !self.show_hidden {
                    continue;
                }
            }
            let link = entry.file_type().is_ok_and(|t| t.is_symlink());
            folders.push(Folder { name, link });
        }
        folders.sort_by_key(|f| name_key(&f.name));
        if folders.len() > MAX_FOLDERS {
            self.cut_off = folders.len() - MAX_FOLDERS;
            folders.truncate(MAX_FOLDERS);
        }
        self.folders = folders;
    }

    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if !self.places.is_empty() {
            rows.push(Row::Heading("Places"));
            rows.extend((0..self.places.len()).map(Row::Place));
        }
        rows.push(Row::Heading("This folder"));
        rows.push(Row::UseThis);
        rows.push(Row::NewFolder);
        if self.current.parent().is_some() {
            rows.push(Row::Up);
        }
        rows.extend((0..self.folders.len()).map(Row::Folder));
        if self.cut_off > 0 {
            rows.push(Row::More);
        }
        if self.hidden_count > 0 {
            rows.push(Row::ToggleHidden);
        }
        rows.push(Row::Cancel);
        rows
    }

    pub fn row_views(&self) -> Vec<RowView> {
        let writable = self.can_choose().is_ok();
        self.rows()
            .into_iter()
            .map(|row| {
                let (label, detail) = match row {
                    Row::Heading(title) => (title.to_string(), String::new()),
                    Row::Place(i) => (self.places[i].label.clone(), self.places[i].detail.clone()),
                    Row::UseThis => ("Use this folder".to_string(), if writable { String::new() } else { "can't be used".to_string() }),
                    Row::NewFolder => ("New folder here...".to_string(), String::new()),
                    Row::Up => (".. Up one level".to_string(), String::new()),
                    Row::Folder(i) => {
                        let f = &self.folders[i];
                        (format!("{}/", ellipsize(&f.name, 70)), if f.link { "link".to_string() } else { String::new() })
                    }
                    Row::ToggleHidden if self.show_hidden => ("Hide hidden folders".to_string(), String::new()),
                    Row::ToggleHidden => (format!("Show hidden folders ({})", self.hidden_count), String::new()),
                    Row::More => (format!("... and {} more folders not shown", self.cut_off), String::new()),
                    Row::Cancel => ("Cancel".to_string(), String::new()),
                };
                RowView { label, detail, heading: matches!(row, Row::Heading(_)), enabled: row.selectable() }
            })
            .collect()
    }

    /// The header's second line: free space, file system and drive of the current folder.
    pub fn info_line(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(free) = self.free_text(&self.current) {
            parts.push(free);
        }
        if let Some(fs_type) = mounts::fs_type_of(&self.current, &self.env.mounts) {
            parts.push(mounts::fs_label(&fs_type));
        }
        if let Some(drive) = mounts::drive_of(&self.current, &self.env.mounts, &self.env.media_roots) {
            parts.push(drive.title());
        } else if self.env.home.as_deref().is_some_and(|h| crate::core::path_safety::is_within(&self.current, h)) {
            parts.push("internal storage".into());
        }
        parts.join(", ")
    }

    /// The header's third line: an error, or the purpose's hint.
    pub fn hint_line(&self) -> String {
        if let Some(e) = &self.error {
            return e.clone();
        }
        if let Err(why) = self.can_choose() {
            return why;
        }
        self.purpose.hint().to_string()
    }

    /// Whether "Use this folder" may pick the current folder.
    pub fn can_choose(&self) -> Result<(), String> {
        if self.error.is_some() {
            return Err("This folder can't be opened, so it can't be used.".into());
        }
        if self.purpose.needs_write() {
            if let Some(fs_type) = mounts::fs_type_of(&self.current, &self.env.mounts).filter(|t| mounts::is_non_linux_fs(t)) {
                return Err(format!("This drive is {}, which can't store Linux programs, so it can't hold ports.", crate::fork::ports_folder::fs_display_name(&fs_type)));
            }
            if !self.writable_cached(&self.current) {
                return Err("You can't write to this folder. Pick or create a folder you own.".into());
            }
        }
        Ok(())
    }

    fn writable_cached(&self, dir: &Path) -> bool {
        *self.write_cache.borrow_mut().entry(dir.to_path_buf()).or_insert_with(|| writable(dir))
    }

    /// Opens a sub-folder; returns false when there is no such row.
    pub fn enter(&mut self, index: usize) -> bool {
        let Some(folder) = self.folders.get(index) else { return false };
        self.current = self.current.join(&folder.name);
        self.reload();
        true
    }

    /// Goes up one level. Returns the folder it came from, so the screen can select it.
    pub fn up(&mut self) -> Option<String> {
        let parent = self.current.parent()?.to_path_buf();
        let came_from = self.current.file_name().map(|n| n.to_string_lossy().into_owned());
        self.current = parent;
        self.reload();
        came_from
    }

    pub fn go_to(&mut self, path: &Path) {
        self.current = nearest_existing_folder(path).unwrap_or_else(|| path.to_path_buf());
        self.places = self.find_places();
        self.reload();
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.reload();
    }

    /// The row index of a listed sub-folder, by name.
    pub fn row_of_folder(&self, name: &str) -> Option<usize> {
        let i = self.folders.iter().position(|f| f.name == name)?;
        self.rows().iter().position(|r| *r == Row::Folder(i))
    }

    pub fn row_of(&self, wanted: Row) -> Option<usize> {
        self.rows().iter().position(|r| *r == wanted)
    }

    /// Where the selection goes after opening a folder: its first sub-folder, else "Use this folder".
    pub fn default_row(&self) -> usize {
        self.row_of(Row::Folder(0)).or_else(|| self.row_of(Row::UseThis)).unwrap_or(0)
    }

    /// The next selectable row from `current` going `delta` rows, skipping headings; stays put at the ends.
    pub fn step(&self, current: usize, delta: i32) -> usize {
        let rows = self.rows();
        if rows.is_empty() || delta == 0 {
            return current;
        }
        let mut i = current as i64;
        loop {
            i += delta.signum() as i64;
            if i < 0 || i >= rows.len() as i64 {
                return current;
            }
            if rows[i as usize].selectable() {
                return i as usize;
            }
        }
    }
}

/// Whether files can be created here, by trying (permissions alone miss read-only mounts).
pub fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".dpm-write-check-{}", std::process::id()));
    match fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(e) => e.kind() == std::io::ErrorKind::AlreadyExists,
    }
}

#[cfg(test)]
mod tests {
    use super::super::mounts::parse_mounts;
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("dpm_picker_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn free(_: &Path) -> Option<u64> {
        Some(391_000_000_000)
    }

    /// A fake machine: home with Games, a "media root" holding a mounted SD card and a stale folder.
    fn env(root: &Path) -> Env {
        let home = root.join("home");
        let media = root.join("media");
        for d in ["home/Games", "home/Documents", "media/SD512/Ports", "media/stale"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        let mounts = format!("/dev/mmcblk0p1 {} ext4 rw 0 0\n/dev/sda1 {} exfat rw 0 0\n", media.join("SD512").display(), media.join("gone").display());
        Env { home: Some(home), mounts: parse_mounts(&mounts), media_roots: vec![media], free_space: free }
    }

    #[test]
    fn places_list_mounted_drives_then_home_and_common_folders() {
        let root = temp("places");
        let picker = Picker::open(&root.join("home"), Purpose::Any, env(&root));
        let labels: Vec<(&str, &str)> = picker.places().iter().map(|p| (p.label.as_str(), p.detail.as_str())).collect();
        // The stale folder is not mounted and the exFAT mount's folder does not exist: neither is offered.
        assert_eq!(labels, [("SD card (SD512)", "391.0 GB free"), ("Home", "391.0 GB free"), ("Games", "~/Games")]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn extra_places_from_the_caller_follow_the_built_in_ones_when_they_exist() {
        let root = temp("extra");
        let roms = root.join("home/Emulation/roms");
        fs::create_dir_all(&roms).unwrap();
        let extra = vec![("Home/Emulation/roms".to_string(), roms.clone()), ("Missing".to_string(), root.join("nope")), ("Home again".to_string(), root.join("home"))];
        let picker = Picker::open_with(&root.join("home"), Purpose::ScanFolder, env(&root), extra);
        let labels: Vec<&str> = picker.places().iter().map(|p| p.label.as_str()).collect();
        assert_eq!(labels, ["SD card (SD512)", "Home", "Games", "Home/Emulation/roms"]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn folders_are_sorted_hidden_ones_on_request_and_files_left_out() {
        let root = temp("listing");
        let dir = root.join("home");
        let mut e = env(&root);
        e.mounts.clear();
        for d in ["zeta", "Alpha", "beta", ".config", ".cache"] {
            fs::create_dir_all(dir.join(d)).unwrap();
        }
        fs::write(dir.join("notes.txt"), b"x").unwrap();
        let mut picker = Picker::open(&dir, Purpose::Any, e);
        let names: Vec<String> = picker.row_views().into_iter().filter(|r| r.label.ends_with('/')).map(|r| r.label).collect();
        assert_eq!(names, ["Alpha/", "beta/", "Documents/", "Games/", "zeta/"]);
        assert!(picker.row_views().iter().any(|r| r.label == "Show hidden folders (2)"));
        picker.toggle_hidden();
        let names: Vec<String> = picker.row_views().into_iter().filter(|r| r.label.ends_with('/')).map(|r| r.label).collect();
        assert_eq!(names[..2], [".cache/".to_string(), ".config/".to_string()]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn browsing_down_and_up_selects_where_it_came_from() {
        let root = temp("browse");
        let mut picker = Picker::open(&root.join("home"), Purpose::Any, env(&root));
        let games = picker.rows().iter().position(|r| *r == Row::Folder(1)).unwrap();
        assert_eq!(picker.row_views()[games].label, "Games/");
        assert!(picker.enter(1));
        assert_eq!(picker.current, root.join("home").join("Games"));
        assert_eq!(picker.default_row(), picker.row_of(Row::UseThis).unwrap(), "an empty folder selects Use this folder");
        let came_from = picker.up().unwrap();
        assert_eq!(came_from, "Games");
        assert_eq!(picker.row_of_folder(&came_from), Some(games));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_selection_skips_headings_and_stops_at_the_ends() {
        let root = temp("step");
        let picker = Picker::open(&root.join("home"), Purpose::Any, env(&root));
        let rows = picker.rows();
        assert_eq!(rows[0], Row::Heading("Places"));
        let last_place = rows.iter().rposition(|r| matches!(r, Row::Place(_))).unwrap();
        let use_this = picker.row_of(Row::UseThis).unwrap();
        assert_eq!(picker.step(last_place, 1), use_this, "the heading between is skipped");
        assert_eq!(picker.step(use_this, -1), last_place);
        assert_eq!(picker.step(1, -1), 1, "nothing selectable above the first place");
        let last = rows.len() - 1;
        assert_eq!(rows[last], Row::Cancel);
        assert_eq!(picker.step(last, 1), last);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn an_exfat_drive_is_marked_and_cannot_be_chosen_as_a_ports_folder() {
        let root = temp("exfat");
        let mut e = env(&root);
        let stick = root.join("media").join("STICK");
        fs::create_dir_all(stick.join("Ports")).unwrap();
        e.mounts.push(super::super::mounts::MountEntry { device: "/dev/sdc1".into(), mount_point: stick.clone(), fs_type: "exfat".into() });
        let picker = Picker::open(&stick.join("Ports"), Purpose::PortsFolder, e);
        let place = picker.places().iter().find(|p| p.label == "USB drive (STICK)").unwrap();
        assert_eq!(place.detail, "391.0 GB free, exfat: can't hold ports");
        let why = picker.can_choose().unwrap_err();
        assert!(why.contains("exFAT") && why.contains("can't hold ports"), "{why}");
        assert_eq!(picker.hint_line(), why, "the header says why");
        // For a scan folder it is fine.
        let mut e = env(&root);
        e.mounts.push(super::super::mounts::MountEntry { device: "/dev/sdc1".into(), mount_point: stick.clone(), fs_type: "exfat".into() });
        assert!(Picker::open(&stick.join("Ports"), Purpose::ScanFolder, e).can_choose().is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_header_names_free_space_file_system_and_drive() {
        let root = temp("header");
        let e = env(&root);
        let sd = root.join("media").join("SD512");
        let picker = Picker::open(&sd.join("Ports"), Purpose::PortsFolder, e);
        assert_eq!(picker.info_line(), "391.0 GB free, Linux format (ext4), SD card (SD512)");
        assert_eq!(picker.hint_line(), Purpose::PortsFolder.hint());
        let home = Picker::open(&root.join("home"), Purpose::Any, env(&root));
        assert!(home.info_line().ends_with("internal storage"), "{}", home.info_line());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_missing_start_opens_its_nearest_parent_and_an_unreadable_one_says_so() {
        let root = temp("missing");
        let picker = Picker::open(&root.join("home/Games/Not/There"), Purpose::Any, env(&root));
        assert_eq!(picker.current, root.join("home/Games"));
        let mut gone = Picker::open(&root.join("home"), Purpose::Any, env(&root));
        gone.current = root.join("vanished");
        gone.reload();
        assert!(gone.error.is_some() && gone.can_choose().is_err());
        assert!(gone.hint_line().starts_with("Could not open"));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_folder_cannot_be_chosen_as_a_ports_folder() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp("readonly");
        let ro = root.join("home/ro");
        fs::create_dir_all(&ro).unwrap();
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).unwrap();
        let picker = Picker::open(&ro, Purpose::PortsFolder, env(&root));
        // Root can write anywhere; the check only means something for a normal user.
        if !writable(&ro) {
            assert!(picker.can_choose().is_err());
            assert_eq!(picker.row_views().iter().find(|r| r.label == "Use this folder").unwrap().detail, "can't be used");
        }
        assert!(Picker::open(&ro, Purpose::ScanFolder, env(&root)).can_choose().is_ok());
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o755)).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_huge_folder_is_cut_off_with_a_note() {
        let root = temp("huge");
        let dir = root.join("big");
        for i in 0..(MAX_FOLDERS + 3) {
            fs::create_dir_all(dir.join(format!("d{i:04}"))).unwrap();
        }
        let mut e = env(&root);
        e.mounts.clear();
        let picker = Picker::open(&dir, Purpose::Any, e);
        assert!(picker.row_views().iter().any(|r| r.label == "... and 3 more folders not shown" && !r.enabled));
        let _ = fs::remove_dir_all(root);
    }
}
