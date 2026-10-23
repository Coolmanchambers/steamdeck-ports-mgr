//! What the "Use a GitHub link..." pages say, without any UI code: the text box page's rows, the help text,
//! and the card that shows the project GitHub described, to confirm. The screens are in
//! `src/app/import_cards_screen.rs`; the checks are in `fork::custom_repo`.

use super::cards::{CardAction, HEADER};
use crate::fork::custom_repo::check::Checked;
use crate::fork::page::{Card, Focus, Page, PageOption};

pub const PLACEHOLDER: &str = "github.com/owner/project";
pub const WINDOW_TITLE: &str = "Import ports: GitHub link";

/// The rows under the text box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkRow {
    Check,
    /// Opens Steam's on-screen keyboard (Game Mode only: Slint does not raise it by itself).
    ShowKeyboard,
    Help,
    Cancel,
}

/// The rows, with the keyboard row only where it can help (Game Mode).
pub fn rows(game_mode: bool) -> Vec<(LinkRow, &'static str)> {
    let mut rows = vec![(LinkRow::Check, "Check this link")];
    if game_mode {
        rows.push((LinkRow::ShowKeyboard, "Show keyboard"));
    }
    rows.push((LinkRow::Help, "What should I look for?"));
    rows.push((LinkRow::Cancel, "Cancel"));
    rows
}

/// What to look for on GitHub, in the words of someone who has not been there.
pub const HELP: &str = "Find the project's GitHub page. Its address looks like github.com/owner/project.\n\n\
It should have a Releases section (on the right of the page) with a download for Linux: a file ending in .AppImage, .tar.gz or .zip.\n\n\
Paste that address here, or type it. We only need that link: we check that the newest release has a Linux file, and nothing is downloaded yet.\n\n\
In Game Mode, press Steam + X to bring up the keyboard.";

/// The text under the text box when Steam's keyboard could not be raised.
pub const KEYBOARD_HINT: &str = "If no keyboard appeared, press the Steam button and X together.";

/// Notes for the text box page: where the text goes and what is checked.
pub fn entry_title() -> &'static str {
    "Use a GitHub link"
}

/// The progress line while GitHub is asked.
pub fn checking_text(slug: &str) -> String {
    format!("Asking GitHub about {slug}...")
}

/// "Is this the project?": what GitHub said, to confirm. Nothing is downloaded or written by Yes: the project
/// is added to the port list just before the move.
pub fn confirm_card(checked: &Checked) -> Page<CardAction> {
    let mut lines = vec![format!("From: {}", checked.repo.slug())];
    if !checked.description.is_empty() {
        lines.push(checked.description.clone());
    }
    lines.push(match &checked.date {
        Some(date) => format!("Latest version: {} ({date})", checked.version),
        None => format!("Latest version: {}", checked.version),
    });
    lines.push(format!("Linux file: {}", checked.linux_file));
    let mut p = Page::<CardAction> {
        window_title: format!("{WINDOW_TITLE} found"),
        large: true,
        header: HEADER.to_string(),
        title: "Is this the project?".to_string(),
        ..Page::default()
    };
    p.card = Some(Card { art: None, title: checked.name.clone(), lines });
    p.options = vec![
        PageOption { title: "Yes, use this".to_string(), detail: "It is added to your port list when the move starts".to_string(), choice: None, enabled: true, recommended: true, action: CardAction::UseIt },
        PageOption { title: "Change the link".to_string(), detail: "Go back and type another".to_string(), choice: None, enabled: true, recommended: false, action: CardAction::ChangeLink },
    ];
    p.initial = Focus::Option(0);
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::custom_repo::parse::parse_link;

    fn checked(description: &str, date: Option<&str>) -> Checked {
        Checked {
            repo: parse_link("github.com/some-owner/Cool-Port").unwrap(),
            name: "Cool Port".into(),
            description: description.into(),
            version: "v1.2".into(),
            date: date.map(str::to_string),
            linux_file: "Cool-Linux.AppImage".into(),
        }
    }

    #[test]
    fn the_rows_offer_the_keyboard_only_in_game_mode() {
        let desktop: Vec<&str> = rows(false).iter().map(|r| r.1).collect();
        assert_eq!(desktop, ["Check this link", "What should I look for?", "Cancel"]);
        let game: Vec<(LinkRow, &str)> = rows(true);
        assert_eq!(game.iter().map(|r| r.1).collect::<Vec<_>>(), ["Check this link", "Show keyboard", "What should I look for?", "Cancel"]);
        assert_eq!(game[0].0, LinkRow::Check, "the first row is the one a stray A should do");
    }

    #[test]
    fn the_help_says_where_to_look_what_file_and_that_nothing_is_downloaded_yet() {
        for must in ["github.com/owner/project", "Releases", ".AppImage", ".tar.gz", ".zip", "nothing is downloaded", "Steam + X"] {
            assert!(HELP.contains(must), "{must}");
        }
        assert!(!HELP.to_lowercase().contains("json"));
        assert_eq!(PLACEHOLDER, "github.com/owner/project");
    }

    #[test]
    fn the_confirm_card_shows_name_source_description_version_date_and_the_linux_file() {
        let p = confirm_card(&checked("A cool port.", Some("2026-10-17")));
        assert_eq!(p.title, "Is this the project?");
        assert_eq!(p.window_title, "Import ports: GitHub link found");
        let card = p.card.as_ref().unwrap();
        assert_eq!(card.title, "Cool Port");
        assert_eq!(card.lines, ["From: some-owner/Cool-Port", "A cool port.", "Latest version: v1.2 (2026-10-17)", "Linux file: Cool-Linux.AppImage"]);
        assert_eq!(p.options.iter().map(|o| (o.title.as_str(), o.recommended)).collect::<Vec<_>>(), [("Yes, use this", true), ("Change the link", false)]);
        assert_eq!(p.initial, Focus::Option(0));
        assert!(p.large && p.step_count == 0);
    }

    #[test]
    fn without_a_description_or_date_those_lines_are_left_out() {
        let p = confirm_card(&checked("", None));
        assert_eq!(p.card.unwrap().lines, ["From: some-owner/Cool-Port", "Latest version: v1.2", "Linux file: Cool-Linux.AppImage"]);
    }

    #[test]
    fn the_checking_line_names_the_project() {
        assert_eq!(checking_text("a/b"), "Asking GitHub about a/b...");
    }
}
