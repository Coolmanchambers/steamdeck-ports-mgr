//! A job that runs while the wizard waits (looking for ports, copying them, scanning the game files folder) as a
//! page: a title, one line per port or folder with its state, and a Cancel. The same words the progress lists of
//! Settings use, in the wizard's page style; the screens that run the jobs replace the page as the lines change.

use crate::fork::page::{Focus, Page, PageOption};
use crate::fork::rom_screens::util::ellipsize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressAction {
    Cancel,
}

/// How many lines are shown before "And N more."
const MAX_LINES: usize = 10;

/// "Cancel (the copy in progress is removed)" is the title "Cancel" and the detail "The copy in progress is removed".
pub fn split_cancel_label(label: &str) -> (String, String) {
    match label.split_once(" (") {
        Some((title, rest)) => {
            let detail = rest.trim_end_matches(')');
            let mut chars = detail.chars();
            (title.to_string(), chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default())
        }
        None => (label.to_string(), String::new()),
    }
}

/// `step` is the wizard step the job belongs to (2 for Import ports, 3 for the game files scan); `text` is what
/// the worker reports, one line per port or the one thing it is doing.
pub fn progress_page(title: &str, sentence: &str, step: usize, text: &str, cancel_label: &str, cancelling: bool) -> Page<ProgressAction> {
    let mut p = Page::<ProgressAction> {
        window_title: format!("Import ports: {}", title.to_lowercase()),
        large: true,
        title: title.to_string(),
        paragraphs: vec![sentence.to_string()],
        ..Page::default()
    };
    let lines: Vec<String> = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| ellipsize(l, 80)).collect();
    p.facts = lines.iter().take(MAX_LINES).cloned().collect();
    if lines.len() > MAX_LINES {
        p.facts.push(format!("And {} more.", lines.len() - MAX_LINES));
    }
    if p.facts.is_empty() {
        p.facts.push("Starting...".to_string());
    }
    let (cancel, detail) = split_cancel_label(cancel_label);
    p.options = vec![if cancelling {
        PageOption { title: "Cancelling...".to_string(), detail: "Stopping the work in progress".to_string(), choice: None, enabled: false, recommended: false, action: ProgressAction::Cancel }
    } else {
        PageOption { title: cancel, detail, choice: None, enabled: true, recommended: false, action: ProgressAction::Cancel }
    }];
    p.initial = Focus::Option(0);
    // B stops the work like the Cancel choice does; the hint bar says so.
    let p = if cancelling { p.b_means("Cancel") } else { p.b_does(&ProgressAction::Cancel) };
    p.in_wizard(step, super::flow::Step::COUNTED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_shows_one_line_per_port_and_a_cancel_with_its_detail() {
        let p = progress_page("Importing ports", "Each port is copied and checked.", 2, "Demo Wolf: waiting\nDemo Blocks: waiting\n", "Cancel (the copy in progress is removed; finished ports stay)", false);
        assert_eq!(p.title, "Importing ports");
        assert_eq!(p.window_title, "Import ports: importing ports");
        assert_eq!(p.facts, ["Demo Wolf: waiting", "Demo Blocks: waiting"]);
        assert_eq!((p.options[0].title.as_str(), p.options[0].detail.as_str()), ("Cancel", "The copy in progress is removed; finished ports stay"));
        assert!(p.options[0].enabled);
        assert_eq!((p.header.as_str(), p.step_text.as_str(), p.step_count), ("Deck Ports Manager", "Step 2 of 5", 5));
        assert_eq!(p.initial, Focus::Option(0));
    }

    #[test]
    fn cancelling_dims_the_button_and_an_empty_text_says_starting() {
        let p = progress_page("Looking for ports", "x", 2, "", "Cancel", true);
        assert_eq!(p.facts, ["Starting..."]);
        assert!(!p.options[0].enabled && p.options[0].title == "Cancelling...");
        let q = progress_page("Looking for ports", "x", 2, "Looking in 3 folders...", "Cancel", false);
        assert_eq!((q.options[0].title.as_str(), q.options[0].detail.as_str()), ("Cancel", ""));
    }

    #[test]
    fn many_lines_are_cut_with_a_count() {
        let text: String = (1..=14).map(|i| format!("Port {i}: waiting\n")).collect();
        let p = progress_page("Importing ports", "x", 2, &text, "Cancel", false);
        assert_eq!(p.facts.len(), MAX_LINES + 1);
        assert_eq!(p.facts.last().unwrap(), "And 4 more.");
    }

    #[test]
    fn the_cancel_label_splits_at_its_note() {
        assert_eq!(split_cancel_label("Cancel"), ("Cancel".to_string(), String::new()));
        assert_eq!(split_cancel_label("Cancel (nothing is changed)"), ("Cancel".to_string(), "Nothing is changed".to_string()));
    }
}
