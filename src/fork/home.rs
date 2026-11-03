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
pub fn hints(focused: Option<TileKind>, view: View) -> String {
    let gap = crate::fork::page::HINT_GAP;
    let switch = view_hint(view);
    match focused {
        None => ["A Choose", "D-pad Move", "Y Settings", "L1/R1 Lists"].join(gap),
        Some(TileKind::Setup) => ["A Set up", "D-pad Move", "Y Settings", switch, "L1/R1 Lists"].join(gap),
        Some(TileKind::Add) => ["A Add", "D-pad Move", "Y Settings", switch, "L1/R1 Lists"].join(gap),
        Some(TileKind::Checkup) => ["A Fix", "D-pad Move", "Y Settings", switch, "L1/R1 Lists"].join(gap),
        Some(_) => ["A Play", "X Options", "D-pad Move", "Y Settings", switch, "L1/R1 Lists"].join(gap),
    }
}

/// The controller button that switches Home between posters and the list: a click of the right stick (Select already
/// toggles the full screen for the whole app, and every face and shoulder button has its job). The hint says what the
/// press shows: "R3 List" while posters show, "R3 Posters" while the list shows.
pub fn view_hint(showing: View) -> &'static str {
    match showing {
        View::Posters => "R3 List",
        View::List => "R3 Posters",
    }
}

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
    /// The frame a cover is drawn in (the tile less its margin, title and badge lines); a poster fills as much of it as its
    /// own shape allows.
    pub cover_w: f32,
    pub cover_h: f32,
}

/// A portrait cover, about 1:1.4.
const COVER_RATIO: f32 = 1.4;
/// What a Tile draws around its cover, in font heights: `TILE_PAD_UNITS` above and below, and two title lines (the second elided, a little smaller) and
/// the badge line between the cover and the bottom margin. `ui/fork/home.slint` (component Tile: `pad`, `text-h`) must say
/// the same numbers; a test reads the file and compares.
pub const TILE_PAD_UNITS: f32 = 0.4;
pub const TILE_TEXT_UNITS: f32 = 3.4;
const TEXT_UNITS: f32 = TILE_PAD_UNITS * 2.0 + TILE_TEXT_UNITS;
/// The smallest cover frame width worth showing everything at once, and the biggest (in font heights; a handful of ports do
/// not become giant posters).
const MIN_COVER_UNITS: f32 = 6.8;
const MAX_COVER_UNITS: f32 = 9.5;
const MAX_COLUMNS: usize = 12;
/// Arrangements whose covers are within this (8%) of the biggest count as equal; of those the one with the fewest columns that still run
/// the width wins (the same cover and a wider title); with too few tiles for that, the one with the most rows, then columns.
const TIE: f32 = 0.92;
/// How much wider than its cover a tile may be (in gaps) and still count as running the width.
const SLACK_GAPS: f32 = 3.0;

/// The poster layout for `count` tiles in a room of `width` by `height` (logical pixels), with `font` the tile text size
/// and `gap` the space between tiles. It takes the arrangement with the biggest covers that shows every tile at once, so no
/// row is cut off (and, among equals, the fewest columns that still run the width); when even the smallest worthwhile cover
/// cannot do that, as many whole rows as keep a cover readable, columns to match, and the rest scrolls a whole row at a time.
pub fn tile_layout(width: f32, height: f32, count: usize, font: f32, gap: f32) -> TileLayout {
    let (width, height) = (width.max(1.0), height.max(1.0));
    let n = count.max(1);
    let (text, margin) = (font * TEXT_UNITS, font * TILE_PAD_UNITS * 2.0);
    let (min_cover, max_cover) = (font * MIN_COVER_UNITS, font * MAX_COVER_UNITS);
    // (columns, rows, tile width at most, row height) and the cover frame width each gives.
    let options: Vec<(usize, usize, f32, f32, f32)> = (1..=n.min(MAX_COLUMNS))
        .map(|columns| {
            let rows = n.div_ceil(columns);
            let tile_max = (width - gap * (columns - 1) as f32) / columns as f32;
            let per_row = (height - gap * (rows - 1) as f32) / rows as f32;
            let cover = (tile_max - margin).min((per_row - text) / COVER_RATIO).min(max_cover);
            (columns, rows, tile_max, per_row, cover)
        })
        .collect();
    let biggest = options.iter().map(|o| o.4).fold(f32::MIN, f32::max);
    if biggest >= min_cover {
        let ties: Vec<&(usize, usize, f32, f32, f32)> = options.iter().filter(|o| o.4 >= biggest * TIE).collect();
        // The fewest columns whose tiles still run the width (a wider title, the same cover); with too few tiles for that, the fullest.
        let pick = ties.iter().filter(|o| o.2 <= o.4 + margin + gap * SLACK_GAPS + 0.01).min_by_key(|o| o.0).or_else(|| ties.iter().max_by_key(|o| (o.1, o.0)));
        if let Some(&&(columns, rows, tile_max, per_row, cover)) = pick {
            let tile_w = tile_max.min(cover + margin + gap * SLACK_GAPS);
            let tile_h = per_row.min(cover * COVER_RATIO + text);
            return TileLayout { columns, tile_w, tile_h, visible_rows: rows, cover_w: tile_w - margin, cover_h: tile_h - text };
        }
    }
    // Too many to show at once: as many whole rows as keep a cover readable, columns to match, the rest scrolls.
    let cover_for = |rows: usize| ((height - gap * (rows - 1) as f32) / rows as f32 - text) / COVER_RATIO;
    let mut visible = 1;
    while visible < 8 && cover_for(visible + 1) >= min_cover {
        visible += 1;
    }
    let per_row = (height - gap * (visible - 1) as f32) / visible as f32;
    let cover = cover_for(visible).clamp(min_cover, max_cover);
    let natural = cover + margin;
    let columns = (((width + gap) / (natural + gap)).floor() as usize).clamp(1, MAX_COLUMNS);
    let tile_w = ((width - gap * (columns - 1) as f32) / columns as f32).min(natural + gap * SLACK_GAPS);
    let tile_h = per_row.max(text + min_cover).min(height);
    TileLayout { columns, tile_w, tile_h, visible_rows: visible, cover_w: tile_w - margin, cover_h: tile_h - text }
}

/// Which tile has the focus, by what it is and not by where it sits: the checkup tile can appear in front of it (the
/// background look finishes) or go away (the problems are fixed), and a port can be installed or removed, so a position
/// would move the focus to another port and the next A would act on the wrong one. The Continue tile and the port's own
/// tile are the same port (Continue moves to another port when one is played).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FocusId {
    Port(String),
    Setup,
    Add,
    Checkup,
}

pub fn focus_id(t: &Tile) -> FocusId {
    match t.kind {
        TileKind::Port | TileKind::Continue => FocusId::Port(t.key.clone()),
        TileKind::Setup => FocusId::Setup,
        TileKind::Add => FocusId::Add,
        TileKind::Checkup => FocusId::Checkup,
    }
}

/// The focus after the tiles were rebuilt: the tile that had it, wherever it is now; when it is gone (uninstalled, the
/// checkup fixed) or nothing was shown before, the old position kept inside the new list.
pub fn resolve_focus(before: Option<&FocusId>, old_index: usize, tiles: &[Tile]) -> usize {
    before
        .and_then(|id| tiles.iter().position(|t| &focus_id(t) == id))
        .unwrap_or_else(|| old_index.min(tiles.len().saturating_sub(1)))
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
        assert!(hints(Some(TileKind::Checkup), View::Posters).starts_with("A Fix"));
        assert!(!hints(Some(TileKind::Checkup), View::Posters).contains("X Options"));
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
        assert!(hints(Some(TileKind::Port), View::Posters).starts_with("A Play"));
        assert!(hints(Some(TileKind::Continue), View::Posters).contains("X Options"));
        assert!(hints(Some(TileKind::Setup), View::Posters).starts_with("A Set up"));
        assert!(hints(Some(TileKind::Add), View::Posters).starts_with("A Add"));
        assert!(!hints(Some(TileKind::Add), View::Posters).contains("X Options"), "nothing to show Info for");
        assert!(!hints(None, View::Posters).contains("X Options"));
        assert!(hints(None, View::Posters).contains("L1/R1 Lists"));
        for kind in [TileKind::Port, TileKind::Continue, TileKind::Setup, TileKind::Add, TileKind::Checkup] {
            assert!(hints(Some(kind), View::Posters).contains("R3 List"), "{kind:?} says the press shows the list");
            assert!(hints(Some(kind), View::List).contains("R3 Posters"), "{kind:?} says the press shows the posters");
            assert!(!hints(Some(kind), View::List).contains("R3 List"));
        }
        assert!(!hints(None, View::Posters).contains("View"), "the empty card has nothing to switch");
        let longest = hints(Some(TileKind::Port), View::List);
        assert!(longest.chars().count() < 100, "one line at 1280 px: {longest}");
        assert!(!hints(Some(TileKind::Port), View::Posters).contains('\u{2014}'), "no em dashes");
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

    /// Whole rows fit, never cut: the rows shown are inside the room.
    fn fits(l: &TileLayout, n: usize, height: f32, gap: f32) -> bool {
        n.div_ceil(l.columns) <= l.visible_rows && l.visible_rows as f32 * l.tile_h + gap * (l.visible_rows as f32 - 1.0) <= height + 0.01
    }

    fn used_width(l: &TileLayout, gap: f32) -> f32 {
        l.columns as f32 * l.tile_w + gap * (l.columns as f32 - 1.0)
    }

    // Measured on the Deck's 1280 x 800 screen in big mode: text 20 px, the room under the top line about 575 px tall. Before
    // this layout the covers were about 136 x 204 and a second row was cut off.
    const DECK: (f32, f32, f32, f32) = (1256.0, 575.0, 20.0, 18.0);

    #[test]
    fn the_decks_screen_shows_up_to_fourteen_tiles_in_two_whole_rows_with_covers_at_least_as_big_as_before() {
        let (w, h, font, gap) = DECK;
        for n in 7usize..=14 {
            let l = tile_layout(w, h, n, font, gap);
            assert!(fits(&l, n, h, gap), "{n} tiles: {l:?}");
            assert_eq!(l.visible_rows, 2, "{n} tiles: two whole rows, nothing cut: {l:?}");
            let expected = if n <= 12 { 6 } else { 7 };
            assert_eq!(l.columns, expected, "{n} tiles: the fewest columns that run the width (a wider title, the same cover): {l:?}");
            assert!(l.cover_w >= 136.0 && l.cover_h >= 193.0, "{n} tiles: covers about 137 x 193 or better (the old ones were 136 x 204 with one title line fewer): {l:?}");
            assert!(used_width(&l, gap) >= w - 1.0, "{n} tiles: no side bands, the tiles run the width: {l:?}");
        }
    }

    #[test]
    fn six_tiles_and_a_few_ports_get_one_row_of_ordinary_covers_and_never_giant_ones() {
        let (w, h, font, gap) = DECK;
        let six = tile_layout(w, h, 6, font, gap);
        assert!(fits(&six, 6, h, gap) && six.visible_rows == 1 && six.cover_w >= 136.0, "{six:?}");
        let l = tile_layout(w, h, 3, font, gap);
        assert_eq!((l.columns, l.visible_rows), (3, 1));
        assert!(l.cover_w <= font * MAX_COVER_UNITS + SLACK_GAPS * gap + 0.01, "{l:?}");
        let one = tile_layout(w, h, 1, font, gap);
        assert_eq!((one.columns, one.visible_rows), (1, 1));
        assert!(fits(&one, 1, h, gap));
        let none = tile_layout(w, h, 0, font, gap);
        assert!(none.columns >= 1 && none.tile_w > 0.0 && none.tile_h > 0.0, "nothing to draw but never a zero size");
    }

    #[test]
    fn too_many_tiles_scroll_in_whole_rows() {
        let (w, h, font, gap) = DECK;
        for n in [15usize, 20, 33, 60] {
            let l = tile_layout(w, h, n, font, gap);
            assert!(l.visible_rows >= 1 && l.columns >= 1, "{l:?}");
            let shown = l.visible_rows as f32 * l.tile_h + gap * (l.visible_rows as f32 - 1.0);
            assert!(shown <= h + 0.01, "{n} tiles: the rows shown are whole and inside the room: {l:?}");
            assert!(l.cover_w >= font * MIN_COVER_UNITS - 0.01, "{n} tiles: still readable: {l:?}");
            assert!(used_width(&l, gap) <= w + 0.01, "{n} tiles: inside the width: {l:?}");
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
                    assert!(l.columns >= 1 && l.columns <= MAX_COLUMNS, "{ctx}");
                    assert!(l.visible_rows >= 1, "{ctx}");
                    assert!(l.tile_w.is_finite() && l.tile_w > 0.0 && l.tile_h.is_finite() && l.tile_h > 0.0, "{ctx}");
                    if h >= font * 9.0 {
                        assert!(l.tile_h <= h + 0.01, "a tile is never taller than the room: {ctx}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_tile_draws_exactly_the_text_block_the_layout_reserves() {
        // The layout reserves TEXT_UNITS font heights (the margins, two title lines and the badge line); `component Tile` in the
        // screen file must draw that and no more (it once drew 5.4 where the layout assumed 3.4, so posters came out smaller).
        let slint = include_str!("../../ui/fork/home.slint");
        let start = slint.find("component Tile inherits").expect("component Tile");
        let tile = &slint[start..slint.find("component Row inherits").expect("component Row")];
        let units_in = |block: &str, prefix: &str| -> f32 {
            let line = block.lines().find(|l| l.trim_start().starts_with(prefix)).unwrap_or_else(|| panic!("Tile has no `{prefix}`"));
            line.rsplit('*').next().unwrap().trim().trim_end_matches(';').parse().unwrap()
        };
        let (pad, text_h) = (units_in(tile, "property <length> pad:"), units_in(tile, "property <length> text-h:"));
        assert_eq!(pad, TILE_PAD_UNITS);
        assert_eq!(text_h, TILE_TEXT_UNITS);
        assert!((TEXT_UNITS - (2.0 * pad + text_h)).abs() < 1e-6);
        // The title: word-wrapped, room for two lines (not one, not three), then the badge line, all inside the text block.
        let title = &tile[tile.find("// Title:").expect("the Tile's title is marked")..tile.find("// The badge:").expect("the badge is marked")];
        let badge = &tile[tile.find("// The badge:").unwrap()..tile.find("if root.selected: FocusRing").unwrap()];
        assert!(title.contains("wrap: word-wrap;") && title.contains("overflow: elide;"), "two lines, the second elided");
        let (title_y, title_h, title_font) = (units_in(title, "y:"), units_in(title, "height:"), units_in(title, "font-size:"));
        let lines = title_h / (title_font * 1.17);
        assert!((2.0..3.0).contains(&lines), "room for two title lines, not one or three: {lines}");
        let (badge_y, badge_h) = (units_in(badge, "y:"), units_in(badge, "height:"));
        assert!(title_y + title_h <= badge_y + 1e-6, "the badge starts under the title");
        assert!(badge_y + badge_h <= TILE_TEXT_UNITS + 1e-6 && badge_y + badge_h >= TILE_TEXT_UNITS - 0.1, "the text block is used and not overrun");
    }

    /// Greedy word wrap with a plain average glyph width (bold, 0.58 of the text size, measured from a Deck-sized shot).
    fn wrap_estimate(text: &str, width_px: f32, text_px: f32) -> Vec<String> {
        let per_line = (width_px / (text_px * 0.58)).floor() as usize;
        let mut lines: Vec<String> = vec![String::new()];
        for word in text.split_whitespace() {
            let cur = lines.last_mut().unwrap();
            if cur.is_empty() {
                *cur = word.to_string();
            } else if cur.chars().count() + 1 + word.chars().count() <= per_line {
                cur.push(' ');
                cur.push_str(word);
            } else {
                lines.push(word.to_string());
            }
        }
        lines
    }

    #[test]
    fn long_zelda_style_titles_show_most_of_their_name_in_two_lines_at_six_columns() {
        let (w, h, font, gap) = DECK;
        let l = tile_layout(w, h, 8, font, gap);
        assert_eq!(l.columns, 6);
        let room = l.tile_w - 2.0 * TILE_PAD_UNITS * font;
        for (name, at_least) in [
            ("The Legend of Dragoon", 1.0),
            ("The Legend of Zelda: Majora's Mask", 0.8),
            ("The Legend of Zelda: The Wind Waker HD", 0.6),
            ("The Legend of Zelda: Twilight Princess", 0.6),
        ] {
            let lines = wrap_estimate(name, room, font * 0.85);
            let shown: usize = lines.iter().take(2).map(|s| s.chars().count()).sum::<usize>() + lines.len().min(2) - 1;
            let share = shown as f32 / name.chars().count() as f32;
            assert!(share >= at_least, "{name}: {lines:?} shows {share:.2}");
        }
    }


    fn tiles_of(ports: &[PortFacts], checkup: bool) -> Vec<Tile> {
        let fix = crate::fork::checkup::HomeTile { title: "Fix 1 problem".into(), note: "Game files".into(), mark: crate::fork::page::Mark::Problem };
        match build(ports, true, checkup.then_some(&fix)) {
            Home::Tiles(t) => t,
            Home::Empty => Vec::new(),
        }
    }

    #[test]
    fn the_focus_follows_the_tile_not_the_position() {
        let ports = [port("a", 30), port("b", 0), port("c", 0)];
        // Before the checkup answers: Continue(a), b, c, Add. Focus on c.
        let before = tiles_of(&ports, false);
        let c = before.iter().position(|t| t.key == "c").unwrap();
        let id = focus_id(&before[c]);
        // The checkup tile arrives in front of it: c moves one place down, and keeps the focus.
        let with_fix = tiles_of(&ports, true);
        let now = resolve_focus(Some(&id), c, &with_fix);
        assert_eq!(with_fix[now].key, "c", "the focus stays on port c");
        assert_ne!(now, c, "and c really did move");
        // The checkup tile goes away again (fixed): back where it was.
        assert_eq!(resolve_focus(Some(&id), now, &before), c);
        // The utility tiles keep it too.
        let fix_id = focus_id(&with_fix.iter().find(|t| t.kind == TileKind::Checkup).cloned().unwrap());
        let moved = tiles_of(&[port("a", 30), port("b", 0), port("c", 0), port("d", 0)], true);
        assert_eq!(moved[resolve_focus(Some(&fix_id), 1, &moved)].kind, TileKind::Checkup);
        let add_id = FocusId::Add;
        assert_eq!(moved[resolve_focus(Some(&add_id), 2, &moved)].kind, TileKind::Add);
    }

    #[test]
    fn the_focus_stays_on_a_port_that_becomes_or_stops_being_the_continue_tile() {
        let before = tiles_of(&[port("a", 10), port("b", 0)], false);
        let b_id = focus_id(before.iter().find(|t| t.key == "b").unwrap());
        // b was played: it is now the Continue tile in front.
        let after = tiles_of(&[port("a", 10), port("b", 50)], false);
        assert_eq!(after[resolve_focus(Some(&b_id), 1, &after)].key, "b");
    }

    #[test]
    fn when_the_focused_tile_is_gone_the_old_position_is_kept_inside_the_list() {
        let before = tiles_of(&[port("a", 0), port("b", 0), port("c", 0)], true);
        let c = before.iter().position(|t| t.key == "c").unwrap();
        let id = focus_id(&before[c]);
        // c was uninstalled: Fix, a, b, Add are left; the old place (3) is still there, so it is used.
        let after = tiles_of(&[port("a", 0), port("b", 0)], true);
        let now = resolve_focus(Some(&id), c, &after);
        assert_eq!(now, c.min(after.len() - 1));
        // A position past the end comes back inside; nothing known before keeps the position clamped too.
        assert_eq!(resolve_focus(Some(&id), 99, &after), after.len() - 1);
        assert_eq!(resolve_focus(None, 99, &after), after.len() - 1);
        assert_eq!(resolve_focus(None, 0, &after), 0);
        assert_eq!(resolve_focus(Some(&id), 3, &[]), 0, "no tiles: zero, never a panic");
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
