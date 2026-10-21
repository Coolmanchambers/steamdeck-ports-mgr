//! Turning a Deck-ready entry the app doesn't have into the user's own `ports.local.json` entry.
//!
//! Unlike an adopted port, this entry keeps its `source`, so the normal installer downloads it and the
//! normal update check follows it. Being a local entry (`user_managed`), its button says Open folder,
//! never Uninstall. For a Deck build, `preferred_asset` names the Deck marker found in the file name
//! ("steamdeck", "steamos"...), not the whole name, so it still matches next version's file.

use super::index::{Entry, Host, Rating};
use crate::fork::adopt::{read_local_file, write_backup, AdoptError};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

const DECK_MARKERS: [&str; 7] = ["steam-deck", "steam_deck", "steam.deck", "steam deck", "steamdeck", "steamos", "deck"];

/// The part of a file name that says it is a Deck build, lowercase. A bare "deck" only counts as a
/// whole word ("game-deck.zip", not "decker.zip").
pub fn deck_marker(file_name: &str) -> Option<&'static str> {
    let lower = file_name.to_lowercase();
    DECK_MARKERS.into_iter().find(|marker| {
        lower.match_indices(marker).any(|(at, _)| {
            if *marker != "deck" {
                return true;
            }
            let before = lower[..at].chars().next_back();
            let after = lower[at + marker.len()..].chars().next();
            !before.is_some_and(char::is_alphabetic) && !after.is_some_and(char::is_alphabetic)
        })
    })
}

pub fn source_url(entry: &Entry) -> Option<String> {
    let repo = entry.repo.as_deref()?;
    match entry.host {
        Host::Github => Some(format!("https://github.com/{repo}")),
        Host::Gitlab => Some(format!("https://gitlab.com/{repo}")),
        Host::Other => None,
    }
}

fn sanitize(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' }).collect();
    cleaned.trim_matches(['-', '.']).to_string()
}

/// The `Library/` folder for a new entry: the catalog's folder name when the index has one, else the
/// repository name; with the owner added when that name is already taken. `None` when every candidate
/// is taken or unusable.
pub fn folder_for(entry: &Entry, taken: impl Fn(&str) -> bool) -> Option<String> {
    let repo = entry.repo.as_deref()?;
    let base = entry.catalog_folder.clone().map(|f| sanitize(&f)).filter(|f| !f.is_empty()).unwrap_or_else(|| sanitize(repo.rsplit('/').next().unwrap_or("")));
    let owner = sanitize(repo.split('/').next().unwrap_or(""));
    let candidates = [base.clone(), format!("{base}-{owner}")];
    candidates.into_iter().find(|c| !c.is_empty() && c != "-" && !c.starts_with('-') && !taken(c))
}

/// Whether `folder` can't be used for a new entry: a port the app knows already uses it (any case),
/// something already exists at `Library/<folder>` (a file, a folder or a link), or it isn't a plain name.
pub fn folder_taken<'a>(library_dir: &Path, known_folders: impl IntoIterator<Item = &'a str>, folder: &str) -> bool {
    known_folders.into_iter().any(|f| f.eq_ignore_ascii_case(folder))
        || crate::core::path_safety::safe_join(library_dir, folder).map_or(true, |p| std::fs::symlink_metadata(p).is_ok())
}

/// The JSON entry written to `ports.local.json`.
pub fn local_entry(entry: &Entry, display_name: &str, folder: &str) -> Option<Value> {
    let source = source_url(entry)?;
    let mut obj = Map::new();
    obj.insert("name".into(), json!(display_name));
    obj.insert("folder".into(), json!(folder));
    obj.insert("source".into(), json!(source));
    obj.insert("website".into(), json!(entry.website.clone().unwrap_or_else(|| source.clone())));
    if let Some(system) = &entry.system {
        obj.insert("tags".into(), json!([system.to_lowercase()]));
    }
    obj.insert(
        "instructions".into(),
        json!(format!("Added from the Deck-ready list. Game files are not included; the project page says what it needs: {source}")),
    );
    if entry.rating == Rating::DeckBuild {
        if let Some(marker) = entry.deck_assets.iter().find_map(|f| deck_marker(f)) {
            // An empty Windows value lets a Windows build pick its own file instead of the Deck one.
            obj.insert("preferred_asset".into(), json!({ "linux": marker, "windows": "" }));
        }
    }
    Some(Value::Object(obj))
}

/// Appends `value` to `ports.local.json`, backing up the old file first (`<file>.bak-<stamp>`).
/// Refuses when the file has an entry for the same folder or is not in the expected shape.
pub fn add_to_local_file(local_json: &Path, value: Value, stamp: &str) -> Result<Option<PathBuf>, AdoptError> {
    let folder = value.get("folder").and_then(Value::as_str).unwrap_or_default().to_string();
    let (mut file, original) = read_local_file(local_json)?;
    let ports = file.entry("ports").or_insert_with(|| Value::Array(Vec::new()));
    let Value::Array(ports) = ports else { return Err(AdoptError::LocalFileUnreadable(local_json.to_path_buf())) };
    if ports.iter().any(|p| p.get("folder").and_then(Value::as_str) == Some(folder.as_str())) {
        return Err(AdoptError::AlreadyRegistered(folder));
    }
    ports.push(value);
    let text = serde_json::to_string_pretty(&file).map_err(|e| AdoptError::Io { action: "encode", path: local_json.to_path_buf(), source: std::io::Error::other(e) })?;
    let backup = match &original {
        Some(bytes) => Some(write_backup(local_json, bytes, stamp)?),
        None => None,
    };
    crate::core::files::write_atomic(local_json, text.as_bytes()).map_err(|source| AdoptError::Io { action: "write", path: local_json.to_path_buf(), source })?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::{port_from_value, SourceType};
    use crate::fork::deck_ready::index::parse;
    use crate::fork::deck_ready::index::tests::{file, project};
    use std::fs;

    fn entry(p: Value) -> Entry {
        parse(&file(vec![p])).unwrap().entries.remove(0)
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dpm_deck_ready_entry_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn deck_markers_from_real_file_names() {
        let cases = [
            ("CaseWestRecomp-steamdeck-x86_64.tar.gz", Some("steamdeck")),
            ("ActRaiserRecompBuilder-steam-deck-portable.tar.xz", Some("steam-deck")),
            ("Severed_Chains_Steam_Deck.tar.gz", Some("steam_deck")),
            ("sm64coopdx_SteamOS.zip", Some("steamos")),
            ("VirtuaFighter2Recomp-deck.tar.gz", Some("deck")),
            ("DKR-R-1.0.8-SteamDeck-x86_64.AppImage", Some("steamdeck")),
            ("decker-linux.tar.gz", None),
            ("game-linux-x86_64.tar.gz", None),
        ];
        for (name, want) in cases {
            assert_eq!(deck_marker(name), want, "{name}");
        }
    }

    #[test]
    fn folder_prefers_the_catalog_name_then_adds_the_owner() {
        let mut p = project("x", "linux_build", "game");
        p["repo"] = json!("Some Owner/My Game!");
        let e = entry(p.clone());
        assert_eq!(folder_for(&e, |_| false).as_deref(), Some("My-Game"));
        assert_eq!(folder_for(&e, |f| f == "My-Game").as_deref(), Some("My-Game-Some-Owner"));
        assert_eq!(folder_for(&e, |_| true), None);
        p["catalog_folder"] = json!("SkyRecomp");
        assert_eq!(folder_for(&entry(p), |_| false).as_deref(), Some("SkyRecomp"));
    }

    #[test]
    fn folder_names_never_escape_the_library() {
        let mut p = project("x", "linux_build", "game");
        p["repo"] = json!("../..");
        assert_eq!(folder_for(&entry(p.clone()), |_| false), None);
        p["catalog_folder"] = json!("../../etc");
        let folder = folder_for(&entry(p), |_| false).unwrap();
        assert!(!folder.contains('/') && !folder.starts_with('.'), "{folder}");
    }

    #[test]
    fn local_entry_parses_back_as_an_installable_github_port() {
        let mut p = project("cw", "deck_build", "game");
        p["system"] = json!("Xbox 360");
        p["evidence"]["deck_assets"] = json!(["CaseWestRecomp-steamdeck-x86_64.tar.gz"]);
        let e = entry(p);
        let value = local_entry(&e, "Case West", "cw").unwrap();
        let port = port_from_value(&value).unwrap();
        assert_eq!(port.source_type, SourceType::Github);
        assert_eq!(port.repo.as_deref(), Some("owner/cw"));
        assert_eq!(port.folder, "cw");
        assert_eq!(port.tags_lower, vec!["xbox 360"]);
        assert_eq!(value["preferred_asset"], json!({"linux": "steamdeck", "windows": ""}));
        #[cfg(target_os = "linux")]
        assert_eq!(crate::core::platform_resolve::resolve_preferred_asset(&value["preferred_asset"]).as_deref(), Some("steamdeck"));
        #[cfg(target_os = "windows")]
        assert_eq!(crate::core::platform_resolve::resolve_preferred_asset(&value["preferred_asset"]), None);
        assert!(port.instructions_link().is_some_and(|l| l == "https://github.com/owner/cw"));
    }

    #[test]
    fn linux_builds_and_download_pages() {
        let e = entry(project("l", "linux_build", "game"));
        assert!(local_entry(&e, "L", "l").unwrap().get("preferred_asset").is_none());
        let mut direct = project("d", "linux_build", "game");
        direct["host"] = json!("direct");
        assert!(local_entry(&entry(direct), "D", "d").is_none());
        let mut gitlab = project("g", "linux_build", "game");
        gitlab["host"] = json!("gitlab");
        gitlab["repo"] = json!("group/sub/g");
        let port = port_from_value(&local_entry(&entry(gitlab), "G", "g").unwrap()).unwrap();
        assert_eq!((port.source_type, port.repo.as_deref()), (SourceType::Gitlab, Some("group/sub/g")));
    }

    #[test]
    fn adding_backs_up_and_refuses_duplicates() {
        let dir = temp_dir("add");
        let local = dir.join("ports.local.json");
        let e = entry(project("a", "linux_build", "game"));
        assert_eq!(add_to_local_file(&local, local_entry(&e, "A", "a").unwrap(), "STAMP").unwrap(), None);
        let backup = add_to_local_file(&local, local_entry(&e, "A2", "a2").unwrap(), "STAMP").unwrap().unwrap();
        assert_eq!(crate::core::config::load_local_config(&backup).len(), 1);
        let ports = crate::core::config::load_local_config(&local);
        assert_eq!(ports.iter().map(|p| p.folder.as_str()).collect::<Vec<_>>(), vec!["a", "a2"]);
        assert!(ports.iter().all(|p| p.user_managed));
        assert!(matches!(add_to_local_file(&local, local_entry(&e, "Again", "a").unwrap(), "STAMP"), Err(AdoptError::AlreadyRegistered(_))));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_is_taken_by_a_known_port_or_anything_on_disk() {
        let dir = temp_dir("taken");
        fs::create_dir_all(dir.join("Existing")).unwrap();
        fs::write(dir.join("loose-file"), "x").unwrap();
        let known = ["Catalog-Port"];
        assert!(folder_taken(&dir, known, "catalog-port"), "catalog folders match in any case");
        assert!(folder_taken(&dir, known, "Existing"));
        assert!(folder_taken(&dir, known, "loose-file"));
        assert!(folder_taken(&dir, known, "../escape"), "a name that is not plain is never usable");
        assert!(!folder_taken(&dir, known, "Free-Name"));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("nowhere"), dir.join("dangling")).unwrap();
            assert!(folder_taken(&dir, known, "dangling"), "a link counts even when it points nowhere");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// For every Deck build in the published list, the marker written as `preferred_asset` must let the
    /// installer find exactly one file, unless the list itself names several Deck files.
    #[test]
    fn published_deck_builds_resolve_to_one_file() {
        let index = parse(include_str!("../../../index/deck-ready.json")).unwrap();
        let mut checked = 0;
        for e in index.entries.iter().filter(|e| e.rating == Rating::DeckBuild) {
            let marker = e.deck_assets.iter().find_map(|f| deck_marker(f)).unwrap_or_else(|| panic!("{}: no marker in {:?}", e.id, e.deck_assets));
            // The index lists a Deck file under Linux too; a release has each name once.
            let mut names: Vec<&String> = Vec::new();
            for n in e.deck_assets.iter().chain(&e.linux_assets).chain(&e.windows_assets) {
                if !names.contains(&n) {
                    names.push(n);
                }
            }
            let assets: Vec<Value> = names.into_iter().map(|n| json!({ "name": n })).collect();
            match crate::core::asset_select::pick_asset(&assets, Some(marker)) {
                Ok(_) => {}
                Err(crate::core::asset_select::AssetSelectionError::Ambiguous(_)) if e.deck_assets.len() > 1 => {}
                Err(err) => panic!("{}: marker {marker:?} over {:?}: {}", e.id, assets, err.message()),
            }
            checked += 1;
        }
        assert!(checked >= 10, "only {checked} Deck builds checked");
    }

    #[test]
    fn a_local_file_in_another_shape_is_left_alone() {
        let dir = temp_dir("shape");
        let local = dir.join("ports.local.json");
        fs::write(&local, "[1, 2, 3]").unwrap();
        let e = entry(project("a", "linux_build", "game"));
        assert!(matches!(add_to_local_file(&local, local_entry(&e, "A", "a").unwrap(), "S"), Err(AdoptError::LocalFileUnreadable(_))));
        assert_eq!(fs::read_to_string(&local).unwrap(), "[1, 2, 3]");
        let _ = fs::remove_dir_all(&dir);
    }
}
