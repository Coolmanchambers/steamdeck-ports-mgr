//! A notice or a yes/no question inside the wizard's Import ports step, as a page: the GitHub link errors, "Leave this
//! folder alone?", "Link it where it stands instead?". The same words the small dialogs used, in the wizard's page
//! style, with one OK for a notice and two choices for a question. The old dialog is kept for long texts, which a
//! page could not scroll with the controller.

use crate::fork::page::{Focus, Page, PageOption};

/// Longer than this stays in the old dialog (a page has nothing to scroll with the D-pad).
pub const MAX_TEXT: usize = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeAction {
    Confirm,
    Cancel,
}

/// Labels that mean "OK, go on": the cancel and the confirm do the same, so there is one button.
pub fn is_single(label: &str) -> bool {
    matches!(label, "OK" | "Continue setup" | "Back to the question")
}

/// The other choice of a question: "Leave it alone" is answered by "Keep it".
fn cancel_label(label: &str) -> &'static str {
    match label {
        "Leave it alone" => "Keep it",
        "Link instead" => "Do not link",
        "Remove" => "Keep it",
        _ => "Cancel",
    }
}

fn shown_title(title: &str, label: &str) -> String {
    match (title, label) {
        ("Import ports", "Leave it alone") => "Leave this folder alone?".to_string(),
        ("Import ports", "Link instead") => "Link them where they stand?".to_string(),
        ("Import ports", l) if is_single(l) => "Please note".to_string(),
        ("Import ports", _) => "Please check".to_string(),
        (t, _) => t.to_string(),
    }
}

/// `preselect_cancel`: the cursor starts on the safe choice, so a stray A does not do the work.
pub fn notice_page(title: &str, text: &str, label: &str, preselect_cancel: bool, step: usize) -> Page<NoticeAction> {
    let shown = shown_title(title, label);
    let mut p = Page::<NoticeAction> {
        window_title: format!("Import ports: {}", shown.to_lowercase()),
        large: true,
        title: shown,
        paragraphs: text.split("\n\n").map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect(),
        ..Page::default()
    };
    let option = |title: &str, action: NoticeAction, recommended: bool| PageOption { title: title.to_string(), detail: String::new(), choice: None, enabled: true, recommended, action };
    if is_single(label) {
        p.options = vec![option(label, NoticeAction::Confirm, true)];
        p.initial = Focus::Option(0);
    } else {
        p.options = vec![option(label, NoticeAction::Confirm, !preselect_cancel), option(cancel_label(label), NoticeAction::Cancel, preselect_cancel)];
        p.initial = Focus::Option(usize::from(preselect_cancel));
    }
    p.in_wizard(step, super::flow::Step::COUNTED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notice_is_one_ok_with_the_text_as_paragraphs() {
        let p = notice_page("Import ports", "GitHub has no project called a/b. Check the link for typos.\n\nSecond paragraph.", "OK", false, 2);
        assert_eq!(p.title, "Please note");
        assert_eq!(p.window_title, "Import ports: please note");
        assert_eq!(p.paragraphs.len(), 2);
        assert_eq!(p.options.len(), 1);
        assert_eq!((p.options[0].title.as_str(), p.options[0].action, p.options[0].recommended), ("OK", NoticeAction::Confirm, true));
        assert_eq!((p.header.as_str(), p.step_text.as_str()), ("Deck Ports Manager", "Step 2 of 5"));
        assert_eq!(p.initial, Focus::Option(0));
    }

    #[test]
    fn not_a_port_is_leave_it_alone_or_keep_it_and_starts_on_keep_it() {
        let p = notice_page("Import ports", "This looks like Demo Wolf, a port you have.\n\nIt will not be offered again.", "Leave it alone", true, 2);
        assert_eq!(p.title, "Leave this folder alone?");
        assert_eq!(p.options.iter().map(|o| (o.title.as_str(), o.action)).collect::<Vec<_>>(), [("Leave it alone", NoticeAction::Confirm), ("Keep it", NoticeAction::Cancel)]);
        assert_eq!(p.initial, Focus::Option(1), "a stray A keeps the folder in the list");
        assert!(p.options[1].recommended && !p.options[0].recommended);
    }

    #[test]
    fn other_questions_have_their_own_second_choice_and_a_default_that_changes_nothing() {
        let link = notice_page("Import ports", "These ports did not fit.", "Link instead", true, 2);
        assert_eq!(link.options[1].title, "Do not link");
        let rm = notice_page("Scan folder", "Stop scanning X?", "Remove", false, 2);
        assert_eq!((rm.title.as_str(), rm.options[1].title.as_str(), rm.initial), ("Scan folder", "Keep it", Focus::Option(0)));
        assert_eq!(notice_page("Import ports", "x", "Continue setup", false, 2).options.len(), 1);
        assert!(is_single("Back to the question") && !is_single("Remove"));
    }
}
