//! Reading `index/deck-ready.json` (schema 1), the list this fork publishes and refreshes weekly.
//!
//! An entry the app can't use (an unknown rating or kind, no id, no name) is skipped, not fatal, so a
//! later builder can add values without breaking older apps. A file that isn't schema 1, or has no
//! usable entry at all, is refused so it never replaces a good saved copy.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rating {
    /// The latest release has a file whose name says Steam Deck or SteamOS.
    DeckBuild,
    /// The latest release has a non-ARM Linux file, none made for the Deck in particular.
    LinuxBuild,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Game,
    /// Found by GitHub topic only; may be a tool. Shown behind a toggle.
    LikelyGame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    Github,
    Gitlab,
    /// A download page outside GitHub and GitLab; installable only through its catalog entry.
    Other,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    /// ISO 8601, as GitHub gives it.
    pub date: String,
    pub prerelease: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Maintainer {
    pub status: Option<String>,
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub system: Option<String>,
    /// `owner/name` on GitHub, the project path on GitLab.
    pub repo: Option<String>,
    pub repo_url: Option<String>,
    pub host: Host,
    pub catalog_folder: Option<String>,
    pub catalog_disabled: bool,
    pub website: Option<String>,
    pub kind: Kind,
    pub rating: Rating,
    pub readme_mentions_deck: Option<bool>,
    pub archived: bool,
    pub sources: Vec<String>,
    pub release: Option<Release>,
    pub deck_assets: Vec<String>,
    pub linux_assets: Vec<String>,
    pub windows_assets: Vec<String>,
    pub maintainer: Option<Maintainer>,
    pub first_seen: Option<String>,
    /// Set when the latest refresh did not find the project; it is removed a cycle later.
    pub missing_since: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    /// The date the list last changed (not the date it was last checked).
    pub generated: String,
    pub entries: Vec<Entry>,
}

impl Index {
    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }
}

const SCHEMA: u64 = 1;

fn text(obj: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

fn entry_from_value(v: &Value) -> Option<Entry> {
    let obj = v.as_object()?;
    let rating = match obj.get("deck_rating").and_then(Value::as_str)? {
        "deck_build" => Rating::DeckBuild,
        "linux_build" => Rating::LinuxBuild,
        _ => return None,
    };
    let kind = match obj.get("kind").and_then(Value::as_str)? {
        "game" => Kind::Game,
        "likely_game" => Kind::LikelyGame,
        _ => return None,
    };
    let host = match obj.get("host").and_then(Value::as_str) {
        Some("github") => Host::Github,
        Some("gitlab") => Host::Gitlab,
        _ => Host::Other,
    };
    let evidence = obj.get("evidence").and_then(Value::as_object);
    let release = evidence.and_then(|e| e.get("release")).and_then(Value::as_object).map(|r| Release {
        tag: text(r, "tag").unwrap_or_default(),
        date: text(r, "date").unwrap_or_default(),
        prerelease: r.get("prerelease").and_then(Value::as_bool).unwrap_or(false),
    });
    let maintainer = obj.get("recomp_json").and_then(Value::as_object).map(|m| Maintainer { status: text(m, "status"), targets: strings(m.get("targets")) });
    Some(Entry {
        id: text(obj, "id")?,
        name: text(obj, "name")?,
        system: text(obj, "system"),
        repo: text(obj, "repo"),
        repo_url: text(obj, "repo_url"),
        host,
        catalog_folder: text(obj, "catalog_folder"),
        catalog_disabled: obj.get("catalog_disabled").and_then(Value::as_bool).unwrap_or(false),
        website: text(obj, "website"),
        kind,
        rating,
        readme_mentions_deck: obj.get("readme_mentions_deck").and_then(Value::as_bool),
        archived: obj.get("archived").and_then(Value::as_bool).unwrap_or(false),
        sources: strings(obj.get("sources")),
        release,
        deck_assets: strings(evidence.and_then(|e| e.get("deck_assets"))),
        linux_assets: strings(evidence.and_then(|e| e.get("linux_assets"))),
        windows_assets: strings(evidence.and_then(|e| e.get("windows_assets"))),
        maintainer,
        first_seen: text(obj, "first_seen"),
        missing_since: text(obj, "missing_since"),
    })
}

pub fn parse(input: &str) -> Result<Index, String> {
    let data: Value = serde_json::from_str(crate::core::config::strip_bom(input)).map_err(|e| format!("not JSON: {e}"))?;
    let obj = data.as_object().ok_or("the root is not an object")?;
    match obj.get("schema").and_then(Value::as_u64) {
        Some(SCHEMA) => {}
        Some(other) => return Err(format!("schema {other} is newer than this app understands")),
        None => return Err("no schema number".to_string()),
    }
    let mut entries: Vec<Entry> = obj.get("projects").and_then(Value::as_array).map(|a| a.iter().filter_map(entry_from_value).collect()).unwrap_or_default();
    if entries.is_empty() {
        return Err("no usable project".to_string());
    }
    let mut seen = std::collections::HashSet::new();
    entries.retain(|e| seen.insert(e.id.clone()));
    Ok(Index { generated: text(obj, "generated").unwrap_or_default(), entries })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn project(id: &str, rating: &str, kind: &str) -> Value {
        json!({
            "id": id, "name": format!("Game {id}"), "system": "N64", "repo": format!("owner/{id}"),
            "repo_url": format!("https://github.com/owner/{id}"), "host": "github",
            "catalog_folder": null, "catalog_disabled": null, "website": null, "kind": kind,
            "deck_rating": rating, "readme_mentions_deck": true, "archived": false, "sources": ["recomp_json"],
            "evidence": {"release": {"tag": "v1.0", "date": "2026-10-18T11:59:13Z", "prerelease": false},
                         "deck_assets": [], "linux_assets": [format!("{id}-linux-x86_64.tar.gz")], "windows_assets": [], "asset_count": 2},
            "recomp_json": null, "first_seen": "2026-10-20"
        })
    }

    pub(crate) fn file(projects: Vec<Value>) -> String {
        json!({"schema": 1, "generated": "2026-10-21", "count": projects.len(), "projects": projects}).to_string()
    }

    #[test]
    fn reads_a_minimal_file() {
        let index = parse(&file(vec![project("a", "linux_build", "game"), project("b", "deck_build", "likely_game")])).unwrap();
        assert_eq!(index.generated, "2026-10-21");
        assert_eq!(index.entries.len(), 2);
        let a = &index.entries[0];
        assert_eq!((a.rating, a.kind, a.host), (Rating::LinuxBuild, Kind::Game, Host::Github));
        assert_eq!(a.repo.as_deref(), Some("owner/a"));
        assert_eq!(a.release.as_ref().unwrap().tag, "v1.0");
        assert_eq!(a.linux_assets, vec!["a-linux-x86_64.tar.gz"]);
        assert!(!a.catalog_disabled && !a.archived && a.missing_since.is_none());
        assert_eq!(index.entries[1].kind, Kind::LikelyGame);
    }

    #[test]
    fn skips_entries_it_cannot_use() {
        let mut no_id = project("x", "linux_build", "game");
        no_id.as_object_mut().unwrap().remove("id");
        let projects = vec![project("a", "proton_only", "game"), project("b", "linux_build", "tool"), no_id, project("c", "deck_build", "game")];
        let index = parse(&file(projects)).unwrap();
        assert_eq!(index.entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), vec!["c"]);
    }

    #[test]
    fn keeps_the_first_of_two_entries_with_one_id() {
        let mut second = project("a", "deck_build", "game");
        second["name"] = json!("Second");
        let index = parse(&file(vec![project("a", "linux_build", "game"), second])).unwrap();
        assert_eq!(index.entries.len(), 1);
        assert_eq!(index.entries[0].name, "Game a");
    }

    #[test]
    fn refuses_files_that_must_not_replace_a_saved_copy() {
        assert!(parse("not json").is_err());
        assert!(parse("[1, 2]").is_err());
        assert!(parse(&json!({"projects": [project("a", "linux_build", "game")]}).to_string()).is_err());
        assert!(parse(&json!({"schema": 2, "projects": [project("a", "linux_build", "game")]}).to_string()).is_err());
        assert!(parse(&file(vec![])).is_err());
        assert!(parse(&file(vec![project("a", "source_only", "game")])).is_err());
    }

    #[test]
    fn nulls_and_odd_types_fall_back_to_defaults() {
        let mut p = project("a", "linux_build", "game");
        p["catalog_disabled"] = json!(true);
        p["archived"] = json!(null);
        p["readme_mentions_deck"] = json!(null);
        p["system"] = json!(null);
        p["host"] = json!("direct");
        p["missing_since"] = json!("2026-10-27");
        p["recomp_json"] = json!({"status": "playable", "targets": ["Linux", 5, "Steam Deck"]});
        p["evidence"]["release"] = json!(null);
        let e = parse(&file(vec![p])).unwrap().entries.remove(0);
        assert!(e.catalog_disabled && !e.archived);
        assert_eq!((e.readme_mentions_deck, e.system, e.host, e.release), (None, None, Host::Other, None));
        assert_eq!(e.missing_since.as_deref(), Some("2026-10-27"));
        assert_eq!(e.maintainer.unwrap().targets, vec!["Linux", "Steam Deck"]);
    }

    #[test]
    fn reads_the_published_file() {
        let index = parse(include_str!("../../../index/deck-ready.json")).unwrap();
        assert!(index.entries.len() > 100, "{} entries", index.entries.len());
        assert!(index.entries.iter().any(|e| e.rating == Rating::DeckBuild));
        assert!(index.entries.iter().any(|e| e.kind == Kind::LikelyGame));
    }
}
