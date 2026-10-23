//! The card model of Import ports: what each page says, in plain words, without any UI code. The screens in
//! `src/app/import_cards_screen.rs` draw these [`Page`]s.
//!
//! The matching engine's output enters in one place only: [`from_scan`] turns what the scan found into
//! [`Match`]es (the port, its cover, how it was matched in words, how sure). Everything after it reads
//! `Match`, so a better matching engine changes that one function.
//!
//! Pages (one short question each, up to three big choices, the recommended one highlighted and focused):
//! - [`found_page`]: "Found 4 ports" with a cover tile for each, or "No ports found";
//! - [`card_page`]: one port at a time ("Port 2 of 5") with its cover, name, where it comes from, how it
//!   matched, its size and whether a Steam shortcut points at it; Yes / Pick another port / Not a port;
//! - [`confirm_page`]: "Move 4 ports into your ports folder?" with the covers of the ones said yes to.
//!
//! "Not a port" is remembered in `fork-settings.json` (`import_ignored`, a list of folders) so the scan
//! stops offering that folder. The folder itself is never touched.

use super::scan::{Confidence, Found, ImportScan, Size};
use crate::core::models::Port;
use crate::fork::adopt::MatchKind;
use crate::fork::art::thumbs::ArtRef;
use crate::fork::page::{Card, Focus, Page, PageButton, PageOption, Tile, LEFT};
use crate::fork::port_mover::old_copies;
use crate::fork::rom_screens::util::human_bytes;
use crate::fork::screens::short_path;
use crate::fork::settings::ForkSettings;
use serde_json::Value;
use std::io;
use std::path::{Path, PathBuf};

pub const KEY_IGNORED: &str = "import_ignored";
pub const HEADER: &str = "Import ports";

/// How a folder came to be matched with a catalog port, in the words the card uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The folder is named exactly like the port's folder.
    FolderNamed,
    /// The folder's name looks like the port's name.
    FolderLooksLike,
    /// The program the port is known by is inside the folder.
    ProgramInside,
    /// Only a program inside has a name like the port's: the one to look at twice.
    ProgramLooksLike,
}

impl Reason {
    /// False for the match that needs the user's eyes: the card does not highlight Yes for it.
    pub fn strong(self) -> bool {
        !matches!(self, Reason::ProgramLooksLike)
    }

    pub fn text(self) -> &'static str {
        match self {
            Reason::FolderNamed => "The folder is named like this port.",
            Reason::FolderLooksLike => "The folder name looks like this port.",
            Reason::ProgramInside => "It has this port's program inside.",
            Reason::ProgramLooksLike => "Only a program inside has a similar name.",
        }
    }
}

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
    pub reason: Reason,
    pub size: Size,
    pub steam_shortcuts: usize,
    /// Position in `ImportScan::found`, to get back to the full find.
    pub found: usize,
}

impl Match {
    /// The short lines of the port's card.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(repo) = &self.repo {
            lines.push(format!("From: {repo}"));
        }
        lines.push(self.reason.text().to_string());
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

fn reason_of(found: &Found) -> Reason {
    match (found.how, found.confidence) {
        (MatchKind::FolderName, Confidence::Exact) => Reason::FolderNamed,
        (MatchKind::FolderName, _) => Reason::FolderLooksLike,
        (MatchKind::ExecutableName, Confidence::CheckThis) => Reason::ProgramLooksLike,
        (MatchKind::ExecutableName, _) => Reason::ProgramInside,
    }
}

/// The adapter between the scan and the cards. Folders the user said are not ports are left out.
pub fn from_scan(scan: &ImportScan, catalog: &[Port], ignored: &[PathBuf]) -> Vec<Match> {
    scan.found
        .iter()
        .enumerate()
        .filter(|(_, f)| !ignored.iter().any(|i| old_copies::same(i, &f.path)))
        .map(|(found, f)| {
            let port = catalog.iter().find(|p| p.key() == f.port_key);
            Match {
                port_key: f.port_key.clone(),
                port_name: f.port_name.clone(),
                catalog_folder: f.catalog_folder.clone(),
                repo: port.and_then(|p| p.repo.clone()),
                art: port.map(ArtRef::of_port),
                path: f.path.clone(),
                reason: reason_of(f),
                size: f.size,
                steam_shortcuts: f.steam.len(),
                found,
            }
        })
        .collect()
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

/// The matches and what the user said about each, in order.
#[derive(Debug, Clone)]
pub struct Cards {
    pub matches: Vec<Match>,
    verdicts: Vec<Verdict>,
}

impl Cards {
    pub fn new(matches: Vec<Match>) -> Cards {
        let verdicts = vec![Verdict::Undecided; matches.len()];
        Cards { matches, verdicts }
    }

    pub fn len(&self) -> usize {
        self.matches.len()
    }

    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    pub fn verdict(&self, i: usize) -> Verdict {
        self.verdicts.get(i).copied().unwrap_or(Verdict::Undecided)
    }

    pub fn set(&mut self, i: usize, verdict: Verdict) {
        if let Some(v) = self.verdicts.get_mut(i) {
            *v = verdict;
        }
    }

    /// The matches the user said yes to.
    pub fn accepted(&self) -> Vec<&Match> {
        self.matches.iter().zip(&self.verdicts).filter(|(_, v)| **v == Verdict::Yes).map(|(m, _)| m).collect()
    }

    pub fn accepted_count(&self) -> usize {
        self.verdicts.iter().filter(|v| **v == Verdict::Yes).count()
    }

    /// Their positions in `ImportScan::found`.
    pub fn accepted_found(&self) -> Vec<usize> {
        self.accepted().iter().map(|m| m.found).collect()
    }

    pub fn reset(&mut self) {
        self.verdicts.iter_mut().for_each(|v| *v = Verdict::Undecided);
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

fn details_button() -> Option<PageButton<CardAction>> {
    Some(PageButton { label: "Details".to_string(), action: CardAction::Details })
}

fn ports(n: usize) -> String {
    if n == 1 { "1 port".to_string() } else { format!("{n} ports") }
}

fn tile_of(m: &Match) -> Tile {
    Tile { art: m.art.clone(), label: m.port_name.clone() }
}

/// "Found 4 ports": the covers of everything the scan matched, then Check them / the leave choice
/// (`leave_label`: "Skip" in the wizard, "Not now" in Settings). With nothing found: where to go from here.
pub fn found_page(cards: &Cards, leave_label: &str) -> Page<CardAction> {
    if cards.is_empty() {
        let mut p = base("nothing found", "", "No ports found");
        p.paragraphs = vec![
            "Nothing that looks like a port turned up outside your ports folder.".to_string(),
            "If a port lives in a folder that was not looked in, add that folder and look again.".to_string(),
        ];
        p.options = vec![option("Look in another folder...", "Choose a folder to look in", CardAction::AddFolder, true), option(leave_label_for_empty(leave_label), "", CardAction::Leave, false)];
        p.buttons[LEFT] = details_button();
        p.initial = Focus::Option(0);
        return p;
    }
    let mut p = base("found", "", &format!("Found {}", ports(cards.len())));
    p.paragraphs = vec!["These look like ports you already have. Check each one, then they can be moved into your ports folder.".to_string()];
    p.tiles = cards.matches.iter().map(tile_of).collect();
    p.options = vec![option("Check them", "Go through them one at a time", CardAction::Start, true), option(leave_label, "", CardAction::Leave, false)];
    p.buttons[LEFT] = details_button();
    p.initial = Focus::Option(0);
    p
}

/// With nothing to check, leaving is the plain way out.
fn leave_label_for_empty(label: &str) -> &str {
    if label == "Skip" { "Next" } else { "Done" }
}

/// One port: "Is this the right port?" with its card and the three choices. A weak match is not highlighted
/// and the cursor starts on "Pick another port", so a stray A does not accept it.
pub fn card_page(cards: &Cards, index: usize) -> Page<CardAction> {
    let m = &cards.matches[index];
    let step = format!("Port {} of {}", index + 1, cards.len());
    let mut p = base(&step.to_lowercase(), &step, "Is this the right port?");
    p.card = Some(Card { art: m.art.clone(), title: m.port_name.clone(), lines: m.lines() });
    let strong = m.reason.strong();
    if !strong {
        p.paragraphs = vec!["This one is a guess. Check the picture before you say yes.".to_string()];
    }
    p.options = vec![
        option("Yes, this is it", "It is copied and checked before anything changes", CardAction::Yes, strong),
        option("Pick another port", "Choose the right port from the list", CardAction::PickAnother, false),
        option("Not a port", "Leave this folder alone and do not ask again", CardAction::NotAPort, false),
    ];
    p.buttons[LEFT] = details_button();
    p.initial = Focus::Option(if strong { 0 } else { 1 });
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
    lines.extend(scan.ambiguous.iter().map(|a| format!("Left out, it fits {} ports: {}", a.matches.len(), short_path(&a.path, home))));
    lines.extend(scan.duplicates.iter().map(|(name, path)| format!("Also found {name} in {}: left out.", short_path(path, home))));
    if !scan.already_in_library.is_empty() {
        lines.push(format!("Already in your ports folder: {}", scan.already_in_library.join(", ")));
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
    use std::fs;

    fn scan_of(h: &Home) -> ImportScan {
        let input = ScanInput {
            roots: ScanFolders::default().roots(&h.home, &h.library),
            catalog: catalog(),
            library_dir: h.library.clone(),
            home: h.home.clone(),
            deeper: false,
            shortcuts: read_shortcuts(&h.home, &ForkSettings::load(&h.home.join("s.json"))),
        };
        run_scan(&input, &|| false, &|_| {})
    }

    fn cards_of(h: &Home) -> (ImportScan, Cards) {
        let scan = scan_of(h);
        let cards = Cards::new(from_scan(&scan, &catalog(), &[]));
        (scan, cards)
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

    #[test]
    fn the_adapter_turns_each_find_into_a_match_with_its_words_cover_and_source() {
        let h = fake_home("cards_adapter");
        let (scan, cards) = cards_of(&h);
        assert_eq!(cards.len(), 2);
        let soh = cards.matches.iter().find(|m| m.port_name == "Ship of Harkinian").unwrap();
        assert_eq!(soh.reason, Reason::FolderNamed);
        assert_eq!(soh.repo.as_deref(), Some("a/soh"));
        assert_eq!(soh.art.as_ref().unwrap().folder, "SoH");
        assert_eq!(soh.steam_shortcuts, 1);
        assert_eq!(scan.found[soh.found].path, soh.path, "the position leads back to the full find");
        let dusk = cards.matches.iter().find(|m| m.port_name == "Dusklight").unwrap();
        assert_eq!(dusk.reason, Reason::ProgramInside, "dusk-old holds the program the catalog names");
        assert_eq!(dusk.steam_shortcuts, 0);
    }

    #[test]
    fn a_program_that_only_looks_like_the_port_is_the_weak_match() {
        let h = fake_home("cards_weak");
        put_exe(&h.home.join("Games/misc/severedchains.x86_64"), b"x");
        let (_, cards) = cards_of(&h);
        let weak = cards.matches.iter().find(|m| m.port_name == "Severed Chains").unwrap();
        assert_eq!(weak.reason, Reason::ProgramLooksLike);
        assert!(!weak.reason.strong());
        assert!(Reason::FolderNamed.strong() && Reason::FolderLooksLike.strong() && Reason::ProgramInside.strong());
    }

    #[test]
    fn folders_the_user_said_are_not_ports_are_left_out() {
        let h = fake_home("cards_ignored");
        let scan = scan_of(&h);
        let ignored = vec![h.home.join("Games/dusk-old")];
        let cards = from_scan(&scan, &catalog(), &ignored);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].port_name, "Ship of Harkinian");
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
        let p = found_page(&cards, "Not now");
        assert_eq!(p.title, "Found 2 ports");
        assert_eq!(p.window_title, "Import ports: found");
        assert_eq!(p.tiles.iter().map(|t| t.label.as_str()).collect::<Vec<_>>(), ["Dusklight", "Ship of Harkinian"]);
        assert_eq!(p.options.iter().map(|o| (o.title.as_str(), o.recommended)).collect::<Vec<_>>(), [("Check them", true), ("Not now", false)]);
        assert_eq!(p.initial, Focus::Option(0));
        assert_eq!(p.options.len(), 2, "no more than three choices");
        let one = Cards::new(cards.matches[..1].to_vec());
        assert_eq!(found_page(&one, "Skip").title, "Found 1 port");
        assert_eq!(found_page(&one, "Skip").options[1].title, "Skip");
    }

    #[test]
    fn nothing_found_says_so_and_offers_another_folder() {
        let p = found_page(&Cards::new(vec![]), "Skip");
        assert_eq!(p.title, "No ports found");
        assert_eq!(p.window_title, "Import ports: nothing found");
        assert!(p.tiles.is_empty());
        assert_eq!(p.options[0].action, CardAction::AddFolder);
        assert!(p.options[0].recommended);
        assert_eq!(p.options[1].title, "Next", "in the wizard leaving goes to the next step");
        assert_eq!(found_page(&Cards::new(vec![]), "Not now").options[1].title, "Done");
    }

    #[test]
    fn a_card_shows_the_cover_name_source_how_it_matched_size_and_steam() {
        let h = fake_home("cards_card");
        let (_, cards) = cards_of(&h);
        let soh = cards.matches.iter().position(|m| m.port_name == "Ship of Harkinian").unwrap();
        let p = card_page(&cards, soh);
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
        let weak = cards.matches.iter().position(|m| m.reason == Reason::ProgramLooksLike).unwrap();
        let p = card_page(&cards, weak);
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
            reason: Reason::FolderNamed,
            size: Size { bytes: 3_000_000_000, files: 5, complete: false },
            steam_shortcuts: 2,
            found: 0,
        };
        assert_eq!(m.lines(), ["The folder is named like this port.", "Size: at least 3.0 GB", "Has 2 Steam shortcuts"]);
        m.steam_shortcuts = 0;
        assert_eq!(m.lines().len(), 2, "no Steam line without a shortcut");
    }

    #[test]
    fn no_page_text_shows_a_file_name_or_a_path_outside_details() {
        let h = fake_home("cards_nopaths");
        let (_, mut cards) = cards_of(&h);
        let pages = [found_page(&cards, "Skip"), card_page(&cards, 0), card_page(&cards, 1), found_page(&Cards::new(vec![]), "Skip")];
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
        let soh = cards.matches.iter().find(|m| m.port_name == "Ship of Harkinian").unwrap();
        let d = soh.details(&h.home, &h.library);
        assert!(d.contains("Found in: ~/Applications/SoH"), "{d}");
        assert!(d.contains("Will be copied to: ~/DeckPorts/Library/SoH"), "{d}");
        assert!(d.contains("Steam shortcut keeps its place"), "{d}");
        assert!(d.contains("old folder stays"), "{d}");
        let dusk = cards.matches.iter().find(|m| m.port_name == "Dusklight").unwrap();
        assert!(!dusk.details(&h.home, &h.library).contains("Steam shortcut"));
    }

    #[test]
    fn every_card_page_has_details_in_the_footer_and_at_most_three_choices() {
        let h = fake_home("cards_footer");
        let (_, mut cards) = cards_of(&h);
        cards.set(0, Verdict::Yes);
        for p in [found_page(&cards, "Skip"), card_page(&cards, 0), confirm_page(&cards, None, &[]), found_page(&Cards::new(vec![]), "Skip")] {
            assert_eq!(p.buttons[LEFT].as_ref().unwrap().action, CardAction::Details, "{}", p.title);
            assert!(p.options.len() <= 3, "{}", p.title);
            assert!(p.buttons[crate::fork::page::RIGHT].is_none(), "the choices are the options, the footer only has Details");
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
    fn the_scan_notes_say_where_it_looked_and_what_it_left_out() {
        let h = fake_home("cards_notes");
        put_file(&h.home.join("Games/Fan Remake/x"), b"1");
        put_file(&h.home.join("Games/soh-copy/shipofharkinian.appimage"), b"1");
        fs::create_dir_all(h.library.join("severed-chains")).unwrap();
        put_file(&h.home.join("Games/severed-chains/x"), b"1");
        let text = scan_notes(&scan_of(&h), &h.home);
        assert!(text.starts_with("Looked in: ~/Applications, ~/Games"), "{text}");
        assert!(text.contains("Left out, it fits 2 ports: ~/Games/Fan Remake"), "{text}");
        assert!(text.contains("Also found Ship of Harkinian in ~/Games/soh-copy: left out."), "{text}");
        assert!(text.contains("Already in your ports folder: Severed Chains"), "{text}");
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
    fn every_card_page_is_drawn_large_and_has_no_progress_bar() {
        let h = fake_home("cards_large");
        let (_, mut cards) = cards_of(&h);
        cards.set(0, Verdict::Yes);
        for p in [found_page(&cards, "Skip"), card_page(&cards, 0), confirm_page(&cards, None, &[]), found_page(&Cards::new(vec![]), "Skip")] {
            assert!(p.large, "{}", p.title);
            assert_eq!(p.step_count, 0, "{}", p.title);
            assert_eq!(p.header, "Import ports");
        }
    }
}
