//! The home screen's model: what the app shows first. "Your ports" as cover tiles for the installed ports, each with
//! one status badge; a Continue tile first for the port played last; and, with nothing installed, one card to set
//! the app up or browse ports. Tested without any UI; `src/app/home_screen.rs` gathers the facts and draws it.

/// The one badge a port tile shows: the thing that most needs doing, else Ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Badge {
    Ready,
    UpdateAvailable,
    GameFilesMissing,
    SteamWaiting,
    SavesNotFound,
}

impl Badge {
    pub fn label(self) -> &'static str {
        match self {
            Badge::Ready => "Ready",
            Badge::UpdateAvailable => "Update available",
            Badge::GameFilesMissing => "Game files missing",
            Badge::SteamWaiting => "Steam waiting",
            Badge::SavesNotFound => "Saves not found",
        }
    }

    /// The status mark drawn with it (the same pictures as the guided pages' summary lines).
    pub fn mark(self) -> crate::fork::page::Mark {
        use crate::fork::page::Mark;
        match self {
            Badge::Ready => Mark::Done,
            Badge::UpdateAvailable => Mark::ToDo,
            Badge::GameFilesMissing => Mark::Problem,
            Badge::SteamWaiting => Mark::Waiting,
            Badge::SavesNotFound => Mark::Problem,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    pub kind: TileKind,
    /// The port's key (empty for the Setup tile).
    pub key: String,
    pub title: String,
    pub badge: Badge,
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
/// is not done, then Continue (the port played last, not repeated below), then every other port by name.
pub fn build(ports: &[PortFacts], setup_done: bool) -> Home {
    if ports.is_empty() {
        return Home::Empty;
    }
    let mut tiles = Vec::new();
    if !setup_done {
        tiles.push(Tile { kind: TileKind::Setup, key: String::new(), title: "Finish setting up".into(), badge: Badge::Ready });
    }
    let first = continue_pick(ports);
    if let Some(i) = first {
        let p = &ports[i];
        tiles.push(Tile { kind: TileKind::Continue, key: p.key.clone(), title: p.name.clone(), badge: badge(p) });
    }
    let mut rest: Vec<&PortFacts> = ports.iter().enumerate().filter(|(i, _)| Some(*i) != first).map(|(_, p)| p).collect();
    rest.sort_by_key(|p| p.name.to_lowercase());
    tiles.extend(rest.into_iter().map(|p| Tile { kind: TileKind::Port, key: p.key.clone(), title: p.name.clone(), badge: badge(p) }));
    Home::Tiles(tiles)
}

/// The controller hint bar for what is focused (the real bindings: A plays, X opens Info, Y opens Settings, L1/R1 and
/// Tab switch lists). `None` focus: the empty card.
pub fn hints(focused: Option<TileKind>) -> String {
    let gap = crate::fork::page::HINT_GAP;
    match focused {
        None => ["A Choose", "D-pad Move", "Y Settings", "L1/R1 Lists"].join(gap),
        Some(TileKind::Setup) => ["A Set up", "D-pad Move", "Y Settings", "L1/R1 Lists"].join(gap),
        Some(_) => ["A Play", "X Info", "D-pad Move", "Y Settings", "L1/R1 Lists"].join(gap),
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

    pub fn name(self) -> &'static str {
        match self {
            List::Home => "Home",
            List::AllPorts => "All ports",
            List::DeckReady => "Deck-ready",
        }
    }
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
        p.game_files_missing = true;
        assert_eq!(badge(&p), Badge::GameFilesMissing, "a game that will not start comes first");
        assert_eq!(Badge::GameFilesMissing.label(), "Game files missing");
    }

    #[test]
    fn four_ports_in_different_states_make_a_continue_tile_then_the_rest_by_name() {
        let mut ports = vec![port("dusk", 100), port("abuse", 0), port("soh", 500), port("2ship", 0)];
        ports[0].update_available = true;
        ports[1].game_files_missing = true;
        ports[3].steam_waiting = true;
        let Home::Tiles(tiles) = build(&ports, true) else { panic!("tiles") };
        assert_eq!(tiles[0], Tile { kind: TileKind::Continue, key: "soh".into(), title: "SOH".into(), badge: Badge::Ready });
        let rest: Vec<(&str, Badge)> = tiles[1..].iter().map(|t| (t.key.as_str(), t.badge)).collect();
        assert_eq!(rest, [("2ship", Badge::SteamWaiting), ("abuse", Badge::GameFilesMissing), ("dusk", Badge::UpdateAvailable)], "by name, Continue not repeated");
        assert!(tiles[1..].iter().all(|t| t.kind == TileKind::Port));
    }

    #[test]
    fn continue_is_the_port_played_last_and_absent_when_none_was_played() {
        assert_eq!(continue_pick(&[port("a", 10), port("b", 30), port("c", 20)]), Some(1));
        assert_eq!(continue_pick(&[port("a", 0), port("b", 0)]), None);
        let Home::Tiles(tiles) = build(&[port("a", 0), port("b", 0)], true) else { panic!() };
        assert!(tiles.iter().all(|t| t.kind == TileKind::Port), "nothing played: no Continue tile");
    }

    #[test]
    fn nothing_installed_is_the_empty_card_and_unfinished_setup_puts_its_tile_first() {
        assert_eq!(build(&[], true), Home::Empty);
        assert_eq!(build(&[], false), Home::Empty);
        let Home::Tiles(tiles) = build(&[port("a", 5)], false) else { panic!() };
        assert_eq!((tiles[0].kind, tiles[1].kind), (TileKind::Setup, TileKind::Continue));
    }

    #[test]
    fn the_lists_go_round_home_all_ports_deck_ready() {
        assert_eq!(List::Home.next(), List::AllPorts);
        assert_eq!(List::AllPorts.next(), List::DeckReady);
        assert_eq!(List::DeckReady.next(), List::Home);
    }

    #[test]
    fn the_hint_bar_names_the_real_buttons() {
        assert!(hints(Some(TileKind::Port)).starts_with("A Play"));
        assert!(hints(Some(TileKind::Continue)).contains("X Info"));
        assert!(hints(Some(TileKind::Setup)).starts_with("A Set up"));
        assert!(!hints(None).contains("X Info"));
        assert!(hints(None).contains("L1/R1 Lists"));
    }

    #[test]
    fn the_apps_play_record_reads_as_seconds() {
        assert_eq!(app_played_at("1970-01-01T00:01:40Z"), 100);
        assert_eq!(app_played_at(""), 0);
    }
}
