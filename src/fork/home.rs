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

/// The controller hint bar for what is focused (the real bindings: A plays, X opens the Port page (Fork, port page), Y opens Settings, R3
/// (a click of the right stick) switches between posters and the list, L1/R1 and Tab switch lists). `None` focus: the empty
/// card (nothing to switch there).
pub fn hints(focused: Option<TileKind>) -> String {
    let gap = crate::fork::page::HINT_GAP;
    match focused {
        None => ["A Choose", "D-pad Move", "Y Settings", "L1/R1 Lists"].join(gap),
        Some(TileKind::Setup) => ["A Set up", "D-pad Move", "Y Settings", VIEW_HINT, "L1/R1 Lists"].join(gap),
        Some(TileKind::Add) => ["A Add", "D-pad Move", "Y Settings", VIEW_HINT, "L1/R1 Lists"].join(gap),
        Some(TileKind::Checkup) => ["A Fix", "D-pad Move", "Y Settings", VIEW_HINT, "L1/R1 Lists"].join(gap),
        Some(_) => ["A Play", "X Options", "D-pad Move", "Y Settings", VIEW_HINT, "L1/R1 Lists"].join(gap),
    }
}

/// The controller button that switches Home between posters and the list: a click of the right stick (Select already
/// toggles the full screen for the whole app, and every face and shoulder button has its job).
pub const VIEW_HINT: &str = "R3 View";

/// How Home shows the ports: cover tiles ("Posters") or one detailed row each ("List"). Kept in `fork-settings.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Posters,
    List,
}

/// The `fork-settings.json` key holding the view.
pub const KEY_VIEW: &str = "home_view";

impl View {
    pub fn other(self) -> View {
        match self {
            View::Posters => View::List,
            View::List => View::Posters,
        }
    }

    /// The number the screen file (`ui/fork/home.slint`) knows it by: 0 posters, 1 list.
    pub fn code(self) -> i32 {
        match self {
            View::Posters => 0,
            View::List => 1,
        }
    }

    pub fn setting(self) -> &'static str {
        match self {
            View::Posters => "posters",
            View::List => "list",
        }
    }

    /// What `fork-settings.json` holds; anything missing or unknown is Posters.
    pub fn from_setting(v: Option<&serde_json::Value>) -> View {
        match v.and_then(serde_json::Value::as_str) {
            Some("list") => View::List,
            _ => View::Posters,
        }
    }

    /// The name the user sees.
    pub fn name(self) -> &'static str {
        match self {
            View::Posters => "Posters",
            View::List => "List",
        }
    }
}

/// The view kept in the settings file, Posters when none was chosen.
pub fn view_from_settings(settings: &crate::fork::settings::ForkSettings) -> View {
    View::from_setting(settings.get(KEY_VIEW))
}

pub fn save_view(settings: &mut crate::fork::settings::ForkSettings, view: View) -> std::io::Result<()> {
    settings.set(KEY_VIEW, serde_json::Value::from(view.setting()))
}

/// What the app knows about a port for the second line of its list row. Every field is optional: a fact the app does
/// not have is left out, never made up.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowFacts {
    pub system: Option<String>,
    pub version: Option<String>,
    /// "today" or "on Oct 20, 2026" (the app's own play record; Steam's dates have no time of day to show).
    pub last_played: Option<String>,
    pub playtime_seconds: u64,
}

/// The system a port is for, from its catalog tags (the first one that names a console or the PC).
pub fn system_of(tags_lower: &[String]) -> Option<String> {
    const SYSTEMS: [(&str, &str); 24] = [
        ("nes", "NES"),
        ("snes", "SNES"),
        ("n64", "N64"),
        ("gc", "GameCube"),
        ("wii", "Wii"),
        ("wiiu", "Wii U"),
        ("switch", "Switch"),
        ("gb", "Game Boy"),
        ("gbc", "Game Boy Color"),
        ("gba", "GBA"),
        ("nds", "DS"),
        ("3ds", "3DS"),
        ("ps1", "PS1"),
        ("ps2", "PS2"),
        ("ps3", "PS3"),
        ("ps4", "PS4"),
        ("psp", "PSP"),
        ("xbox", "Xbox"),
        ("360", "Xbox 360"),
        ("smd", "Genesis"),
        ("dreamcast", "Dreamcast"),
        ("saturn", "Saturn"),
        ("pc", "PC"),
        ("arcade", "Arcade"),
    ];
    tags_lower.iter().find_map(|t| SYSTEMS.iter().find(|(tag, _)| *tag == t.trim()).map(|(_, name)| (*name).to_string()))
}

/// "12 min" under an hour, "2.1 h" from an hour on; `None` for no play time at all.
pub fn playtime_text(seconds: u64) -> Option<String> {
    match seconds {
        0 => None,
        s if s < 3600 => Some(format!("{} min", (s / 60).max(1))),
        s => Some(format!("{:.1} h", s as f64 / 3600.0)),
    }
}

/// The second, muted line of a port's list row: system, installed version, when it was last played and for how long,
/// whichever of them the app knows.
pub fn detail_line(f: &RowFacts) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.extend(f.system.clone().filter(|s| !s.trim().is_empty()));
    parts.extend(f.version.clone().filter(|s| !s.trim().is_empty()));
    parts.extend(f.last_played.as_ref().map(|d| format!("Played {d}")));
    parts.extend(playtime_text(f.playtime_seconds));
    parts.join(" \u{b7} ")
}

/// The second line of the rows that are not ports.
pub fn action_detail(kind: TileKind) -> &'static str {
    match kind {
        TileKind::Setup => "Pick the ports folder and connect Steam",
        TileKind::Add => "Install a new port",
        TileKind::Checkup => "Open the list of fixes",
        TileKind::Port | TileKind::Continue => "",
    }
}

/// The big letter on a cover-less thumbnail: the title's first letter or digit.
pub fn initial_of(title: &str) -> String {
    title.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".to_string())
}

/// How the poster tiles are laid out in the room Home gives them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileLayout {
    pub columns: usize,
    pub tile_w: f32,
    pub tile_h: f32,
    /// Whole rows that fit the room at once (rows beyond them scroll a whole row at a time).
    pub visible_rows: usize,
}

const COVER_RATIO: f32 = 1.4;
/// The room under a cover, in font heights: its padding, two title lines and the badge line.
const TEXT_UNITS: f32 = 3.4;
/// The narrowest tile worth showing everything at once (in font heights); narrower, and the tiles scroll instead.
const MIN_TILE_W_UNITS: f32 = 6.5;
/// The widest a tile grows (in font heights), so a handful of ports do not become giant posters.
const MAX_TILE_W_UNITS: f32 = 12.0;
/// How much taller than its natural height a tile may stretch to use up the room of whole rows.
const STRETCH: f32 = 1.12;
/// The narrowest a tile gets so its badge line ("Add game files") shows whole (in font heights); a tile whose cover is limited by the room's height
/// is made this wide anyway, the cover simply sitting in a wider frame.
const TEXT_W_UNITS: f32 = 8.6;

/// A tile's height for a width: a portrait cover (about 1:1.4) and the room for two title lines and the badge line.
fn natural_tile_h(w: f32, font: f32) -> f32 {
    w * COVER_RATIO + font * TEXT_UNITS
}

/// The poster layout for `count` tiles in a room of `width` by `height` (logical pixels), with `font` the tile text size
/// and `gap` the space between tiles. It takes the arrangement with the biggest tiles that shows every tile at once, so no
/// row is cut off; when even the narrowest worthwhile tile cannot do that, as many columns as the width gives and as many
/// whole rows as the height gives, and the rest scrolls a whole row at a time.
pub fn tile_layout(width: f32, height: f32, count: usize, font: f32, gap: f32) -> TileLayout {
    let (width, height) = (width.max(1.0), height.max(1.0));
    let n = count.max(1);
    let max_w = font * MAX_TILE_W_UNITS;
    // (rows, columns, cover width, tile width): the biggest tile wins, a tile narrower than its badge words counting as that wide.
    let mut best: Option<(usize, usize, f32, f32)> = None;
    for rows in 1..=n {
        let columns = n.div_ceil(rows);
        if n.div_ceil(columns) != rows {
            continue;
        }
        let by_width = (width - gap * (columns - 1) as f32) / columns as f32;
        let per_row = (height - gap * (rows - 1) as f32) / rows as f32;
        let by_height = (per_row - font * TEXT_UNITS) / COVER_RATIO;
        let w = by_width.min(by_height).min(max_w);
        let tile_w = w.max(font * TEXT_W_UNITS).min(by_width);
        if best.is_none_or(|(_, _, _, b)| tile_w > b * 1.03) {
            best = Some((rows, columns, w, tile_w));
        }
    }
    if let Some((rows, columns, w, tile_w)) = best.filter(|(_, _, w, _)| *w >= font * MIN_TILE_W_UNITS) {
        let per_row = (height - gap * (rows - 1) as f32) / rows as f32;
        return TileLayout { columns, tile_w, tile_h: per_row.min(natural_tile_h(w, font) * STRETCH), visible_rows: rows };
    }
    // Too many to show at once: as many whole rows as keep a cover readable, columns to match, the rest scrolls.
    let cover_w = |rows: usize| ((height - gap * (rows - 1) as f32) / rows as f32 - font * TEXT_UNITS) / COVER_RATIO;
    let mut visible = 1;
    while visible < 8 && cover_w(visible + 1) >= font * MIN_TILE_W_UNITS {
        visible += 1;
    }
    let per_row = (height - gap * (visible - 1) as f32) / visible as f32;
    let target = cover_w(visible).clamp(font * MIN_TILE_W_UNITS, font * 11.0).max(font * TEXT_W_UNITS);
    let columns = (((width + gap) / (target + gap)).floor() as usize).clamp(1, 8);
    let w = ((width - gap * (columns - 1) as f32) / columns as f32).min(max_w * 1.2);
    TileLayout { columns, tile_w: w, tile_h: per_row.max(font * 9.0).min(height), visible_rows: visible }
}

/// The D-pad in the List view: one row up (`dy` < 0) or down (`dy` > 0), staying on the first and the last row.
pub fn list_step(at: usize, dy: i32, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    match dy.signum() {
        -1 => at.saturating_sub(1),
        1 => (at + 1).min(len - 1),
        _ => at.min(len - 1),
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

    /// "Add a port" when the list was opened from that page (B goes back to it), else "Home"; empty on Home.
    /// "Home"; empty on Home.
    pub fn back_hint(self, searching: bool, from_add_port: bool) -> &'static str {
        match (self, searching) {
            (List::Home, _) => "",
            (_, true) => "B / Esc  Clear search",
            (_, false) if from_add_port => "B / Esc  Add a port",
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
/// GitLab release, the port's own update switch on (no record counts as on), and the last check (`last_update_check`,
/// empty = never) older than the time Play waits before asking again (12 h since the upstream catch-up of
/// 2026-10-31; before it was 24 h since the install). Play and the home screen both ask this.
pub fn update_check_due(release_sync: bool, source: &crate::core::models::SourceType, port_update_on: Option<bool>, last_update_check: &str) -> bool {
    use crate::core::models::SourceType;
    release_sync && matches!(source, SourceType::Github | SourceType::Gitlab) && port_update_on.unwrap_or(true) && crate::core::state::is_due_for_update_check(last_update_check)
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

/// What Play takes from the home screen's last answer for this port: `Some(true)` = the badge says "Update ready"
/// (Play installs it, no second request), `Some(false)` = a check after the install found nothing within the time
/// Play waits between its own checks (Play starts the game), `None` = no usable answer (Play follows its own rule).
/// Keeps the badge and Play in step: a tile that says "Update ready" never starts the old version.
pub fn play_answer(seen: Option<UpdateSeen>, installed_at: i64, now: i64) -> Option<bool> {
    let s = seen.filter(|s| s.at >= installed_at)?;
    if s.available {
        return Some(true);
    }
    (now >= s.at && now - s.at < UPDATE_CHECK_EVERY_SECS).then_some(false)
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
        assert_eq!(List::AllPorts.back_hint(true, true), "B / Esc  Clear search", "the first B clears the search, wherever the list came from");
        assert_eq!(List::AllPorts.back_hint(false, true), "B / Esc  Add a port", "after the search is cleared, B goes back to Add a port");
        assert_eq!(List::DeckReady.back_hint(false, true), "B / Esc  Add a port");
        assert_eq!(List::DeckReady.back_hint(false, false), "B / Esc  Home");
        assert_eq!(List::Home.back_hint(true, true), "");
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
        assert!(!update_check_due(true, &SourceType::Github, Some(true), &just_now), "checked a moment ago");
        assert!(update_check_due(true, &SourceType::Github, Some(true), ""), "never checked");
    }

    #[test]
    fn play_follows_the_home_badge_and_a_fresh_no_answer_skips_its_own_check() {
        let (installed, now) = (1_000_000, 1_010_000);
        let ready = UpdateSeen { available: true, at: 1_005_000 };
        let none_fresh = UpdateSeen { available: false, at: 1_005_000 };
        let none_old = UpdateSeen { available: false, at: now - UPDATE_CHECK_EVERY_SECS - 1 };
        assert_eq!(play_answer(Some(ready), installed, now), Some(true), "the badge says Update ready: Play updates");
        assert_eq!(play_answer(Some(none_fresh), installed, now), Some(false), "checked lately, nothing: Play starts the game");
        assert_eq!(play_answer(Some(none_old), installed, now + UPDATE_CHECK_EVERY_SECS * 2), None, "an old no-answer: Play checks itself");
        assert_eq!(play_answer(Some(UpdateSeen { available: true, at: 999_000 }), installed, now), None, "an answer from before the install does not count");
        assert_eq!(play_answer(None, installed, now), None);
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
        for kind in [TileKind::Port, TileKind::Continue, TileKind::Setup, TileKind::Add, TileKind::Checkup] {
            assert!(hints(Some(kind)).contains("R3 View"), "{kind:?} names the view switch");
        }
        assert!(!hints(None).contains("View"), "the empty card has nothing to switch");
        let longest = hints(Some(TileKind::Port));
        assert!(longest.chars().count() < 90, "one line at 1280 px: {longest}");
        assert!(!hints(Some(TileKind::Port)).contains('\u{2014}'), "no em dashes");
    }

    #[test]
    fn the_view_is_posters_unless_the_settings_file_says_list() {
        assert_eq!(View::default(), View::Posters);
        assert_eq!(View::from_setting(None), View::Posters);
        assert_eq!(View::from_setting(Some(&serde_json::json!("list"))), View::List);
        assert_eq!(View::from_setting(Some(&serde_json::json!("posters"))), View::Posters);
        assert_eq!(View::from_setting(Some(&serde_json::json!("grid"))), View::Posters, "unknown: the default");
        assert_eq!(View::from_setting(Some(&serde_json::json!(3))), View::Posters, "not text: the default");
        assert_eq!((View::Posters.other(), View::List.other()), (View::List, View::Posters));
        assert_eq!((View::Posters.code(), View::List.code()), (0, 1));
    }

    #[test]
    fn the_view_survives_a_restart_through_the_settings_file() {
        use crate::fork::settings::ForkSettings;
        let mut path = std::env::temp_dir();
        path.push(format!("dpm_home_view_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut settings = ForkSettings::load(&path);
        assert_eq!(view_from_settings(&settings), View::Posters, "nothing chosen yet");
        settings.set("other_key", serde_json::json!(7)).unwrap();
        save_view(&mut settings, View::List).unwrap();
        let again = ForkSettings::load(&path);
        assert_eq!(view_from_settings(&again), View::List);
        assert_eq!(again.get("other_key"), Some(&serde_json::json!(7)), "other keys are kept");
        let mut back = ForkSettings::load(&path);
        save_view(&mut back, View::Posters).unwrap();
        assert_eq!(view_from_settings(&ForkSettings::load(&path)), View::Posters);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_list_row_says_only_what_the_app_knows() {
        assert_eq!(detail_line(&RowFacts::default()), "", "nothing known: nothing said");
        let all = RowFacts { system: Some("N64".into()), version: Some("v2.1.0".into()), last_played: Some("today".into()), playtime_seconds: 7560 };
        assert_eq!(detail_line(&all), "N64 \u{b7} v2.1.0 \u{b7} Played today \u{b7} 2.1 h");
        let some = RowFacts { system: Some("PC".into()), last_played: Some("on Oct 20, 2026".into()), ..RowFacts::default() };
        assert_eq!(detail_line(&some), "PC \u{b7} Played on Oct 20, 2026");
        let blanks = RowFacts { system: Some("  ".into()), version: Some(String::new()), playtime_seconds: 90, ..RowFacts::default() };
        assert_eq!(detail_line(&blanks), "1 min", "blank facts are skipped");
        assert!(!detail_line(&all).contains('\u{2014}'));
    }

    #[test]
    fn play_time_reads_as_minutes_then_hours() {
        assert_eq!(playtime_text(0), None);
        assert_eq!(playtime_text(20).as_deref(), Some("1 min"), "a short run still counts");
        assert_eq!(playtime_text(59 * 60).as_deref(), Some("59 min"));
        assert_eq!(playtime_text(3600).as_deref(), Some("1.0 h"));
        assert_eq!(playtime_text(7560).as_deref(), Some("2.1 h"));
    }

    #[test]
    fn the_system_comes_from_the_first_tag_that_names_one() {
        let tags = |t: &[&str]| t.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(system_of(&tags(&["n64", "platformer"])).as_deref(), Some("N64"));
        assert_eq!(system_of(&tags(&["platformer", "360"])).as_deref(), Some("Xbox 360"));
        assert_eq!(system_of(&tags(&["free", "macos", "shooter, multilingual"])), None, "genres and flags are not systems");
        assert_eq!(system_of(&tags(&["pc", "pinball"])).as_deref(), Some("PC"));
        assert_eq!(system_of(&[]), None);
    }

    #[test]
    fn rows_that_are_not_ports_say_what_they_do() {
        assert!(action_detail(TileKind::Add).starts_with("Install"));
        assert!(!action_detail(TileKind::Setup).is_empty() && !action_detail(TileKind::Checkup).is_empty());
        assert_eq!((action_detail(TileKind::Port), action_detail(TileKind::Continue)), ("", ""), "ports use their own facts");
        assert_eq!(initial_of("1080\u{b0} Snowboarding"), "1");
        assert_eq!(initial_of("abuse"), "A");
        assert_eq!(initial_of("  \u{2022} Zelda"), "Z");
        assert_eq!(initial_of("..."), "?");
    }

    /// Whole rows fit, never cut: the last row's bottom is inside the room.
    fn fits(l: &TileLayout, n: usize, height: f32, gap: f32) -> bool {
        let rows = n.div_ceil(l.columns);
        rows <= l.visible_rows && l.visible_rows as f32 * l.tile_h + gap * (l.visible_rows as f32 - 1.0) <= height + 0.01
    }

    #[test]
    fn seven_tiles_on_the_decks_screen_all_fit_in_whole_rows() {
        // Big mode at 1280 x 800: the room under the top line and above the hint line is about 640 px tall.
        let (font, gap) = (18.4, 16.6);
        let l = tile_layout(1256.0, 640.0, 7, font, gap);
        assert!(fits(&l, 7, 640.0, gap), "{l:?}");
        assert_eq!((l.columns, l.visible_rows), (4, 2), "two full rows of four, not six and one");
        assert!(l.tile_w >= font * MIN_TILE_W_UNITS && l.tile_w <= font * MAX_TILE_W_UNITS + 0.01, "{l:?}");
        assert!(l.tile_h >= natural_tile_h(l.tile_w, font) / 1.2, "the cover keeps its portrait shape: {l:?}");
        assert!(l.tile_h <= natural_tile_h(l.tile_w, font) * STRETCH + 0.01);
    }

    #[test]
    fn seven_and_eight_tiles_fit_the_room_the_deck_really_gives() {
        // Measured on the 1280 x 800 screen in big mode: text 20 px, the room under the top line about 575 px tall.
        let (font, gap) = (20.0, 18.0);
        for n in [6usize, 7, 8] {
            let l = tile_layout(1256.0, 575.0, n, font, gap);
            assert!(fits(&l, n, 575.0, gap), "{n} tiles: {l:?}");
            assert_eq!(l.visible_rows, n.div_ceil(l.columns), "{n} tiles: nothing is left to scroll to: {l:?}");
            assert!(l.visible_rows <= 2, "{n} tiles: {l:?}");
            assert!(l.tile_w >= font * TEXT_W_UNITS - 0.01, "{n} tiles: wide enough for the badge words: {l:?}");
            assert!(l.tile_h <= natural_tile_h(l.tile_w, font) * STRETCH + 0.01, "{n} tiles: {l:?}");
            assert!(l.columns as f32 * l.tile_w + gap * (l.columns as f32 - 1.0) <= 1256.0 + 0.01, "{n} tiles: inside the width: {l:?}");
        }
    }

    #[test]
    fn a_few_ports_make_one_row_of_ordinary_tiles_and_never_giant_ones() {
        let (font, gap) = (18.4, 16.6);
        let l = tile_layout(1256.0, 640.0, 3, font, gap);
        assert_eq!((l.columns, l.visible_rows), (3, 1));
        assert!(l.tile_w <= font * MAX_TILE_W_UNITS + 0.01, "{l:?}");
        let one = tile_layout(1256.0, 640.0, 1, font, gap);
        assert_eq!((one.columns, one.visible_rows), (1, 1));
        assert!(fits(&one, 1, 640.0, gap));
        let none = tile_layout(1256.0, 640.0, 0, font, gap);
        assert!(none.columns >= 1 && none.tile_w > 0.0 && none.tile_h > 0.0, "nothing to draw but never a zero size");
    }

    #[test]
    fn too_many_tiles_scroll_in_whole_rows() {
        let (font, gap) = (18.4, 16.6);
        for n in [20, 33, 60] {
            let l = tile_layout(1256.0, 640.0, n, font, gap);
            assert!(l.visible_rows >= 1 && l.columns >= 1, "{l:?}");
            let shown = l.visible_rows as f32 * l.tile_h + gap * (l.visible_rows as f32 - 1.0);
            assert!(shown <= 640.0 + 0.01, "{n} tiles: the rows shown are whole and inside the room: {l:?}");
            assert!(l.tile_w >= font * MIN_TILE_W_UNITS, "{n} tiles: still readable: {l:?}");
            assert!(l.tile_w >= font * TEXT_W_UNITS - 0.01, "{n} tiles: wide enough for the badge words: {l:?}");
            assert!(l.columns as f32 * l.tile_w + gap * (l.columns as f32 - 1.0) <= 1256.0 + 0.01, "{n} tiles: inside the width: {l:?}");
            assert!(n.div_ceil(l.columns) > l.visible_rows, "{n} tiles really do scroll");
        }
    }

    #[test]
    fn the_layout_is_sane_for_every_window_size_and_count() {
        for (w, h) in [(360.0, 330.0), (560.0, 340.0), (1256.0, 640.0), (200.0, 120.0), (3000.0, 1500.0), (50.0, 50.0)] {
            for font in [11.0, 14.0, 18.4, 30.0] {
                for n in [0usize, 1, 2, 5, 8, 13, 40] {
                    let gap = font * 0.9;
                    let l = tile_layout(w, h, n, font, gap);
                    let ctx = format!("{w}x{h} font {font} n {n}: {l:?}");
                    assert!(l.columns >= 1 && l.columns <= 8.max(n.max(1)), "{ctx}");
                    assert!(l.visible_rows >= 1, "{ctx}");
                    assert!(l.tile_w.is_finite() && l.tile_w > 0.0 && l.tile_h.is_finite() && l.tile_h > 0.0, "{ctx}");
                    let enough_room = h >= font * 9.0;
                    if enough_room {
                        assert!(l.tile_h <= h + 0.01, "a tile is never taller than the room: {ctx}");
                    }
                }
            }
        }
    }


    #[test]
    fn the_d_pad_walks_the_list_one_row_and_stops_at_both_ends() {
        assert_eq!(list_step(2, 1, 5), 3);
        assert_eq!(list_step(2, -1, 5), 1);
        assert_eq!(list_step(0, -1, 5), 0, "the first row stays");
        assert_eq!(list_step(4, 1, 5), 4, "the last row stays");
        assert_eq!(list_step(3, 0, 5), 3);
        assert_eq!(list_step(9, 0, 5), 4, "a selection past the end comes back");
        assert_eq!(list_step(0, 1, 0), 0, "nothing to walk");
    }

    #[test]
    fn the_apps_play_record_reads_as_seconds() {
        assert_eq!(app_played_at("1970-01-01T00:01:40Z"), 100);
        assert_eq!(app_played_at(""), 0);
    }
}
