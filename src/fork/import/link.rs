//! What the "Use a GitHub link..." pages say, without any UI code: the text box page's rows, the help text,
//! and the card that shows the project GitHub described, to confirm. The screens are in
//! `src/app/import_cards_screen.rs`; the checks are in `fork::custom_repo`.

use super::cards::{CardAction, HEADER};
use crate::fork::custom_repo::check::Checked;
use crate::fork::page::{Card, Focus, InputBox, Page, PageButton, PageOption, LEFT};

pub const PLACEHOLDER: &str = "github.com/owner/project";
pub const WINDOW_TITLE: &str = "Import ports: GitHub link";

/// What the choices under the text box do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkRow {
    Check,
    /// Opens Steam's on-screen keyboard (Game Mode only: Slint does not raise it by itself).
    ShowKeyboard,
    Help,
    Cancel,
}

/// What to look for on GitHub, in the words of someone who has not been there. Both lines are on the page itself.
pub const LOOK_FOR: [&str; 2] = [
    "Open the project's GitHub page and copy its address, like github.com/owner/project.",
    "It needs a Releases section with a Linux download (.AppImage, .tar.gz or .zip).",
];

/// More help, behind a choice for anyone who wants it.
pub const HELP: &str = "Find the project's GitHub page. Its address looks like github.com/owner/project.\n\n\
It should have a Releases section (on the right of the page) with a download for Linux: a file ending in .AppImage, .tar.gz or .zip.\n\n\
Paste that address here, or type it. We only need that link: we check that the newest release has a Linux file, and nothing is downloaded yet.\n\n\
In Game Mode, press Steam + X to bring up the keyboard.";

/// The text under the text box when Steam's keyboard could not be raised.
pub const KEYBOARD_HINT: &str = "If no keyboard appeared, press the Steam button and X together.";

/// The text box page: the two lines of guidance, the box (with the Steam + X hint inside it in Game Mode), then
/// the choices. B and Cancel leave it; whatever was typed stays in `text`.
pub fn entry_page(text: &str, game_mode: bool) -> Page<LinkRow> {
    let mut p = Page::<LinkRow> {
        window_title: WINDOW_TITLE.to_string(),
        large: true,
        header: HEADER.to_string(),
        title: entry_title().to_string(),
        paragraphs: LOOK_FOR.iter().map(|l| l.to_string()).collect(),
        ..Page::default()
    };
    let placeholder = if game_mode { "Press Steam + X to type" } else { PLACEHOLDER };
    p.input = Some(InputBox { placeholder: placeholder.to_string(), text: text.to_string() });
    let option = |title: &str, detail: &str, action: LinkRow, recommended: bool| PageOption { title: title.to_string(), detail: detail.to_string(), choice: None, enabled: true, recommended, action };
    p.options.push(option("Check this link", "Asks GitHub. Nothing is downloaded yet", LinkRow::Check, true));
    if game_mode {
        p.options.push(option("Show keyboard", "", LinkRow::ShowKeyboard, false));
    }
    p.options.push(option("More help", "What to look for on GitHub", LinkRow::Help, false));
    p.buttons[LEFT] = Some(PageButton { label: "Cancel".to_string(), action: LinkRow::Cancel });
    p.initial = Focus::Input;
    p
}

/// "More help" as a page of its own: the same words as [`HELP`], one OK.
pub fn help_page() -> Page<CardAction> {
    let mut p = Page::<CardAction> {
        window_title: format!("{HEADER}: help"),
        large: true,
        header: HEADER.to_string(),
        title: "What should I look for?".to_string(),
        paragraphs: HELP.split("

").map(str::to_string).collect(),
        ..Page::default()
    };
    p.options = vec![PageOption { title: "OK".to_string(), detail: String::new(), choice: None, enabled: true, recommended: true, action: CardAction::Back }];
    p.initial = Focus::Option(0);
    p
}

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
    p.card = Some(Card { art: None, qr: None, title: checked.name.clone(), lines });
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
    fn the_page_shows_the_guidance_the_box_and_big_choices() {
        let p = entry_page("", false);
        assert_eq!(p.title, "Use a GitHub link");
        assert!(p.large);
        assert_eq!(p.paragraphs, LOOK_FOR);
        assert!(p.paragraphs.len() == 2 && p.paragraphs.iter().all(|l| l.len() < 90), "two short lines");
        let input = p.input.as_ref().unwrap();
        assert_eq!((input.placeholder.as_str(), input.text.as_str()), (PLACEHOLDER, ""));
        assert_eq!(p.options.iter().map(|o| o.title.as_str()).collect::<Vec<_>>(), ["Check this link", "More help"]);
        assert!(p.options[0].recommended);
        assert_eq!(p.buttons[LEFT].as_ref().map(|b| b.action), Some(LinkRow::Cancel));
        assert_eq!(p.initial, Focus::Input);
    }

    #[test]
    fn game_mode_adds_the_keyboard_and_the_hint_in_the_box_and_keeps_typed_text() {
        let p = entry_page("github.com/a/b", true);
        assert_eq!(p.options.iter().map(|o| o.title.as_str()).collect::<Vec<_>>(), ["Check this link", "Show keyboard", "More help"]);
        let input = p.input.unwrap();
        assert!(input.placeholder.contains("Steam + X"));
        assert_eq!(input.text, "github.com/a/b");
    }

    #[test]
    fn the_help_is_a_page_with_one_ok_and_no_cancel() {
        let p = help_page();
        assert_eq!(p.window_title, "Import ports: help");
        assert_eq!(p.paragraphs.len(), 4);
        assert_eq!(p.options.len(), 1);
        assert!(p.buttons.iter().all(Option::is_none), "nothing to cancel");
        assert_eq!(p.initial, Focus::Option(0));
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
