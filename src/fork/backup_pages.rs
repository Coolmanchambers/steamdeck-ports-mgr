//! Back up saves (Settings > Saves, and Fix problems) as two pages: a waiting page while every port's saves are copied
//! into one backup, and a done page in plain sentences ("4 ports' saves were backed up. 2 had no saves yet. 1 could not
//! be backed up: the drive is full."). One name for the action, "Back up saves", and one for the place: the folder's
//! real name, "Saves Backup", told the human way. Replaces upstream's "Saves Backup" progress dialog and its "4 save
//! folder(s) backed up, 2 skipped (empty or missing), 1 failed (disk full or permission denied?)." message.
//!
//! Pure data and words; `src/app/save_backup_screen.rs` runs them.

use crate::core::save_backup::PortBackup;
use crate::fork::page::{Focus, Mark, Page, PageOption};
use crate::fork::rom_screens::util::ellipsize;

/// The action's one name (the Settings card, Fix problems, the pages).
pub const ACTION: &str = "Back up saves";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupAct {
    Ok,
}

fn base(window: &str, title: &str) -> Page<BackupAct> {
    Page { window_title: window.to_string(), large: true, header: ACTION.to_string(), title: title.to_string(), ..Page::default() }
}

/// While the saves are copied: the port it is on now, and a moving bar. It cannot be stopped part way (B does nothing).
pub fn waiting_page(place: &str, now: &str) -> Page<BackupAct> {
    let mut p = base("Back up saves: working", "Backing up your saves");
    p.paragraphs.push(format!("Every port's saves go into one backup in {place}. Nothing in your ports is changed."));
    p.facts.push(if now.trim().is_empty() { "Starting...".to_string() } else { format!("Looking at {}", ellipsize(now.trim(), 60)) });
    p.no_back = true;
    p.with_progress(None)
}

fn ports(n: usize) -> String {
    if n == 1 { "1 port".to_string() } else { format!("{n} ports") }
}

/// "Dusk, SoH and 3 more".
fn names(list: &[&str]) -> String {
    match list.len() {
        0 => String::new(),
        1 => list[0].to_string(),
        2 | 3 => format!("{} and {}", list[..list.len() - 1].join(", "), list[list.len() - 1]),
        n => format!("{} and {} more", list[..2].join(", "), n - 2),
    }
}

/// What happened: one sentence per kind of outcome, the failed ports named with their reason, and where the backup is.
/// `problem` is a reason nothing at all could be done.
pub fn done_page(outcomes: &[(String, PortBackup)], problem: Option<&str>, place: &str) -> Page<BackupAct> {
    let count = |want: fn(&PortBackup) -> bool| outcomes.iter().filter(|(_, o)| want(o)).count();
    let backed = count(|o| *o == PortBackup::BackedUp);
    let nothing = count(|o| *o == PortBackup::NothingYet);
    let failed: Vec<(&str, &str)> = outcomes.iter().filter_map(|(n, o)| if let PortBackup::Failed(why) = o { Some((n.as_str(), why.as_str())) } else { None }).collect();
    let no_place: Vec<&str> = outcomes.iter().filter(|(_, o)| *o == PortBackup::NoPlace).map(|(n, _)| n.as_str()).collect();
    let title = match (problem, backed, failed.len()) {
        (Some(_), _, _) => "Your saves were not backed up",
        (None, 0, 0) => "There were no saves to back up",
        (None, _, 0) if !no_place.is_empty() => "Most saves are backed up",
        (None, _, 0) => "Your saves are backed up",
        (None, 0, _) => "Your saves were not backed up",
        (None, _, _) => "Some saves could not be backed up",
    };
    let mut p = base("Back up saves: done", title);
    if let Some(why) = problem {
        p.paragraphs.push(format!("Nothing was backed up: {why}."));
    }
    if backed > 0 {
        p.paragraphs.push(format!("{} saves were backed up, into {place}.", if backed == 1 { "1 port's".to_string() } else { format!("{backed} ports'") }));
    }
    if nothing > 0 {
        p.paragraphs.push(format!("{} had no saves yet.", ports(nothing)));
    }
    match failed.as_slice() {
        [] => {}
        [(name, why)] => p.paragraphs.push(format!("1 could not be backed up ({name}): {why}.")),
        many => {
            p.paragraphs.push(format!("{} could not be backed up:", many.len()));
            for (name, why) in many.iter().take(5) {
                p.fact(Mark::Problem, format!("{name}: {why}"));
            }
            if many.len() > 5 {
                p.fact(Mark::Problem, format!("And {} more.", many.len() - 5));
            }
        }
    }
    if !no_place.is_empty() {
        let one = no_place.len() == 1;
        p.paragraphs.push(format!(
            "The app does not know where {} {} {} saves, so {} not backed up: {}.",
            ports(no_place.len()),
            if one { "keeps" } else { "keep" },
            if one { "its" } else { "their" },
            if one { "it was" } else { "they were" },
            names(&no_place)
        ));
    }
    if problem.is_none() && backed == 0 && nothing == 0 && failed.is_empty() && no_place.is_empty() {
        p.paragraphs.push("None of your ports has saves to back up yet.".to_string());
    }
    p.options = vec![PageOption { title: "OK".to_string(), detail: String::new(), choice: None, enabled: true, recommended: true, action: BackupAct::Ok }];
    p.initial = Focus::Option(0);
    p.b_does(&BackupAct::Ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::rom_screens::settings_pages::tests::{assert_fits, assert_plain, shown};

    const PLACE: &str = "Internal storage > Applications > Deck Ports Manager > Saves Backup";

    fn all(outcomes: &[(&str, PortBackup)]) -> Vec<(String, PortBackup)> {
        outcomes.iter().map(|(n, o)| (n.to_string(), o.clone())).collect()
    }

    #[test]
    fn the_done_page_counts_ports_in_plain_sentences() {
        let o = all(&[("A", PortBackup::BackedUp), ("B", PortBackup::BackedUp), ("C", PortBackup::BackedUp), ("D", PortBackup::BackedUp), ("E", PortBackup::NothingYet), ("F", PortBackup::NothingYet), ("Dusk", PortBackup::Failed("the drive is full".into()))]);
        let p = done_page(&o, None, PLACE);
        assert_eq!(p.title, "Some saves could not be backed up");
        assert_eq!(p.paragraphs, [format!("4 ports' saves were backed up, into {PLACE}."), "2 ports had no saves yet.".to_string(), "1 could not be backed up (Dusk): the drive is full.".to_string()]);
        assert_eq!(p.back_hint, "OK");
        let one = done_page(&all(&[("A", PortBackup::BackedUp)]), None, PLACE);
        assert_eq!((one.title.as_str(), one.paragraphs[0].as_str()), ("Your saves are backed up", &*format!("1 port's saves were backed up, into {PLACE}.")));
    }

    #[test]
    fn many_failures_are_named_and_ports_that_do_not_say_where_are_named() {
        let failed: Vec<(String, PortBackup)> = (0..7).map(|i| (format!("P{i}"), PortBackup::Failed("a file could not be read (no permission)".into()))).collect();
        let p = done_page(&failed, None, PLACE);
        assert_eq!(p.title, "Your saves were not backed up");
        assert_eq!(p.paragraphs, ["7 could not be backed up:"]);
        assert_eq!(p.facts.len(), 6);
        assert_eq!(p.facts[5], "And 2 more.");
        let q = done_page(&all(&[("Osu", PortBackup::NoPlace), ("X", PortBackup::NoPlace), ("Y", PortBackup::NoPlace), ("Z", PortBackup::NoPlace)]), None, PLACE);
        assert_eq!(q.title, "There were no saves to back up");
        assert_eq!(q.paragraphs, ["The app does not know where 4 ports keep their saves, so they were not backed up: Osu, X and 2 more."]);
        let most = done_page(&all(&[("A", PortBackup::BackedUp), ("Space Cadet", PortBackup::NoPlace)]), None, PLACE);
        assert_eq!(most.title, "Most saves are backed up");
        assert_eq!(most.paragraphs[1], "The app does not know where 1 port keeps its saves, so it was not backed up: Space Cadet.");
        let none = done_page(&[], None, PLACE);
        assert_eq!(none.paragraphs, ["None of your ports has saves to back up yet."]);
        let broken = done_page(&[], Some("the backup folder could not be made: the drive is full"), PLACE);
        assert_eq!(broken.paragraphs, ["Nothing was backed up: the backup folder could not be made: the drive is full."]);
    }

    #[test]
    fn the_pages_fit_and_say_it_plainly() {
        let long = "The Legend of Zelda: The Wind Waker HD (ZeldaWWHDRecomp)";
        let o = all(&[(long, PortBackup::Failed("a file could not be copied (Input/output error)".into())), ("B", PortBackup::NoPlace), ("C", PortBackup::BackedUp)]);
        for p in [waiting_page(PLACE, long), waiting_page(PLACE, ""), done_page(&o, None, PLACE), done_page(&[], Some("x"), PLACE)] {
            assert_fits(&p);
            assert_plain(&p, &["ZeldaWWHDRecomp"]);
            assert!(p.window_title.starts_with("Back up saves: "));
            for t in shown(&p) {
                let lower = t.to_lowercase();
                for word in ["zip", "/run/media", "/home/", "~/", "(s)", "json", "skipped", "save folder(", "saves backup/"] {
                    assert!(!lower.contains(word), "{word:?} in {t:?}");
                }
            }
        }
        let w = waiting_page(PLACE, "Dusk");
        assert!(w.no_back && w.options.is_empty());
        assert_eq!(w.facts, ["Looking at Dusk"]);
    }
}
