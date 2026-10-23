//! What a scan matches folders against: the catalog, plus the Deck-ready list (`index/deck-ready.json`,
//! this fork's own list). The list adds two things:
//! - the release file names of each project's latest download, so a folder named like the download
//!   matches (`SoH-Ackbar-Delta-Linux` and `SoH-Dewey-Bravo.appimage`);
//! - projects the catalog does not have (Severed Chains: upstream's catalog disables it). A folder that
//!   matches one is offered like any other find; importing it first adds the project to the user's own
//!   port list (`ports.local.json`, backed up), with the same entry the Deck-ready screen's "Add and
//!   install" writes (`fork::deck_ready::entry::local_entry`), then moves it in through the port mover.
//!
//! Reads only.

use crate::core::models::{port_from_value, Port};
use crate::fork::adopt::Hints;
use crate::fork::deck_ready::entry;
use crate::fork::deck_ready::index::{Entry, Index};
use crate::fork::deck_ready::view::{self, KnownPort};
use serde_json::Value;
use std::path::Path;

/// A Deck-ready project the catalog does not have, ready to be added to the user's port list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEntry {
    /// The project as the list names it ("The Legend of Dragoon (Severed-Chains)").
    pub list_name: String,
    /// The entry written to `ports.local.json` (built and parsed before anything is written).
    pub value: Value,
}

#[derive(Debug, Clone, Default)]
pub struct Targets {
    /// The catalog, then one entry per Deck-ready project it does not have.
    pub catalog: Vec<Port>,
    pub hints: Hints,
    /// `Port::key()` of each added project and what importing it writes first.
    pub new_entries: Vec<(String, NewEntry)>,
}

impl Targets {
    pub fn new_entry(&self, key: &str) -> Option<&NewEntry> {
        self.new_entries.iter().find(|(k, _)| k == key).map(|(_, e)| e)
    }
}

fn downloads(entry: &Entry) -> impl Iterator<Item = &String> {
    entry.deck_assets.iter().chain(&entry.linux_assets)
}

/// The catalog alone when there is no Deck-ready list (never fetched, offline).
pub fn build(catalog: &[Port], index: Option<&Index>, library_dir: &Path) -> Targets {
    let mut targets = Targets { catalog: catalog.to_vec(), ..Targets::default() };
    let Some(index) = index else { return targets };
    let known: Vec<KnownPort> =
        catalog.iter().map(|p| KnownPort { key: p.key().to_string(), folder: p.folder.clone(), repo: p.repo.clone(), user_managed: p.user_managed, installed: false }).collect();
    for e in &index.entries {
        if let Some(port) = view::match_known(e, &known) {
            for file in downloads(e) {
                targets.hints.add_asset(&port.key, file);
            }
            continue;
        }
        if !view::can_add(e) {
            continue;
        }
        let taken = |f: &str| entry::folder_taken(library_dir, targets.catalog.iter().map(|p| p.folder.as_str()), f);
        let Some(folder) = entry::folder_for(e, taken) else { continue };
        let Some(value) = entry::local_entry(e, &view::display_name(e), &folder) else { continue };
        let Ok(mut port) = port_from_value(&value) else { continue };
        port.user_managed = true;
        let key = port.key().to_string();
        if targets.catalog.iter().any(|p| p.key() == key) {
            continue;
        }
        for file in downloads(e) {
            targets.hints.add_asset(&key, file);
        }
        targets.catalog.push(port);
        targets.new_entries.push((key, NewEntry { list_name: e.name.clone(), value }));
    }
    targets
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::fork::deck_ready::index::parse;
    use serde_json::json;

    /// The Deck-ready entries for Tim's four ports, as the list has them (2026-10-23).
    pub fn tims_index() -> Index {
        let project = |id: &str, name: &str, repo: &str, folder: &str, disabled: bool, rating: &str, deck: &[&str], linux: &[&str]| {
            json!({
                "id": id, "name": name, "system": "N64", "repo": repo, "repo_url": format!("https://github.com/{repo}"), "host": "github",
                "catalog_folder": folder, "catalog_disabled": disabled, "kind": "game", "deck_rating": rating,
                "evidence": {"release": {"tag": "1", "date": "2026-10-17T01:16:33Z", "prerelease": false}, "deck_assets": deck, "linux_assets": linux, "windows_assets": []}
            })
        };
        let projects = vec![
            project("github:harbourmasters/shipwright", "The Legend of Zelda: Ocarina of Time (Shipwright)", "HarbourMasters/Shipwright", "Shipwright", false, "linux_build", &[], &["SoH-Dewey-Bravo.appimage"]),
            project("github:2ship2harkinian/2ship2harkinian", "The Legend of Zelda: Majora's Mask (2ship2harkinian)", "2ship2harkinian/2ship2harkinian", "2ship2harkinian", false, "linux_build", &[], &["2Ship-Battler-Bravo-Linux.zip"]),
            project("github:twilitrealm/dusklight", "The Legend of Zelda: Twilight Princess (dusklight)", "TwilitRealm/dusklight", "dusklight", false, "linux_build", &[], &["Dusklight-v2.0.3-linux-x86_64.AppImage"]),
            project(
                "github:legend-of-dragoon-modding/severed-chains",
                "The Legend of Dragoon (Severed-Chains)",
                "Legend-of-Dragoon-Modding/Severed-Chains",
                "Severed-Chains",
                true,
                "deck_build",
                &["Severed_Chains_Steam_Deck.tar.gz"],
                &["Severed_Chains_Linux.tar.gz"],
            ),
        ];
        parse(&json!({"schema": 1, "generated": "2026-10-23", "projects": projects}).to_string()).unwrap()
    }

    /// Tim's catalog: upstream's, which disables Severed Chains (so it is not there).
    pub fn tims_catalog() -> Vec<Port> {
        [
            json!({"name": "The Legend of Zelda: Ocarina of Time (Shipwright)", "folder": "Shipwright", "source": "https://github.com/HarbourMasters/Shipwright"}),
            json!({"name": "The Legend of Zelda: Majora's Mask (2ship2harkinian)", "folder": "2ship2harkinian", "source": "https://github.com/HarbourMasters/2ship2harkinian"}),
            json!({"name": "The Legend of Zelda: Majora's Mask (Zelda64Recomp)", "folder": "Zelda64Recomp", "source": "https://github.com/Zelda64Recomp/Zelda64Recomp"}),
            json!({"name": "The Legend of Zelda: Twilight Princess (dusklight)", "folder": "dusklight", "source": "https://github.com/TwilitRealm/dusklight"}),
        ]
        .iter()
        .map(|v| port_from_value(v).unwrap())
        .collect()
    }

    #[test]
    fn a_project_the_catalog_lacks_becomes_a_new_entry_and_known_ones_give_their_download_names() {
        let library = std::env::temp_dir().join(format!("dpm_targets_{}", std::process::id()));
        let t = build(&tims_catalog(), Some(&tims_index()), &library);
        assert_eq!(t.catalog.len(), 5);
        assert_eq!(t.new_entries.len(), 1);
        let (key, new) = &t.new_entries[0];
        assert_eq!(key, "Legend-of-Dragoon-Modding/Severed-Chains");
        assert_eq!(new.list_name, "The Legend of Dragoon (Severed-Chains)");
        assert_eq!(new.value["folder"], "Severed-Chains");
        assert_eq!(new.value["name"], "The Legend of Dragoon");
        assert_eq!(new.value["source"], "https://github.com/Legend-of-Dragoon-Modding/Severed-Chains");
        // The Deck build of the main release: the same entry the Deck-ready screen's "Add and install" writes.
        assert_eq!(new.value["preferred_asset"], json!({"linux": "steam_deck", "windows": ""}));
        let port = t.catalog.iter().find(|p| p.key() == key).unwrap();
        assert!(port.user_managed && port.folder == "Severed-Chains");
        assert!(!t.hints.is_empty());
        assert!(t.new_entry(key).is_some() && t.new_entry("HarbourMasters/Shipwright").is_none());
    }

    #[test]
    fn without_a_list_the_catalog_is_all_there_is() {
        let t = build(&tims_catalog(), None, Path::new("/nowhere"));
        assert_eq!(t.catalog.len(), 4);
        assert!(t.hints.is_empty() && t.new_entries.is_empty());
    }
}
