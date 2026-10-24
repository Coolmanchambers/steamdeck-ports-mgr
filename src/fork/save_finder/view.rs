//! The pages of the save finder, as plain data for `fork::page` (drawn by `app::page_dialog`). English only,
//! short, one question per page, no file paths except on the page that confirms a folder.

use super::search::{ago, Candidate};
use crate::fork::page::{Focus, Page, PageButton, PageOption, LEFT, RIGHT};
use std::time::SystemTime;

/// What the user can choose on any page of the finder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// "Look for them for me".
    Search,
    /// "Pick the folder...".
    Pick,
    /// "This game has no saves".
    NoSaves,
    /// Stop the search, or leave the finder.
    Cancel,
    /// B: one page back.
    Back,
    /// A candidate card (index into the results).
    Use(usize),
    /// "None of these": go to the folder picker.
    NoneOfThese,
    BackUpNow,
    NotNow,
    /// Take the choice just made back.
    Undo,
    /// Remove the override: the catalog's own answer applies again.
    UseCatalog,
}

fn window(name: &str) -> String {
    format!("Find saves: {name}")
}

fn base(name: &str, title: String) -> Page<Act> {
    Page { window_title: window(name), large: true, header: "Find saves".into(), title, initial: Focus::Option(0), ..Page::default() }
}

fn option(title: &str, detail: &str, recommended: bool, action: Act) -> PageOption<Act> {
    PageOption { title: title.into(), detail: detail.into(), choice: None, enabled: true, recommended, action }
}

fn button(label: &str, action: Act) -> Option<PageButton<Act>> {
    Some(PageButton { label: label.into(), action })
}

/// Page 1: where does the game keep its saves?
pub fn ask_page(name: &str) -> Page<Act> {
    let mut page = base(name, format!("Where does {name} keep its saves?"));
    page.paragraphs = vec!["The app does not know yet, so it cannot back them up. Nothing is moved or changed.".into()];
    page.options = vec![
        option("Look for them for me", "Checks the likely places. Only reads folders.", true, Act::Search),
        option("Pick the folder...", "Choose it yourself.", false, Act::Pick),
        option("This game has no saves", "The warning goes away and stays away.", false, Act::NoSaves),
    ];
    page.buttons[LEFT] = button("Cancel", Act::Cancel);
    page
}

/// What the finder is set to for a port: opened from Info when an answer is already stored.
pub fn current_page(name: &str, what: &str) -> Page<Act> {
    let mut page = base(name, format!("Saves for {name}"));
    page.facts = vec![what.to_string()];
    page.paragraphs = vec!["This is what you told the app. Nothing was moved, and the game's files are untouched.".into()];
    page.options = vec![
        option("Look for them again", "Checks the likely places. Only reads folders.", false, Act::Search),
        option("Pick another folder...", "Choose it yourself.", false, Act::Pick),
        option("Use what the catalog says", "Forgets your answer.", false, Act::UseCatalog),
    ];
    page.buttons[LEFT] = button("Close", Act::Cancel);
    page
}

/// Page 2 while the search runs.
pub fn searching_page(name: &str, progress: &str) -> Page<Act> {
    let mut page = base(name, format!("Looking for {name}'s saves..."));
    page.paragraphs = vec!["This only reads folders and takes a few seconds. Nothing is changed.".into()];
    if !progress.is_empty() {
        page.facts = vec![progress.to_string()];
    }
    page.buttons[LEFT] = button("Cancel", Act::Cancel);
    page.initial = Focus::Button(LEFT);
    page.primary = Some(LEFT);
    page
}

/// Page 2 when it is done: a card for each folder it found, newest first.
pub fn results_page(name: &str, found: &[Candidate], stopped_early: bool, now: SystemTime) -> Page<Act> {
    let mut page;
    if found.is_empty() {
        page = base(name, format!("I could not find {name}'s saves"));
        page.paragraphs = vec![if stopped_early {
            "The search ran out of time or was stopped. You can pick the folder yourself.".into()
        } else {
            "None of the likely places has a folder that looks like its saves. If the game has been played, you can pick the folder yourself.".to_string()
        }];
        page.options = vec![
            option("Pick the folder...", "Choose it yourself.", true, Act::Pick),
            option("This game has no saves", "The warning goes away and stays away.", false, Act::NoSaves),
        ];
    } else {
        page = base(name, format!("Is one of these {name}'s saves?"));
        page.paragraphs = vec!["Newest first. Nothing is moved; the app only remembers where to look.".into()];
        for (i, c) in found.iter().enumerate() {
            let files = if c.files == 1 { "1 file".to_string() } else { format!("{} files", c.files) };
            let detail = format!("{files}, changed {}, {}", ago(now, c.newest), c.place.label());
            page.options.push(option(&format!("Use {}", c.name), &detail, i == 0, Act::Use(i)));
        }
        page.options.push(option("None of these", "Pick the folder yourself.", false, Act::NoneOfThese));
        if stopped_early {
            page.footnote = "The search stopped early, so there may be more.".into();
        }
    }
    page.buttons[LEFT] = button("Back", Act::Back);
    page
}

/// Page 3: what was stored, and the offer to back up now.
pub fn saved_page(name: &str, what: &str, can_back_up: bool) -> Page<Act> {
    let mut page = base(name, "Saved".to_string());
    page.facts = vec![what.to_string()];
    page.paragraphs = vec!["Nothing was moved. Backups, restores and updates of this game now use that.".into()];
    if can_back_up {
        page.options = vec![option("Back up now", "Makes a backup of these saves, to restore from later.", true, Act::BackUpNow), option("Not now", "Done.", false, Act::NotNow)];
    } else {
        page.options = vec![option("Done", "", true, Act::NotNow)];
    }
    page.buttons[LEFT] = button("Undo", Act::Undo);
    page.buttons[RIGHT] = None;
    page
}

pub fn backing_up_page(name: &str) -> Page<Act> {
    let mut page = base(name, "Backing up...".to_string());
    page.paragraphs = vec!["Copying the saves into a zip in Saves Backup. Please wait.".into()];
    page.initial = Focus::Button(LEFT);
    page
}

/// A short message with one way out.
pub fn message_page(name: &str, title: &str, paragraphs: Vec<String>, label: &str) -> Page<Act> {
    let mut page = base(name, title.to_string());
    page.paragraphs = paragraphs;
    page.buttons[RIGHT] = button(label, Act::Back);
    page.primary = Some(RIGHT);
    page.initial = Focus::Button(RIGHT);
    page
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::page::{action_at, focusables};
    use crate::fork::save_finder::search::Place;
    use std::path::PathBuf;
    use std::time::Duration;

    fn cand(name: &str, files: usize, days: u64, place: Place) -> Candidate {
        Candidate { path: PathBuf::from("/x").join(name), name: name.into(), files, newest: SystemTime::UNIX_EPOCH + Duration::from_secs(10_000_000 - days * 86_400), place }
    }

    #[test]
    fn first_page_has_three_choices_and_the_search_is_recommended() {
        let page = ask_page("Severed Chains");
        assert_eq!(page.title, "Where does Severed Chains keep its saves?");
        let acts: Vec<Act> = page.options.iter().map(|o| o.action.clone()).collect();
        assert_eq!(acts, vec![Act::Search, Act::Pick, Act::NoSaves]);
        assert!(page.options[0].recommended && !page.options[1].recommended);
        assert!(page.options.iter().all(|o| !o.title.contains('/')), "no paths");
        assert!(page.window_title.starts_with("Find saves: "));
    }

    #[test]
    fn results_show_folder_name_file_count_and_age() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000_000);
        let page = results_page("Dusklight", &[cand("Dusklight", 12, 2, Place::Proton), cand("dusklight", 1, 40, Place::LocalShare)], false, now);
        assert_eq!(page.options[0].title, "Use Dusklight");
        assert_eq!(page.options[0].detail, "12 files, changed 2 days ago, in Steam's Windows files");
        assert_eq!(page.options[1].detail, "1 file, changed 40 days ago, in your Linux app data");
        assert_eq!(page.options[2].action, Act::NoneOfThese);
        assert_eq!(action_at(&page, 0), Some(Act::Use(0)));
        assert!(focusables(&page).len() >= 4);
    }

    #[test]
    fn no_results_offers_the_picker_first() {
        let page = results_page("Dusklight", &[], false, SystemTime::UNIX_EPOCH);
        assert_eq!(page.options[0].action, Act::Pick);
        assert_eq!(page.options[1].action, Act::NoSaves);
    }

    #[test]
    fn saved_page_offers_backup_and_undo() {
        let page = saved_page("X", "Using the Save folder.", true);
        assert_eq!(page.options[0].action, Act::BackUpNow);
        assert_eq!(page.buttons[LEFT].as_ref().map(|b| b.action.clone()), Some(Act::Undo));
        assert_eq!(saved_page("X", "y", false).options.len(), 1);
    }
}
