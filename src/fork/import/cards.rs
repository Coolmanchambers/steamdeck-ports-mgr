//! The card model of Import ports: what each page says, in plain words, without any UI code. The screens in
//! `src/app/import_cards_screen.rs` draw these [`Page`]s.
//!
//! The matching engine's output enters in one place only: [`Cards::from_scan`] turns what the scan found into
//! [`Item`]s: a [`Match`] (the port, its cover, how it was matched in words, how sure) or an [`Unsure`] folder
//! (it fits two or three ports, or a program and no port: the user says which). Everything after it reads
//! `Item`, so a better matching engine changes that one function.
//!
//! Pages (one short question each, up to three big choices, the recommended one highlighted and focused):
//! - [`found_page`]: "Found 4 ports" with a cover tile for each, or "No ports found";
//! - [`card_page`]: one port at a time ("Port 2 of 5") with its cover, name, where it comes from, how it
//!   matched, its size and whether a Steam shortcut points at it; Yes / Pick another port / Not a port;
//! - [`which_page`]: "Which port is this?" for a folder that fits several ports or none, the likely ports as
//!   tiles to choose, Another port / a GitHub link, and Not a port in the footer;
//! - [`confirm_page`]: "Move 4 ports into your ports folder?" with the covers of the ones said yes to.
//!
//! "Not a port" is remembered in `fork-settings.json` (`import_ignored`, a list of folders) so the scan
//! stops offering that folder. The folder itself is never touched.

use super::scan::{Confidence, Found, ImportScan, Size};
use crate::core::models::Port;
use crate::fork::adopt::MatchKind;
use crate::fork::art::thumbs::ArtRef;
use crate::fork::page::{Card, Focus, Page, PageButton, PageOption, Tile, LEFT, MIDDLE};
use crate::fork::port_mover::old_copies;
use crate::fork::rom_screens::util::human_bytes;
use crate::fork::screens::short_path;
use crate::fork::settings::ForkSettings;
use serde_json::Value;
use std::io;
use std::path::{Path, PathBuf};

pub const KEY_IGNORED: &str = "import_ignored";
pub const HEADER: &str = "Import ports";
/// How many ports a "Which port is this?" page offers as tiles.
const MAX_TILES: usize = 3;

/// One folder on disk that probably is a catalog port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub port_key: String,
    pub port_name: String,
    pub catalog_folder: String,
    /// `owner/project` when the catalog says where the port comes from.
    pub repo: Option<String>,
    pub art: Option<ArtRef>,
    pub path: PathBuf,
    /// How it matched, as a sentence.
    pub why: String,
    /// The match the user must look at twice: Yes is not highlighted for it.
    pub strong: bool,
    pub size: Size,
    pub steam_shortcuts: usize,
    /// A Deck-ready project the catalog does not have: it is added to the user's port list first.
    pub new_to_list: bool,
    /// The user picked this port for the folder (so nothing is left to confirm).
    pub chosen: bool,
    /// Position in `ImportScan::found`, to get back to the full find.
    pub found: usize,
}

impl Match {
    /// A find as the cards show it. `port` is the catalog entry it matched (for its cover and source).
    pub fn of_found(found: &Found, index: usize, port: Option<&Port>) -> Match {
        let strong = found.confidence != Confidence::CheckThis;
        Match {
            port_key: found.port_key.clone(),
            port_name: found.port_name.clone(),
            catalog_folder: found.catalog_folder.clone(),
            repo: port.and_then(|p| p.repo.clone()),
            art: port.map(ArtRef::of_port),
            path: found.path.clone(),
            why: why_text(found, strong),
            strong,
            size: found.size,
            steam_shortcuts: found.steam.len(),
            new_to_list: found.new_entry.is_some(),
            chosen: found.how == MatchKind::Chosen,
            found: index,
        }
    }

    /// The short lines of the port's card.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(repo) = &self.repo {
            lines.push(format!("From: {repo}"));
        }
        lines.push(self.why.clone());
        if self.new_to_list {
            lines.push("New to your port list. It is added first.".to_string());
        }
        lines.push(format!("Size: {}", size_text(self.size)));
        match self.steam_shortcuts {
            0 => {}
            1 => lines.push("Has a Steam shortcut".to_string()),
            n => lines.push(format!("Has {n} Steam shortcuts")),
        }
        lines
    }

    /// The paths, shown only when the user asks for Details.
    pub fn details(&self, home: &Path, ports_folder: &Path) -> String {
        let mut text = format!(
            "{}\n\nFound in: {}\nWill be copied to: {}",
            self.port_name,
            short_path(&self.path, home),
            short_path(&ports_folder.join(&self.catalog_folder), home)
        );
        if self.new_to_list {
            text.push_str("\n\nThis port is not in the catalog. It is added to your own port list just before the move, and taken off again if it does not move.");
        }
        if self.steam_shortcuts > 0 {
            text.push_str("\n\nThe Steam shortcut keeps its place and follows the port to its new folder.");
        }
        text.push_str("\n\nThe old folder stays where it is until you choose to remove it.");
        text
    }
}

fn size_text(size: Size) -> String {
    format!("{}{}", if size.complete { "" } else { "at least " }, human_bytes(size.bytes))
}

/// The scan's `why` ("folder name", "folder name starts with \"SoH\"", "program soh.appimage") as a sentence
/// without file names.
fn why_text(found: &Found, strong: bool) -> String {
    match found.how {
        MatchKind::Chosen => "You picked this port.".to_string(),
        MatchKind::FolderName if found.why == "folder name" && found.confidence == Confidence::Exact => "The folder is named like this port.".to_string(),
        MatchKind::FolderName => "The folder name looks like this port.".to_string(),
        MatchKind::ReleaseName => "The folder is named like this port's download.".to_string(),
        MatchKind::ReleaseWord => match found.why.split_once("starts with ") {
            Some((_, word)) => format!("The folder name starts with {word}."),
            None => "The folder name starts with this port's name.".to_string(),
        },
        MatchKind::ExecutableName if strong => "It has this port's program inside.".to_string(),
        MatchKind::ExecutableName => "Only a program inside has a similar name.".to_string(),
    }
}

/// A port a folder could be, for a "Which port is this?" tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Maybe {
    pub key: String,
    pub name: String,
    pub art: Option<ArtRef>,
    /// Another folder of this scan was already found as this port (its name).
    pub found_in: Option<String>,
    pub installed: bool,
}

impl Maybe {
    pub fn of(port: &Port, scan: &ImportScan, target: &Path, ports_folder: &Path) -> Maybe {
        Maybe {
            key: port.key().to_string(),
            name: port.name.clone(),
            art: Some(ArtRef::of_port(port)),
            found_in: scan.found.iter().find(|f| f.port_key == port.key() && f.path != target).map(|f| f.dir_name.clone()),
            installed: crate::core::installer::is_installed(port, ports_folder),
        }
    }

    /// The tile's label: the port's name, and when it is already taken, where.
    pub fn label(&self) -> String {
        match (&self.found_in, self.installed) {
            (_, true) => format!("{} (already in your ports folder)", self.name),
            (Some(folder), false) => format!("{} (already found in {folder})", self.name),
            (None, false) => self.name.clone(),
        }
    }
}

/// A folder the scan could not settle: it fits two or three ports, or holds a program and fits none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsure {
    pub path: PathBuf,
    pub dir_name: String,
    /// The ports it could be (empty when nothing fits).
    pub could_be: Vec<Maybe>,
}

/// What the card walk goes through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Match(Match),
    Unsure(Unsure),
}

impl Item {
    pub fn path(&self) -> &Path {
        match self {
            Item::Match(m) => &m.path,
            Item::Unsure(u) => &u.path,
        }
    }
}

// -- "Not a port" ---------------------------------------------------------------------------------

/// The folders the user said are not ports.
pub fn ignored_paths(settings: &ForkSettings) -> Vec<PathBuf> {
    settings
        .get(KEY_IGNORED)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).filter(|s| !s.is_empty()).map(PathBuf::from).collect())
        .unwrap_or_default()
}

/// Remembers that `path` is not a port. Only the settings file changes; the folder is not touched.
pub fn ignore_path(settings: &mut ForkSettings, path: &Path) -> io::Result<()> {
    let mut list = ignored_paths(settings);
    if list.iter().any(|p| old_copies::same(p, path)) {
        return Ok(());
    }
    list.push(path.to_path_buf());
    settings.set(KEY_IGNORED, Value::Array(list.iter().map(|p| Value::from(p.to_string_lossy().into_owned())).collect()))
}

// -- The answers ----------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Undecided,
    Yes,
    NotAPort,
}

/// The items and what the user said about each, in order.
#[derive(Debug, Clone)]
pub struct Cards {
    items: Vec<Item>,
    verdicts: Vec<Verdict>,
    /// Said already (the user picked the port in an earlier scan of this screen): not asked again.
    settled: Vec<bool>,
}

impl Cards {
    pub fn new(items: Vec<Item>) -> Cards {
        let settled: Vec<bool> = items.iter().map(|i| matches!(i, Item::Match(m) if m.chosen)).collect();
        let verdicts = settled.iter().map(|s| if *s { Verdict::Yes } else { Verdict::Undecided }).collect();
        Cards { items, verdicts, settled }
    }

    /// The adapter between the scan and the cards: the matches, then the folders to choose for. Folders the
    /// user said are not ports are left out.
    pub fn from_scan(scan: &ImportScan, ignored: &[PathBuf], ports_folder: &Path) -> Cards {
        let skip = |p: &Path| ignored.iter().any(|i| old_copies::same(i, p));
        let port = |key: &str| scan.ports.iter().find(|p| p.key() == key);
        let mut items: Vec<Item> = scan
            .found
            .iter()
            .enumerate()
            .filter(|(_, f)| !skip(&f.path))
            .map(|(i, f)| Item::Match(Match::of_found(f, i, port(&f.port_key))))
            .collect();
        for a in scan.ambiguous.iter().filter(|a| !skip(&a.path)) {
            let could_be = a.keys.iter().filter_map(|k| port(k)).take(MAX_TILES).map(|p| Maybe::of(p, scan, &a.path, ports_folder)).collect();
            items.push(Item::Unsure(Unsure { path: a.path.clone(), dir_name: a.dir_name.clone(), could_be }));
        }
        for p in scan.unrecognised.iter().filter(|p| !skip(p)) {
            let dir_name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            items.push(Item::Unsure(Unsure { path: p.clone(), dir_name, could_be: Vec::new() }));
        }
        Cards::new(items)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn item(&self, i: usize) -> Option<&Item> {
        self.items.get(i)
    }

    /// A folder the user picked by hand (nothing was found, so there was nothing to walk): it becomes an item
    /// still to be given a port. Returns its position.
    pub fn add_folder(&mut self, path: PathBuf) -> usize {
        let dir_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.items.push(Item::Unsure(Unsure { path, dir_name, could_be: Vec::new() }));
        self.verdicts.push(Verdict::Undecided);
        self.settled.push(false);
        self.items.len() - 1
    }

    /// The matches among the items, in order.
    pub fn matches(&self) -> Vec<&Match> {
        self.items.iter().filter_map(|i| if let Item::Match(m) = i { Some(m) } else { None }).collect()
    }

    pub fn unsure_count(&self) -> usize {
        self.items.iter().filter(|i| matches!(i, Item::Unsure(_))).count()
    }

    /// The positions the walk asks about: everything the user has not settled already.
    pub fn walk(&self) -> Vec<usize> {
        (0..self.items.len()).filter(|i| !self.settled[*i]).collect()
    }

    pub fn verdict(&self, i: usize) -> Verdict {
        self.verdicts.get(i).copied().unwrap_or(Verdict::Undecided)
    }

    pub fn set(&mut self, i: usize, verdict: Verdict) {
        if let Some(v) = self.verdicts.get_mut(i) {
            *v = verdict;
        }
    }

    /// The user picked `chosen` for the folder at `i` (a card's "Pick another port", a tile of "Which port is
    /// this?"): the item becomes that port, said yes to, and it is still part of the walk.
    pub fn choose(&mut self, i: usize, chosen: Match) {
        if let Some(item) = self.items.get_mut(i) {
            *item = Item::Match(chosen);
            self.verdicts[i] = Verdict::Yes;
        }
    }

    /// The matches the user said yes to.
    pub fn accepted(&self) -> Vec<&Match> {
        self.items.iter().zip(&self.verdicts).filter(|(_, v)| **v == Verdict::Yes).filter_map(|(i, _)| if let Item::Match(m) = i { Some(m) } else { None }).collect()
    }

    pub fn accepted_count(&self) -> usize {
        self.accepted().len()
    }

    /// Their positions in `ImportScan::found`.
    pub fn accepted_found(&self) -> Vec<usize> {
        self.accepted().iter().map(|m| m.found).collect()
    }

    /// Starts the walk again: what the user settled stays, everything else is asked again.
    pub fn reset(&mut self) {
        for (i, v) in self.verdicts.iter_mut().enumerate() {
            *v = if self.settled[i] { Verdict::Yes } else { Verdict::Undecided };
        }
    }
}

// -- Pages ----------------------------------------------------------------------------------------

/// What an item on a card page does. The screen decides the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardAction {
    /// Go through the found ports one by one.
    Start,
    /// Leave Import ports (Skip in the wizard, Not now, Done).
    Leave,
    /// This is the port the card shows.
    Yes,
    PickAnother,
    /// This tile (an index into the folder's `could_be`) is the port.
    Pick(usize),
    /// Add the project from a GitHub link.
    GithubLink,
    /// The project GitHub described is the one: use it.
    UseIt,
    /// Go back to the text box and change the link.
    ChangeLink,
    NotAPort,
    Details,
    /// Copy the ports in.
    Move,
    /// One page back.
    Back,
    /// Look in one more folder.
    AddFolder,
}

fn base(window: &str, step_text: &str, title: &str) -> Page<CardAction> {
    Page {
        window_title: format!("{HEADER}: {window}"),
        large: true,
        header: HEADER.to_string(),
        step_text: step_text.to_string(),
        title: title.to_string(),
        ..Page::default()
    }
}

fn option(title: &str, detail: &str, action: CardAction, recommended: bool) -> PageOption<CardAction> {
    PageOption { title: title.to_string(), detail: detail.to_string(), choice: None, enabled: true, recommended, action }
}

fn footer(label: &str, action: CardAction) -> Option<PageButton<CardAction>> {
    Some(PageButton { label: label.to_string(), action })
}

fn details_button() -> Option<PageButton<CardAction>> {
    footer("Details", CardAction::Details)
}

fn ports(n: usize) -> String {
    if n == 1 { "1 port".to_string() } else { format!("{n} ports") }
}

fn folders(n: usize) -> String {
    if n == 1 { "1 folder".to_string() } else { format!("{n} folders") }
}

fn tile_of(m: &Match) -> Tile<CardAction> {
    Tile::shown(m.art.clone(), m.port_name.clone())
}

/// "Found 4 ports": the covers of everything the scan matched, then Check them / the leave choice
/// (`leave_label`: "Skip" in the wizard, "Not now" in Settings). With nothing found: where to go from here.
/// `deck_ready_note` says when the Deck-ready list could not be loaded, which matters when nothing was found.
pub fn found_page(cards: &Cards, leave_label: &str, deck_ready_note: Option<&str>) -> Page<CardAction> {
    let matches = cards.matches();
    if cards.is_empty() {
        let mut p = base("nothing found", "", "No ports found");
        p.paragraphs = vec![
            "Nothing that looks like a port turned up outside your ports folder.".to_string(),
            "If a port lives in a folder that was not looked in, add that folder and look again.".to_string(),
        ];
        if let Some(note) = deck_ready_note {
            p.paragraphs.push(note.to_string());
        }
        p.options = vec![
            option("Look in another folder...", "Choose a folder to look in", CardAction::AddFolder, true),
            option("Use a GitHub link...", "For a port you have that is not on the list", CardAction::GithubLink, false),
            option(leave_label_for_empty(leave_label), "", CardAction::Leave, false),
        ];
        p.buttons[LEFT] = details_button();
        p.initial = Focus::Option(0);
        return p;
    }
    let unsure = cards.unsure_count();
    let mut p = if matches.is_empty() {
        let mut p = base("found", "", &format!("{} to look at", folders(unsure)));
        p.paragraphs = vec!["Nothing was recognised for sure. You can say which port each of these is.".to_string()];
        p
    } else {
        let mut p = base("found", "", &format!("Found {}", ports(matches.len())));
        p.paragraphs = vec!["These look like ports you already have. Check each one, then they can be moved into your ports folder.".to_string()];
        if unsure > 0 {
            p.paragraphs.push(if unsure == 1 { "1 more folder could be a port. You will be asked which.".to_string() } else { format!("{unsure} more folders could be ports. You will be asked which.") });
        }
        p
    };
    if let Some(note) = deck_ready_note {
        p.paragraphs.push(note.to_string());
    }
    p.tiles = matches.iter().map(|m| tile_of(m)).collect();
    p.options = vec![option("Check them", "Go through them one at a time", CardAction::Start, true), option(leave_label, "", CardAction::Leave, false)];
    p.buttons[LEFT] = details_button();
    p.initial = Focus::Option(0);
    p
}

/// With nothing to check, leaving is the plain way out.
fn leave_label_for_empty(label: &str) -> &str {
    if label == "Skip" { "Next" } else { "Done" }
}

fn step_of(position: usize, total: usize) -> String {
    format!("Port {} of {}", position + 1, total)
}

/// One port: "Is this the right port?" with its card and the three choices. A weak match is not highlighted
/// and the cursor starts on "Pick another port", so a stray A does not accept it. `position` and `total`
/// count the pages of the walk.
pub fn card_page(m: &Match, position: usize, total: usize) -> Page<CardAction> {
    let step = step_of(position, total);
    let mut p = base(&step.to_lowercase(), &step, "Is this the right port?");
    p.card = Some(Card { art: m.art.clone(), title: m.port_name.clone(), lines: m.lines() });
    if !m.strong {
        p.paragraphs = vec!["This one is a guess. Check the picture before you say yes.".to_string()];
    }
    p.options = vec![
        option("Yes, this is it", "It is copied and checked before anything changes", CardAction::Yes, m.strong),
        option("Pick another port", "Choose the right port from the list", CardAction::PickAnother, false),
        option("Not a port", "Leave this folder alone and do not ask again", CardAction::NotAPort, false),
    ];
    p.buttons[LEFT] = details_button();
    p.initial = Focus::Option(if m.strong { 0 } else { 1 });
    p
}

/// "Which port is this?" for a folder the scan could not settle: the ports it could be as tiles (up to three),
/// then Another port... and Use a GitHub link..., with Not a port in the footer.
pub fn which_page(u: &Unsure, position: usize, total: usize) -> Page<CardAction> {
    let step = step_of(position, total);
    let mut p = base(&step.to_lowercase(), &step, "Which port is this?");
    p.paragraphs = if u.could_be.is_empty() {
        vec![format!("I found a folder called {} but could not tell which port it is.", u.dir_name)]
    } else {
        vec![format!("The folder {} fits more than one port. Pick the one it is.", u.dir_name)]
    };
    p.tiles = u
        .could_be
        .iter()
        .enumerate()
        .map(|(i, m)| Tile::choice(m.art.clone(), m.label(), CardAction::Pick(i)))
        .collect();
    p.options = vec![
        option("Another port...", "Choose from the whole list", CardAction::PickAnother, u.could_be.is_empty()),
        option("Use a GitHub link...", "For a port that is not on the list", CardAction::GithubLink, false),
    ];
    p.buttons[LEFT] = footer("Not a port", CardAction::NotAPort);
    p.buttons[MIDDLE] = details_button();
    p.initial = if u.could_be.is_empty() { Focus::Option(0) } else { Focus::Tile(0) };
    p
}

/// "Move 4 ports into your ports folder?" with the covers of the ones accepted. `free` is the room left where
/// they are going, when known; `blocked` lists the ports that cannot be moved right now with the reason, so the
/// user hears it before pressing Move.
pub fn confirm_page(cards: &Cards, free: Option<u64>, blocked: &[(String, String)]) -> Page<CardAction> {
    let accepted = cards.accepted();
    let n = accepted.len();
    let mut p = base("move", "", &format!("Move {} into your ports folder?", ports(n)));
    p.paragraphs = vec!["Each one is copied and checked first. Old copies stay until you say.".to_string()];
    p.tiles = accepted.iter().map(|m| tile_of(m)).collect();
    let total: u64 = accepted.iter().map(|m| m.size.bytes).sum();
    let at_least = if accepted.iter().all(|m| m.size.complete) { "" } else { "at least " };
    p.facts = vec![match free {
        Some(free) => format!("Together {at_least}{}. {} free.", human_bytes(total), human_bytes(free)),
        None => format!("Together {at_least}{}.", human_bytes(total)),
    }];
    let new: Vec<&str> = accepted.iter().filter(|m| m.new_to_list).map(|m| m.port_name.as_str()).collect();
    if !new.is_empty() {
        p.facts.push(format!("New to your port list, added first: {}.", new.join(", ")));
    }
    p.facts.extend(blocked.iter().map(|(name, why)| format!("{name} cannot be moved now: {why}")));
    p.options = vec![option(&format!("Move {}", ports(n)), "", CardAction::Move, true), option("Back", "", CardAction::Back, false)];
    p.buttons[LEFT] = details_button();
    p.initial = Focus::Option(0);
    p
}

/// Where the scan looked and what it left out, for Details on the found page.
pub fn scan_notes(scan: &ImportScan, home: &Path) -> String {
    let mut lines: Vec<String> = Vec::new();
    let roots: Vec<String> = scan.roots_scanned.iter().map(|r| short_path(r, home)).collect();
    lines.push(if roots.is_empty() { "None of your scan folders exist yet.".to_string() } else { format!("Looked in: {}", roots.join(", ")) });
    if !scan.collections.is_empty() {
        let names: Vec<String> = scan.collections.iter().map(|p| short_path(p, home)).collect();
        lines.push(format!("Skipped, it looks like a collection of many games: {}", names.join(", ")));
    }
    if !scan.left_alone.is_empty() {
        let names: Vec<String> = scan.left_alone.iter().map(|p| short_path(p, home)).collect();
        lines.push(format!("Left alone (you said not a port): {}", names.join(", ")));
    }
    lines.extend(scan.duplicates.iter().map(|(name, path)| format!("Also found {name} in {}: left out.", short_path(path, home))));
    if !scan.already_in_library.is_empty() {
        lines.push(format!("Already in your ports folder: {}", scan.already_in_library.join(", ")));
    }
    if let Some(note) = &scan.deck_ready_note {
        lines.push(note.clone());
    }
    if let Some(note) = &scan.steam_note {
        lines.push(note.clone());
    }
    if scan.cancelled {
        lines.push("The search was stopped, so this may be incomplete.".to_string());
    }
    if scan.deep_limit_hit {
        lines.push(format!("Looking deeper stopped after {} folders.", crate::fork::adopt::MAX_DEEP_FOLDERS));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::super::scan::fixtures::*;
    use super::super::scan::{read_shortcuts, run_scan, ScanInput};
    use super::*;
    use crate::fork::import::folders::ScanFolders;
    use crate::fork::page::RIGHT;
    use std::fs;

    fn scan_of(h: &Home) -> ImportScan {
        let input = ScanInput {
            roots: ScanFolders::default().roots(&h.home, &h.library),
            catalog: catalog(),
            library_dir: h.library.clone(),
            home: h.home.clone(),
            deeper: false,
            shortcuts: read_shortcuts(&h.home, &ForkSettings::load(&h.home.join("s.json"))),
            ..Default::default()
        };
        run_scan(&input, &|| false, &|_| {})
    }

    fn cards_of(h: &Home) -> (ImportScan, Cards) {
        let scan = scan_of(h);
        let cards = Cards::from_scan(&scan, &[], &h.library);
        (scan, cards)
    }

    fn named<'a>(cards: &'a Cards, name: &str) -> &'a Match {
        cards.matches().into_iter().find(|m| m.port_name == name).unwrap_or_else(|| panic!("no match {name}"))
    }

    fn position(cards: &Cards, name: &str) -> usize {
        (0..cards.len()).find(|i| matches!(cards.item(*i), Some(Item::Match(m)) if m.port_name == name)).unwrap()
    }

    /// Tim's SD card, the way the import screen scans it, plus extra folders.
    fn tims(dir: &Path, extra: impl FnOnce(&Path)) -> (PathBuf, ImportScan, Cards) {
        let sd = crate::fork::adopt::test_fixtures::tims_sd_card(dir);
        extra(&sd);
        let input = super::super::scan::tests::tims_input(&sd, Default::default());
        let scan = run_scan(&input, &|| false, &|_| {});
        let cards = Cards::from_scan(&scan, &[], &sd.join("Ports"));
        (sd, scan, cards)
    }

    fn all_text(p: &Page<CardAction>) -> String {
        let mut parts: Vec<String> = vec![p.title.clone(), p.step_text.clone(), p.header.clone()];
        parts.extend(p.paragraphs.iter().cloned());
        parts.extend(p.facts.iter().cloned());
        if let Some(c) = &p.card {
            parts.push(c.title.clone());
            parts.extend(c.lines.iter().cloned());
        }
        parts.extend(p.tiles.iter().map(|t| t.label.clone()));
        parts.extend(p.options.iter().flat_map(|o| [o.title.clone(), o.detail.clone()]));
        parts.extend(p.buttons.iter().flatten().map(|b| b.label.clone()));
        parts.join("\n")
    }

    fn found(cards: &Cards) -> Page<CardAction> {
        found_page(cards, "Skip", None)
    }

    #[test]
    fn the_adapter_turns_each_find_into_a_match_with_its_words_cover_and_source() {
        let h = fake_home("cards_adapter");
        let (scan, cards) = cards_of(&h);
        assert_eq!(cards.len(), 2);
        let soh = named(&cards, "Ship of Harkinian");
        assert_eq!(soh.why, "The folder is named like this port.");
        assert!(soh.strong);
        assert_eq!(soh.repo.as_deref(), Some("a/soh"));
        assert_eq!(soh.art.as_ref().unwrap().folder, "SoH");
        assert_eq!(soh.steam_shortcuts, 1);
        assert!(!soh.new_to_list && !soh.chosen);
        assert_eq!(scan.found[soh.found].path, soh.path, "the position leads back to the full find");
        let dusk = named(&cards, "Dusklight");
        assert_eq!(dusk.why, "It has this port's program inside.", "dusk-old holds the program the catalog names");
        assert_eq!(dusk.steam_shortcuts, 0);
    }

    #[test]
    fn a_program_that_only_looks_like_the_port_is_the_weak_match() {
        let h = fake_home("cards_weak");
        put_exe(&h.home.join("Games/misc/severedchains.x86_64"), b"x");
        let (_, cards) = cards_of(&h);
        let weak = named(&cards, "Severed Chains");
        assert_eq!(weak.why, "Only a program inside has a similar name.");
        assert!(!weak.strong);
        assert!(named(&cards, "Dusklight").strong);
    }

    #[test]
    fn the_scans_reasons_become_plain_sentences_without_file_names() {
        let dir = temp_dir("cards_why");
        let (_, _, cards) = tims(&dir, |_| {});
        let sentences: Vec<String> = cards.matches().iter().map(|m| m.why.clone()).collect();
        assert!(sentences.iter().any(|s| s == "The folder name starts with \"SoH\"."), "{sentences:?}");
        assert!(sentences.iter().all(|s| !s.contains(".appimage") && !s.contains(".zip") && s.ends_with('.')), "{sentences:?}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_deck_ready_only_port_says_it_is_new_to_the_port_list_and_the_move_page_repeats_it() {
        let dir = temp_dir("cards_new_entry");
        let (sd, _, mut cards) = tims(&dir, |_| {});
        let sc = named(&cards, "The Legend of Dragoon (Severed-Chains)").clone();
        assert!(sc.new_to_list);
        assert!(sc.lines().contains(&"New to your port list. It is added first.".to_string()), "{:?}", sc.lines());
        assert!(sc.details(&sd, &sd.join("Ports")).contains("not in the catalog"), "details explain it");
        assert!(cards.matches().iter().filter(|m| m.port_key != sc.port_key).all(|m| !m.new_to_list), "only the Deck-ready only port is new");
        for i in 0..cards.len() {
            cards.set(i, Verdict::Yes);
        }
        let p = confirm_page(&cards, None, &[]);
        assert!(p.facts.iter().any(|f| f == "New to your port list, added first: The Legend of Dragoon (Severed-Chains)."), "{:?}", p.facts);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn folders_the_user_said_are_not_ports_are_left_out() {
        let h = fake_home("cards_ignored");
        let scan = scan_of(&h);
        let ignored = vec![h.home.join("Games/dusk-old")];
        let cards = Cards::from_scan(&scan, &ignored, &h.library);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards.matches()[0].port_name, "Ship of Harkinian");
    }

    #[test]
    fn not_a_port_is_saved_once_in_the_settings_file_and_leaves_the_folder_alone() {
        let h = fake_home("cards_ignore_save");
        let path = h.home.join("fork-settings.json");
        fs::write(&path, r#"{"setup_done": true}"#).unwrap();
        let folder = h.home.join("Games/dusk-old");
        let before = fs::read(folder.join("dusk")).unwrap();
        let mut settings = ForkSettings::load(&path);
        ignore_path(&mut settings, &folder).unwrap();
        ignore_path(&mut settings, &folder).unwrap();
        let again = ForkSettings::load(&path);
        assert_eq!(ignored_paths(&again), [folder.clone()]);
        assert_eq!(again.get("setup_done"), Some(&Value::Bool(true)), "other settings survive");
        assert_eq!(fs::read(folder.join("dusk")).unwrap(), before, "the folder is untouched");
        assert!(folder.is_dir());
    }

    #[test]
    fn a_settings_file_that_is_not_an_object_is_not_overwritten_by_not_a_port() {
        let h = fake_home("cards_ignore_garbage");
        let path = h.home.join("fork-settings.json");
        fs::write(&path, "[1, 2").unwrap();
        let mut settings = ForkSettings::load(&path);
        assert!(ignore_path(&mut settings, &h.home.join("Games/dusk-old")).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "[1, 2");
    }

    #[test]
    fn a_wrong_kind_of_value_in_the_ignore_list_reads_as_empty() {
        let h = fake_home("cards_ignore_wrong");
        let path = h.home.join("fork-settings.json");
        fs::write(&path, r#"{"import_ignored": "nope"}"#).unwrap();
        assert!(ignored_paths(&ForkSettings::load(&path)).is_empty());
        fs::write(&path, r#"{"import_ignored": ["", 5, "/a/b"]}"#).unwrap();
        assert_eq!(ignored_paths(&ForkSettings::load(&path)), [PathBuf::from("/a/b")]);
    }

    #[test]
    fn the_found_page_says_how_many_and_shows_a_cover_tile_for_each() {
        let h = fake_home("cards_found");
        let (_, cards) = cards_of(&h);
        let p = found_page(&cards, "Not now", None);
        assert_eq!(p.title, "Found 2 ports");
        assert_eq!(p.window_title, "Import ports: found");
        assert_eq!(p.tiles.iter().map(|t| t.label.as_str()).collect::<Vec<_>>(), ["Dusklight", "Ship of Harkinian"]);
        assert!(p.tiles.iter().all(|t| t.action.is_none()), "the found tiles are shown, not chosen");
        assert_eq!(p.options.iter().map(|o| (o.title.as_str(), o.recommended)).collect::<Vec<_>>(), [("Check them", true), ("Not now", false)]);
        assert_eq!(p.initial, Focus::Option(0));
        let one = Cards::new(cards.matches().into_iter().take(1).cloned().map(Item::Match).collect());
        assert_eq!(found(&one).title, "Found 1 port");
        assert_eq!(found(&one).options[1].title, "Skip");
    }

    #[test]
    fn nothing_found_says_so_offers_another_folder_and_passes_on_the_deck_ready_note() {
        let p = found(&Cards::new(vec![]));
        assert_eq!(p.title, "No ports found");
        assert_eq!(p.window_title, "Import ports: nothing found");
        assert!(p.tiles.is_empty());
        assert_eq!(p.options[0].action, CardAction::AddFolder);
        assert_eq!(p.options[1].action, CardAction::GithubLink, "nothing matched: the GitHub link is the way to say what the folder is");
        assert!(p.options[0].recommended);
        assert_eq!(p.options[2].title, "Next", "in the wizard leaving goes to the next step");
        assert_eq!(found_page(&Cards::new(vec![]), "Not now", None).options[2].title, "Done");
        assert_eq!(p.paragraphs.len(), 2);
        let note = "The Deck-ready list couldn't be loaded, so ports only on that list aren't recognised.";
        assert_eq!(found_page(&Cards::new(vec![]), "Skip", Some(note)).paragraphs.last().unwrap(), note);
    }

    #[test]
    fn a_card_shows_the_cover_name_source_how_it_matched_size_and_steam() {
        let h = fake_home("cards_card");
        let (_, cards) = cards_of(&h);
        let soh = position(&cards, "Ship of Harkinian");
        let Some(Item::Match(m)) = cards.item(soh) else { panic!() };
        let p = card_page(m, soh, cards.len());
        assert_eq!(p.step_text, format!("Port {} of 2", soh + 1));
        assert_eq!(p.window_title, format!("Import ports: port {} of 2", soh + 1));
        let card = p.card.as_ref().unwrap();
        assert_eq!(card.title, "Ship of Harkinian");
        assert_eq!(card.lines, ["From: a/soh", "The folder is named like this port.", "Size: 8 KB", "Has a Steam shortcut"]);
        assert_eq!(card.art.as_ref().unwrap().folder, "SoH");
        assert_eq!(p.options.iter().map(|o| o.title.as_str()).collect::<Vec<_>>(), ["Yes, this is it", "Pick another port", "Not a port"]);
        assert!(p.options[0].recommended && !p.options[1].recommended && !p.options[2].recommended);
        assert_eq!(p.initial, Focus::Option(0), "the recommended choice has the focus");
        assert!(p.paragraphs.is_empty());
    }

    #[test]
    fn a_weak_match_loses_the_highlight_says_check_the_picture_and_does_not_start_on_yes() {
        let h = fake_home("cards_card_weak");
        put_exe(&h.home.join("Games/misc/severedchains.x86_64"), b"x");
        let (_, cards) = cards_of(&h);
        let weak = (0..cards.len()).find(|i| matches!(cards.item(*i), Some(Item::Match(m)) if !m.strong)).unwrap();
        let Some(Item::Match(m)) = cards.item(weak) else { panic!() };
        let p = card_page(m, weak, cards.len());
        assert!(p.paragraphs[0].contains("Check the picture"));
        assert!(p.options.iter().all(|o| !o.recommended), "nothing is highlighted");
        assert_eq!(p.initial, Focus::Option(1), "a stray A opens the list, it does not accept the guess");
        assert!(p.card.as_ref().unwrap().lines.contains(&"Only a program inside has a similar name.".to_string()));
    }

    #[test]
    fn several_steam_shortcuts_are_counted_and_an_unfinished_size_says_at_least() {
        let mut m = Match {
            port_key: "k".into(),
            port_name: "X".into(),
            catalog_folder: "x".into(),
            repo: None,
            art: None,
            path: "/a/x".into(),
            why: "The folder is named like this port.".into(),
            strong: true,
            size: Size { bytes: 3_000_000_000, files: 5, complete: false },
            steam_shortcuts: 2,
            new_to_list: false,
            chosen: false,
            found: 0,
        };
        assert_eq!(m.lines(), ["The folder is named like this port.", "Size: at least 3.0 GB", "Has 2 Steam shortcuts"]);
        m.steam_shortcuts = 0;
        assert_eq!(m.lines().len(), 2, "no Steam line without a shortcut");
    }

    #[test]
    fn folders_that_fit_several_ports_or_none_become_which_port_is_this_pages() {
        let dir = temp_dir("cards_which");
        let (sd, scan, cards) = tims(&dir, |sd| {
            put_exe(&sd.join("The Legend of Zelda Majoras Mask/mm.appimage"), b"x");
            put_exe(&sd.join("Mystery Game/game.x86_64"), b"x");
        });
        assert_eq!(cards.matches().len(), 4);
        assert_eq!(cards.unsure_count(), 2);
        assert_eq!(found(&cards).title, "Found 4 ports");
        assert!(found(&cards).paragraphs.iter().any(|p| p == "2 more folders could be ports. You will be asked which."), "{:?}", found(&cards).paragraphs);
        let ambiguous = (0..cards.len()).find(|i| matches!(cards.item(*i), Some(Item::Unsure(u)) if !u.could_be.is_empty())).unwrap();
        let Some(Item::Unsure(u)) = cards.item(ambiguous) else { panic!() };
        assert_eq!(u.path, sd.join("The Legend of Zelda Majoras Mask"));
        assert_eq!(u.could_be.len(), 2);
        let p = which_page(u, ambiguous, cards.len());
        assert_eq!(p.title, "Which port is this?");
        assert!(p.window_title.starts_with("Import ports: port "), "{}", p.window_title);
        assert!(p.paragraphs[0].contains("fits more than one port"));
        assert_eq!(p.tiles.len(), 2);
        assert_eq!(p.tiles[0].label, "The Legend of Zelda: Majora's Mask (2ship2harkinian) (already found in 2Ship-Keiichi-Charlie-Linux)");
        assert_eq!(p.tiles[1].label, "The Legend of Zelda: Majora's Mask (Zelda64Recomp)");
        assert_eq!(p.tiles[0].action, Some(CardAction::Pick(0)));
        assert_eq!(p.options.iter().map(|o| o.title.as_str()).collect::<Vec<_>>(), ["Another port...", "Use a GitHub link..."]);
        assert_eq!(p.buttons[LEFT].as_ref().unwrap().label, "Not a port");
        assert_eq!(p.initial, Focus::Tile(0), "the cursor starts on the first likely port");
        assert!(p.options.iter().all(|o| !o.recommended));
        let _ = scan;
        let none = (0..cards.len()).find(|i| matches!(cards.item(*i), Some(Item::Unsure(u)) if u.could_be.is_empty())).unwrap();
        let Some(Item::Unsure(u)) = cards.item(none) else { panic!() };
        let p = which_page(u, none, cards.len());
        assert!(p.tiles.is_empty());
        assert!(p.paragraphs[0].contains("Mystery Game") && p.paragraphs[0].contains("could not tell"), "{:?}", p.paragraphs);
        assert!(p.options[0].recommended && p.initial == Focus::Option(0), "with no likely port, Another port is the way on");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn choosing_a_port_for_a_folder_turns_it_into_an_accepted_match_in_the_same_place() {
        let dir = temp_dir("cards_choose");
        let (sd, mut scan, mut cards) = tims(&dir, |sd| put_exe(&sd.join("Mystery Game/game.x86_64"), b"x"));
        let at = (0..cards.len()).find(|i| matches!(cards.item(*i), Some(Item::Unsure(_)))).unwrap();
        let before = cards.len();
        let port = scan.ports.iter().find(|p| p.key() == "Zelda64Recomp/Zelda64Recomp").unwrap().clone();
        let found = super::super::scan::measure_choice(&sd.join("Mystery Game"), &port, &[], None, &|| false);
        scan.found.push(found.clone());
        cards.choose(at, Match::of_found(&found, scan.found.len() - 1, Some(&port)));
        assert_eq!(cards.len(), before, "the folder was one item and still is");
        assert_eq!(cards.verdict(at), Verdict::Yes);
        let Some(Item::Match(m)) = cards.item(at) else { panic!() };
        assert!(m.chosen && m.why == "You picked this port.");
        assert!(cards.walk().contains(&at), "a pick made in this walk is still shown in it");
        assert!(cards.accepted().iter().any(|a| a.port_key == "Zelda64Recomp/Zelda64Recomp"));
        assert_eq!(scan.found[*cards.accepted_found().last().unwrap()].path, sd.join("Mystery Game"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_pick_from_an_earlier_scan_is_settled_and_not_asked_again() {
        let dir = temp_dir("cards_settled");
        let (sd, scan, cards) = tims(&dir, |_| {});
        let port = scan.ports.iter().find(|p| p.key() == "Zelda64Recomp/Zelda64Recomp").unwrap().clone();
        let mut scan = scan;
        put_exe(&sd.join("Mystery Game/game.x86_64"), b"x");
        let found = super::super::scan::measure_choice(&sd.join("Mystery Game"), &port, &[], None, &|| false);
        scan.found.push(found);
        let mut again = Cards::from_scan(&scan, &[], &sd.join("Ports"));
        let settled = (0..again.len()).find(|i| matches!(again.item(*i), Some(Item::Match(m)) if m.chosen)).unwrap();
        assert_eq!(again.verdict(settled), Verdict::Yes);
        assert!(!again.walk().contains(&settled), "the walk skips it");
        assert_eq!(again.walk().len(), again.len() - 1);
        assert!(again.accepted_count() >= 1);
        again.set(settled, Verdict::NotAPort);
        again.reset();
        assert_eq!(again.verdict(settled), Verdict::Yes, "starting the walk again keeps what was settled");
        assert_eq!(cards.len() + 1, again.len());
        let _ = fs::remove_dir_all(dir);
    }

#[test]    fn a_folder_picked_by_hand_is_an_item_still_to_be_given_a_port() {        let mut cards = Cards::new(vec![]);        let at = cards.add_folder(PathBuf::from("/home/deck/Games/My Game"));        assert_eq!(at, 0);        assert_eq!(cards.walk(), [0]);        assert_eq!(cards.verdict(0), Verdict::Undecided);        let Some(Item::Unsure(u)) = cards.item(0) else { panic!() };        assert_eq!(u.dir_name, "My Game");        assert!(u.could_be.is_empty());        assert_eq!(cards.accepted_count(), 0);    }
    #[test]
    fn no_page_text_shows_a_file_name_or_a_path_outside_details() {
        let h = fake_home("cards_nopaths");
        let (_, mut cards) = cards_of(&h);
        let total = cards.len();
        let mut pages = vec![found(&cards), found(&Cards::new(vec![]))];
        for i in 0..total {
            let Some(Item::Match(m)) = cards.item(i) else { panic!() };
            pages.push(card_page(m, i, total));
        }
        for p in &pages {
            let t = all_text(p);
            assert!(!t.contains('/') || t.contains("From: "), "a path in {t}");
            assert!(!t.contains("/home") && !t.contains("~/"), "{t}");
            assert!(!t.to_lowercase().contains("json"), "{t}");
        }
        cards.set(0, Verdict::Yes);
        let t = all_text(&confirm_page(&cards, Some(1_000_000), &[]));
        assert!(!t.contains("~/") && !t.to_lowercase().contains("json"), "{t}");
    }

    #[test]
    fn details_hold_the_paths_and_say_the_old_folder_stays() {
        let h = fake_home("cards_details");
        let (_, cards) = cards_of(&h);
        let soh = named(&cards, "Ship of Harkinian");
        let d = soh.details(&h.home, &h.library);
        assert!(d.contains("Found in: ~/Applications/SoH"), "{d}");
        assert!(d.contains("Will be copied to: ~/DeckPorts/Library/SoH"), "{d}");
        assert!(d.contains("Steam shortcut keeps its place"), "{d}");
        assert!(d.contains("old folder stays"), "{d}");
        assert!(!named(&cards, "Dusklight").details(&h.home, &h.library).contains("Steam shortcut"));
    }

    #[test]
    fn every_card_page_has_details_in_the_footer_at_most_three_choices_and_is_drawn_large() {
        let h = fake_home("cards_footer");
        let (_, mut cards) = cards_of(&h);
        cards.set(0, Verdict::Yes);
        let Some(Item::Match(m)) = cards.item(0) else { panic!() };
        let unsure = Unsure { path: "/a/b".into(), dir_name: "b".into(), could_be: vec![] };
        for p in [found(&cards), card_page(m, 0, 2), confirm_page(&cards, None, &[]), found(&Cards::new(vec![])), which_page(&unsure, 0, 1)] {
            let has_details = p.buttons.iter().flatten().any(|b| b.action == CardAction::Details);
            assert!(has_details, "{}", p.title);
            assert!(p.options.len() <= 3, "{}", p.title);
            assert!(p.buttons[RIGHT].is_none(), "the choices are the options, the footer only has small buttons");
            assert!(p.large && p.step_count == 0 && p.header == "Import ports", "{}", p.title);
        }
    }

    #[test]
    fn the_answers_are_kept_in_order_and_lead_back_to_the_finds() {
        let h = fake_home("cards_verdicts");
        let (scan, mut cards) = cards_of(&h);
        assert_eq!(cards.accepted_count(), 0);
        assert_eq!(cards.verdict(0), Verdict::Undecided);
        cards.set(1, Verdict::Yes);
        cards.set(0, Verdict::NotAPort);
        assert_eq!(cards.accepted().iter().map(|m| m.port_name.as_str()).collect::<Vec<_>>(), ["Ship of Harkinian"]);
        assert_eq!(scan.found[cards.accepted_found()[0]].port_name, "Ship of Harkinian");
        cards.set(99, Verdict::Yes);
        assert_eq!(cards.verdict(99), Verdict::Undecided);
        cards.reset();
        assert_eq!(cards.accepted_count(), 0);
        assert_eq!(cards.walk(), [0, 1]);
    }

    #[test]
    fn the_move_page_names_the_count_shows_the_covers_and_says_old_copies_stay() {
        let h = fake_home("cards_confirm");
        let (_, mut cards) = cards_of(&h);
        cards.set(0, Verdict::Yes);
        cards.set(1, Verdict::Yes);
        let p = confirm_page(&cards, Some(120_000_000_000), &[]);
        assert_eq!(p.title, "Move 2 ports into your ports folder?");
        assert_eq!(p.window_title, "Import ports: move");
        assert_eq!(p.tiles.len(), 2);
        assert_eq!(p.paragraphs, ["Each one is copied and checked first. Old copies stay until you say."]);
        assert_eq!(p.facts, ["Together 8 KB. 120.0 GB free."]);
        assert_eq!(p.options.iter().map(|o| (o.title.as_str(), o.action.clone(), o.recommended)).collect::<Vec<_>>(), [("Move 2 ports", CardAction::Move, true), ("Back", CardAction::Back, false)]);
        cards.set(0, Verdict::NotAPort);
        let one = confirm_page(&cards, None, &[]);
        assert_eq!(one.title, "Move 1 port into your ports folder?");
        assert_eq!(one.options[0].title, "Move 1 port");
        assert_eq!(one.facts, ["Together 8 KB."]);
    }

    #[test]
    fn a_port_that_cannot_be_moved_now_is_named_with_its_reason_on_the_move_page() {
        let h = fake_home("cards_confirm_blocked");
        let (_, mut cards) = cards_of(&h);
        cards.set(0, Verdict::Yes);
        cards.set(1, Verdict::Yes);
        let blocked = vec![("Dusklight".to_string(), "not enough free space".to_string())];
        let p = confirm_page(&cards, None, &blocked);
        assert_eq!(p.facts.last().unwrap(), "Dusklight cannot be moved now: not enough free space");
        assert_eq!(p.title, "Move 2 ports into your ports folder?", "the count is what was chosen; the reason is said under it");
    }

    #[test]
    fn the_scan_notes_say_where_it_looked_and_what_it_left_out() {
        let h = fake_home("cards_notes");
        put_file(&h.home.join("Games/Fan Remake/x"), b"1");
        put_file(&h.home.join("Games/soh-copy/shipofharkinian.appimage"), b"1");
        fs::create_dir_all(h.library.join("severed-chains")).unwrap();
        put_file(&h.home.join("Games/severed-chains/x"), b"1");
        let mut scan = scan_of(&h);
        scan.deck_ready_note = Some("The Deck-ready list couldn't be loaded.".into());
        scan.left_alone.push(h.home.join("Games/other"));
        let text = scan_notes(&scan, &h.home);
        assert!(text.starts_with("Looked in: ~/Applications, ~/Games"), "{text}");
        assert!(text.contains("Also found Ship of Harkinian in ~/Games/soh-copy: left out."), "{text}");
        assert!(text.contains("Already in your ports folder: Severed Chains"), "{text}");
        assert!(text.contains("The Deck-ready list couldn't be loaded."), "{text}");
        assert!(text.contains("Left alone (you said not a port): ~/Games/other"), "{text}");
    }
}
