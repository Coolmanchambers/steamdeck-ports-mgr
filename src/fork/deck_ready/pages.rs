//! The Deck-ready list's Filters (Y) and Info (X) as pages: Filters as three cards with the value as the detail (Builds
//! changes in place, System opens a choice page, Community finds is a switch), and Info as a few plain facts with
//! Install / Play, the project page and the releases page as cards. No "README says", "Maintainer says" or "Found by"
//! labels, no file lists (the Windows files the rating ignores are not shown).
//!
//! Pure data and words; `src/app/deck_ready_screen.rs` runs them. The filters and what Install does are unchanged.

use super::index::{Entry, Host, Kind, Rating};
use super::view::{build_label, date_long, Filters, Standing};
use crate::fork::page::{Focus, Mark, Page, PageButton, PageOption, LEFT};
use crate::fork::rom_screens::util::ellipsize;

/// The Filters page's window title (the smoke test looks for it).
pub const FILTERS_WINDOW: &str = "Deck-ready: Filters";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrAct {
    /// Filters: change Builds (the next value, in place).
    Builds,
    /// Filters: the System choice page.
    System,
    /// Filters: Community finds on or off.
    Community,
    /// System choice: 0 is All systems, then the systems in order.
    PickSystem(usize),
    /// Info: Install, Play or Open (what the row's button does).
    Main,
    ProjectPage,
    ReleasesPage,
    Done,
    Back,
}

fn option(title: impl Into<String>, detail: impl Into<String>, action: DrAct) -> PageOption<DrAct> {
    PageOption { title: title.into(), detail: detail.into(), choice: None, enabled: true, recommended: false, action }
}

fn base(window: &str, title: &str) -> Page<DrAct> {
    Page { window_title: window.to_string(), large: true, header: "Deck-ready".to_string(), title: title.to_string(), ..Page::default() }
}

/// Y in the Deck-ready list: Builds, System and Community finds, each with its value as the detail.
pub fn filters_page(filters: &Filters) -> Page<DrAct> {
    let mut p = base(FILTERS_WINDOW, "Filters");
    p.paragraphs.push("Choose which ports the list shows. The list behind changes at once.".to_string());
    p.options = vec![
        option("Builds", build_label(filters.build), DrAct::Builds),
        option("System", filters.system.clone().unwrap_or_else(|| "All systems".to_string()), DrAct::System),
        option("Community finds", if filters.community { "Shown: some of these may be tools, not games" } else { "Hidden" }, DrAct::Community),
    ];
    p = p.toggle(2, filters.community);
    p.buttons[LEFT] = Some(PageButton { label: "Done".to_string(), action: DrAct::Done });
    p.initial = Focus::Option(0);
    p.b_does(&DrAct::Done)
}

/// System: All systems, then each system the list has; the one in use marked.
pub fn system_page(systems: &[String], current: Option<&str>) -> Page<DrAct> {
    let mut p = base("Deck-ready: System", "Which system?");
    p.options.push(PageOption { choice: Some(current.is_none()), ..option("All systems", "", DrAct::PickSystem(0)) });
    for (i, s) in systems.iter().enumerate() {
        p.options.push(PageOption { choice: Some(current == Some(s.as_str())), ..option(ellipsize(s, 40), "", DrAct::PickSystem(i + 1)) });
    }
    let at = current.and_then(|c| systems.iter().position(|s| s == c)).map_or(0, |i| i + 1);
    p.buttons[LEFT] = Some(PageButton { label: "Back".to_string(), action: DrAct::Back });
    p.initial = Focus::Option(at);
    p
}

/// What the Info page knows besides the entry: how the row stands, its button, and which pages it can show.
pub struct InfoFacts<'a> {
    pub name: &'a str,
    pub standing: Standing,
    /// "Install", "Play", or empty when the row can do nothing.
    pub button: &'a str,
    pub has_project: bool,
    pub has_releases: bool,
}

/// Things worth a warning, in plain words.
fn cautions(entry: &Entry, standing: Standing) -> Vec<String> {
    let mut out = Vec::new();
    if entry.missing_since.is_some() {
        out.push("It was not found the last time the list was made".to_string());
    }
    if entry.kind == Kind::LikelyGame {
        out.push("It may be a tool, not a game".to_string());
    }
    if entry.release.as_ref().is_some_and(|r| r.prerelease) {
        out.push("Only a test version is out so far".to_string());
    }
    if entry.archived {
        out.push("Its makers have stopped working on it".to_string());
    }
    if standing == Standing::New && entry.host == Host::Other {
        out.push("It can only be installed from the app's port list".to_string());
    }
    out
}

/// X in the Deck-ready list: a few plain facts, then Install / Play and the project and releases pages.
pub fn info_page(entry: &Entry, f: &InfoFacts) -> Page<DrAct> {
    let mut p = base("Deck-ready: Info", &ellipsize(f.name, 40));
    p.fact(
        Mark::None,
        match entry.rating {
            Rating::DeckBuild => "Has a download made for the Steam Deck",
            Rating::LinuxBuild => "Has a Linux download; not made for the Deck in particular",
        },
    );
    if let Some(r) = &entry.release {
        let date = date_long(&r.date).unwrap_or_else(|| "date unknown".to_string());
        let version = if r.tag.is_empty() { String::new() } else { format!("{}, ", ellipsize(&r.tag, 20)) };
        p.fact(Mark::None, format!("Newest version: {version}{date}"));
    }
    if let Some(system) = &entry.system {
        p.fact(Mark::None, format!("System: {}", ellipsize(system, 50)));
    }
    p.fact(
        Mark::None,
        match f.standing {
            Standing::Installed => "Installed",
            Standing::Yours => "In your port list (you added it)",
            Standing::InCatalog => "In the app's port list, not installed",
            Standing::New if f.button.is_empty() => "Not in the app's port list",
            Standing::New => "Install adds it to your port list",
        },
    );
    for c in cautions(entry, f.standing) {
        p.fact(Mark::Problem, c);
    }
    p.paragraphs.push("The rating comes from the names of its download files. Only starting it on your Deck proves it runs.".to_string());
    if !f.button.is_empty() {
        p.options.push(PageOption { recommended: true, ..option(f.button, if f.button == "Play" { "Start it now" } else { "Download it into your ports folder" }, DrAct::Main) });
    }
    if f.has_project {
        p.options.push(option("Project page", "Where it is made, and what it needs", DrAct::ProjectPage));
    }
    if f.has_releases {
        p.options.push(option("Releases page", "Every version and its download files", DrAct::ReleasesPage));
    }
    p.buttons[LEFT] = Some(PageButton { label: "Back".to_string(), action: DrAct::Back });
    p.initial = if p.options.is_empty() { Focus::Button(LEFT) } else { Focus::Option(0) };
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::deck_ready::index::parse;
    use crate::fork::deck_ready::index::tests::{file, project};
    use crate::fork::deck_ready::view::BuildFilter;
    use crate::fork::page::{action_at, initial_focus};
    use crate::fork::rom_screens::settings_pages::tests::{assert_fits, shown};

    const LONG: &str = "The Legend of Zelda: The Wind Waker HD (ZeldaWWHDRecomp)";

    fn entry() -> Entry {
        let mut e = parse(&file(vec![project("zelda", "deck_build", "game")])).unwrap().entries.remove(0);
        e.name = LONG.into();
        e.windows_assets = vec!["zelda-windows-x64.zip".into()];
        e.sources = vec!["catalog".into(), "topic:steamdeck".into()];
        e
    }

    fn facts(button: &str, standing: Standing) -> InfoFacts<'static> {
        InfoFacts { name: LONG, standing, button: if button == "Play" { "Play" } else if button.is_empty() { "" } else { "Install" }, has_project: true, has_releases: true }
    }

    fn plain(p: &Page<DrAct>) {
        assert_fits(p);
        for t in shown(p) {
            assert!(!t.contains('\u{2014}') && !t.contains("--"), "{t}");
            for word in ["README", "Maintainer", "Found by", "Windows", ".zip", "catalog", "Library", "json", "GitHub topic", "x86_64"] {
                assert!(!t.contains(word), "{word:?} in {t:?}");
            }
        }
    }

    #[test]
    fn filters_are_three_cards_with_their_value_as_the_detail() {
        let mut f = Filters { build: BuildFilter::All, system: None, community: true };
        let p = filters_page(&f);
        let cards: Vec<(&str, &str)> = p.options.iter().map(|o| (o.title.as_str(), o.detail.as_str())).collect();
        assert_eq!(cards, [("Builds", "All builds"), ("System", "All systems"), ("Community finds", "Shown: some of these may be tools, not games")]);
        assert!(cards.iter().all(|(_, d)| d.chars().count() <= 66));
        assert_eq!(p.toggle_of(2), Some(true));
        assert_eq!(p.window_title, FILTERS_WINDOW);
        assert_eq!(p.back_hint, "Done");
        f.community = false;
        f.system = Some("Nintendo 64".into());
        let q = filters_page(&f);
        assert_eq!((q.options[1].detail.as_str(), q.options[2].detail.as_str(), q.toggle_of(2)), ("Nintendo 64", "Hidden", Some(false)));
        plain(&q);
    }

    #[test]
    fn the_system_page_is_a_choice_with_the_one_in_use_marked() {
        let systems = vec!["N64".to_string(), "PS2".to_string()];
        let p = system_page(&systems, Some("PS2"));
        assert_eq!(p.options.iter().map(|o| o.choice).collect::<Vec<_>>(), [Some(false), Some(false), Some(true)]);
        assert_eq!(action_at(&p, initial_focus(&p)), Some(DrAct::PickSystem(2)));
        assert_eq!(system_page(&systems, None).options[0].choice, Some(true));
        plain(&p);
    }

    #[test]
    fn info_is_a_few_plain_facts_and_install_with_no_jargon_labels_and_no_file_lists() {
        let e = entry();
        let p = info_page(&e, &facts("Install", Standing::New));
        plain(&p);
        assert_eq!(p.facts[0], "Has a download made for the Steam Deck");
        assert!(p.facts.iter().any(|f| f == "Install adds it to your port list"), "{:?}", p.facts);
        assert_eq!(p.options.iter().map(|o| o.action).collect::<Vec<_>>(), [DrAct::Main, DrAct::ProjectPage, DrAct::ReleasesPage]);
        assert_eq!(action_at(&p, initial_focus(&p)), Some(DrAct::Main));
        let play = info_page(&e, &facts("Play", Standing::Installed));
        assert_eq!((play.options[0].title.as_str(), play.options[0].detail.as_str()), ("Play", "Start it now"));
        let mut odd = entry();
        odd.kind = Kind::LikelyGame;
        odd.archived = true;
        odd.rating = Rating::LinuxBuild;
        let none = info_page(&odd, &InfoFacts { has_project: false, has_releases: false, ..facts("", Standing::New) });
        plain(&none);
        assert!(none.options.is_empty() && none.initial == Focus::Button(LEFT));
        assert!(none.facts.iter().any(|f| f == "It may be a tool, not a game"));
        assert_eq!(none.mark_of(none.facts.len() - 1), Mark::Problem);
    }
}
