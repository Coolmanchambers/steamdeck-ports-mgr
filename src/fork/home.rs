//! The home screen's model: what the app shows first. "Your ports" as cover tiles for the installed ports, each with
//! one status badge; a Continue tile first for the port played last; and, with nothing installed, one card to set
//! the app up or browse ports. Tested without any UI; `src/app/home_screen.rs` gathers the facts and draws it.

/// The one badge a port tile shows: the thing that most needs doing, else Ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Badge {
    Ready,
    UpdateAvailable,
    GameFilesMissing,
    /// The entry names only a game title: the app cannot check for it, so this is a thing to do, not red.
    AddGameFiles,
    SteamWaiting,
    SavesNotFound,
}

impl Badge {
    /// Short, so it fits under a tile on a 7 inch screen.
    pub fn label(self) -> &'static str {
        match self {
            Badge::Ready => "Ready",
            Badge::UpdateAvailable => "Update ready",
            Badge::GameFilesMissing => "No game files",
            Badge::AddGameFiles => "Add game files",
            Badge::SteamWaiting => "Steam waiting",
            Badge::SavesNotFound => "Find saves",
        }
    }

    /// The status mark drawn with it (the same pictures as the guided pages' summary lines). Red only for a game that
    /// will not start; saves to find and an update are things to do.
    pub fn mark(self) -> crate::fork::page::Mark {
        use crate::fork::page::Mark;
        match self {
            Badge::Ready => Mark::Done,
            Badge::UpdateAvailable => Mark::ToDo,
            Badge::GameFilesMissing => Mark::Problem,
            Badge::AddGameFiles => Mark::ToDo,
            Badge::SteamWaiting => Mark::Waiting,
            Badge::SavesNotFound => Mark::ToDo,
        }
    }
}

/// What the app knows about one installed port, from the sources it already keeps: the update check, the game files
/// records, the Steam queue, the save finder, the app's and Steam's play records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortFacts {
    pub key: String,
    pub name: String,
    pub update_available: bool,
    pub game_files_missing: bool,
    /// Names a game by title only and nothing says it is there (not red).
    pub game_files_to_add: bool,
    pub steam_waiting: bool,
    pub saves_not_found: bool,
    /// When it was last played, seconds since 1970 (the later of the app's record and Steam's), 0 when never.
    pub last_played: i64,
}

/// The badge of a port: a missing game file first (the game will not start), then an update, then Steam still
/// pointing at an old copy, then saves the app cannot back up; Ready when none of these.
pub fn badge(p: &PortFacts) -> Badge {
    if p.game_files_missing {
        Badge::GameFilesMissing
    } else if p.update_available {
        Badge::UpdateAvailable
    } else if p.game_files_to_add {
        Badge::AddGameFiles
    } else if p.steam_waiting {
        Badge::SteamWaiting
    } else if p.saves_not_found {
        Badge::SavesNotFound
    } else {
        Badge::Ready
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileKind {
    /// "Continue": the port played last.
    Continue,
    Port,
    /// "Finish setting up": shown first while setup was never finished (ports are installed already).
    Setup,
    /// "Add a port": always the last tile; opens the Add a port page.
    Add,
    /// "Fix N problems": only while the checkup (`fork::checkup`) found something; opens the Fix problems pages.
    Checkup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    pub kind: TileKind,
    /// The port's key (empty for the Setup tile).
    pub key: String,
    pub title: String,
    pub badge: Badge,
    /// The Checkup tile's own badge line and mark (the other tiles use `badge`).
    pub checkup: Option<crate::fork::checkup::HomeTile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Home {
    /// Nothing installed: one card, "Set up Deck Ports Manager" and "Browse ports".
    Empty,
    Tiles(Vec<Tile>),
}

/// The port to continue: the one played last (ties go to the first in the list). `None` when nothing was played.
pub fn continue_pick(ports: &[PortFacts]) -> Option<usize> {
    let mut best: Option<usize> = None;
    for (i, p) in ports.iter().enumerate() {
        if p.last_played > 0 && best.is_none_or(|b| p.last_played > ports[b].last_played) {
            best = Some(i);
        }
    }
    best
}

/// The home screen for these installed ports. With none, the empty card. Otherwise: a Setup tile first while setup
/// is not done, then Continue (the port played last, not repeated below), then "Fix N problems" while the checkup has
/// something (just after Continue, so the cursor still starts on the game to play), then every other port by name, and
/// last the "Add a port" tile.
pub fn build(ports: &[PortFacts], setup_done: bool, checkup: Option<&crate::fork::checkup::HomeTile>) -> Home {
    if ports.is_empty() {
        return Home::Empty;
    }
    let tile = |kind, key: &str, title: &str, badge| Tile { kind, key: key.to_string(), title: title.to_string(), badge, checkup: None };
    let mut tiles = Vec::new();
    if !setup_done {
        tiles.push(tile(TileKind::Setup, "", "Finish setting up", Badge::Ready));
    }
    let first = continue_pick(ports);
    if let Some(i) = first {
        let p = &ports[i];
        tiles.push(tile(TileKind::Continue, &p.key, &p.name, badge(p)));
    }
    if let Some(c) = checkup {
        tiles.push(Tile { checkup: Some(c.clone()), ..tile(TileKind::Checkup, "", &c.title, Badge::Ready) });
    }
    let mut rest: Vec<&PortFacts> = ports.iter().enumerate().filter(|(i, _)| Some(*i) != first).map(|(_, p)| p).collect();
    rest.sort_by_key(|p| p.name.to_lowercase());
    tiles.extend(rest.into_iter().map(|p| tile(TileKind::Port, &p.key, &p.name, badge(p))));
    tiles.push(tile(TileKind::Add, "", "Add a port", Badge::Ready));
    Home::Tiles(tiles)
}

/// The controller hint bar for what is focused (the real bindings: A plays, X opens the Port page (Fork, port page), Y opens Settings, L1/R1 and
/// Tab switch lists). `None` focus: the empty card.
pub fn hints(focused: Option<TileKind>) -> String {
    let gap = crate::fork::page::HINT_GAP;
    match focused {
        None => ["A Choose", "D-pad Move", "Y Settings", "L1/R1 Lists"].join(gap),
        Some(TileKind::Setup) => ["A Set up", "D-pad Move", "Y Settings", "L1/R1 Lists"].join(gap),
        Some(TileKind::Add) => ["A Add", "D-pad Move", "Y Settings", "L1/R1 Lists"].join(gap),
        Some(TileKind::Checkup) => ["A Fix", "D-pad Move", "Y Settings", "L1/R1 Lists"].join(gap),
        Some(_) => ["A Play", "X Options", "D-pad Move", "Y Settings", "L1/R1 Lists"].join(gap),
    }
}

/// The lists L1/R1 (and Tab, and the footer button) step through, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum List {
    Home,
    AllPorts,
    DeckReady,
}

impl List {
    pub fn next(self) -> List {
        match self {
            List::Home => List::AllPorts,
            List::AllPorts => List::DeckReady,
            List::DeckReady => List::Home,
        }
    }

    /// The next list as the screen is: in big mode All ports shows only the installed ports (upstream's card grid), so
    /// the list of ports to install (Deck-ready) comes right after Home there.
    pub fn next_in(self, big_mode: bool) -> List {
        if !big_mode {
            return self.next();
        }
        match self {
            List::Home => List::DeckReady,
            List::DeckReady => List::AllPorts,
            List::AllPorts => List::Home,
        }
    }

    /// Where B (or Escape) goes from this list: from Deck-ready and All ports back to Home, in both modes (Fork, port page:
    /// Tim was stuck on a list with no way back); on Home B does nothing.
    pub fn back(self) -> Option<List> {
        match self {
            List::DeckReady | List::AllPorts => Some(List::Home),
            List::Home => None,
        }
    }

    /// The words of the hint for B on this list: "Clear search" while there is search text (the first B clears it), else
    /// "Home"; empty on Home.
    pub fn back_hint(self, searching: bool) -> &'static str {
        match (self, searching) {
            (List::Home, _) => "",
            (_, true) => "B / Esc  Clear search",
            (_, false) => "B / Esc  Home",
        }
    }

    /// Where "Browse ports" goes: a list of ports to install.
    pub fn browse(big_mode: bool) -> List {
        if big_mode { List::DeckReady } else { List::AllPorts }
    }

    pub fn name(self) -> &'static str {
        match self {
            List::Home => "Home",
            List::AllPorts => "All ports",
            List::DeckReady => "Deck-ready",
        }
    }
}

/// Whether Play's own update check is due for an installed port: "check for updates" on, the port follows a GitHub or
/// GitLab release, the port's own update switch on (no record counts as on), and the install older than the time
/// Play waits before asking again. Play and the home screen both ask this.
pub fn update_check_due(release_sync: bool, source: &crate::core::models::SourceType, port_update_on: Option<bool>, installed_at: &str) -> bool {
    use crate::core::models::SourceType;
    release_sync && matches!(source, SourceType::Github | SourceType::Gitlab) && port_update_on.unwrap_or(true) && crate::core::state::is_stale_for_update_check(installed_at)
}

/// The home screen asks for updates at most this often (and only when "check for updates" is on).
pub const UPDATE_CHECK_EVERY_SECS: i64 = 12 * 3600;

/// Whether the home screen's update check may run now, after the last one at `last` (seconds since 1970).
pub fn update_round_due(last: Option<i64>, now: i64) -> bool {
    last.is_none_or(|t| now < t || now - t >= UPDATE_CHECK_EVERY_SECS)
}

/// What a check found for one port and when (seconds since 1970). Shown until a newer check or a newer install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateSeen {
    pub available: bool,
    pub at: i64,
}

/// The badge's "Update ready": a check found one after the port was last installed.
pub fn update_shown(seen: Option<UpdateSeen>, installed_at: i64) -> bool {
    seen.is_some_and(|s| s.available && s.at >= installed_at)
}

/// The results kept in `fork-settings.json` (`home_update_checks`), read back.
pub fn updates_from_json(v: Option<&serde_json::Value>) -> std::collections::HashMap<String, UpdateSeen> {
    v.and_then(serde_json::Value::as_object)
        .map(|o| {
            o.iter()
                .filter_map(|(k, e)| Some((k.clone(), UpdateSeen { available: e.get("available")?.as_bool()?, at: e.get("at")?.as_i64()? })))
                .collect()
        })
        .unwrap_or_default()
}

pub fn updates_to_json(m: &std::collections::HashMap<String, UpdateSeen>) -> serde_json::Value {
    serde_json::Value::Object(m.iter().map(|(k, s)| (k.clone(), serde_json::json!({ "available": s.available, "at": s.at }))).collect())
}

/// A failed check that means "stop asking for now" (the API's rate limit).
pub fn is_rate_limit(error: &str) -> bool {
    let e = error.to_lowercase();
    e.contains("rate limit") || e.contains("429") || e.contains("too many requests")
}

/// When a port was last played by the app's record ("2026-10-24T18:00:00Z"), seconds since 1970; 0 when never or
/// unreadable.
pub fn app_played_at(stamp: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(stamp).map_or(0, |t| t.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(key: &str, played: i64) -> PortFacts {
        PortFacts { key: key.into(), name: key.to_uppercase(), last_played: played, ..PortFacts::default() }
    }

    #[test]
    fn each_port_gets_the_badge_that_most_needs_doing() {
        let mut p = port("soh", 0);
        assert_eq!(badge(&p), Badge::Ready);
        p.saves_not_found = true;
        assert_eq!(badge(&p), Badge::SavesNotFound);
        p.steam_waiting = true;
        assert_eq!(badge(&p), Badge::SteamWaiting);
        p.update_available = true;
        assert_eq!(badge(&p), Badge::UpdateAvailable);
        p.game_files_to_add = true;
        assert_eq!(badge(&p), Badge::UpdateAvailable, "an update comes before game files that cannot be checked");
        p.update_available = false;
        assert_eq!(badge(&p), Badge::AddGameFiles);
        assert_eq!(Badge::AddGameFiles.label(), "Add game files");
        assert_eq!(Badge::AddGameFiles.mark(), crate::fork::page::Mark::ToDo, "not red: the app cannot check a title");
        p.update_available = true;
        p.game_files_missing = true;
        assert_eq!(badge(&p), Badge::GameFilesMissing, "a game that will not start comes first");
        assert_eq!(Badge::GameFilesMissing.label(), "No game files");
        assert_eq!(Badge::SavesNotFound.mark(), crate::fork::page::Mark::ToDo, "red only for a game that will not start");
        assert_eq!(Badge::GameFilesMissing.mark(), crate::fork::page::Mark::Problem);
    }

    #[test]
    fn four_ports_in_different_states_make_a_continue_tile_then_the_rest_by_name() {
        let mut ports = vec![port("dusk", 100), port("abuse", 0), port("soh", 500), port("2ship", 0)];
        ports[0].update_available = true;
        ports[1].game_files_missing = true;
        ports[3].steam_waiting = true;
        let Home::Tiles(tiles) = build(&ports, true, None) else { panic!("tiles") };
        assert_eq!(tiles[0], Tile { kind: TileKind::Continue, key: "soh".into(), title: "SOH".into(), badge: Badge::Ready, checkup: None });
        let rest: Vec<(&str, Badge)> = tiles[1..tiles.len() - 1].iter().map(|t| (t.key.as_str(), t.badge)).collect();
        assert_eq!(rest, [("2ship", Badge::SteamWaiting), ("abuse", Badge::GameFilesMissing), ("dusk", Badge::UpdateAvailable)], "by name, Continue not repeated");
        assert!(tiles[1..tiles.len() - 1].iter().all(|t| t.kind == TileKind::Port));
        assert_eq!(tiles.last().map(|t| (t.kind, t.title.as_str())), Some((TileKind::Add, "Add a port")), "the plus tile is always last");
    }

    #[test]
    fn the_fix_problems_tile_comes_after_continue_only_when_there_is_something_to_fix() {
        use crate::fork::checkup::HomeTile;
        let ports = [port("a", 10), port("b", 30)];
        let Home::Tiles(quiet) = build(&ports, true, None) else { panic!() };
        assert!(quiet.iter().all(|t| t.kind != TileKind::Checkup), "no tile when the checkup found nothing");
        let tile = HomeTile { title: "Fix 2 problems".into(), note: "Game files and 1 more".into(), mark: crate::fork::page::Mark::Problem };
        let Home::Tiles(tiles) = build(&ports, true, Some(&tile)) else { panic!() };
        let kinds: Vec<TileKind> = tiles.iter().map(|t| t.kind).collect();
        assert_eq!(kinds, [TileKind::Continue, TileKind::Checkup, TileKind::Port, TileKind::Add], "the cursor still starts on the game to play");
        assert_eq!(tiles[1].title, "Fix 2 problems");
        assert_eq!(tiles[1].checkup.as_ref().map(|c| c.mark), Some(crate::fork::page::Mark::Problem));
        let Home::Tiles(setup) = build(&ports, false, Some(&tile)) else { panic!() };
        assert_eq!(setup.iter().map(|t| t.kind).collect::<Vec<_>>(), [TileKind::Setup, TileKind::Continue, TileKind::Checkup, TileKind::Port, TileKind::Add]);
        let Home::Tiles(unplayed) = build(&[port("a", 0)], true, Some(&tile)) else { panic!() };
        assert_eq!(unplayed.iter().map(|t| t.kind).collect::<Vec<_>>(), [TileKind::Checkup, TileKind::Port, TileKind::Add]);
        assert_eq!(build(&[], true, Some(&tile)), Home::Empty, "nothing installed: nothing to fix");
        assert!(hints(Some(TileKind::Checkup)).starts_with("A Fix"));
        assert!(!hints(Some(TileKind::Checkup)).contains("X Options"));
    }

    #[test]
    fn continue_is_the_port_played_last_and_absent_when_none_was_played() {
        assert_eq!(continue_pick(&[port("a", 10), port("b", 30), port("c", 20)]), Some(1));
        assert_eq!(continue_pick(&[port("a", 0), port("b", 0)]), None);
        let Home::Tiles(tiles) = build(&[port("a", 0), port("b", 0)], true, None) else { panic!() };
        assert!(tiles.iter().filter(|t| t.kind != TileKind::Add).all(|t| t.kind == TileKind::Port), "nothing played: no Continue tile");
    }

    #[test]
    fn nothing_installed_is_the_empty_card_and_unfinished_setup_puts_its_tile_first() {
        assert_eq!(build(&[], true, None), Home::Empty);
        assert_eq!(build(&[], false, None), Home::Empty);
        let Home::Tiles(tiles) = build(&[port("a", 5)], false, None) else { panic!() };
        assert_eq!((tiles[0].kind, tiles[1].kind), (TileKind::Setup, TileKind::Continue));
        assert_eq!(tiles.len(), 3, "Setup, Continue, and the Add a port tile");
    }

    #[test]
    fn the_lists_go_round_home_all_ports_deck_ready() {
        assert_eq!(List::Home.next(), List::AllPorts);
        assert_eq!(List::AllPorts.next(), List::DeckReady);
        assert_eq!(List::DeckReady.next(), List::Home);
    }

    #[test]
    fn in_big_mode_the_ports_to_install_come_right_after_home() {
        assert_eq!(List::Home.next_in(true), List::DeckReady, "big-mode All ports shows only installed ports");
        assert_eq!(List::DeckReady.next_in(true), List::AllPorts);
        assert_eq!(List::AllPorts.next_in(true), List::Home);
        assert_eq!(List::Home.next_in(false), List::AllPorts);
        assert_eq!((List::browse(true), List::browse(false)), (List::DeckReady, List::AllPorts));
        assert_eq!(List::DeckReady.back(), Some(List::Home), "B leaves Deck-ready for Home, also in big mode where next_in goes to All ports");
        assert_eq!((List::Home.back(), List::AllPorts.back()), (None, Some(List::Home)), "B goes Home from All ports too");
        assert_eq!(List::AllPorts.back_hint(true), "B / Esc  Clear search");
        assert_eq!(List::DeckReady.back_hint(false), "B / Esc  Home");
        assert_eq!(List::Home.back_hint(true), "");
    }

    #[test]
    fn the_update_check_is_due_exactly_when_plays_is() {
        use crate::core::models::SourceType;
        let old = "2020-01-01T00:00:00Z";
        assert!(update_check_due(true, &SourceType::Github, Some(true), old));
        assert!(update_check_due(true, &SourceType::Gitlab, None, old), "no record counts as the switch on");
        assert!(!update_check_due(false, &SourceType::Github, Some(true), old), "check for updates off");
        assert!(!update_check_due(true, &SourceType::Github, Some(false), old), "the port's own switch off");
        assert!(!update_check_due(true, &SourceType::DirectUrl, Some(true), old), "no release to compare with");
        let just_now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        assert!(!update_check_due(true, &SourceType::Github, Some(true), &just_now), "installed a moment ago");
    }

    #[test]
    fn the_home_check_runs_at_most_every_12_hours_and_keeps_what_it_found() {
        assert!(update_round_due(None, 1_000));
        assert!(!update_round_due(Some(1_000), 1_000 + UPDATE_CHECK_EVERY_SECS - 1));
        assert!(update_round_due(Some(1_000), 1_000 + UPDATE_CHECK_EVERY_SECS));
        assert!(update_round_due(Some(5_000), 1_000), "a clock set back does not block it for good");
        let mut m = std::collections::HashMap::new();
        m.insert("a/b".to_string(), UpdateSeen { available: true, at: 900 });
        assert_eq!(updates_from_json(Some(&updates_to_json(&m))), m, "a restart shows the last known badge");
        assert!(update_shown(m.get("a/b").copied(), 800));
        assert!(!update_shown(m.get("a/b").copied(), 950), "installed after the check: the update is in");
        assert!(is_rate_limit("GitHub API rate limit reached. Add a token in state.json."));
        assert!(!is_rate_limit("No release found for a/b."));
    }

    #[test]
    fn the_hint_bar_names_the_real_buttons() {
        assert!(hints(Some(TileKind::Port)).starts_with("A Play"));
        assert!(hints(Some(TileKind::Continue)).contains("X Options"));
        assert!(hints(Some(TileKind::Setup)).starts_with("A Set up"));
        assert!(hints(Some(TileKind::Add)).starts_with("A Add"));
        assert!(!hints(Some(TileKind::Add)).contains("X Options"), "nothing to show Info for");
        assert!(!hints(None).contains("X Options"));
        assert!(hints(None).contains("L1/R1 Lists"));
    }

    #[test]
    fn the_apps_play_record_reads_as_seconds() {
        assert_eq!(app_played_at("1970-01-01T00:01:40Z"), 100);
        assert_eq!(app_played_at(""), 0);
    }
}
