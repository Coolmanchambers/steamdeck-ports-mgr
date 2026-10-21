//! What the Deck-ready screen shows: which entries, in what order, with which tags, and what A does.
//! No UI here; `src/app/deck_ready_screen.rs` draws it.
//!
//! An entry the app already has is never offered twice. It is matched to a catalog port by the
//! catalog folder the index names, then by repository. A match installs or plays that port exactly as
//! All ports does (the catalog entry, or the user's own entry from `ports.local.json`). Only entries
//! with no match are added as a new own entry.

use super::index::{Entry, Host, Index, Kind, Rating};

/// What the screen needs to know about one port the app already has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownPort {
    pub key: String,
    pub folder: String,
    pub repo: Option<String>,
    pub user_managed: bool,
    pub installed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BuildFilter {
    #[default]
    All,
    Deck,
    Linux,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Filters {
    pub build: BuildFilter,
    pub system: Option<String>,
    /// Show `likely_game` entries (found by topic only).
    pub community: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Install or play the port the app already has, as All ports does.
    Existing { key: String },
    /// Add as the user's own entry in `ports.local.json`, then install it.
    AddAndInstall,
    /// Nothing to install from here: gone from the latest refresh, or not on GitHub or GitLab.
    InfoOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagKind {
    Plain,
    Dashed,
    Warning,
    Muted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tag {
    pub label: &'static str,
    pub kind: TagKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Not in the catalog (or turned off there) and not added yet.
    New,
    InCatalog,
    Installed,
    Yours,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Position in `Index::entries` of the index these rows were built from; only valid for that index.
    pub entry: usize,
    /// The entry's `id`. Use this to find the entry again later: a refresh can replace the index.
    pub id: String,
    pub name: String,
    pub sub: String,
    pub deck: bool,
    pub tags: Vec<Tag>,
    pub standing: Standing,
    pub action: Action,
    /// "Install", "Play" or "" (no button).
    pub button: &'static str,
}

pub fn date_long(iso: &str) -> Option<String> {
    let date = chrono::NaiveDate::parse_from_str(iso.get(..10)?, "%Y-%m-%d").ok()?;
    Some(date.format("%b %-d, %Y").to_string())
}

pub fn date_short(iso: &str) -> Option<String> {
    let date = chrono::NaiveDate::parse_from_str(iso.get(..10)?, "%Y-%m-%d").ok()?;
    Some(date.format("%b %-d").to_string())
}

/// The index names catalog entries "Game (folder)"; the folder part is noise on screen.
pub fn display_name(entry: &Entry) -> String {
    let name = entry.name.trim();
    if let Some(open) = name.strip_suffix(')').and_then(|n| n.rfind('(')) {
        let inner = name[open + 1..name.len() - 1].to_lowercase();
        let base = entry.repo.as_deref().and_then(|r| r.rsplit('/').next()).unwrap_or("").to_lowercase();
        let folder = entry.catalog_folder.as_deref().unwrap_or("").to_lowercase();
        if !inner.is_empty() && (inner == base || inner == folder) {
            let head = name[..open].trim_end();
            if !head.is_empty() {
                return head.to_string();
            }
        }
    }
    name.to_string()
}

pub fn match_known<'a>(entry: &Entry, known: &'a [KnownPort]) -> Option<&'a KnownPort> {
    if let Some(folder) = &entry.catalog_folder {
        if let Some(port) = known.iter().find(|p| &p.folder == folder) {
            return Some(port);
        }
    }
    let repo = entry.repo.as_deref().filter(|_| entry.host != Host::Other)?;
    known.iter().find(|p| p.repo.as_deref().is_some_and(|r| r.eq_ignore_ascii_case(repo)))
}

pub fn can_add(entry: &Entry) -> bool {
    entry.missing_since.is_none() && entry.host != Host::Other && entry.repo.is_some()
}

fn row(index: usize, entry: &Entry, known: &[KnownPort]) -> Row {
    let plain = |label| Tag { label, kind: TagKind::Plain };
    let mut tags = Vec::new();
    let (standing, action, button) = match match_known(entry, known) {
        Some(port) => {
            let button = if port.installed { "Play" } else { "Install" };
            let standing = if port.user_managed {
                tags.push(plain("Yours"));
                Standing::Yours
            } else if port.installed {
                Standing::Installed
            } else {
                tags.push(plain("Catalog"));
                Standing::InCatalog
            };
            (standing, Action::Existing { key: port.key.clone() }, button)
        }
        None => {
            if entry.catalog_disabled {
                tags.push(Tag { label: "Off in catalog", kind: TagKind::Muted });
            }
            if can_add(entry) { (Standing::New, Action::AddAndInstall, "Install") } else { (Standing::New, Action::InfoOnly, "") }
        }
    };
    if entry.release.as_ref().is_some_and(|r| r.prerelease) {
        tags.push(Tag { label: "Pre-release", kind: TagKind::Warning });
    }
    if entry.archived {
        tags.push(Tag { label: "Archived", kind: TagKind::Warning });
    }
    if entry.kind == Kind::LikelyGame {
        tags.push(Tag { label: "Community find", kind: TagKind::Dashed });
    }
    if entry.missing_since.is_some() {
        tags.push(Tag { label: "Not found", kind: TagKind::Warning });
    }
    let mut sub = vec![entry.system.clone().unwrap_or_else(|| "Unknown system".to_string())];
    if let Some(release) = &entry.release {
        sub.extend([Some(release.tag.clone()).filter(|t| !t.is_empty()), date_short(&release.date)].into_iter().flatten());
    }
    Row { entry: index, id: entry.id.clone(), name: display_name(entry), sub: sub.join("  ·  "), deck: entry.rating == Rating::DeckBuild, tags, standing, action, button }
}

fn shown(entry: &Entry, filters: &Filters) -> bool {
    (filters.community || entry.kind == Kind::Game)
        && match filters.build {
            BuildFilter::All => true,
            BuildFilter::Deck => entry.rating == Rating::DeckBuild,
            BuildFilter::Linux => entry.rating == Rating::LinuxBuild,
        }
        && filters.system.as_ref().is_none_or(|s| entry.system.as_ref() == Some(s))
}

fn matches_query(entry: &Entry, name: &str, query: &str) -> bool {
    query.is_empty() || [Some(name), entry.system.as_deref(), entry.repo.as_deref()].into_iter().flatten().any(|f| f.to_lowercase().contains(query))
}

/// The rows to show, Deck builds first, then the newest release, then by name.
pub fn rows(index: &Index, known: &[KnownPort], filters: &Filters, query: &str) -> Vec<Row> {
    let query = query.trim().to_lowercase();
    let mut out: Vec<Row> = index
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| shown(e, filters))
        .map(|(i, e)| row(i, e, known))
        .filter(|r| matches_query(&index.entries[r.entry], &r.name, &query))
        .collect();
    let date = |r: &Row| index.entries[r.entry].release.as_ref().map(|x| x.date.clone()).unwrap_or_default();
    out.sort_by(|a, b| b.deck.cmp(&a.deck).then_with(|| date(b).cmp(&date(a))).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

/// Systems to cycle through in Filters: most entries first, then by name.
pub fn systems(index: &Index, community: bool) -> Vec<String> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for e in index.entries.iter().filter(|e| community || e.kind == Kind::Game) {
        let Some(system) = &e.system else { continue };
        match counts.iter_mut().find(|(s, _)| s == system) {
            Some((_, n)) => *n += 1,
            None => counts.push((system.clone(), 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts.into_iter().map(|(s, _)| s).collect()
}

pub fn next_build(build: BuildFilter) -> BuildFilter {
    match build {
        BuildFilter::All => BuildFilter::Deck,
        BuildFilter::Deck => BuildFilter::Linux,
        BuildFilter::Linux => BuildFilter::All,
    }
}

pub fn next_system(current: Option<&str>, systems: &[String]) -> Option<String> {
    match current.and_then(|c| systems.iter().position(|s| s == c)) {
        None => systems.first().cloned(),
        Some(i) => systems.get(i + 1).cloned(),
    }
}

pub fn build_label(build: BuildFilter) -> &'static str {
    match build {
        BuildFilter::All => "All builds",
        BuildFilter::Deck => "Deck build only",
        BuildFilter::Linux => "Linux build only",
    }
}

pub fn filter_summary(filters: &Filters) -> String {
    let system = filters.system.as_deref().map_or("All systems".to_string(), |s| format!("System: {s}"));
    let community = if filters.community { "Community finds shown" } else { "Community finds hidden" };
    format!("{}  ·  {system}  ·  {community}", build_label(filters.build))
}

/// The rows of the Filters dialog; choosing one changes its value, the last one closes.
pub fn filter_lines(filters: &Filters) -> Vec<String> {
    vec![
        format!("Builds: {}", build_label(filters.build)),
        format!("System: {}", filters.system.as_deref().unwrap_or("All systems")),
        format!("Community finds: {} (found by GitHub topic only, may be tools)", if filters.community { "shown" } else { "hidden" }),
        "Done".to_string(),
    ]
}

fn rating_line(rating: Rating) -> (&'static str, &'static str) {
    match rating {
        Rating::DeckBuild => ("Deck build", "a file made for Steam Deck or SteamOS"),
        Rating::LinuxBuild => ("Linux build", "a Linux x86_64 file"),
    }
}

pub fn rating_name(rating: Rating) -> &'static str {
    rating_line(rating).0
}

pub fn source_label(entry: &Entry) -> String {
    match (entry.host, &entry.repo) {
        (Host::Github, Some(repo)) => format!("github.com/{repo}"),
        (Host::Gitlab, Some(repo)) => format!("gitlab.com/{repo}"),
        _ => entry.repo_url.clone().or_else(|| entry.website.clone()).unwrap_or_else(|| "unknown".to_string()),
    }
}

fn found_by(entry: &Entry) -> String {
    let mut names: Vec<&str> = Vec::new();
    for s in &entry.sources {
        let name = match s.split(':').next().unwrap_or("") {
            "catalog" => "Ports Launcher catalog",
            "recomp_json" => "maintainer's .recomp.json",
            _ => "GitHub topic",
        };
        if !names.contains(&name) {
            names.push(name);
        }
    }
    if names.is_empty() { "unknown".to_string() } else { names.join(", ") }
}

/// "today 09:14" or "Oct 20, 09:14" in `now`'s time zone; "not yet" when there is no check.
pub fn checked_label<Tz: chrono::TimeZone>(checked_at: &str, now: &chrono::DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let Ok(at) = chrono::DateTime::parse_from_rfc3339(checked_at) else { return "not yet".to_string() };
    let at = at.with_timezone(&now.timezone());
    if at.date_naive() == now.date_naive() { at.format("today %H:%M").to_string() } else { at.format("%b %-d, %H:%M").to_string() }
}

pub fn warnings(entry: &Entry, standing: Standing) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(since) = &entry.missing_since {
        out.push(format!("Not found in the latest refresh (since {}). It may have been removed or renamed.", date_long(since).unwrap_or_else(|| since.clone())));
    }
    if entry.kind == Kind::LikelyGame {
        out.push("Found by GitHub topic only. It may be a tool, not a game.".to_string());
    }
    if entry.release.as_ref().is_some_and(|r| r.prerelease) {
        out.push("Only a pre-release is available.".to_string());
    }
    if entry.archived {
        out.push("This repository is archived. No new releases are expected.".to_string());
    }
    if entry.catalog_disabled && standing == Standing::New {
        out.push("The catalog has this entry turned off. Install adds your own entry instead.".to_string());
    }
    if standing == Standing::New && entry.host == Host::Other {
        out.push("This project is not on GitHub or GitLab, so it can only be installed through a catalog entry.".to_string());
    }
    out
}

/// The body of the Info dialog. `checked` comes from `checked_label`.
pub fn info_text(entry: &Entry, standing: Standing, list_changed: &str, checked: &str) -> String {
    let (rating, rating_short) = rating_line(entry.rating);
    let mut lines: Vec<String> = warnings(entry, standing).into_iter().map(|w| format!("! {w}")).collect();
    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines.push(format!("Rating: {rating} ({rating_short})"));
    let release = match &entry.release {
        Some(r) => {
            let date = date_long(&r.date).unwrap_or_else(|| "date unknown".to_string());
            format!("{}, {date}{}", if r.tag.is_empty() { "untagged" } else { &r.tag }, if r.prerelease { " (pre-release)" } else { "" })
        }
        None => "unknown".to_string(),
    };
    lines.push(format!("Latest release: {release}"));
    lines.push(format!("Source: {}", source_label(entry)));
    lines.push(format!("System: {}", entry.system.as_deref().unwrap_or("unknown")));
    let catalog = match standing {
        Standing::Yours => "This is your own entry (ports.local.json).",
        Standing::Installed => "Yes, and it is installed.",
        Standing::InCatalog => "Yes. Install works like in All ports.",
        Standing::New if entry.catalog_disabled => "Turned off in the catalog.",
        Standing::New if can_add(entry) => "Not yet. Install adds it as your own entry.",
        Standing::New => "No.",
    };
    lines.push(format!("In the catalog: {catalog}"));
    let readme = match entry.readme_mentions_deck {
        Some(true) => "Steam Deck is mentioned",
        Some(false) => "Steam Deck is not mentioned",
        None => "not checked",
    };
    lines.push(format!("README says: {readme}"));
    let maintainer = match &entry.maintainer {
        Some(m) => {
            let mut parts = vec![m.status.clone().unwrap_or_else(|| "status not given".to_string())];
            if !m.targets.is_empty() {
                parts.push(m.targets.join(", "));
            }
            parts.join("  ·  ")
        }
        None => "no .recomp.json file in the repository".to_string(),
    };
    lines.push(format!("Maintainer says: {maintainer}"));
    let first = entry.first_seen.as_deref().and_then(date_long).map(|d| format!("  ·  first listed {d}")).unwrap_or_default();
    lines.push(format!("Found by: {}{first}", found_by(entry)));
    let changed = date_long(list_changed).unwrap_or_else(|| "unknown".to_string());
    lines.push(format!("Checked: list changed {changed}  ·  this app checked {checked}"));
    lines.push(String::new());
    lines.push("Release files the rating is based on:".to_string());
    let files: Vec<String> = entry
        .deck_assets
        .iter()
        .map(|f| format!("  Deck build: {f}"))
        .chain(entry.linux_assets.iter().map(|f| format!("  Linux: {f}")))
        .chain(entry.windows_assets.iter().map(|f| format!("  Windows (not used): {f}")))
        .collect();
    if files.is_empty() {
        lines.push("  none listed".to_string());
    }
    lines.extend(files);
    lines.push(String::new());
    let explain = match entry.rating {
        Rating::DeckBuild => {
            "The latest release has a file whose name says Steam Deck or SteamOS. That is a rating from file names. It does not mean someone ran it. Only a launch on your Deck proves it."
        }
        Rating::LinuxBuild => {
            "The latest release has a Linux x86_64 file, but none made for the Deck in particular. Many run fine on a Deck. Nobody has checked this one. Only a launch on your Deck proves it."
        }
    };
    lines.push(format!("What \"{rating}\" means: {explain}"));
    lines.join("\n")
}

/// The body of the "Add ... ?" dialog for an entry that becomes the user's own.
pub fn confirm_text(entry: &Entry, folder: &str) -> String {
    let file = match (entry.rating, entry.deck_assets.as_slice(), entry.linux_assets.as_slice()) {
        (Rating::DeckBuild, [one], _) => format!("{one} (Deck build)"),
        (Rating::DeckBuild, many, _) => format!("a Deck build file (you choose from {})", many.len()),
        (Rating::LinuxBuild, _, [one]) => format!("{one} (Linux build)"),
        (Rating::LinuxBuild, _, _) => "a Linux file (if the installer can't tell which, you choose)".to_string(),
    };
    let from = match &entry.release {
        Some(r) => format!("{}, {}  ·  {}", r.tag, date_long(&r.date).unwrap_or_default(), source_label(entry)),
        None => source_label(entry),
    };
    let mut lines = vec![
        format!("Download: {file}"),
        format!("From: {from}"),
        format!("Goes into: Library/{folder}"),
        "Your entry: saved in ports.local.json (the old file is backed up). Its button says Open folder, not Uninstall.".to_string(),
        "Game files: not included. Info shows the project page, which says what it needs.".to_string(),
        "Saves: adding and installing never touch saves. The app doesn't know where this port keeps them, so its updates are not backed up first: copy its save folder yourself before an update.".to_string(),
    ];
    if entry.catalog_disabled {
        lines.push(String::new());
        lines.push("The catalog has this entry turned off. This adds your own entry and leaves the catalog alone.".to_string());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::deck_ready::index::parse;
    use crate::fork::deck_ready::index::tests::{file, project};
    use serde_json::json;

    fn known(folder: &str, repo: Option<&str>, user_managed: bool, installed: bool) -> KnownPort {
        KnownPort { key: repo.unwrap_or(folder).to_string(), folder: folder.into(), repo: repo.map(str::to_string), user_managed, installed }
    }

    fn index_of(projects: Vec<serde_json::Value>) -> Index {
        parse(&file(projects)).unwrap()
    }

    #[test]
    fn display_name_drops_only_the_folder_note() {
        let mut p = project("ace", "linux_build", "game");
        p["name"] = json!("Ace Combat 3 (Japan) (ace-combat-3)");
        p["catalog_folder"] = json!("ace-combat-3");
        let index = index_of(vec![p, json!({"id": "x", "name": "Game (Fan Port)", "kind": "game", "deck_rating": "linux_build", "repo": "o/x"})]);
        assert_eq!(display_name(&index.entries[0]), "Ace Combat 3 (Japan)");
        assert_eq!(display_name(&index.entries[1]), "Game (Fan Port)");
    }

    #[test]
    fn matching_uses_the_catalog_folder_then_the_repository() {
        let mut by_folder = project("a", "deck_build", "game");
        by_folder["catalog_folder"] = json!("A-Folder");
        let mut by_repo = project("b", "linux_build", "game");
        by_repo["repo"] = json!("Owner/B");
        let index = index_of(vec![by_folder, by_repo, project("c", "linux_build", "game")]);
        let ports = vec![known("A-Folder", Some("someone/else"), false, false), known("b-folder", Some("owner/b"), false, true)];
        assert_eq!(match_known(&index.entries[0], &ports).unwrap().folder, "A-Folder");
        assert_eq!(match_known(&index.entries[1], &ports).unwrap().folder, "b-folder");
        assert!(match_known(&index.entries[2], &ports).is_none());
    }

    #[test]
    fn a_download_page_entry_never_matches_by_repository() {
        let mut p = project("d", "linux_build", "game");
        p["host"] = json!("direct");
        let index = index_of(vec![p]);
        assert!(match_known(&index.entries[0], &[known("x", Some("owner/d"), false, false)]).is_none());
        assert!(!can_add(&index.entries[0]));
    }

    #[test]
    fn each_standing_gets_its_tag_button_and_action() {
        let mut in_catalog = project("cat", "linux_build", "game");
        in_catalog["catalog_folder"] = json!("cat");
        let mut installed = project("inst", "linux_build", "game");
        installed["catalog_folder"] = json!("inst");
        let mut yours = project("own", "deck_build", "game");
        yours["catalog_folder"] = json!("own");
        yours["catalog_disabled"] = json!(true);
        let mut off = project("off", "deck_build", "game");
        off["catalog_folder"] = json!("off");
        off["catalog_disabled"] = json!(true);
        let mut gone = project("gone", "linux_build", "game");
        gone["missing_since"] = json!("2026-10-27");
        let index = index_of(vec![in_catalog, installed, yours, off, gone, project("new", "linux_build", "game")]);
        let ports = vec![known("cat", Some("owner/cat"), false, false), known("inst", Some("owner/inst"), false, true), known("own", Some("owner/own"), true, true)];
        let rows = rows(&index, &ports, &Filters::default(), "");
        let by = |name: &str| rows.iter().find(|r| r.name == format!("Game {name}")).unwrap();
        let labels = |r: &Row| r.tags.iter().map(|t| t.label).collect::<Vec<_>>();

        assert_eq!((by("cat").standing, by("cat").button, labels(by("cat"))), (Standing::InCatalog, "Install", vec!["Catalog"]));
        assert_eq!(by("cat").action, Action::Existing { key: "owner/cat".into() });
        assert_eq!((by("inst").standing, by("inst").button), (Standing::Installed, "Play"));
        assert!(by("inst").tags.is_empty());
        assert_eq!((by("own").standing, by("own").button, labels(by("own"))), (Standing::Yours, "Play", vec!["Yours"]));
        assert_eq!((by("off").action.clone(), labels(by("off"))), (Action::AddAndInstall, vec!["Off in catalog"]));
        assert_eq!((by("gone").action.clone(), by("gone").button, labels(by("gone"))), (Action::InfoOnly, "", vec!["Not found"]));
        assert_eq!((by("new").action.clone(), by("new").button), (Action::AddAndInstall, "Install"));
    }

    #[test]
    fn defaults_hide_community_finds_and_sort_deck_builds_first() {
        let mut old_deck = project("old-deck", "deck_build", "game");
        old_deck["evidence"]["release"]["date"] = json!("2025-01-01T00:00:00Z");
        let mut new_linux = project("new-linux", "linux_build", "game");
        new_linux["evidence"]["release"]["date"] = json!("2026-10-20T00:00:00Z");
        let mut finds = project("find", "deck_build", "likely_game");
        finds["system"] = json!("PS1");
        let index = index_of(vec![new_linux, project("mid-linux", "linux_build", "game"), finds, old_deck]);
        let names = |f: &Filters, q: &str| rows(&index, &[], f, q).into_iter().map(|r| r.name).collect::<Vec<_>>();

        assert_eq!(names(&Filters::default(), ""), vec!["Game old-deck", "Game new-linux", "Game mid-linux"]);
        let with_finds = Filters { community: true, ..Default::default() };
        assert_eq!(names(&with_finds, "")[0], "Game find");
        assert_eq!(names(&Filters { build: BuildFilter::Linux, ..Default::default() }, ""), vec!["Game new-linux", "Game mid-linux"]);
        assert_eq!(names(&Filters { system: Some("PS1".into()), community: true, ..Default::default() }, ""), vec!["Game find"]);
        assert_eq!(names(&Filters::default(), "  MID "), vec!["Game mid-linux"]);
        assert_eq!(names(&Filters::default(), "owner/new"), vec!["Game new-linux"]);
    }

    #[test]
    fn systems_and_filter_cycles() {
        let mut ps1 = project("p", "linux_build", "game");
        ps1["system"] = json!("PS1");
        let mut find = project("f", "linux_build", "likely_game");
        find["system"] = json!("Xbox 360");
        let index = index_of(vec![project("a", "linux_build", "game"), project("b", "linux_build", "game"), ps1, find]);
        assert_eq!(systems(&index, false), vec!["N64", "PS1"]);
        assert_eq!(systems(&index, true), vec!["N64", "PS1", "Xbox 360"]);
        let list = systems(&index, false);
        assert_eq!(next_system(None, &list).as_deref(), Some("N64"));
        assert_eq!(next_system(Some("N64"), &list).as_deref(), Some("PS1"));
        assert_eq!(next_system(Some("PS1"), &list), None);
        assert_eq!(next_system(Some("Gone"), &list).as_deref(), Some("N64"));
        assert_eq!(next_build(next_build(next_build(BuildFilter::All))), BuildFilter::All);
        assert_eq!(filter_lines(&Filters::default()).len(), 4);
        assert_eq!(filter_summary(&Filters::default()), "All builds  ·  All systems  ·  Community finds hidden");
    }

    #[test]
    fn info_text_explains_the_rating_and_warns() {
        let mut p = project("w", "deck_build", "likely_game");
        p["catalog_disabled"] = json!(true);
        p["archived"] = json!(true);
        p["evidence"]["release"]["prerelease"] = json!(true);
        p["evidence"]["deck_assets"] = json!(["w-steamdeck.tar.gz"]);
        p["evidence"]["windows_assets"] = json!(["w-win64.zip"]);
        p["recomp_json"] = json!({"status": "playable", "targets": ["Linux", "Steam Deck"]});
        let index = index_of(vec![p]);
        let text = info_text(&index.entries[0], Standing::New, "2026-10-21", "today 09:14");
        for want in [
            "! Found by GitHub topic only",
            "! Only a pre-release is available.",
            "! This repository is archived",
            "! The catalog has this entry turned off",
            "Rating: Deck build (a file made for Steam Deck or SteamOS)",
            "Latest release: v1.0, Oct 18, 2026 (pre-release)",
            "Source: github.com/owner/w",
            "In the catalog: Turned off in the catalog.",
            "Maintainer says: playable  ·  Linux, Steam Deck",
            "Found by: maintainer's .recomp.json  ·  first listed Oct 20, 2026",
            "Checked: list changed Oct 21, 2026  ·  this app checked today 09:14",
            "  Deck build: w-steamdeck.tar.gz",
            "  Windows (not used): w-win64.zip",
            "What \"Deck build\" means:",
        ] {
            assert!(text.contains(want), "missing {want:?} in\n{text}");
        }
    }

    #[test]
    fn confirm_text_names_the_file_and_the_folder() {
        let mut deck = project("d", "deck_build", "game");
        deck["evidence"]["deck_assets"] = json!(["d-SteamDeck.tar.gz"]);
        deck["catalog_disabled"] = json!(true);
        let index = index_of(vec![deck, project("l", "linux_build", "game")]);
        let deck_text = confirm_text(&index.entries[0], "d");
        assert!(deck_text.contains("Download: d-SteamDeck.tar.gz (Deck build)"));
        assert!(deck_text.contains("Goes into: Library/d"));
        assert!(deck_text.contains("turned off"));
        let linux_text = confirm_text(&index.entries[1], "l");
        assert!(linux_text.contains("Download: l-linux-x86_64.tar.gz (Linux build)"));
        assert!(!linux_text.contains("turned off"));
    }

    #[test]
    fn checked_label_is_relative_to_today() {
        use chrono::TimeZone;
        let zone = chrono::FixedOffset::west_opt(7 * 3600).unwrap();
        let now = zone.with_ymd_and_hms(2026, 10, 21, 10, 0, 0).unwrap();
        assert_eq!(checked_label("2026-10-21T16:14:00Z", &now), "today 09:14");
        assert_eq!(checked_label("2026-10-20T16:14:00Z", &now), "Oct 20, 09:14");
        assert_eq!(checked_label("", &now), "not yet");
    }
}
