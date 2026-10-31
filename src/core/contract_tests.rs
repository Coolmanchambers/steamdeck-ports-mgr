//! Contract tests of this fork: they pin the upstream behavior our own features rely on, so an
//! upstream sync that changes it fails here instead of on the Deck. Real asset names below come from
//! the public releases of SkyRecomp, Severed Chains and Dusklight (checked 2026-10-20). Each test names
//! the feature that depends on the behavior.

use super::asset_select::{latest_installable_release, pick_asset, AssetSelectionError};
#[cfg(target_os = "linux")]
use super::asset_select::{choose_release, ReleaseChoice};
use super::config::{load_local_config, merge_local_catalog, parse_catalog};
use super::github_api::pick_release_asset;
use super::jobs::update_decision;
use super::models::{Port, SourceType};
use super::state::StateManager;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

fn assets(names: &[&str]) -> Vec<Value> {
    names.iter().map(|n| json!({"name": n})).collect()
}

fn release(tag: &str, asset_names: &[&str]) -> Value {
    json!({"tag_name": tag, "assets": assets(asset_names)})
}

fn picked_name(result: Result<Value, AssetSelectionError>) -> String {
    result.expect("an asset is picked")["name"].as_str().unwrap().to_string()
}

fn temp_file(name: &str, contents: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("dpm_contract_{}_{}.json", std::process::id(), name));
    fs::write(&path, contents).unwrap();
    path
}

const SKY_V105: &[&str] = &[
    "SkyRecomp-v1.0.5-Linux.tar.gz",
    "SkyRecomp-v1.0.5-Linux.zip",
    "SkyRecomp-v1.0.5-macOS.dmg",
    "SkyRecomp-v1.0.5-macOS.zip",
    "SkyRecomp-v1.0.5-Windows.zip",
];

const SEVERED_CURRENT: &[&str] = &[
    "Severed_Chains_Linux.tar.gz",
    "Severed_Chains_Linux_ARM64.tar.gz",
    "Severed_Chains_MacOS_Intel.tar.gz",
    "Severed_Chains_MacOS_M1.tar.gz",
    "Severed_Chains_Steam_Deck.tar.gz",
    "Severed_Chains_Windows.zip",
];

const DUSKLIGHT_V203: &[&str] = &[
    "Dusklight-v2.0.3-android-arm64.apk",
    "Dusklight-v2.0.3-ios-arm64.ipa",
    "Dusklight-v2.0.3-linux-arm64.AppImage",
    "Dusklight-v2.0.3-linux-x86_64.AppImage",
    "Dusklight-v2.0.3-macos-arm64.zip",
    "Dusklight-v2.0.3-macos-x86_64.zip",
    "Dusklight-v2.0.3-win32-arm64.zip",
    "Dusklight-v2.0.3-win32-x86_64.zip",
];

// Asset choice (catalog entries for our ports; the update check in C1 relies on the same rules)

#[test]
fn skyrecomp_default_pick_is_the_linux_tarball() {
    assert_eq!(picked_name(pick_asset(&assets(SKY_V105), None)), "SkyRecomp-v1.0.5-Linux.tar.gz");
}

#[test]
fn skyrecomp_preferred_asset_keeps_the_tarball_when_an_appimage_appears() {
    let mut names = SKY_V105.to_vec();
    names.push("SkyRecomp-v1.0.6-linux-x86_64.AppImage");
    // Without the hint the x86_64 tie-break takes the AppImage, which is why our entry sets preferred_asset.
    assert_eq!(picked_name(pick_asset(&assets(&names), None)), "SkyRecomp-v1.0.6-linux-x86_64.AppImage");
    assert_eq!(picked_name(pick_asset(&assets(&names), Some("Linux.tar.gz"))), "SkyRecomp-v1.0.5-Linux.tar.gz");
}

#[test]
fn severed_chains_needs_preferred_asset_to_reach_the_steam_deck_tarball() {
    assert!(matches!(pick_asset(&assets(SEVERED_CURRENT), None), Err(AssetSelectionError::Ambiguous(_))));
    assert_eq!(picked_name(pick_asset(&assets(SEVERED_CURRENT), Some("Severed_Chains_Steam_Deck.tar.gz"))), "Severed_Chains_Steam_Deck.tar.gz");
}

fn severed_chains_releases() -> Vec<Value> {
    // Real order and flags from the GitHub API (2026-10-21): neither release-3.0.0 nor devbuild is a prerelease.
    vec![
        release("release-3.0.0", SEVERED_CURRENT),
        release("devbuild", SEVERED_CURRENT),
        release("recommended-build-2", &[]),
        release("recommended-build-1", &["Linux.zip", "Mac.zip", "Windows.zip"]),
    ]
}

// Fork (upstream bug O2): this test replaced `severed_chains_without_preferred_asset_walks_down_to_the_2023_zip`,
// which pinned upstream's silent install of the 2023 release. Tim's ruling: Severed Chains follows its main release.
#[test]
#[cfg(target_os = "linux")]
fn severed_chains_without_preferred_asset_asks_for_a_file_of_the_newest_release() {
    match choose_release(severed_chains_releases(), None, pick_release_asset).unwrap() {
        ReleaseChoice::AskForNewest { release, candidates } => {
            assert_eq!(release["tag_name"], "release-3.0.0");
            let names: Vec<&str> = candidates.iter().map(|a| a["name"].as_str().unwrap()).collect();
            assert_eq!(names, ["Severed_Chains_Linux.tar.gz", "Severed_Chains_Linux_ARM64.tar.gz"]);
        }
        other => panic!("expected the file picker for release-3.0.0, got {other:?}"),
    }
    assert!(matches!(latest_installable_release(severed_chains_releases(), None, pick_release_asset), Err(AssetSelectionError::Ambiguous(_))));
}

#[test]
fn severed_chains_with_preferred_asset_installs_the_main_release_not_devbuild() {
    let (tag, asset) = latest_installable_release(severed_chains_releases(), Some("Severed_Chains_Steam_Deck.tar.gz"), pick_release_asset).unwrap();
    assert_eq!((tag["tag_name"].as_str().unwrap(), asset["name"].as_str().unwrap()), ("release-3.0.0", "Severed_Chains_Steam_Deck.tar.gz"));
}

// Fork (upstream bug O3): a recycled tag published as a prerelease is skipped even when it is listed first.
#[test]
fn a_devbuild_prerelease_listed_first_is_skipped_for_the_main_release() {
    let mut releases = severed_chains_releases();
    releases.swap(0, 1);
    releases[0]["prerelease"] = true.into();
    let (tag, _) = latest_installable_release(releases, Some("Severed_Chains_Steam_Deck.tar.gz"), pick_release_asset).unwrap();
    assert_eq!(tag["tag_name"], "release-3.0.0");
}

#[test]
fn dusklight_default_pick_is_the_x86_64_appimage() {
    assert_eq!(picked_name(pick_asset(&assets(DUSKLIGHT_V203), None)), "Dusklight-v2.0.3-linux-x86_64.AppImage");
}

// Update decision (the daily "update available" prompt, and the recycled `devbuild` tag of Severed Chains)

#[test]
fn recycled_tag_is_an_update_only_when_the_asset_is_newer_than_the_install() {
    let asset_updated = "2026-10-17T01:15:35Z";
    assert!(update_decision(Some("devbuild"), "2026-10-16T00:00:00Z", "devbuild", Some(asset_updated)));
    assert!(!update_decision(Some("devbuild"), "2026-10-18T00:00:00Z", "devbuild", Some(asset_updated)));
}

#[test]
fn switching_between_release_and_devbuild_is_an_update_even_when_the_asset_is_older() {
    assert!(update_decision(Some("release-3.0.0"), "2026-10-18T00:00:00Z", "devbuild", Some("2026-10-17T01:15:35Z")));
}

#[test]
fn installed_at_uses_the_same_utc_format_as_github_dates() {
    let path = temp_file("installed_at", "{}");
    let mut state = StateManager::load(&path);
    state.mark_installed("owner/repo", Some("v1".to_string()));
    let installed_at = state.get("owner/repo").unwrap().installed_at.clone();
    // The date comparison in update_decision is a string comparison; it only works if both sides look like this.
    assert_eq!(installed_at.len(), "2026-10-17T01:15:35Z".len(), "{installed_at}");
    assert!(installed_at.ends_with('Z') && installed_at.as_bytes()[10] == b'T', "{installed_at}");
    let _ = fs::remove_file(&path);
}

// Local entries (ports.local.json)

fn official_catalog() -> Vec<Port> {
    parse_catalog(
        &json!({"ports": [
            {"-name": "Disabled upstream", "folder": "SkyRecomp", "source": "https://github.com/Apolinario-coder/SkyRecomp"},
            {"name": "Twilight Princess (dusklight)", "folder": "dusklight", "source": "https://github.com/TwilitRealm/dusklight",
             "tags": ["gc"], "instructions": "Required files: a disc", "image": "https://example.com/i.png", "save": "%APPDATA%\\TwilitRealm"},
            {"name": "Other", "folder": "other", "source": "https://example.com/other.zip"}
        ]})
        .to_string(),
    )
    .expect("catalog parses")
}

fn local_ports(name: &str, text: &str) -> Vec<Port> {
    let path = temp_file(name, text);
    let ports = load_local_config(&path);
    let _ = fs::remove_file(&path);
    ports
}

#[test]
fn upstream_disabled_entries_without_a_name_are_not_in_the_catalog() {
    let official = official_catalog();
    assert_eq!(official.len(), 2);
    assert!(official.iter().all(|p| p.folder != "SkyRecomp"));
}

#[test]
fn local_entry_with_the_same_folder_replaces_the_official_one_whole() {
    let local = local_ports(
        "override",
        r#"{"ports":[{"name":"Dusklight (mine)","folder":"dusklight","source":"https://github.com/TwilitRealm/dusklight",
            "save":{"windows":"%APPDATA%\\TwilitRealm","linux":"%HOME%/.local/share/TwilitRealm"}}]}"#,
    );
    let merged = merge_local_catalog(official_catalog(), local);
    assert_eq!(merged.len(), 2);
    let mine = merged.iter().find(|p| p.folder == "dusklight").unwrap();
    assert_eq!(mine.name, "Dusklight (mine)");
    assert!(mine.user_managed);
    // No field merging: what the local entry does not repeat is gone, so our override must repeat everything.
    assert!(mine.image.is_none() && mine.instructions.is_empty() && mine.tags_lower.is_empty());
    assert!(mine.save.as_ref().unwrap().is_object());
    assert_eq!(merged.last().unwrap().folder, "dusklight", "the local entry is appended at the end");
}

#[test]
fn local_entries_get_their_own_state_key() {
    let local = local_ports(
        "own_keys",
        r#"{"ports":[
            {"name":"Skyward (SkyRecomp)","folder":"SkyRecomp","source":"https://github.com/Apolinario-coder/SkyRecomp"},
            {"name":"Placeholder","folder":"CourageReborn"}]}"#,
    );
    let merged = merge_local_catalog(official_catalog(), local);
    assert_eq!(merged.len(), 4);
    let sky = merged.iter().find(|p| p.folder == "SkyRecomp").unwrap();
    assert_eq!((sky.source_type, sky.key()), (SourceType::Github, "Apolinario-coder/SkyRecomp"));
    let placeholder = merged.iter().find(|p| p.folder == "CourageReborn").unwrap();
    assert_eq!((placeholder.source_type, placeholder.key()), (SourceType::None, "CourageReborn"));
    let mut keys: Vec<&str> = merged.iter().map(Port::key).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), merged.len());
}

// state.json (read by our updater tests, the discovery notes and, later, the Steam shortcuts library)

#[test]
fn state_json_installed_entry_keys_are_read_and_written_as_documented() {
    let path = temp_file(
        "state_read",
        r#"{"release_sync": false, "installed": {"owner/repo": {"installed_tag": "v2", "installed_at": "2026-10-17T01:15:35Z",
            "favorite_exe": "bin/game", "update": false, "playtime_seconds": 90, "last_played_at": "2026-10-18T00:00:00Z"}}}"#,
    );
    let state = StateManager::load(&path);
    assert!(!state.release_sync);
    let info = state.get("owner/repo").unwrap();
    assert_eq!(info.installed_tag.as_deref(), Some("v2"));
    assert_eq!(info.installed_at, "2026-10-17T01:15:35Z");
    assert_eq!(info.favorite_exe.as_deref(), Some("bin/game"));
    assert!(!info.update);
    assert_eq!(info.playtime_seconds, 90);
    assert_eq!(info.last_played_at, "2026-10-18T00:00:00Z");
    let _ = fs::remove_file(&path);

    let path = temp_file("state_write", "{}");
    let mut state = StateManager::load(&path);
    state.mark_installed("owner/repo", Some("v3".to_string()));
    state.set_favorite_exe("owner/repo", Some("bin/game".to_string()));
    let written: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let entry = &written["installed"]["owner/repo"];
    for key in ["installed_tag", "installed_at", "favorite_exe", "update", "playtime_seconds", "last_played_at"] {
        assert!(entry.get(key).is_some(), "state.json lost the key {key}");
    }
    assert_eq!(entry["installed_tag"], "v3");
    assert_eq!(entry["favorite_exe"], "bin/game");
    assert!(written.get("release_sync").is_some());
    let _ = fs::remove_file(&path);
}

/// Source hygiene (upstream catch-up 2026-10-31): a merge resolved on Windows once re-encoded eight files (a UTF-8
/// BOM, and every non-ASCII name in the Language list turned to "FranA\u{0303}ais"-style garbage). rustc strips a BOM
/// and the strings still compile, so only a test catches it.
#[test]
fn sources_have_no_bom_and_no_double_encoded_text() {
    fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if matches!(path.extension().and_then(|e| e.to_str()), Some("rs" | "slint" | "sh" | "md")) {
                out.push(path);
            }
        }
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in ["src", "ui", "tools"] {
        walk(&root.join(dir), &mut files);
    }
    assert!(files.len() > 100, "walked {} files", files.len());
    // A UTF-8 file read as cp1252 and written as UTF-8 again: "A tilde" (C3 83), "a-circumflex euro" (C3 A2 E2 80),
    // "ETH" (C3 90) and "U grave" (C3 99) lead the Latin, Cyrillic and CJK garbage.
    let mojibake: [&[u8]; 4] = [b"\xC3\x83", b"\xC3\xA2\xE2\x80", b"\xC3\x90", b"\xC3\x99"];
    for path in files {
        let bytes = fs::read(&path).unwrap();
        assert!(!bytes.starts_with(b"\xEF\xBB\xBF"), "{} starts with a UTF-8 BOM", path.display());
        for m in mojibake {
            assert!(!bytes.windows(m.len()).any(|w| w == m), "{} holds double-encoded text", path.display());
        }
    }
    // The Language list keeps its real names (French, Japanese, Russian).
    let dialogs = fs::read_to_string(root.join("src/app/dialogs.rs")).unwrap();
    for name in ["Fran\u{e7}ais", "\u{65e5}\u{672c}\u{8a9e}", "\u{420}\u{443}\u{441}\u{441}\u{43a}\u{438}\u{439}"] {
        assert!(dialogs.contains(name), "the Language list lost {name}");
    }
}
