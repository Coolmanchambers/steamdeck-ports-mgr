//! Working out, and then doing, what "Add to Steam" changes in `shortcuts.vdf`.
//!
//! `wanted_from_port` turns an installed port into the shortcut we would want for it. `plan` compares
//! those with the file and says, per port, whether it would be added, updated in place, already fine,
//! or blocked and why. `apply` makes the changes in memory and saves through `ShortcutsFile::save`,
//! which refuses while Steam runs, backs up first and writes atomically.
//!
//! An existing shortcut counts as the port's when it points at the same program, or else has the same
//! name (ignoring case). Either way it is updated in place, so its app ID, play time, artwork and
//! controller layout stay. Two or more equally good matches block the port: nothing is guessed.

use super::shortcuts::{self, Added, NewShortcut, Shortcut, ShortcutUpdate, Target, Updated};
use super::store::{check_steam, SaveReport, ShortcutsFile, SteamProcessCheck, StoreError};
use super::vdf::Document;
use crate::core::executable_detect::{resolve_executable, ExecutableSelectionError};
use crate::core::models::Port;
use crate::core::path_safety::safe_join;
use std::path::{Path, PathBuf};

/// What a shortcut for one port should look like. Paths are plain (quotes are added when written).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WantedShortcut {
    pub port_key: String,
    pub app_name: String,
    pub exe: String,
    pub start_dir: String,
    pub launch_options: String,
    /// Plain path of an icon file to set, if artwork was placed for it.
    pub icon: Option<String>,
}

/// One line of a field-by-field change, for the review screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldChange {
    pub field: &'static str,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Add,
    /// `by_name`: the shortcut was found by its name only, so it points at a different program today.
    Update { key: String, app_id: u32, changes: Vec<FieldChange>, by_name: bool },
    UpToDate { app_id: Option<u32> },
    Blocked(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanItem {
    pub wanted: WantedShortcut,
    pub action: Action,
}

impl PlanItem {
    pub fn changes_the_file(&self) -> bool {
        matches!(self.action, Action::Add | Action::Update { .. })
    }
}

/// The app ID the shortcut has, or will have once added (the stored ID of an existing entry; the
/// CRC-based one for a new entry). `None` for a blocked item.
pub fn expected_app_id(item: &PlanItem) -> Option<u32> {
    match &item.action {
        Action::Add => {
            let exe = shortcuts::quote_path(&item.wanted.exe).ok()?;
            Some(super::app_id::shortcut_app_id(&exe, &item.wanted.app_name))
        }
        Action::Update { app_id, .. } => Some(*app_id),
        Action::UpToDate { app_id } => *app_id,
        Action::Blocked(_) => None,
    }
}

/// Why a port has no shortcut to offer yet.
pub fn wanted_from_port(port: &Port, library_dir: &Path, favorite_exe: Option<&str>) -> Result<WantedShortcut, String> {
    if !library_dir.is_absolute() {
        return Err("the library folder is not an absolute path".into());
    }
    let game_dir = safe_join(library_dir, &port.folder).map_err(|_| "its folder name is not usable".to_string())?;
    if !game_dir.is_dir() {
        return Err("not installed".into());
    }
    let exe = match favorite_exe.filter(|f| !f.is_empty()).and_then(|f| safe_join(&game_dir, f).ok()).filter(|p| p.is_file()) {
        Some(favorite) => favorite,
        None => match resolve_executable(port.executable.as_ref(), &game_dir) {
            Ok(path) => path,
            Err(ExecutableSelectionError::NotFound) => return Err("no program found in its folder".into()),
            Err(ExecutableSelectionError::Ambiguous(_)) => return Err("several programs found; choose a favorite in its info first".into()),
        },
    };
    if !exe.is_file() {
        return Err("its program file is missing".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&exe).map(|m| m.permissions().mode()).unwrap_or(0);
        if mode & 0o111 == 0 {
            return Err("its program is not marked executable; start it once from here first".into());
        }
    }
    let exe_s = path_text(&exe)?;
    let start_dir = exe.parent().map(path_text).transpose()?.unwrap_or_else(|| exe_s.clone());
    Ok(WantedShortcut {
        port_key: port.key().to_string(),
        app_name: port.display_name().to_string(),
        exe: exe_s,
        start_dir,
        launch_options: String::new(),
        icon: None,
    })
}

fn path_text(path: &Path) -> Result<String, String> {
    let text = path.to_str().ok_or_else(|| "its path is not plain text".to_string())?;
    if text.contains('"') {
        return Err("its path contains a double quote".into());
    }
    Ok(text.to_string())
}

fn same_text(stored: &str, wanted: &str) -> bool {
    stored == wanted
}

/// Steam writes a start folder with a trailing slash; ours has none. Same folder either way.
fn same_dir(stored: &str, wanted: &str) -> bool {
    stored.trim_end_matches(['/', '\\']) == wanted.trim_end_matches(['/', '\\'])
}

fn diff(existing: &Shortcut, wanted: &WantedShortcut) -> Vec<FieldChange> {
    let mut changes = Vec::new();
    let mut check = |field: &'static str, from: &str, to: &str, same: fn(&str, &str) -> bool| {
        if !same(from, to) {
            changes.push(FieldChange { field, from: from.to_string(), to: to.to_string() });
        }
    };
    check("program", existing.exe_path(), &wanted.exe, same_text);
    check("start folder", existing.start_dir_path(), &wanted.start_dir, same_dir);
    check("launch options", &existing.launch_options, &wanted.launch_options, same_text);
    if let Some(icon) = &wanted.icon {
        check("icon", &existing.icon, icon, same_text);
    }
    changes
}

pub fn plan(doc: &Document, wanted: &[WantedShortcut]) -> Vec<PlanItem> {
    let existing = shortcuts::list(doc);
    wanted
        .iter()
        .map(|w| {
            let by_exe: Vec<&Shortcut> = existing.iter().filter(|s| s.exe_path() == w.exe).collect();
            let by_name = by_exe.is_empty();
            let matches = if by_name {
                existing.iter().filter(|s| s.app_name.eq_ignore_ascii_case(&w.app_name)).collect()
            } else {
                by_exe
            };
            let action = match matches.as_slice() {
                [] => Action::Add,
                [one] => {
                    let changes = diff(one, w);
                    match (changes.is_empty(), one.app_id) {
                        (true, id) => Action::UpToDate { app_id: id },
                        (false, Some(app_id)) => Action::Update { key: one.key.clone(), app_id, changes, by_name },
                        (false, None) => Action::Blocked(
                            "its Steam entry has no stored app ID; open and quit Steam once, then try again".into(),
                        ),
                    }
                }
                _ => Action::Blocked("more than one shortcut in Steam matches it".into()),
            };
            PlanItem { wanted: w.clone(), action }
        })
        .collect()
}

#[derive(Debug)]
pub enum SyncError {
    Store(StoreError),
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncError::Store(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SyncError {}

impl From<StoreError> for SyncError {
    fn from(e: StoreError) -> Self {
        SyncError::Store(e)
    }
}

#[derive(Debug)]
pub struct ApplyReport {
    pub added: Vec<(String, Added)>,
    pub updated: Vec<(String, Updated)>,
    /// Port key and why it was left out (blocked in the plan, or refused when applying).
    pub skipped: Vec<(String, String)>,
    pub unchanged: Vec<String>,
    pub save: SaveReport,
    pub backup: Option<PathBuf>,
}

/// Applies a plan and saves. Refuses first, before any edit, while Steam runs. If the save itself
/// fails the file on disk is as it was and `file.doc` holds unsaved edits: load the file again to retry.
pub fn apply(file: &mut ShortcutsFile, items: &[PlanItem], steam: &dyn SteamProcessCheck, stamp: &str) -> Result<ApplyReport, SyncError> {
    check_steam(steam)?;
    let mut report = ApplyReport {
        added: Vec::new(),
        updated: Vec::new(),
        skipped: Vec::new(),
        unchanged: Vec::new(),
        save: SaveReport { wrote: false, backup: None },
        backup: None,
    };
    for item in items {
        let key = item.wanted.port_key.clone();
        match &item.action {
            Action::Blocked(reason) => report.skipped.push((key, reason.clone())),
            Action::UpToDate { .. } => report.unchanged.push(key),
            Action::Add => {
                let new = NewShortcut {
                    app_name: item.wanted.app_name.clone(),
                    exe: item.wanted.exe.clone(),
                    start_dir: item.wanted.start_dir.clone(),
                    icon: item.wanted.icon.clone(),
                    launch_options: item.wanted.launch_options.clone(),
                    tags: Vec::new(),
                };
                match shortcuts::add(&mut file.doc, &new) {
                    Ok(added) => report.added.push((key, added)),
                    Err(e) => report.skipped.push((key, e.to_string())),
                }
            }
            Action::Update { key: entry_key, changes, .. } => {
                let has = |field: &str| changes.iter().any(|c| c.field == field);
                let change = ShortcutUpdate {
                    exe: has("program").then(|| item.wanted.exe.clone()),
                    start_dir: has("start folder").then(|| item.wanted.start_dir.clone()),
                    launch_options: has("launch options").then(|| item.wanted.launch_options.clone()),
                    icon: has("icon").then(|| item.wanted.icon.clone().unwrap_or_default()),
                    derive_missing_app_id: false,
                };
                match shortcuts::update(&mut file.doc, Target::Key(entry_key), &change) {
                    Ok(updated) => report.updated.push((key, updated)),
                    Err(e) => report.skipped.push((key, e.to_string())),
                }
            }
        }
    }
    report.save = file.save(steam, stamp)?;
    report.backup = report.save.backup.clone();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::super::store::backup_stamp;
    use super::super::vdf::fixtures::*;
    use super::*;
    use crate::core::models::port_from_value;
    use serde_json::json;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("dpm_steam_sync_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn no_steam() -> std::io::Result<Vec<String>> {
        Ok(Vec::new())
    }

    fn wanted(name: &str, exe: &str) -> WantedShortcut {
        WantedShortcut {
            port_key: name.to_string(),
            app_name: name.to_string(),
            exe: exe.to_string(),
            start_dir: exe.rsplit_once('/').unwrap().0.to_string(),
            launch_options: String::new(),
            icon: None,
        }
    }

    fn doc() -> Document {
        Document::parse(&plain_file()).unwrap()
    }

    fn setup(name: &str) -> (PathBuf, PathBuf) {
        let dir = temp_dir(name);
        let path = dir.join("config").join("shortcuts.vdf");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, plain_file()).unwrap();
        (dir, path)
    }

    #[test]
    fn a_port_with_no_match_is_added() {
        let items = plan(&doc(), &[wanted("Dusklight", "/lib/dusklight/Dusklight")]);
        assert_eq!(items[0].action, Action::Add);
    }

    #[test]
    fn same_program_means_update_in_place_and_shows_only_what_differs() {
        // plain_file has "Zelda Ship" at /home/deck/Ports/soh/soh.AppImage with start dir ".../soh/".
        let mut w = wanted("Ship of Harkinian", "/home/deck/Ports/soh/soh.AppImage");
        w.start_dir = "/elsewhere/soh".into();
        let items = plan(&doc(), &[w]);
        match &items[0].action {
            Action::Update { key, app_id, changes, by_name } => {
                assert_eq!((key.as_str(), *app_id, *by_name), ("0", 0xF154_8865, false));
                assert_eq!(changes.iter().map(|c| c.field).collect::<Vec<_>>(), ["start folder"]);
                assert_eq!(changes[0].from, "/home/deck/Ports/soh/");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_start_folder_with_or_without_a_trailing_slash_is_the_same_folder() {
        let mut w = wanted("Ship of Harkinian", "/home/deck/Ports/soh/soh.AppImage");
        w.start_dir = "/home/deck/Ports/soh".into();
        assert_eq!(plan(&doc(), &[w])[0].action, Action::UpToDate { app_id: Some(0xF154_8865) });
    }

    #[test]
    fn same_name_with_a_moved_program_is_a_repoint_keeping_the_id() {
        let items = plan(&doc(), &[wanted("zelda ship", "/lib/SoH/soh.AppImage")]);
        match &items[0].action {
            Action::Update { app_id, changes, by_name, .. } => {
                assert!(*by_name, "found by its name, not its program");
                assert_eq!(*app_id, 0xF154_8865);
                assert_eq!(changes.iter().map(|c| c.field).collect::<Vec<_>>(), ["program", "start folder"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_identical_entry_is_up_to_date() {
        let mut w = wanted("Chiaki", "/usr/bin/chiaki");
        w.start_dir = "/usr/bin/".into();
        assert_eq!(plan(&doc(), &[w])[0].action, Action::UpToDate { app_id: Some(0x9962_3236) });
    }

    #[test]
    fn two_matches_block_the_port() {
        let two = file_of(&[
            steam_entry(0, 1, "Dusk", "\"/a/dusk\"", "\"/a/\"", &[]),
            steam_entry(1, 2, "dusk", "\"/b/dusk\"", "\"/b/\"", &[]),
        ]);
        let items = plan(&Document::parse(&two).unwrap(), &[wanted("Dusk", "/c/dusk")]);
        assert!(matches!(&items[0].action, Action::Blocked(r) if r.contains("more than one")));
    }

    #[test]
    fn an_entry_without_an_app_id_that_needs_changes_is_blocked_not_guessed() {
        let mut entry = Vec::new();
        entry.extend(str_field("AppName", b"Old"));
        entry.extend(str_field("Exe", b"\"/old/exe\""));
        entry.extend(str_field("StartDir", b"\"/old/\""));
        let bytes = file_of(&[object("0", &[entry])]);
        let items = plan(&Document::parse(&bytes).unwrap(), &[wanted("Old", "/new/exe")]);
        assert!(matches!(&items[0].action, Action::Blocked(r) if r.contains("open and quit Steam")));
    }

    #[test]
    fn apply_adds_and_updates_with_a_backup_and_keeps_ids() {
        let (dir, path) = setup("apply");
        let mut file = ShortcutsFile::load(&path).unwrap();
        let items = plan(
            &file.doc,
            &[wanted("Dusklight", "/lib/dusklight/Dusklight"), wanted("Zelda Ship", "/lib/SoH/soh.AppImage"), {
                let mut w = wanted("Chiaki", "/usr/bin/chiaki");
                w.start_dir = "/usr/bin/".into();
                w
            }],
        );
        let report = apply(&mut file, &items, &no_steam, "T1").unwrap();

        assert_eq!((report.added.len(), report.updated.len(), report.unchanged.len(), report.skipped.len()), (1, 1, 1, 0));
        assert_eq!(fs::read(report.backup.as_ref().unwrap()).unwrap(), plain_file());
        let saved = shortcuts::list(&ShortcutsFile::load(&path).unwrap().doc);
        assert_eq!(saved.len(), 3);
        let zelda = saved.iter().find(|s| s.app_name == "Zelda Ship").unwrap();
        assert_eq!((zelda.app_id, zelda.exe_path()), (Some(0xF154_8865), "/lib/SoH/soh.AppImage"));
        assert_eq!(zelda.tags, ["Ports", "Favorites"]);
        let dusk = saved.iter().find(|s| s.app_name == "Dusklight").unwrap();
        assert_eq!(dusk.app_id, Some(report.added[0].1.app_id));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn apply_while_steam_runs_changes_nothing_and_says_so() {
        let (dir, path) = setup("running");
        let mut file = ShortcutsFile::load(&path).unwrap();
        let items = plan(&file.doc, &[wanted("Dusklight", "/lib/d/Dusklight")]);
        let running = || -> std::io::Result<Vec<String>> { Ok(vec!["12 steam".to_string()]) };
        let err = apply(&mut file, &items, &running, "T").unwrap_err();
        assert!(matches!(err, SyncError::Store(StoreError::SteamRunning(_))));
        assert!(err.to_string().contains("close it"));
        assert_eq!(fs::read(&path).unwrap(), plain_file());
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        assert_eq!(shortcuts::list(&file.doc).len(), 2, "the in-memory document was not edited either");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn apply_with_nothing_to_change_writes_nothing() {
        let (dir, path) = setup("nochange");
        let mut file = ShortcutsFile::load(&path).unwrap();
        let mut w = wanted("Chiaki", "/usr/bin/chiaki");
        w.start_dir = "/usr/bin/".into();
        let items = plan(&file.doc, &[w]);
        let report = apply(&mut file, &items, &no_steam, &backup_stamp()).unwrap();
        assert!(!report.save.wrote && report.backup.is_none());
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn blocked_items_are_reported_and_the_rest_still_go_through() {
        let (dir, path) = setup("blocked");
        let mut file = ShortcutsFile::load(&path).unwrap();
        let mut items = plan(&file.doc, &[wanted("Dusklight", "/lib/d/Dusklight")]);
        items.push(PlanItem { wanted: wanted("Nope", "/n/n"), action: Action::Blocked("because".into()) });
        let report = apply(&mut file, &items, &no_steam, "T").unwrap();
        assert_eq!(report.added.len(), 1);
        assert_eq!(report.skipped, [("Nope".to_string(), "because".to_string())]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_icon_in_the_plan_is_written_on_add_and_on_update() {
        let (dir, path) = setup("icon");
        let mut file = ShortcutsFile::load(&path).unwrap();
        let mut a = wanted("Dusklight", "/lib/d/Dusklight");
        a.icon = Some("/grid/1_icon.png".into());
        let mut b = wanted("Chiaki", "/usr/bin/chiaki");
        b.start_dir = "/usr/bin/".into();
        b.icon = Some("/grid/2_icon.png".into());
        let items = plan(&file.doc, &[a, b]);
        apply(&mut file, &items, &no_steam, "T").unwrap();
        let saved = shortcuts::list(&ShortcutsFile::load(&path).unwrap().doc);
        assert_eq!(saved.iter().find(|s| s.app_name == "Dusklight").unwrap().icon, "/grid/1_icon.png");
        assert_eq!(saved.iter().find(|s| s.app_name == "Chiaki").unwrap().icon, "/grid/2_icon.png");
        let _ = fs::remove_dir_all(dir);
    }

    // -- wanted_from_port ---------------------------------------------------------------------

    fn port(extra: serde_json::Value) -> Port {
        let mut v = json!({"name": "Ship (Ocarina)", "folder": "SoH", "source": "https://example.com/x.zip"});
        v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        port_from_value(&v).unwrap()
    }

    /// Writes a program file; the mode only matters on Unix.
    fn exe(path: &Path, executable: bool) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(if executable { 0o755 } else { 0o644 })).unwrap();
        }
        let _ = executable;
    }

    #[test]
    fn an_installed_port_with_one_program_gets_a_shortcut_for_it() {
        let lib = temp_dir("wanted_one");
        exe(&lib.join("SoH").join("soh.exe"), true);
        let w = wanted_from_port(&port(json!({})), &lib, None).unwrap();
        assert_eq!(w.app_name, "Ship");
        assert_eq!(w.exe, lib.join("SoH").join("soh.exe").to_str().unwrap());
        assert_eq!(w.start_dir, lib.join("SoH").to_str().unwrap());
        let _ = fs::remove_dir_all(lib);
    }

    #[test]
    fn the_catalog_executable_and_then_the_favorite_decide_which_program() {
        let lib = temp_dir("wanted_choice");
        exe(&lib.join("SoH").join("bin").join("main"), true);
        exe(&lib.join("SoH").join("other"), true);
        let p = port(json!({"executable": "bin/main"}));
        let ends = |w: WantedShortcut, tail: &str| Path::new(&w.exe).ends_with(tail);
        assert!(ends(wanted_from_port(&p, &lib, None).unwrap(), "SoH/bin/main"));
        assert!(ends(wanted_from_port(&p, &lib, Some("other")).unwrap(), "SoH/other"));
        // A favorite that is gone or tries to leave the folder falls back to the catalog's program.
        assert!(ends(wanted_from_port(&p, &lib, Some("../../x")).unwrap(), "SoH/bin/main"));
        assert!(ends(wanted_from_port(&p, &lib, Some("missing")).unwrap(), "SoH/bin/main"));
        let _ = fs::remove_dir_all(lib);
    }

    #[test]
    fn reasons_a_port_cannot_be_offered_are_plain() {
        let lib = temp_dir("wanted_reasons");
        let p = port(json!({}));
        assert_eq!(wanted_from_port(&p, &lib, None).unwrap_err(), "not installed");
        fs::create_dir_all(lib.join("SoH")).unwrap();
        assert_eq!(wanted_from_port(&p, &lib, None).unwrap_err(), "no program found in its folder");
        exe(&lib.join("SoH").join("a.exe"), true);
        exe(&lib.join("SoH").join("b.exe"), true);
        assert!(wanted_from_port(&p, &lib, None).unwrap_err().starts_with("several programs"));
        assert!(wanted_from_port(&p, Path::new("relative/lib"), None).is_err());
        let _ = fs::remove_dir_all(lib);
    }

    #[cfg(unix)]
    #[test]
    fn a_program_without_the_executable_bit_is_refused_with_a_hint() {
        let lib = temp_dir("wanted_not_exec");
        exe(&lib.join("SoH").join("a"), false);
        let q = port(json!({"executable": "a"}));
        assert!(wanted_from_port(&q, &lib, None).unwrap_err().contains("not marked executable"));
        let _ = fs::remove_dir_all(lib);
    }

    #[cfg(unix)]
    #[test]
    fn a_path_with_a_double_quote_is_refused_with_a_reason() {
        let lib = temp_dir("wanted_quote");
        exe(&lib.join("SoH").join("a\"b"), true);
        assert!(wanted_from_port(&port(json!({"executable": "a\"b"})), &lib, None).unwrap_err().contains("double quote"));
        let _ = fs::remove_dir_all(lib);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_adopted_port_gets_the_library_path_not_the_target() {
        let dir = temp_dir("wanted_link");
        let real = dir.join("Applications").join("SoH");
        exe(&real.join("soh.appimage"), true);
        let lib = dir.join("Library");
        fs::create_dir_all(&lib).unwrap();
        std::os::unix::fs::symlink(&real, lib.join("SoH")).unwrap();
        let w = wanted_from_port(&port(json!({})), &lib, None).unwrap();
        assert!(w.exe.starts_with(lib.to_str().unwrap()), "{}", w.exe);
        let _ = fs::remove_dir_all(dir);
    }
}
