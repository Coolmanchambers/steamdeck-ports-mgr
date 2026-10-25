use super::*;
use crate::core::models::port_from_value;
use crate::fork::deck_ready::entry::{local_entry, local_entry_with};
use crate::fork::deck_ready::index::parse;
use crate::fork::deck_ready::index::tests::{file, project};
use serde_json::json;

/// Upstream's shape for Severed Chains: turned off with `-name`, still carrying its cover, icon and notes.
const CATALOG: &str = r#"{"ports": [
  {"name": "On Port", "folder": "OnPort", "source": "https://github.com/a/onport", "image": "https://x/on.png", "icon": "https://x/on-icon.png",
   "instructions": "Required files: On (USA).z64 (CRC32: 12345678)"},
  {"-name": "The Legend of Dragoon (Severed-Chains)", "tags": ["ps1", "rpg"], "source": "https://github.com/owner/sc",
   "folder": "Severed-Chains", "instructions": "Required files: The Legend of Dragoon .iso\n\nLanguages: English.",
   "image": "https://x/sc.png", "icon": "https://x/sc-icon.png", "save": {"linux": "~/.local/share/sc"}, "preferred_asset": "Steam_Deck"},
  {"folder": "nameless", "image": "https://x/n.png"}
]}"#;

fn sc_entry() -> crate::fork::deck_ready::index::Entry {
    let mut p = project("sc", "deck_build", "game");
    p["name"] = json!("The Legend of Dragoon (Severed-Chains)");
    p["repo"] = json!("owner/sc");
    p["catalog_folder"] = json!("Severed-Chains");
    p["catalog_disabled"] = json!(true);
    p["system"] = json!("PS1");
    p["evidence"]["deck_assets"] = json!(["Severed_Chains_Steam_Deck.tar.gz"]);
    parse(&file(vec![p])).unwrap().entries.remove(0)
}

#[test]
fn the_catalog_parser_really_drops_the_turned_off_entry_and_the_raw_reader_keeps_it() {
    let kept = crate::core::config::parse_catalog(CATALOG).unwrap();
    assert!(kept.iter().all(|p| p.folder != "Severed-Chains"), "upstream's parser drops an entry without a name");
    let raw = RawCatalog::parse(CATALOG);
    assert!(raw.for_folder("Severed-Chains").is_some());
    assert!(raw.for_folder("severed-chains").is_some(), "any case");
    assert!(raw.for_folder("OnPort").is_some(), "an enabled entry too");
    assert!(raw.for_folder("nameless").is_none(), "no name and no turned-off name is not an entry");
    assert!(raw.for_folder("nope").is_none());
}

#[test]
fn a_damaged_or_missing_catalog_gives_none() {
    assert!(RawCatalog::parse("{not json").is_empty());
    assert!(RawCatalog::parse("[]").is_empty());
    assert!(RawCatalog::load(std::path::Path::new("/nonexistent/ports.json")).is_empty());
}

#[test]
fn a_new_entry_copies_the_describing_fields_and_not_the_source() {
    let raw = RawCatalog::parse(CATALOG);
    let value = local_entry_with(&sc_entry(), "The Legend of Dragoon (Severed-Chains)", "Severed-Chains", &raw).unwrap();
    assert_eq!(value["image"], "https://x/sc.png");
    assert_eq!(value["icon"], "https://x/sc-icon.png");
    assert_eq!(value["tags"], json!(["ps1", "rpg"]));
    assert_eq!(value["save"], json!({"linux": "~/.local/share/sc"}));
    assert!(value["instructions"].as_str().unwrap().starts_with("Required files: The Legend of Dragoon"));
    assert_eq!(value["source"], "https://github.com/owner/sc", "the Deck-ready list's source stays");
    assert_eq!(value["preferred_asset"], json!({"linux": "steam_deck", "windows": ""}), "the asset choice is the entry's own, not the catalog's");
    let port = port_from_value(&value).unwrap();
    assert_eq!(port.image.as_deref(), Some("https://x/sc.png"));
    assert_eq!(port.icon.as_deref(), Some("https://x/sc-icon.png"));
}

#[test]
fn without_a_catalog_entry_the_new_entry_is_as_before() {
    let plain = local_entry(&sc_entry(), "N", "Severed-Chains").unwrap();
    assert!(plain.get("image").is_none() && plain.get("icon").is_none());
    assert!(plain["instructions"].as_str().unwrap().starts_with(GENERIC_PREFIX));
    let same = local_entry_with(&sc_entry(), "N", "Severed-Chains", &RawCatalog::parse(CATALOG.replace("Severed-Chains\"", "Other\"").as_str())).unwrap();
    assert_eq!(same, plain, "another folder in the catalog changes nothing");
}

fn old_entry() -> Port {
    let mut value = local_entry(&sc_entry(), "The Legend of Dragoon (Severed-Chains)", "Severed-Chains").unwrap();
    value["tags"] = json!(["ps1"]);
    let mut port = port_from_value(&value).unwrap();
    port.user_managed = true;
    port
}

#[test]
fn an_entry_made_before_gets_the_fields_in_memory() {
    let mut port = old_entry();
    assert!(port.image.is_none() && port.icon.is_none());
    assert!(apply_to_port(&mut port, &RawCatalog::parse(CATALOG)));
    assert_eq!(port.image.as_deref(), Some("https://x/sc.png"));
    assert_eq!(port.icon.as_deref(), Some("https://x/sc-icon.png"));
    assert!(port.instructions.starts_with("Required files: The Legend of Dragoon"));
    assert_eq!(port.tags_lower, ["ps1", "rpg"]);
    assert!(port.save.is_some());
    assert_eq!(port.source.as_str(), Some("https://github.com/owner/sc"), "the source is untouched");
    assert_eq!(port.repo.as_deref(), Some("owner/sc"));
    assert!(!apply_to_port(&mut port, &RawCatalog::parse(CATALOG)), "the second time there is nothing generic left to fill");
}

#[test]
fn an_entry_the_user_described_or_chose_art_for_is_left_alone() {
    let raw = RawCatalog::parse(CATALOG);
    let mut edited = old_entry();
    edited.instructions = "My own notes".into();
    assert!(!apply_to_port(&mut edited, &raw));
    assert_eq!(edited.image, None);

    let mut has_image = old_entry();
    has_image.image = Some("https://mine/cover.png".into());
    has_image.save = Some(json!("~/mine"));
    assert!(apply_to_port(&mut has_image, &raw));
    assert_eq!(has_image.image.as_deref(), Some("https://mine/cover.png"), "an image already set is kept");
    assert_eq!(has_image.save, Some(json!("~/mine")), "a save folder already set is kept");
    assert_eq!(has_image.icon.as_deref(), Some("https://x/sc-icon.png"));

    let mut catalog_port = port_from_value(&json!({"name": "A", "folder": "Severed-Chains", "instructions": GENERIC_PREFIX})).unwrap();
    catalog_port.user_managed = false;
    assert!(!apply_to_port(&mut catalog_port, &raw), "only the user's own entries");
}

#[test]
fn a_catalog_entry_for_another_project_with_the_same_folder_is_not_used() {
    let mut port = old_entry();
    port.repo = Some("someone/else".into());
    assert!(!apply_to_port(&mut port, &RawCatalog::parse(CATALOG)));
    assert!(port.image.is_none());
}

#[test]
fn apply_counts_the_ports_it_filled() {
    let mut catalog = vec![old_entry(), port_from_value(&json!({"name": "OnPort", "folder": "OnPort", "source": "https://github.com/a/onport"})).unwrap()];
    assert_eq!(apply(&mut catalog, &RawCatalog::parse(CATALOG)), 1);
    assert_eq!(apply(&mut catalog, &RawCatalog::default()), 0);
}

#[test]
fn the_shipped_catalog_has_severed_chains_turned_off_with_a_cover_and_an_icon() {
    let raw = RawCatalog::parse(include_str!("../../../ports.json"));
    let sc = raw.for_folder("Severed-Chains").expect("Severed-Chains is in the catalog");
    assert!(sc.contains_key("-name") && !sc.contains_key("name"));
    let mut port = port_from_value(&json!({"name": "x", "folder": "Severed-Chains", "source": "https://github.com/Legend-of-Dragoon-Modding/Severed-Chains", "instructions": GENERIC_PREFIX})).unwrap();
    port.user_managed = true;
    assert!(apply_to_port(&mut port, &raw));
    assert!(port.image.is_some() && port.icon.is_some());
    assert!(port.instructions.contains("Required files"));
}

#[test]
fn the_import_path_builds_the_same_entry() {
    use crate::fork::import::targets::build_described;
    let index = parse(&file(vec![{
        let mut p = project("sc", "deck_build", "game");
        p["name"] = json!("The Legend of Dragoon (Severed-Chains)");
        p["repo"] = json!("owner/sc");
        p["catalog_folder"] = json!("Severed-Chains");
        p["evidence"]["deck_assets"] = json!(["Severed_Chains_Steam_Deck.tar.gz"]);
        p
    }]))
    .unwrap();
    let dir = std::env::temp_dir().join(format!("dpm_catalog_extras_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let t = build_described(&[], Some(&index), &dir, &RawCatalog::parse(CATALOG));
    assert_eq!(t.new_entries.len(), 1);
    assert_eq!(t.new_entries[0].1.value["image"], "https://x/sc.png");
    assert_eq!(t.catalog[0].icon.as_deref(), Some("https://x/sc-icon.png"));
    let _ = std::fs::remove_dir_all(&dir);
}
