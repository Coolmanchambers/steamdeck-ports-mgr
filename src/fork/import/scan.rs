//! Finding the ports to import: the folder matching of `fork::adopt` plus what the review list shows
//! for each find (how sure the match is, how big it is, whether a Steam shortcut points into it).
//!
//! Everything here only reads. The Steam shortcuts file is read, never written, and a missing or broken
//! one is reported as a note, not an error.

use crate::core::models::Port;
use super::targets::NewEntry;
use crate::fork::adopt::{self, Ambiguous, Hints, MatchKind};
use crate::fork::artwork;
use crate::fork::settings::ForkSettings;
use crate::fork::shortcuts::{self, Shortcut};
use crate::fork::steam_user;
use crate::fork::store::ShortcutsFile;
use std::fs;
use std::path::{Path, PathBuf};

/// A folder with more entries than this reads as "at least" its measured size.
const MAX_SIZE_ENTRIES: usize = 400_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// The folder is named exactly like the catalog entry's folder.
    Exact,
    /// Named like the port, or the program the catalog names is inside.
    Likely,
    /// Only a program inside is named like the port; the user should look before importing.
    CheckThis,
    /// The user said which port the folder is.
    Chosen,
}

impl Confidence {
    pub fn label(self) -> &'static str {
        match self {
            Confidence::Exact => "exact match",
            Confidence::Likely => "likely match",
            Confidence::CheckThis => "check this",
            Confidence::Chosen => "you chose",
        }
    }

    /// Whether the row starts ticked.
    pub fn preselected(self) -> bool {
        !matches!(self, Confidence::CheckThis)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub bytes: u64,
    pub files: u64,
    /// False when the walk stopped early (too many entries, or the scan was cancelled).
    pub complete: bool,
}

/// A Steam shortcut whose program or start folder is inside an install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamRef {
    pub app_id: Option<u32>,
    pub name: String,
    /// Plain paths, as Steam stores them (no quotes).
    pub exe: String,
    pub start_dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    pub dir_name: String,
    pub port_key: String,
    pub port_name: String,
    pub catalog_folder: String,
    pub how: MatchKind,
    /// Why it matched, in plain words ("folder name", "program soh.appimage", "you chose").
    pub why: String,
    pub confidence: Confidence,
    pub size: Size,
    pub steam: Vec<SteamRef>,
    /// Set when the port is a Deck-ready project the catalog does not have: importing adds it first.
    pub new_entry: Option<NewEntry>,
}

#[derive(Debug, Default)]
pub struct ImportScan {
    pub found: Vec<Found>,
    /// Folders that fit two or three ports: the user chooses.
    pub ambiguous: Vec<Ambiguous>,
    /// Top-level folders with a program that fit no port: the user may choose one.
    pub unrecognised: Vec<PathBuf>,
    /// Folders that look like a collection of many games; never offered.
    pub collections: Vec<PathBuf>,
    /// Folders the user said are not ports (this screen session).
    pub left_alone: Vec<PathBuf>,
    /// Everything a folder can be matched or chosen as (the catalog plus Deck-ready projects it lacks).
    pub ports: Vec<Port>,
    /// What importing writes first for a Deck-ready project, by port key.
    pub new_entries: Vec<(String, NewEntry)>,
    /// The Steam shortcuts read for this scan, to measure a folder the user chooses later.
    pub shortcuts: Vec<Shortcut>,
    pub duplicates: Vec<(String, PathBuf)>,
    pub already_in_library: Vec<String>,
    pub roots_scanned: Vec<PathBuf>,
    pub deep_limit_hit: bool,
    pub cancelled: bool,
    /// Why Steam shortcuts were not checked (no Steam account folder, an unreadable file), if so.
    pub steam_note: Option<String>,
}

/// The shortcuts of the active Steam account, read only.
#[derive(Debug, Clone, Default)]
pub struct ShortcutsRead {
    pub shortcuts: Vec<Shortcut>,
    pub note: Option<String>,
}

/// Reads `shortcuts.vdf` for the Steam account the Add to Steam screen would use.
pub fn read_shortcuts(home: &Path, settings: &ForkSettings) -> ShortcutsRead {
    for root in artwork::existing_steam_roots(home) {
        let Ok(accounts) = steam_user::discover_accounts(&root) else { continue };
        if accounts.is_empty() {
            continue;
        }
        let Some(account) = steam_user::pick_active(&accounts, settings.steam_account()) else { continue };
        return match ShortcutsFile::load(&artwork::shortcuts_path(&account.dir)) {
            Ok(file) => ShortcutsRead { shortcuts: shortcuts::list(&file.doc), note: None },
            Err(e) => ShortcutsRead { shortcuts: Vec::new(), note: Some(format!("Steam shortcuts could not be read ({e}), so none are shown.")) },
        };
    }
    ShortcutsRead { shortcuts: Vec::new(), note: Some("No Steam account folder was found, so Steam shortcuts were not checked.".into()) }
}

/// The shortcuts that point into `install` (program or start folder), by path, not by name. The same
/// rule the move uses to repoint them (`port_mover::steam::moved_path`), so what is shown is what happens.
pub fn steam_refs(shortcuts: &[Shortcut], install: &Path) -> Vec<SteamRef> {
    let inside = |p: &str| crate::fork::port_mover::steam::moved_path(p, install, install).is_some();
    shortcuts
        .iter()
        .filter(|s| inside(s.exe_path()) || inside(s.start_dir_path()))
        .map(|s| SteamRef { app_id: s.app_id, name: s.app_name.clone(), exe: s.exe_path().to_string(), start_dir: s.start_dir_path().to_string() })
        .collect()
}

/// Bytes and files under `dir`, links not followed. Stops at `MAX_SIZE_ENTRIES` entries or when
/// `cancelled` says so (`complete` is then false).
pub fn dir_size(dir: &Path, cancelled: &dyn Fn() -> bool) -> Size {
    let mut size = Size { bytes: 0, files: 0, complete: true };
    let mut stack = vec![dir.to_path_buf()];
    let mut seen = 0usize;
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_SIZE_ENTRIES || (seen % 500 == 0 && cancelled()) {
                size.complete = false;
                return size;
            }
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() {
                size.files += 1;
                size.bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    size
}

fn confidence(candidate: &adopt::Candidate, port: &Port) -> Confidence {
    match candidate.how {
        MatchKind::FolderName if adopt::norm(&candidate.dir_name) == adopt::norm(&port.folder) => Confidence::Exact,
        MatchKind::FolderName | MatchKind::ReleaseName => Confidence::Likely,
        // Only the first word of the folder: trusted when a program is inside, as in every download.
        MatchKind::ReleaseWord if !adopt::programs(&candidate.path).is_empty() => Confidence::Likely,
        MatchKind::ReleaseWord => Confidence::CheckThis,
        MatchKind::ExecutableName if adopt::declared_program_inside(port, &adopt::programs(&candidate.path)) => Confidence::Likely,
        MatchKind::ExecutableName => Confidence::CheckThis,
        MatchKind::Chosen => Confidence::Chosen,
    }
}

/// What the user said a folder is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// This port (`Port::key()`).
    Port(String),
    NotAPort,
}

/// The user's choices for folders, remembered while the Import ports screen is open (a new scan keeps them).
#[derive(Debug, Clone, Default)]
pub struct Choices(Vec<(PathBuf, Choice)>);

impl Choices {
    pub fn set(&mut self, path: &Path, choice: Choice) {
        self.0.retain(|(p, _)| p != path);
        self.0.push((path.to_path_buf(), choice));
    }

    pub fn get(&self, path: &Path) -> Option<&Choice> {
        self.0.iter().find(|(p, _)| p == path).map(|(_, c)| c)
    }
}

/// A folder the user chose a port for, measured like any find: size and the Steam shortcuts pointing in.
/// It then goes through the same plan and checks as every other find.
pub fn measure_choice(path: &Path, port: &Port, shortcuts: &[Shortcut], new_entry: Option<NewEntry>, cancelled: &dyn Fn() -> bool) -> Found {
    let new_name = new_entry.as_ref().map(|n| n.list_name.clone());
    Found {
        path: path.to_path_buf(),
        dir_name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        port_key: port.key().to_string(),
        port_name: new_name.unwrap_or_else(|| port.display_name().to_string()),
        catalog_folder: port.folder.clone(),
        how: MatchKind::Chosen,
        why: "you chose".to_string(),
        confidence: Confidence::Chosen,
        size: dir_size(path, cancelled),
        steam: steam_refs(shortcuts, path),
        new_entry,
    }
}

#[derive(Default)]
pub struct ScanInput {
    pub roots: Vec<PathBuf>,
    pub catalog: Vec<Port>,
    pub library_dir: PathBuf,
    /// The user's home folder: a program found directly inside it is never preselected.
    pub home: PathBuf,
    pub deeper: bool,
    pub shortcuts: ShortcutsRead,
    /// Release file names to match by (from the Deck-ready list).
    pub hints: Hints,
    /// Deck-ready projects in `catalog` that the real catalog lacks, by port key.
    pub new_entries: Vec<(String, NewEntry)>,
    /// What the user already said about folders in this screen session.
    pub choices: Choices,
}

/// Matches the folders, then measures each find and looks for Steam shortcuts pointing into it.
/// `progress` gets short status lines; `cancelled` is polled throughout and a cancelled scan keeps what
/// it has (`cancelled` is set on the result).
pub fn run_scan(input: &ScanInput, cancelled: &dyn Fn() -> bool, progress: &dyn Fn(String)) -> ImportScan {
    progress(format!("Looking in {} folder{}...", input.roots.len(), if input.roots.len() == 1 { "" } else { "s" }));
    let raw = adopt::scan_with_hints(&input.roots, &input.catalog, &input.hints, &input.library_dir, input.deeper, cancelled);
    let new_entry = |key: &str| input.new_entries.iter().find(|(k, _)| k == key).map(|(_, e)| e.clone());
    // The user's earlier choices for folders that need one.
    let mut ambiguous = Vec::new();
    let mut unrecognised = Vec::new();
    let mut left_alone = Vec::new();
    let mut chosen: Vec<(PathBuf, &Port)> = Vec::new();
    let mut decide = |path: &Path| -> bool {
        match input.choices.get(path) {
            Some(Choice::NotAPort) => left_alone.push(path.to_path_buf()),
            Some(Choice::Port(key)) => match input.catalog.iter().find(|p| p.key() == key) {
                Some(port) => chosen.push((path.to_path_buf(), port)),
                None => return false,
            },
            None => return false,
        }
        true
    };
    for a in raw.ambiguous {
        if !decide(&a.path) {
            ambiguous.push(a);
        }
    }
    for u in raw.unrecognised {
        if !decide(&u) {
            unrecognised.push(u);
        }
    }
    let total = raw.candidates.len() + chosen.len();
    let mut found = Vec::with_capacity(total);
    for (i, c) in raw.candidates.iter().enumerate() {
        if cancelled() {
            break;
        }
        progress(format!("Measuring {} ({} of {total})...", c.port_name, i + 1));
        let Some(port) = input.catalog.iter().find(|p| p.key() == c.port_key) else { continue };
        // A match by a program name inside is only trusted one level under a scan folder, and never
        // directly under the home folder (a program in `~/Downloads` does not make `Downloads` a port).
        let same = crate::fork::port_mover::old_copies::same;
        let parent = c.path.parent();
        let under_root = input.roots.iter().any(|r| parent.is_some_and(|p| same(p, r)));
        let under_home = parent.is_some_and(|p| same(p, &input.home));
        let mut sure = confidence(c, port);
        if c.how == MatchKind::ExecutableName && (!under_root || under_home) {
            sure = Confidence::CheckThis;
        }
        let new = new_entry(&c.port_key);
        found.push(Found {
            path: c.path.clone(),
            dir_name: c.dir_name.clone(),
            port_key: c.port_key.clone(),
            port_name: new.as_ref().map_or_else(|| c.port_name.clone(), |n| n.list_name.clone()),
            catalog_folder: c.catalog_folder.clone(),
            how: c.how,
            why: c.why.clone(),
            confidence: sure,
            size: dir_size(&c.path, cancelled),
            steam: steam_refs(&input.shortcuts.shortcuts, &c.path),
            new_entry: new,
        });
    }
    for (i, (path, port)) in chosen.iter().enumerate() {
        if cancelled() {
            break;
        }
        progress(format!("Measuring {} ({} of {total})...", port.display_name(), raw.candidates.len() + i + 1));
        found.push(measure_choice(path, port, &input.shortcuts.shortcuts, new_entry(port.key()), cancelled));
    }
    found.sort_by_key(|f| f.port_name.to_lowercase());
    ImportScan {
        found,
        ambiguous,
        unrecognised,
        collections: raw.collections,
        left_alone,
        ports: input.catalog.clone(),
        new_entries: input.new_entries.clone(),
        shortcuts: input.shortcuts.shortcuts.clone(),
        duplicates: raw.duplicates,
        already_in_library: raw.already_in_library,
        roots_scanned: raw.roots_scanned,
        deep_limit_hit: raw.deep_limit_hit,
        cancelled: cancelled(),
        steam_note: input.shortcuts.note.clone(),
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Shared by the import tests: a catalog, a fake home with Steam, and install folders.
    use crate::core::models::{port_from_value, Port};
    use crate::fork::vdf::fixtures::{file_of, steam_entry};
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};

    pub fn temp_dir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("dpm_import_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    pub fn catalog() -> Vec<Port> {
        [
            json!({"name": "Ship of Harkinian", "folder": "SoH", "source": "https://github.com/a/soh", "executable": {"linux": "soh.appimage", "windows": "soh.exe"}}),
            json!({"name": "Dusklight", "folder": "dusklight", "source": "https://github.com/a/dusklight", "executable": "dusk"}),
            json!({"name": "Severed Chains", "folder": "severed-chains", "source": "https://github.com/a/sc"}),
            json!({"name": "Fan Remake", "folder": "fanremake", "source": "https://github.com/x/fanremake"}),
            json!({"name": "Fan Remake Deluxe", "folder": "Fan-Remake", "source": "https://github.com/x/fanremake2"}),
        ]
        .iter()
        .map(|v| port_from_value(v).unwrap())
        .collect()
    }

    pub fn put_file(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    pub fn put_exe(path: &Path, bytes: &[u8]) {
        put_file(path, bytes);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// A home with `Applications/SoH` (program + a 3000-byte save), `Games/dusk-old` (program only, so
    /// only a program-name match), and a Steam account whose shortcuts file has one hand-made
    /// "Ship of Harkinian" pointing into `Applications/SoH`, one unrelated shortcut and one pointing at
    /// `Applications/SoH2` (a different folder with a similar name).
    pub struct Home {
        pub home: PathBuf,
        pub library: PathBuf,
        pub shortcuts_path: PathBuf,
    }

    pub fn fake_home(name: &str) -> Home {
        let home = temp_dir(name);
        put_exe(&home.join("Applications/SoH/soh.appimage"), &[1u8; 5000]);
        put_file(&home.join("Applications/SoH/saves/file1.sav"), &[2u8; 3000]);
        put_exe(&home.join("Games/dusk-old/dusk"), &[3u8; 700]);
        let steam = home.join(".local/share/Steam");
        let user = steam.join("userdata").join("1234");
        fs::create_dir_all(user.join("config")).unwrap();
        fs::create_dir_all(steam.join("config")).unwrap();
        let soh_exe = format!("\"{}\"", home.join("Applications/SoH/soh.appimage").to_string_lossy());
        let soh_dir = format!("\"{}/\"", home.join("Applications/SoH").to_string_lossy());
        let other_exe = format!("\"{}\"", home.join("Applications/SoH2/run").to_string_lossy());
        let entries = [
            steam_entry(0, 0xAAAA_0001, "Ship of Harkinian", &soh_exe, &soh_dir, &[]),
            steam_entry(1, 0xAAAA_0002, "Something Else", "\"/usr/bin/other\"", "\"/usr/bin/\"", &[]),
            steam_entry(2, 0xAAAA_0003, "Lookalike", &other_exe, "\"/x/\"", &[]),
        ];
        let shortcuts_path = user.join("config").join("shortcuts.vdf");
        fs::write(&shortcuts_path, file_of(&entries)).unwrap();
        let library = home.join("DeckPorts").join("Library");
        fs::create_dir_all(&library).unwrap();
        Home { home, library, shortcuts_path }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::fixtures::*;
    use super::*;
    use std::cell::RefCell;

    fn no_cancel() -> bool {
        false
    }

    fn input(h: &Home, deeper: bool) -> ScanInput {
        let settings = ForkSettings::load(&h.home.join("s.json"));
        ScanInput {
            roots: crate::fork::import::folders::ScanFolders::default().roots(&h.home, &h.library),
            catalog: catalog(),
            library_dir: h.library.clone(),
            home: h.home.clone(),
            deeper,
            shortcuts: read_shortcuts(&h.home, &settings),
            ..Default::default()
        }
    }

    #[test]
    fn finds_show_match_confidence_size_location_and_steam_shortcuts() {
        let h = fake_home("scan_basic");
        let scan = run_scan(&input(&h, false), &no_cancel, &|_| {});
        let by = |name: &str| scan.found.iter().find(|f| f.port_name == name).unwrap();

        let soh = by("Ship of Harkinian");
        assert_eq!(soh.path, h.home.join("Applications/SoH"));
        assert_eq!(soh.confidence, Confidence::Exact);
        assert_eq!(soh.how, MatchKind::FolderName);
        assert_eq!(soh.size, Size { bytes: 8000, files: 2, complete: true });
        assert_eq!(soh.steam.len(), 1, "only the shortcut inside SoH, not the SoH2 look-alike");
        assert_eq!(soh.steam[0].name, "Ship of Harkinian");
        assert_eq!(soh.steam[0].app_id, Some(0xAAAA_0001));
        assert!(Path::new(&soh.steam[0].exe).ends_with("Applications/SoH/soh.appimage"), "quotes are gone: {}", soh.steam[0].exe);

        let dusk = by("Dusklight");
        assert_eq!(dusk.path, h.home.join("Games/dusk-old"));
        assert_eq!(dusk.how, MatchKind::ExecutableName);
        assert_eq!(dusk.confidence, Confidence::Likely, "the program the catalog names is inside");
        assert_eq!(dusk.size.bytes, 700);
        assert!(dusk.steam.is_empty());
        assert!(scan.steam_note.is_none() && !scan.cancelled);
    }

    #[test]
    fn a_match_by_a_program_name_the_catalog_does_not_declare_says_check_this_and_starts_unticked() {
        let h = fake_home("scan_check");
        put_exe(&h.home.join("Games/misc/severedchains.x86_64"), b"x");
        let scan = run_scan(&input(&h, false), &no_cancel, &|_| {});
        let sc = scan.found.iter().find(|f| f.port_name == "Severed Chains").unwrap();
        assert_eq!(sc.confidence, Confidence::CheckThis);
        assert!(!sc.confidence.preselected() && Confidence::Exact.preselected() && Confidence::Likely.preselected());
        assert_eq!(Confidence::CheckThis.label(), "check this");
    }

    #[test]
    fn a_folder_named_like_the_port_but_not_the_catalog_folder_is_a_likely_match() {
        let h = fake_home("scan_likely");
        let games = temp_dir("scan_likely_games");
        put_file(&games.join("Ship of Harkinian/x.bin"), b"x");
        let mut scan_input = input(&h, false);
        scan_input.roots = vec![games];
        let scan = run_scan(&scan_input, &no_cancel, &|_| {});
        assert_eq!(scan.found.len(), 1);
        assert_eq!((scan.found[0].how, scan.found[0].confidence), (MatchKind::FolderName, Confidence::Likely));
    }

    #[test]
    fn no_steam_account_is_a_note_not_an_error() {
        let h = fake_home("scan_nosteam");
        let bare = temp_dir("scan_nosteam_home");
        put_file(&bare.join("Applications/SoH/soh.appimage"), b"x");
        let settings = ForkSettings::load(&bare.join("s.json"));
        let read = read_shortcuts(&bare, &settings);
        assert!(read.shortcuts.is_empty());
        assert!(read.note.as_deref().unwrap().contains("No Steam account folder"));
        let scan = run_scan(
            &ScanInput { roots: vec![bare.join("Applications")], catalog: catalog(), library_dir: h.library.clone(), home: bare.clone(), deeper: false, shortcuts: read, ..Default::default() },
            &no_cancel,
            &|_| {},
        );
        assert_eq!(scan.found.len(), 1);
        assert!(scan.steam_note.is_some());
    }

    #[test]
    fn an_unreadable_shortcuts_file_is_reported_and_the_scan_goes_on() {
        let h = fake_home("scan_badvdf");
        fs::write(&h.shortcuts_path, b"not a vdf file").unwrap();
        let read = read_shortcuts(&h.home, &ForkSettings::load(&h.home.join("s.json")));
        assert!(read.shortcuts.is_empty() && read.note.as_deref().unwrap().contains("could not be read"), "{:?}", read.note);
        assert_eq!(fs::read(&h.shortcuts_path).unwrap(), b"not a vdf file", "read only");
    }

    #[test]
    fn reading_shortcuts_never_changes_the_file() {
        let h = fake_home("scan_readonly");
        let before = fs::read(&h.shortcuts_path).unwrap();
        let read = read_shortcuts(&h.home, &ForkSettings::load(&h.home.join("s.json")));
        assert_eq!(read.shortcuts.len(), 3);
        assert_eq!(fs::read(&h.shortcuts_path).unwrap(), before);
        assert!(fs::read_dir(h.shortcuts_path.parent().unwrap()).unwrap().count() == 1, "no backup or temp file appeared");
    }

    #[test]
    fn a_shortcut_pointing_only_at_the_start_folder_still_counts() {
        let shortcut = |exe: &str, dir: &str| Shortcut {
            key: "0".into(),
            app_id: None,
            app_name: "x".into(),
            exe: format!("\"{exe}\""),
            start_dir: format!("\"{dir}\""),
            icon: String::new(),
            launch_options: String::new(),
            shortcut_path: String::new(),
            tags: vec![],
        };
        let list = [shortcut("/usr/bin/env", "/home/d/Applications/SoH/"), shortcut("/home/d/Applications/SoHX/run", "/home/d/Applications/SoHX/")];
        let refs = steam_refs(&list, Path::new("/home/d/Applications/SoH"));
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].app_id, None);
    }

    #[test]
    fn look_deeper_finds_a_port_one_folder_further_in_and_normal_scans_do_not() {
        let h = fake_home("scan_deeper");
        put_file(&h.home.join("Games/collection/SoH/x"), b"1");
        let plain = run_scan(&input(&h, false), &no_cancel, &|_| {});
        assert!(!plain.found.iter().any(|f| f.path.ends_with("collection/SoH")));
        let deep = run_scan(&input(&h, true), &no_cancel, &|_| {});
        let paths: Vec<_> = deep.found.iter().map(|f| f.path.clone()).collect();
        // Applications/SoH was already offered, so the deeper SoH is the second folder for that port.
        assert!(paths.contains(&h.home.join("Applications/SoH")));
        assert!(deep.duplicates.iter().any(|(_, p)| p.ends_with("collection/SoH")), "{:?}", deep.duplicates);
        assert!(!deep.deep_limit_hit);
    }

    #[test]
    fn look_deeper_is_bounded() {
        let h = fake_home("scan_deeper_cap");
        for i in 0..(adopt::MAX_DEEP_FOLDERS + 20) {
            fs::create_dir_all(h.home.join("Games/many").join(format!("d{i:04}"))).unwrap();
        }
        let deep = run_scan(&input(&h, true), &no_cancel, &|_| {});
        assert!(deep.deep_limit_hit);
    }

    #[test]
    fn a_cancelled_scan_stops_and_says_so() {
        let h = fake_home("scan_cancel");
        let scan = run_scan(&input(&h, false), &|| true, &|_| {});
        assert!(scan.cancelled);
        assert!(scan.found.is_empty());
    }

    #[test]
    fn size_counts_files_not_links_and_cuts_off_when_cancelled() {
        let dir = temp_dir("size");
        for i in 0..1200 {
            put_file(&dir.join(format!("f{i}")), b"ab");
        }
        assert_eq!(dir_size(&dir, &no_cancel), Size { bytes: 2400, files: 1200, complete: true });
        let cut = dir_size(&dir, &|| true);
        assert!(!cut.complete && cut.files < 1200);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&dir, dir.join("loop")).unwrap();
            assert_eq!(dir_size(&dir, &no_cancel).files, 1200, "a link back up the tree is not followed");
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn progress_names_what_is_being_measured() {
        let h = fake_home("scan_progress");
        let lines = RefCell::new(Vec::new());
        run_scan(&input(&h, false), &no_cancel, &|l| lines.borrow_mut().push(l));
        let lines = lines.into_inner();
        assert_eq!(lines[0], "Looking in 2 folders...");
        assert!(lines.iter().any(|l| l.starts_with("Measuring Ship of Harkinian (")), "{lines:?}");
    }

    #[test]
    fn a_program_name_match_is_never_preselected_directly_under_the_home_folder_or_below_the_first_level() {
        let h = fake_home("scan_exe_rules");
        // A program named like Dusklight's sits in ~/Downloads, and another two folders into ~/Games.
        put_exe(&h.home.join("Downloads/dusk"), b"x");
        put_exe(&h.home.join("Games/collection/inner/dusk"), b"x");
        let mut scan_input = input(&h, true);
        scan_input.roots = vec![h.home.clone()];
        let scan = run_scan(&scan_input, &no_cancel, &|_| {});
        let dusk: Vec<&Found> = scan.found.iter().filter(|f| f.port_name == "Dusklight").collect();
        assert!(!dusk.is_empty());
        assert!(dusk.iter().all(|f| f.confidence == Confidence::CheckThis), "{:?}", dusk.iter().map(|f| (&f.path, f.confidence)).collect::<Vec<_>>());
        // The same program one level under an ordinary scan folder keeps its "likely" match.
        let normal = run_scan(&input(&h, false), &no_cancel, &|_| {});
        let ordinary = normal.found.iter().find(|f| f.port_name == "Dusklight").unwrap();
        assert_eq!(ordinary.confidence, Confidence::Likely);
    }

    #[test]
    fn a_folder_match_below_the_first_level_is_still_preselected() {
        let h = fake_home("scan_folder_deep");
        put_file(&h.home.join("Games/collection/severed-chains/x"), b"x");
        let scan = run_scan(&input(&h, true), &no_cancel, &|_| {});
        let sc = scan.found.iter().find(|f| f.port_name == "Severed Chains").unwrap();
        assert_eq!((sc.how, sc.confidence), (MatchKind::FolderName, Confidence::Exact));
    }

    /// The import scan over Tim's SD card with his catalog (no Severed Chains) and the Deck-ready list.
    pub(crate) fn tims_input(sd: &Path, choices: Choices) -> ScanInput {
        use crate::fork::import::targets::{self, tests::{tims_catalog, tims_index}};
        let library = sd.join("Ports");
        let t = targets::build(&tims_catalog(), Some(&tims_index()), &library);
        ScanInput {
            roots: vec![sd.to_path_buf()],
            catalog: t.catalog,
            library_dir: library,
            home: sd.parent().unwrap().join("home"),
            hints: t.hints,
            new_entries: t.new_entries,
            choices,
            ..Default::default()
        }
    }

    #[test]
    fn tims_sd_card_gives_his_four_ports_ticked_and_nothing_else() {
        let dir = temp_dir("tims_sd_scan");
        let sd = crate::fork::adopt::test_fixtures::tims_sd_card(&dir);
        let scan = run_scan(&tims_input(&sd, Choices::default()), &no_cancel, &|_| {});
        let rows: Vec<(&str, &str, Confidence)> = scan.found.iter().map(|f| (f.port_name.as_str(), f.dir_name.as_str(), f.confidence)).collect();
        assert_eq!(
            rows,
            [
                ("The Legend of Dragoon (Severed-Chains)", "Severed_Chains_Steam_Deck", Confidence::Likely),
                ("The Legend of Zelda: Majora's Mask", "2Ship-Keiichi-Charlie-Linux", Confidence::Likely),
                ("The Legend of Zelda: Ocarina of Time", "SoH-Ackbar-Delta-Linux", Confidence::Likely),
                ("The Legend of Zelda: Twilight Princess", "Dusklight-twilight-princess", Confidence::Likely),
            ]
        );
        assert!(scan.found.iter().all(|f| f.confidence.preselected() && !f.why.is_empty()));
        let keys: Vec<&str> = scan.found.iter().map(|f| f.catalog_folder.as_str()).collect();
        assert_eq!(keys, ["Severed-Chains", "2ship2harkinian", "Shipwright", "dusklight"]);
        let severed = &scan.found[0];
        let new = severed.new_entry.as_ref().expect("Severed Chains is added to the port list first");
        assert_eq!(new.value["folder"], "Severed-Chains");
        assert!(scan.found[1..].iter().all(|f| f.new_entry.is_none()));
        assert!(scan.ambiguous.is_empty() && scan.unrecognised.is_empty() && scan.collections.is_empty(), "{:?} {:?} {:?}", scan.ambiguous, scan.unrecognised, scan.collections);
        assert!(scan.duplicates.is_empty() && scan.already_in_library.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn without_the_deck_ready_list_tims_three_catalog_ports_still_match() {
        let dir = temp_dir("tims_sd_offline");
        let sd = crate::fork::adopt::test_fixtures::tims_sd_card(&dir);
        let input = ScanInput {
            roots: vec![sd.clone()],
            catalog: crate::fork::import::targets::tests::tims_catalog(),
            library_dir: sd.join("Ports"),
            home: dir.join("home"),
            ..Default::default()
        };
        let scan = run_scan(&input, &no_cancel, &|_| {});
        let folders: Vec<&str> = scan.found.iter().map(|f| f.catalog_folder.as_str()).collect();
        assert_eq!(folders, ["2ship2harkinian", "Shipwright", "dusklight"]);
        assert!(scan.found.iter().all(|f| f.confidence.preselected()));
        // Severed Chains is unknown without the list, but its folder holds a program, so it can be chosen.
        assert_eq!(scan.unrecognised, [sd.join("Severed_Chains_Steam_Deck")]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_choice_made_earlier_in_the_screen_survives_a_new_scan() {
        let dir = temp_dir("choices_rescan");
        let sd = crate::fork::adopt::test_fixtures::tims_sd_card(&dir);
        let mystery = sd.join("Mystery Game");
        let other = sd.join("Other Thing");
        crate::fork::adopt::test_fixtures::touch_exe(&mystery.join("game.x86_64"));
        crate::fork::adopt::test_fixtures::touch_exe(&other.join("tool.sh"));
        let before = run_scan(&tims_input(&sd, Choices::default()), &no_cancel, &|_| {});
        assert_eq!(before.unrecognised, [mystery.clone(), other.clone()]);
        let mut choices = Choices::default();
        choices.set(&mystery, Choice::Port("Zelda64Recomp/Zelda64Recomp".into()));
        choices.set(&other, Choice::NotAPort);
        let after = run_scan(&tims_input(&sd, choices), &no_cancel, &|_| {});
        assert!(after.unrecognised.is_empty());
        assert_eq!(after.left_alone, [other]);
        let chosen = after.found.iter().find(|f| f.path == mystery).unwrap();
        assert_eq!((chosen.how, chosen.confidence, chosen.why.as_str()), (MatchKind::Chosen, Confidence::Chosen, "you chose"));
        assert_eq!(chosen.catalog_folder, "Zelda64Recomp");
        assert!(chosen.confidence.preselected());
        assert_eq!(Confidence::Chosen.label(), "you chose");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn choices_replace_each_other_per_folder() {
        let mut c = Choices::default();
        c.set(Path::new("/a"), Choice::NotAPort);
        c.set(Path::new("/a"), Choice::Port("k".into()));
        assert_eq!(c.get(Path::new("/a")), Some(&Choice::Port("k".into())));
        assert_eq!(c.get(Path::new("/b")), None);
    }

    /// The same card against the real catalog (`ports.json`) and Deck-ready list (`index/deck-ready.json`) in
    /// this repository: a catalog or list change that would stop Tim's four ports from matching fails here.
    #[test]
    fn tims_sd_card_matches_with_the_real_catalog_and_deck_ready_list() {
        use crate::fork::import::targets;
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let catalog = crate::core::config::load_config(&root.join("ports.json")).unwrap();
        let index = crate::fork::deck_ready::index::parse(&fs::read_to_string(root.join("index/deck-ready.json")).unwrap()).unwrap();
        assert!(!catalog.iter().any(|p| p.folder == "Severed-Chains"), "upstream disables Severed Chains; if it comes back, update this test");
        let dir = temp_dir("tims_sd_real");
        let sd = crate::fork::adopt::test_fixtures::tims_sd_card(&dir);
        let library = sd.join("Ports");
        let t = targets::build(&catalog, Some(&index), &library);
        let input = ScanInput {
            roots: vec![sd.clone()],
            catalog: t.catalog,
            library_dir: library,
            home: dir.join("home"),
            hints: t.hints,
            new_entries: t.new_entries,
            ..Default::default()
        };
        let scan = run_scan(&input, &no_cancel, &|_| {});
        let mut rows: Vec<(&str, &str)> = scan.found.iter().map(|f| (f.dir_name.as_str(), f.catalog_folder.as_str())).collect();
        rows.sort();
        assert_eq!(
            rows,
            [
                ("2Ship-Keiichi-Charlie-Linux", "2ship2harkinian"),
                ("Dusklight-twilight-princess", "dusklight"),
                ("Severed_Chains_Steam_Deck", "Severed-Chains"),
                ("SoH-Ackbar-Delta-Linux", "Shipwright"),
            ],
            "{:?}",
            scan.found.iter().map(|f| (&f.dir_name, &f.port_name, f.how, &f.why)).collect::<Vec<_>>()
        );
        assert!(scan.found.iter().all(|f| f.confidence.preselected()), "{:?}", scan.found.iter().map(|f| (&f.dir_name, f.confidence)).collect::<Vec<_>>());
        assert!(scan.found.iter().find(|f| f.catalog_folder == "Severed-Chains").unwrap().new_entry.is_some());
        assert!(scan.ambiguous.is_empty() && scan.unrecognised.is_empty() && scan.collections.is_empty(), "{:?} {:?} {:?}", scan.ambiguous, scan.unrecognised, scan.collections);
        let _ = fs::remove_dir_all(dir);
    }

    /// Everyday folders next to ports never match a real catalog or Deck-ready entry by their name.
    #[test]
    fn everyday_folder_names_do_not_match_the_real_catalog() {
        use crate::fork::import::targets;
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let catalog = crate::core::config::load_config(&root.join("ports.json")).unwrap();
        let index = crate::fork::deck_ready::index::parse(&fs::read_to_string(root.join("index/deck-ready.json")).unwrap()).unwrap();
        let dir = temp_dir("everyday_real");
        let names = [
            "Downloads", "Documents", "Music", "Pictures", "Videos", "Desktop", "Games", "roms", "ROMs", "bios", "Saves", "backups", "AppImages",
            "Heroic", "Lutris", "PortMaster", "retrodeck", "Steam-Screenshots", "Mods", "Tools", "Install", "New Folder", "Old-Stuff-2024",
            "Super-Backup", "The-Files", "Game-Saves", "Linux-ISOs", "Ports-old",
        ];
        for name in names {
            put_file(&dir.join("sd").join(name).join("notes.txt"), b"x");
        }
        let t = targets::build(&catalog, Some(&index), &dir.join("sd/Ports"));
        let input = ScanInput { roots: vec![dir.join("sd")], catalog: t.catalog, library_dir: dir.join("sd/Ports"), home: dir.join("home"), hints: t.hints, new_entries: t.new_entries, ..Default::default() };
        let scan = run_scan(&input, &no_cancel, &|_| {});
        assert!(scan.found.is_empty(), "{:?}", scan.found.iter().map(|f| (&f.dir_name, &f.port_name, &f.why)).collect::<Vec<_>>());
        assert!(scan.ambiguous.is_empty(), "{:?}", scan.ambiguous);
        let _ = fs::remove_dir_all(dir);
    }
}
