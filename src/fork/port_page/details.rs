//! "All details": one port's facts as plain summary lines, its instructions, and the extra links. It replaces upstream's
//! Info dialog (version in "tag" words, "Favorite executable", an Enable/Disable picker for updates, a dozen buttons that
//! the Port page's cards already do). Reached from More on the Port page, and on its own for a port that is not installed
//! (X on a row of All ports): the page then has no disk facts.
//!
//! Pure data and words; `src/app/port_page_screen.rs` gathers the facts and opens the links.

use super::paths::{human_path, Places};
use super::{card_page, file_name, option, Act, Card, DiskFacts, PortFacts, ProgramFacts};
use crate::fork::page::{Mark, Page};
use std::path::Path;

/// What the page shows that the Port page's own facts do not hold.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetailsExtra {
    /// The port is installed (its folder is there).
    pub installed: bool,
    /// The catalog's instructions for the port (may be empty).
    pub instructions: String,
    /// The first web address in the instructions, when there is one.
    pub instructions_link: Option<String>,
    /// The port's mods page.
    pub mods_link: Option<String>,
}

/// "Program it starts: dusk.sh", in the words the Port page's Program card uses.
pub fn program_line(f: &ProgramFacts) -> String {
    let named = |p: &str| file_name(Path::new(p));
    let ask = "asks you each time";
    let what = if let Some(fav) = &f.favorite {
        if f.favorite_found { named(fav) } else { format!("{ask} ({} is gone)", named(fav)) }
    } else if let Some(exe) = &f.catalog_exe {
        named(exe)
    } else {
        match crate::fork::program_detect::shallowest(&f.game_dir, f.candidates.clone()).as_slice() {
            [] => "none found".to_string(),
            [one] => file_name(one),
            _ => ask.to_string(),
        }
    };
    format!("Program it starts: {what}")
}

/// "Version: v2.1.0", or what is known when the version is not.
pub fn version_line(p: &PortFacts, installed: bool) -> String {
    match (&p.version, installed) {
        (Some(v), _) => format!("Version: {v}"),
        (None, false) => "Version: not installed".to_string(),
        (None, true) if p.tracks_releases => "Version: not known (updates are still checked)".to_string(),
        (None, true) => "Version: not known (this port does not say which one it is)".to_string(),
    }
}

/// "Updates: on", "Updates: off", or why there is no switch.
pub fn updates_line(p: &PortFacts) -> String {
    if !p.tracks_releases {
        return "Updates: not available (this port has no releases to follow)".to_string();
    }
    format!("Updates: {}", if p.update_on { "on" } else { "off" })
}

/// "Last played: today", "Last played: never".
pub fn last_played_line(p: &PortFacts) -> String {
    match &p.last_played {
        Some(when) => format!("Last played: {when}"),
        None => "Last played: never".to_string(),
    }
}

/// "Play time: 2.1 h", "Play time: none yet".
pub fn play_time_line(p: &PortFacts) -> String {
    format!("Play time: {}", p.playtime.as_deref().unwrap_or("none yet"))
}

/// The page: facts first, then the instructions, then the links. `disk` is the worker's reading of the port's folder (it
/// gives the program and the place); without it those two lines are left out.
pub fn details_page(p: &PortFacts, x: &DetailsExtra, disk: Option<&DiskFacts>, places: &Places) -> Page<Act> {
    let mut page = card_page(p, Card::More);
    page.title = "All details".to_string();
    page.window_title = format!("{}: All details", page.window_title);
    for paragraph in x.instructions.split("\n\n").map(str::trim).filter(|t| !t.is_empty()) {
        page.paragraphs.push(paragraph.to_string());
    }
    page.fact(Mark::None, version_line(p, x.installed));
    if x.installed {
        if let Some(d) = disk {
            page.fact(Mark::None, program_line(&d.program));
        }
    }
    page.fact(Mark::None, updates_line(p));
    page.fact(Mark::None, last_played_line(p));
    page.fact(Mark::None, play_time_line(p));
    if x.installed {
        if let Some(d) = disk {
            page.fact(Mark::None, format!("Folder: {}", human_path(&d.files.dir, places)));
        }
    }
    if x.instructions_link.is_some() {
        page.options.push(option("Open the instructions link", "Shows its address, with a code to scan with your phone", Act::InstructionsLink));
    }
    if p.project_url.is_some() {
        page.options.push(option("Open the project page", "Shows its address, with a code to scan with your phone", Act::ProjectPage));
    }
    if x.mods_link.is_some() {
        page.options.push(option("Open the mods page", "Shows its address, with a code to scan with your phone", Act::ModsLink));
    }
    page
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::page::LEFT;
    use crate::fork::port_page::{play_time_words, FilesFacts};
    use std::path::PathBuf;

    const NAMES: [&str; 3] = ["X", "The Wind Waker HD", "The Legend of Zelda: The Wind Waker HD (ZeldaWWHDRecomp)"];

    fn facts(name: &str) -> PortFacts {
        PortFacts {
            key: "dusk".into(),
            name: name.into(),
            version: Some("v1.4.1".into()),
            update_on: true,
            tracks_releases: true,
            last_played: Some("on 2026-10-01".into()),
            playtime: play_time_words(7560),
            project_url: Some("https://github.com/a/b".into()),
            ..PortFacts::default()
        }
    }

    fn disk() -> DiskFacts {
        let dir = PathBuf::from("/home/deck/Ports/dusk");
        DiskFacts {
            program: ProgramFacts { game_dir: dir.clone(), candidates: vec![dir.join("dusk.sh")], ..ProgramFacts::default() },
            files: FilesFacts { dir, ..FilesFacts::default() },
            ..DiskFacts::default()
        }
    }

    fn places() -> Places {
        Places { home: Some(PathBuf::from("/home/deck")), ..Places::default() }
    }

    fn extra() -> DetailsExtra {
        DetailsExtra {
            installed: true,
            instructions: "Required files: oot.z64\n\nSee the readme for the controls.".into(),
            instructions_link: Some("https://example.com/how-to".into()),
            mods_link: Some("https://example.com/mods".into()),
        }
    }

    #[test]
    fn the_play_time_is_one_wording() {
        assert_eq!(play_time_words(0), None);
        assert_eq!(play_time_words(20).as_deref(), Some("1 minute"));
        assert_eq!(play_time_words(59 * 60).as_deref(), Some("59 minutes"));
        assert_eq!(play_time_words(3600).as_deref(), Some("1 hour"));
        assert_eq!(play_time_words(7560).as_deref(), Some("2.1 hours"));
    }

    #[test]
    fn the_facts_are_plain_summary_lines() {
        let p = details_page(&facts("Dusklight"), &extra(), Some(&disk()), &places());
        assert_eq!(p.title, "All details");
        assert_eq!(
            p.facts,
            [
                "Version: v1.4.1",
                "Program it starts: dusk.sh",
                "Updates: on",
                "Last played: on 2026-10-01",
                "Play time: 2.1 hours",
                "Folder: Internal storage > Ports > dusk",
            ]
        );
        assert_eq!(p.paragraphs, ["Required files: oot.z64", "See the readme for the controls."]);
        assert_eq!(p.window_title, "Port: Dusklight: More: All details");
        assert_eq!(p.buttons[LEFT].as_ref().unwrap().action, Act::Back);
    }

    #[test]
    fn no_jargon_in_any_line() {
        let mut unknown = facts("Dusklight");
        unknown.version = None;
        let off = PortFacts { update_on: false, last_played: None, playtime: None, ..unknown.clone() };
        let none = PortFacts { tracks_releases: false, ..unknown.clone() };
        for (p, installed) in [(&facts("Dusklight"), true), (&unknown, true), (&off, true), (&none, true), (&none, false)] {
            let page = details_page(p, &DetailsExtra { installed, ..extra() }, Some(&disk()), &places());
            for line in page.facts.iter().chain(page.options.iter().map(|o| &o.title)).chain(page.options.iter().map(|o| &o.detail)) {
                for bad in ["tag", "Favorite", "favorite", "executable", "GitHub", "--", "unknown", "\u{2014}"] {
                    assert!(!line.contains(bad), "{bad:?} in {line:?}");
                }
            }
        }
    }

    #[test]
    fn an_unknown_version_says_so_in_words() {
        let mut p = facts("Dusklight");
        p.version = None;
        assert_eq!(version_line(&p, true), "Version: not known (updates are still checked)");
        p.tracks_releases = false;
        assert_eq!(version_line(&p, true), "Version: not known (this port does not say which one it is)");
        assert_eq!(version_line(&p, false), "Version: not installed");
        assert_eq!(updates_line(&p), "Updates: not available (this port has no releases to follow)");
        p.tracks_releases = true;
        p.update_on = false;
        assert_eq!(updates_line(&p), "Updates: off");
    }

    #[test]
    fn the_program_line_names_what_play_starts() {
        let dir = PathBuf::from("/home/deck/Ports/dusk");
        let mut f = ProgramFacts { game_dir: dir.clone(), candidates: vec![dir.join("a.sh"), dir.join("b.sh")], ..ProgramFacts::default() };
        assert_eq!(program_line(&f), "Program it starts: asks you each time");
        f.favorite = Some("b.sh".into());
        f.favorite_found = true;
        assert_eq!(program_line(&f), "Program it starts: b.sh");
        f.favorite_found = false;
        assert_eq!(program_line(&f), "Program it starts: asks you each time (b.sh is gone)");
        assert_eq!(program_line(&ProgramFacts { game_dir: dir, ..ProgramFacts::default() }), "Program it starts: none found");
    }

    #[test]
    fn a_port_that_is_not_installed_has_no_disk_lines_and_keeps_its_links() {
        let mut p = facts("Dusklight");
        p.version = None;
        let page = details_page(&p, &DetailsExtra { installed: false, ..extra() }, None, &places());
        assert_eq!(page.facts, ["Version: not installed", "Updates: on", "Last played: on 2026-10-01", "Play time: 2.1 hours"]);
        let acts: Vec<Act> = page.options.iter().map(|o| o.action).collect();
        assert_eq!(acts, [Act::InstructionsLink, Act::ProjectPage, Act::ModsLink]);
        let bare = details_page(&p, &DetailsExtra::default(), None, &places());
        assert!(bare.options.is_empty() || bare.options.iter().all(|o| o.action == Act::ProjectPage));
    }

    #[test]
    fn it_fits_with_the_longest_real_names() {
        for name in NAMES {
            let page = details_page(&facts(name), &extra(), Some(&disk()), &places());
            assert!(page.title.chars().count() <= 40);
            for o in &page.options {
                assert!(o.title.chars().count() <= 40 && o.detail.chars().count() <= 66, "{} / {}", o.title, o.detail);
            }
            assert_eq!(crate::fork::port_page::port_in_window_title(&page.window_title).as_deref(), Some(name), "{}", page.window_title);
        }
    }
}
