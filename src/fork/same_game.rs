//! Same game, another project. Several catalog entries are one game made by different projects (Majora's Mask by
//! 2 Ship 2 Harkinian and by Zelda64Recomp, Star Fox 64 by Starship and by Starfox64Recomp). Installing a second one
//! adds a second copy with its own saves and settings, which is rarely what someone means; the lists say so and the
//! install asks first. A remake on another system (Majora's Mask 3D, The Wind Waker HD on Wii U) is only mentioned.
//!
//! The rule is deliberately plain, so it never claims a match it cannot stand behind:
//! - A port's game is its title with the project part removed: the trailing "(folder)" note, and the words Recomp,
//!   Decomp and "PC port" when the title has them. Then letters and digits only, lower case ([`Game::game_key`]).
//! - The same game on the same system (or a system one side does not name) is the SAME GAME.
//! - The same game on two named systems, or the same title apart from a trailing "3D" or "HD", is ANOTHER VERSION.
//! - Anything else is no match: no fuzzy or one-word matching ("Star Fox" and "Star Fox 64" are different games).
//!
//! No UI here; `src/app/same_game_screen.rs` asks the question and the list screens draw the badges.

use crate::fork::art::thumbs::ArtRef;
use crate::fork::page::{Card, Focus, Page, PageOption};
use crate::fork::rom_library::system::{system_from_folder_name, System};

/// One game as the matcher sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    /// The title with the "(folder)" note removed, as the lists show it.
    pub title: String,
    /// The title without the project part, letters and digits only. Empty: matches nothing.
    pub game_key: String,
    /// [`Game::game_key`] without a trailing "3D" or "HD" edition word.
    pub family_key: String,
    pub system: Option<System>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Same game, same system: a second copy of what is installed.
    SameGame,
    /// The same game on another system, or a 3D or HD edition of it: shown, never asked about.
    AnotherVersion,
}

impl Verdict {
    /// The word the log uses.
    pub fn word(self) -> &'static str {
        match self {
            Verdict::SameGame => "same-game",
            Verdict::AnotherVersion => "another-version",
        }
    }
}

/// "Title (note)" split into the title and the note, the way `Port::display_name` drops it: only one trailing note
/// with no brackets inside it.
pub fn split_annotation(name: &str) -> (&str, Option<&str>) {
    let trimmed = name.trim();
    if let Some(open) = trimmed.strip_suffix(')').and_then(|n| n.rfind('(')) {
        let inner = &trimmed[open + 1..trimmed.len() - 1];
        if !inner.contains(['(', ')']) {
            let inner = inner.trim();
            return (trimmed[..open].trim_end(), (!inner.is_empty()).then_some(inner));
        }
    }
    (trimmed, None)
}

/// Lowercase letters and digits only (as `fork::adopt::norm`).
fn norm(s: &str) -> String {
    crate::fork::adopt::norm(s)
}

/// The title's words without the ones that name the kind of project (Recomp, Decomp, "PC port").
fn game_words(title: &str) -> Vec<&str> {
    let words: Vec<&str> = title.split_whitespace().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let w = norm(words[i]);
        if w == "recomp" || w == "decomp" {
            i += 1;
            continue;
        }
        if w == "pc" && words.get(i + 1).is_some_and(|n| norm(n) == "port") {
            i += 2;
            continue;
        }
        out.push(words[i]);
        i += 1;
    }
    out
}

/// A catalog tag that names a system (`n64`, `3ds`, `gc`, `wiiu`; the catalog writes Xbox 360 as `360`).
pub fn system_from_tags(tags_lower: &[String]) -> Option<System> {
    tags_lower.iter().find_map(|t| if t == "360" { Some(System::Xbox360) } else { system_from_folder_name(t) })
}

/// The Deck-ready list's system label ("N64", "3DS", "Wii U", "Xbox 360") as the same [`System`].
pub fn system_from_label(label: &str) -> Option<System> {
    system_from_folder_name(label)
}

impl Game {
    pub fn new(name: &str, system: Option<System>) -> Game {
        let (title, _) = split_annotation(name);
        let words = game_words(title);
        let game_key = norm(&words.join(" "));
        let mut family = words.clone();
        if family.len() > 1 && family.last().is_some_and(|w| matches!(norm(w).as_str(), "3d" | "hd")) {
            family.pop();
        }
        let family_key = norm(&family.join(" "));
        Game { title: title.to_string(), game_key, family_key, system }
    }

    /// A catalog port: its name and its tags.
    pub fn of_port(name: &str, tags_lower: &[String]) -> Game {
        Game::new(name, system_from_tags(tags_lower))
    }

    /// A Deck-ready entry: its name and its system label.
    pub fn of_entry(name: &str, system: Option<&str>) -> Game {
        Game::new(name, system.and_then(system_from_label))
    }
}

/// How `b` relates to `a`, or `None` when they are different games.
pub fn verdict(a: &Game, b: &Game) -> Option<Verdict> {
    if a.game_key.is_empty() || b.game_key.is_empty() {
        return None;
    }
    if a.game_key == b.game_key {
        return Some(match (a.system, b.system) {
            (Some(x), Some(y)) if x != y => Verdict::AnotherVersion,
            _ => Verdict::SameGame,
        });
    }
    (!a.family_key.is_empty() && a.family_key == b.family_key).then_some(Verdict::AnotherVersion)
}

/// An installed port, as the lists compare others with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owned {
    /// Its key (`Port::key`), so a port is never compared with itself.
    pub key: String,
    /// What it is called on screen: the catalog's "(folder)" note ("2ship2harkinian", "Shipwright"), else its title.
    pub project: String,
    pub game: Game,
    /// Its cover, for the question page.
    pub art: Option<ArtRef>,
}

impl Owned {
    pub fn new(key: &str, name: &str, tags_lower: &[String]) -> Owned {
        let game = Game::of_port(name, tags_lower);
        let project = split_annotation(name).1.map_or_else(|| game.title.clone(), str::to_string);
        Owned { key: key.to_string(), project, game, art: None }
    }
}

/// An installed port that a candidate is the same game as (or another version of).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Found<'a> {
    pub verdict: Verdict,
    pub owned: &'a Owned,
}

/// What the installed ports say about a candidate with key `key`: the same game first, else another version, in
/// the order given. The candidate itself (same key) is never reported.
pub fn find<'a>(owned: &'a [Owned], key: &str, game: &Game) -> Option<Found<'a>> {
    let mut softer = None;
    for o in owned.iter().filter(|o| o.key != key) {
        match verdict(&o.game, game) {
            Some(Verdict::SameGame) => return Some(Found { verdict: Verdict::SameGame, owned: o }),
            Some(Verdict::AnotherVersion) if softer.is_none() => softer = Some(Found { verdict: Verdict::AnotherVersion, owned: o }),
            _ => {}
        }
    }
    softer
}

pub const SAME_GAME_BADGE: &str = "You have this game";
pub const ANOTHER_VERSION_BADGE: &str = "Other version installed";

/// A list row's badge and the line that names what is installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Badge {
    pub verdict: Verdict,
    pub label: &'static str,
    pub note: String,
}

pub fn badge(found: &Found) -> Badge {
    match found.verdict {
        Verdict::SameGame => Badge { verdict: Verdict::SameGame, label: SAME_GAME_BADGE, note: format!("You have it as {}", found.owned.project) },
        Verdict::AnotherVersion => Badge { verdict: Verdict::AnotherVersion, label: ANOTHER_VERSION_BADGE, note: format!("{ANOTHER_VERSION_BADGE}: {}", found.owned.project) },
    }
}

/// The line the log gets for a match (the smoke test can look for it).
pub fn log_line(verdict: Verdict, candidate_key: &str, owned_key: &str) -> String {
    format!("same game: {} {candidate_key} vs {owned_key}", verdict.word())
}

pub const WINDOW_TITLE: &str = "Deck Ports Manager: Same game";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameAct {
    NotNow,
    InstallAnyway,
}

/// The question before installing a port that is the same game as one installed. `candidate` is the name of the port
/// about to be installed, as the list shows it.
pub fn confirm_page(candidate: &str, owned: &Owned) -> Page<SameAct> {
    let mut page = Page::<SameAct> { window_title: WINDOW_TITLE.to_string(), large: true, header: candidate.to_string(), title: "You already have this game".to_string(), initial: Focus::Option(0), ..Page::default() };
    page.card = Some(Card { art: owned.art.clone(), qr: None, title: owned.game.title.clone(), lines: vec![format!("Installed as {}", owned.project)] });
    page.paragraphs.push(format!(
        "You already have {} from {}. This is a different project for the same game; installing it adds a second copy with its own saves and settings.",
        owned.game.title, owned.project
    ));
    page.options.push(PageOption { title: "Not now".to_string(), detail: "Nothing is downloaded".to_string(), choice: None, enabled: true, recommended: true, action: SameAct::NotNow });
    page.options.push(PageOption { title: "Install it as well".to_string(), detail: format!("{candidate} is installed beside it"), choice: None, enabled: true, recommended: false, action: SameAct::InstallAnyway });
    page.b_does(&SameAct::NotNow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::Port;

    fn catalog() -> Vec<Port> {
        crate::core::config::parse_catalog(include_str!("../../ports.json")).unwrap()
    }

    fn port<'a>(all: &'a [Port], name: &str) -> &'a Port {
        all.iter().find(|p| p.name == name).unwrap_or_else(|| panic!("{name} is not in ports.json"))
    }

    fn game(all: &[Port], name: &str) -> Game {
        let p = port(all, name);
        Game::of_port(&p.name, &p.tags_lower)
    }

    fn owned(all: &[Port], name: &str) -> Owned {
        let p = port(all, name);
        Owned::new(p.key(), &p.name, &p.tags_lower)
    }

    fn v(all: &[Port], a: &str, b: &str) -> Option<Verdict> {
        verdict(&game(all, a), &game(all, b))
    }

    const MM_2SHIP: &str = "The Legend of Zelda: Majora's Mask (2ship2harkinian)";
    const MM_RECOMP: &str = "The Legend of Zelda: Majora's Mask (Zelda64Recomp)";
    const MM_3D: &str = "The Legend of Zelda: Majora's Mask 3D (MM3D-PC)";

    #[test]
    fn majoras_mask_by_two_projects_is_the_same_game_and_3d_is_another_version() {
        let all = catalog();
        assert_eq!(v(&all, MM_2SHIP, MM_RECOMP), Some(Verdict::SameGame));
        assert_eq!(v(&all, MM_RECOMP, MM_2SHIP), Some(Verdict::SameGame));
        assert_eq!(v(&all, MM_2SHIP, MM_3D), Some(Verdict::AnotherVersion));
        assert_eq!(game(&all, MM_3D).system, Some(System::N3ds));
        assert_eq!(game(&all, MM_2SHIP).system, Some(System::N64));
    }

    #[test]
    fn ocarina_of_time_3d_is_another_version() {
        let all = catalog();
        assert_eq!(v(&all, "The Legend of Zelda: Ocarina of Time (Shipwright)", "The Legend of Zelda: Ocarina of Time 3D (TriAevum)"), Some(Verdict::AnotherVersion));
        assert_eq!(v(&all, "The Legend of Zelda: Ocarina of Time 3D (Ocarina-ReTuned)", "The Legend of Zelda: Ocarina of Time 3D (TriAevum)"), Some(Verdict::SameGame));
    }

    #[test]
    fn the_wind_waker_hd_on_wii_u_and_on_gamecube_are_other_versions() {
        let all = catalog();
        // The GameCube entry is Android-only and has no folder, so it is not in the parsed catalog; its tags are copied.
        let gc = Game::of_port("The Legend of Zelda: The Wind Waker HD (WindWakerHD-Android-Plus)", &["gc".into(), "action-adventure".into(), "android_only".into()]);
        let wiiu = game(&all, "The Legend of Zelda: The Wind Waker HD (ZeldaWWHDRecomp)");
        assert_eq!(wiiu.system, Some(System::WiiU));
        assert_eq!(verdict(&wiiu, &gc), Some(Verdict::AnotherVersion));
        let plain = Game::new("The Legend of Zelda: The Wind Waker (some-port)", Some(System::GameCube));
        assert_eq!(verdict(&plain, &wiiu), Some(Verdict::AnotherVersion), "HD at the end is an edition of the same title");
    }

    #[test]
    fn star_fox_64_by_two_projects_is_the_same_game_and_64_is_never_dropped() {
        let all = catalog();
        assert_eq!(v(&all, "Star Fox 64 (Starfox64Recomp)", "Star Fox 64 (starship)"), Some(Verdict::SameGame));
        assert_eq!(v(&all, "Star Fox 64 (starship)", "Star Fox (starfox-enhanced)"), None);
        assert_eq!(game(&all, "Star Fox 64 (starship)").family_key, "starfox64");
    }

    #[test]
    fn goldeneye_on_n64_twice_is_the_same_game_and_the_xbox_360_one_another_version() {
        let all = catalog();
        let (pc, midas, xbla) = ("GoldenEye 007 (goldeneye-pc-port)", "GoldenEye 007 (Project-Midas)", "GoldenEye 007 (GoldenEye-Recomp)");
        assert_eq!(v(&all, pc, midas), Some(Verdict::SameGame));
        // The catalog tags the third one 360 (the Xbox Live Arcade remaster): the rule's "two named systems".
        assert_eq!(game(&all, xbla).system, Some(System::Xbox360));
        assert_eq!(v(&all, pc, xbla), Some(Verdict::AnotherVersion));
        assert_eq!(v(&all, midas, xbla), Some(Verdict::AnotherVersion));
    }

    #[test]
    fn perfect_dark_by_two_projects_is_the_same_game() {
        let all = catalog();
        assert_eq!(v(&all, "Perfect Dark (perfect-dark-dabs-mod)", "Perfect Dark (perfect_dark)"), Some(Verdict::SameGame));
    }

    #[test]
    fn banjo_games_that_differ_are_not_matched() {
        let all = catalog();
        assert_eq!(v(&all, "Banjo-Kazooie (BanjoRecomp)", "Banjo-Kazooie (Lighthouse)"), Some(Verdict::SameGame));
        assert_eq!(v(&all, "Banjo-Kazooie (BanjoRecomp)", "Banjo-Kazooie: Nuts & Bolts (reNut)"), None);
        assert_eq!(verdict(&Game::new("Banjo-Kazooie (a)", Some(System::N64)), &Game::new("Banjo-Tooie (b)", Some(System::N64))), None);
    }

    #[test]
    fn two_titles_sharing_a_word_are_not_matched() {
        let all = catalog();
        assert_eq!(v(&all, "Star Fox 64 (starship)", "Star Fox Adventures (foxhollow)"), None);
        assert_eq!(v(&all, "Mario Kart 64 (MarioKart64Recomp)", "Mario Kart DS (DS-Kart-Recompiled)"), None);
        assert_eq!(v(&all, "Mega Man X (MegaManXSNESRecomp)", "Mega Man X2 (MegaManX2Recomp)"), None);
        assert_eq!(verdict(&Game::new("Zelda (a)", None), &Game::new("Zelda II (b)", None)), None);
    }

    #[test]
    fn a_port_is_never_reported_against_itself() {
        let all = catalog();
        let mine = vec![owned(&all, MM_2SHIP)];
        let me = port(&all, MM_2SHIP);
        assert_eq!(find(&mine, me.key(), &game(&all, MM_2SHIP)), None);
        let other = port(&all, MM_RECOMP);
        let found = find(&mine, other.key(), &game(&all, MM_RECOMP)).unwrap();
        assert_eq!((found.verdict, found.owned.project.as_str()), (Verdict::SameGame, "2ship2harkinian"));
    }

    #[test]
    fn the_same_game_wins_over_another_version_whatever_the_order() {
        let all = catalog();
        let mine = vec![owned(&all, MM_3D), owned(&all, MM_2SHIP)];
        let found = find(&mine, "x", &game(&all, MM_RECOMP)).unwrap();
        assert_eq!((found.verdict, found.owned.project.as_str()), (Verdict::SameGame, "2ship2harkinian"));
        let only_3d = vec![owned(&all, MM_3D)];
        assert_eq!(find(&only_3d, "x", &game(&all, MM_RECOMP)).unwrap().verdict, Verdict::AnotherVersion);
    }

    #[test]
    fn project_words_are_not_part_of_the_game() {
        let a = Game::new("Some Game Recomp (one)", Some(System::N64));
        let b = Game::new("Some Game PC Port (two)", Some(System::N64));
        let c = Game::new("Some Game Decomp", None);
        assert_eq!(a.game_key, "somegame");
        assert_eq!((verdict(&a, &b), verdict(&a, &c)), (Some(Verdict::SameGame), Some(Verdict::SameGame)));
        assert_eq!(Game::new("Recomp", None).game_key, "", "nothing left: matches nothing");
        assert_eq!(verdict(&Game::new("Recomp", None), &Game::new("Decomp", None)), None);
        assert_eq!(Game::new("HD", None).family_key, "hd", "a lone edition word stays");
    }

    #[test]
    fn an_unknown_system_on_one_side_still_counts_as_the_same_game() {
        let all = catalog();
        let entry = Game::of_entry("Star Fox 64", None);
        assert_eq!(verdict(&game(&all, "Star Fox 64 (starship)"), &entry), Some(Verdict::SameGame));
    }

    #[test]
    fn deck_ready_system_labels_match_the_catalog_tags() {
        let all = catalog();
        for (label, want) in [("N64", System::N64), ("3DS", System::N3ds), ("Wii U", System::WiiU), ("GameCube", System::GameCube), ("Xbox 360", System::Xbox360), ("NDS", System::Nds), ("PS1", System::Ps1)] {
            assert_eq!(system_from_label(label), Some(want), "{label}");
        }
        assert_eq!(system_from_label("PC"), None);
        let entry = Game::of_entry(MM_RECOMP, Some("N64"));
        assert_eq!(entry.system, game(&all, MM_2SHIP).system);
        assert_eq!(verdict(&game(&all, MM_2SHIP), &entry), Some(Verdict::SameGame));
        assert_eq!(verdict(&game(&all, MM_2SHIP), &Game::of_entry(MM_3D, Some("3DS"))), Some(Verdict::AnotherVersion));
    }

    #[test]
    fn the_published_deck_ready_list_agrees_with_the_catalog_on_majoras_mask() {
        let all = catalog();
        let index = crate::fork::deck_ready::index::parse(include_str!("../../index/deck-ready.json")).unwrap();
        let mine = vec![owned(&all, MM_2SHIP)];
        let by_name = |n: &str| index.entries.iter().find(|e| e.name == n).unwrap_or_else(|| panic!("{n} is not in deck-ready.json"));
        let recomp = by_name(MM_RECOMP);
        assert_eq!(find(&mine, "", &Game::of_entry(&recomp.name, recomp.system.as_deref())).map(|f| f.verdict), Some(Verdict::SameGame));
        let three_d = by_name(MM_3D);
        assert_eq!(find(&mine, "", &Game::of_entry(&three_d.name, three_d.system.as_deref())).map(|f| f.verdict), Some(Verdict::AnotherVersion));
    }

    #[test]
    fn the_installed_project_is_the_catalog_note_or_the_title() {
        assert_eq!(Owned::new("k", "Star Fox 64 (starship)", &[]).project, "starship");
        assert_eq!(Owned::new("k", "Some Game", &[]).project, "Some Game");
        assert_eq!(split_annotation("Ace Combat 3 (Japan) (ace-combat-3)"), ("Ace Combat 3 (Japan)", Some("ace-combat-3")));
        assert_eq!(split_annotation("Game (a (b))"), ("Game (a (b))", None));
    }

    #[test]
    fn badges_say_what_is_installed() {
        let all = catalog();
        let mine = vec![owned(&all, MM_2SHIP)];
        let same = badge(&find(&mine, "x", &game(&all, MM_RECOMP)).unwrap());
        assert_eq!((same.label, same.note.as_str()), ("You have this game", "You have it as 2ship2harkinian"));
        let other = badge(&find(&mine, "x", &game(&all, MM_3D)).unwrap());
        assert_eq!((other.label, other.note.as_str()), ("Other version installed", "Other version installed: 2ship2harkinian"));
        assert_eq!(log_line(Verdict::SameGame, "Zelda64Recomp/Zelda64Recomp", "2ship2harkinian/2ship2harkinian"), "same game: same-game Zelda64Recomp/Zelda64Recomp vs 2ship2harkinian/2ship2harkinian");
    }

    #[test]
    fn the_question_starts_on_not_now_and_b_means_not_now() {
        let all = catalog();
        let mut mine = owned(&all, MM_2SHIP);
        mine.art = Some(ArtRef { folder: "2ship2harkinian".into(), url: None });
        let page = confirm_page("The Legend of Zelda: Majora's Mask", &mine);
        assert_eq!(page.window_title, "Deck Ports Manager: Same game");
        assert_eq!(page.title, "You already have this game");
        assert_eq!(page.header, "The Legend of Zelda: Majora's Mask");
        let titles: Vec<&str> = page.options.iter().map(|o| o.title.as_str()).collect();
        assert_eq!(titles, ["Not now", "Install it as well"]);
        assert!(page.options[0].recommended && !page.options[1].recommended);
        assert_eq!(page.options.iter().map(|o| o.action).collect::<Vec<_>>(), [SameAct::NotNow, SameAct::InstallAnyway]);
        assert_eq!(crate::fork::page::action_at(&page, crate::fork::page::initial_focus(&page)), Some(SameAct::NotNow));
        assert_eq!(page.back_hint, "Not now");
        assert_eq!(
            page.paragraphs[0],
            "You already have The Legend of Zelda: Majora's Mask from 2ship2harkinian. This is a different project for the same game; installing it adds a second copy with its own saves and settings."
        );
        let card = page.card.as_ref().unwrap();
        assert_eq!((card.title.as_str(), card.art.as_ref().map(|a| a.folder.as_str())), ("The Legend of Zelda: Majora's Mask", Some("2ship2harkinian")));
    }
}
