//! Finding the ports to import: the folder matching of `fork::adopt` plus what the review list shows
//! for each find (how sure the match is, how big it is, whether a Steam shortcut points into it).
//!
//! Everything here only reads. The Steam shortcuts file is read, never written, and a missing or broken
//! one is reported as a note, not an error.

use crate::core::models::Port;
use crate::fork::adopt::{self, Ambiguous, MatchKind};
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
}

impl Confidence {
    pub fn label(self) -> &'static str {
        match self {
            Confidence::Exact => "exact match",
            Confidence::Likely => "likely match",
            Confidence::CheckThis => "check this",
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
    pub confidence: Confidence,
    pub size: Size,
    pub steam: Vec<SteamRef>,
}

#[derive(Debug, Default)]
pub struct ImportScan {
    pub found: Vec<Found>,
    pub ambiguous: Vec<Ambiguous>,
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
        MatchKind::FolderName => Confidence::Likely,
        MatchKind::ExecutableName => {
            let stems = adopt::program_stems(&candidate.path);
            match adopt::declared_exe_stem(port) {
                Some(declared) if declared.len() >= 4 && stems.contains(&declared) => Confidence::Likely,
                _ => Confidence::CheckThis,
            }
        }
    }
}

pub struct ScanInput {
    pub roots: Vec<PathBuf>,
    pub catalog: Vec<Port>,
    pub library_dir: PathBuf,
    /// The user's home folder: a program found directly inside it is never preselected.
    pub home: PathBuf,
    pub deeper: bool,
    pub shortcuts: ShortcutsRead,
}

/// Matches the folders, then measures each find and looks for Steam shortcuts pointing into it.
/// `progress` gets short status lines; `cancelled` is polled throughout and a cancelled scan keeps what
/// it has (`cancelled` is set on the result).
pub fn run_scan(input: &ScanInput, cancelled: &dyn Fn() -> bool, progress: &dyn Fn(String)) -> ImportScan {
    progress(format!("Looking in {} folder{}...", input.roots.len(), if input.roots.len() == 1 { "" } else { "s" }));
    let raw = adopt::scan_with(&input.roots, &input.catalog, &input.library_dir, input.deeper, cancelled);
    let total = raw.candidates.len();
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
        found.push(Found {
            path: c.path.clone(),
            dir_name: c.dir_name.clone(),
            port_key: c.port_key.clone(),
            port_name: c.port_name.clone(),
            catalog_folder: c.catalog_folder.clone(),
            how: c.how,
            confidence: sure,
            size: dir_size(&c.path, cancelled),
            steam: steam_refs(&input.shortcuts.shortcuts, &c.path),
        });
    }
    found.sort_by_key(|f| f.port_name.to_lowercase());
    ImportScan {
        found,
        ambiguous: raw.ambiguous,
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
            json!({"name": "Dusklight", "folder": "dusklight", "source": "https://github.com/a/dusk", "executable": "dusk"}),
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
mod tests {
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
            &ScanInput { roots: vec![bare.join("Applications")], catalog: catalog(), library_dir: h.library.clone(), home: bare.clone(), deeper: false, shortcuts: read },
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
}
