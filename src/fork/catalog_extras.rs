//! The describing fields of catalog entries the app does not list.
//!
//! Upstream turns an entry off by renaming its `name` key to `-name` (Severed Chains is one), and
//! `parse_catalog` drops entries without a `name`. The entry still carries its cover (`image`), icon,
//! game file notes (`instructions`), tags and save paths, which a port added from the Deck-ready list
//! needs: without them Home shows a text tile, Steam art finds nothing, and the game files screens know
//! nothing. This module reads the raw `ports.json` (enabled or turned off) and copies those fields.
//!
//! Two ways it is used:
//! - a new entry made from the Deck-ready list gets the fields when it is written
//!   (`deck_ready::entry::local_entry_with`, `import::targets::build_described`);
//! - an entry made earlier (recognised by the generic text it was given) gets them in memory each time the
//!   catalog is loaded (`apply`). Nothing is written to `ports.local.json`, so no data is lost and no
//!   backup is needed; when the catalog changes, the entry follows it.
//!
//! Never copied: `source` and the asset choice. The entry keeps the source the Deck-ready list gave it.

use crate::core::models::Port;
use serde_json::{Map, Value};
use std::path::Path;

/// The text the Deck-ready list gives a new entry before it knows anything better.
pub const GENERIC_PREFIX: &str = "Added from the Deck-ready list.";

/// The fields that describe a port (not where it comes from).
const FIELDS: [&str; 6] = ["image", "icon", "instructions", "tags", "save", "save2"];

/// Every object of the catalog's `ports` list that has a `name` or a turned-off `-name`.
#[derive(Debug, Clone, Default)]
pub struct RawCatalog {
    entries: Vec<Map<String, Value>>,
}

impl RawCatalog {
    pub fn parse(text: &str) -> RawCatalog {
        let Ok(data) = serde_json::from_str::<Value>(text.trim_start_matches('\u{FEFF}')) else { return RawCatalog::default() };
        let entries = data
            .get("ports")
            .and_then(Value::as_array)
            .map(|ports| ports.iter().filter_map(Value::as_object).filter(|o| o.contains_key("name") || o.contains_key("-name")).cloned().collect())
            .unwrap_or_default();
        RawCatalog { entries }
    }

    /// The catalog file the app keeps (`ports.json` in its own folder); a missing or damaged file gives none.
    pub fn load(ports_json: &Path) -> RawCatalog {
        std::fs::read_to_string(ports_json).map(|t| RawCatalog::parse(&t)).unwrap_or_default()
    }

    /// The entry for a `Library/` folder name (any case), enabled or turned off.
    pub fn for_folder(&self, folder: &str) -> Option<&Map<String, Value>> {
        self.entries.iter().find(|e| e.get("folder").and_then(Value::as_str).is_some_and(|f| f.eq_ignore_ascii_case(folder)))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Copies the describing fields of `raw` into a new local entry's JSON, replacing the generic text. `name`,
/// `folder`, `source`, `website` and `preferred_asset` are left as they are.
pub fn enrich_value(value: &mut Value, raw: &Map<String, Value>) {
    let Some(obj) = value.as_object_mut() else { return };
    for field in FIELDS {
        if let Some(v) = raw.get(field).filter(|v| !v.is_null() && v.as_str() != Some("")) {
            obj.insert(field.to_string(), v.clone());
        }
    }
}

/// Whether the port is an entry the Deck-ready list made and nobody has described since.
fn is_generic(port: &Port) -> bool {
    port.user_managed && port.instructions.starts_with(GENERIC_PREFIX)
}

/// Both name the same project: no `repo` to compare means nothing contradicts.
fn same_project(port: &Port, raw: &Map<String, Value>) -> bool {
    match (port.repo.as_deref(), raw.get("source").and_then(Value::as_str)) {
        (Some(repo), Some(source)) => source.to_lowercase().contains(&repo.to_lowercase()),
        _ => true,
    }
}

/// Fills a Deck-ready entry made before this module existed. Returns whether anything changed.
pub fn apply_to_port(port: &mut Port, raw: &RawCatalog) -> bool {
    if !is_generic(port) {
        return false;
    }
    let Some(entry) = raw.for_folder(&port.folder).filter(|e| same_project(port, e)) else { return false };
    let mut changed = false;
    if port.image.is_none() {
        if let Some(v) = entry.get("image").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            port.image = Some(v.to_string());
            changed = true;
        }
    }
    if port.icon.is_none() {
        if let Some(v) = entry.get("icon").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            port.icon = Some(v.to_string());
            changed = true;
        }
    }
    if let Some(v) = entry.get("instructions").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        port.instructions = v.to_string();
        changed = true;
    }
    if let Some(tags) = entry.get("tags").and_then(Value::as_array) {
        let tags: Vec<String> = tags.iter().filter_map(Value::as_str).map(str::to_lowercase).collect();
        if !tags.is_empty() {
            port.tags_lower = tags;
            changed = true;
        }
    }
    if port.save.is_none() {
        if let Some(v) = entry.get("save").filter(|v| !v.is_null()) {
            port.save = Some(v.clone());
            changed = true;
        }
    }
    if port.save2.is_none() {
        if let Some(v) = entry.get("save2").filter(|v| !v.is_null()) {
            port.save2 = Some(v.clone());
            changed = true;
        }
    }
    changed
}

/// [`apply_to_port`] for a whole catalog. Returns how many ports were filled.
pub fn apply(catalog: &mut [Port], raw: &RawCatalog) -> usize {
    if raw.is_empty() {
        return 0;
    }
    let mut n = 0;
    for port in catalog.iter_mut() {
        if apply_to_port(port, raw) {
            eprintln!("catalog: {} took its cover, icon and notes from the catalog entry for {}", port.name, port.folder);
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests;
