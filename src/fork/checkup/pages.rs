//! The pages of the checkup as plain data: the wait while it looks, the overview (one line per check), one page per
//! problem, the page after a fix that shows the line again, and the last page. `src/app/checkup_screen.rs` shows them.
//!
//! One question per page, a few big cards, the recommended one tagged, short plain text (Tim's rule for every screen).

use super::{Check, Outcome, Problem, Results};
use crate::fork::page::{Focus, Page, PageButton, PageOption, RIGHT};

pub const HEADER: &str = "Deck Ports Manager";
pub const WINDOW_TITLE: &str = "Fix problems";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// "Fix them one by one": visit the problems in order.
    FixAll,
    /// Leave the checkup.
    NotNow,
    /// Open the fix for the problem on this page.
    Fix,
    /// Leave this problem as it is and go on.
    Skip,
    /// After a fix: the next problem.
    Next,
    /// After a fix that did not clear it: the fix again.
    TryAgain,
    /// Look again.
    CheckAgain,
    Done,
    /// Back to the overview.
    Overview,
    /// "Yes" on a page that asks before it changes something.
    Yes,
}

pub type CheckupPage = Page<Act>;

fn base(title: &str) -> CheckupPage {
    Page { window_title: WINDOW_TITLE.to_string(), large: true, header: HEADER.to_string(), title: title.to_string(), ..Page::default() }
}

fn option(title: &str, detail: &str, action: Act, recommended: bool) -> PageOption<Act> {
    PageOption { title: title.to_string(), detail: detail.to_string(), choice: None, enabled: true, recommended, action }
}

/// The overview's line: `Title: words`, the words starting in lower case after the colon when that reads on ("Steam
/// icons: icon file is missing for ..."), never for a name or a number ("3D", "SoH", "2 old copies").
pub fn line(check: Check, outcome: &Outcome) -> String {
    let words = outcome.words();
    let first: String = words.chars().take_while(|c| c.is_alphabetic()).collect();
    let plain_word = first.chars().next().is_some_and(char::is_uppercase) && first.chars().skip(1).all(char::is_lowercase) && first.chars().count() > 1;
    let words = if plain_word && !words.starts_with("Update ready") { words.chars().flat_map(|c| c.to_lowercase()).take(1).chain(words.chars().skip(1)).collect() } else { words };
    format!("{}: {}", check.title(), words)
}

/// While the checkup looks (a moment; nothing is changed).
pub fn looking_page() -> CheckupPage {
    let mut p = base("Looking over your setup");
    p.paragraphs = vec!["This only looks. Nothing is changed.".to_string()];
    p.buttons[RIGHT] = Some(PageButton { label: "Cancel".to_string(), action: Act::NotNow });
    p.initial = Focus::Button(RIGHT);
    p.with_progress(None).b_means("Cancel")
}

/// The looking failed (a worker stopped): say so, nothing is changed.
pub fn failed_page(why: &str) -> CheckupPage {
    let mut p = base("The checkup could not run");
    p.paragraphs = vec![why.to_string(), "Nothing was changed.".to_string()];
    p.options = vec![option("OK", "", Act::NotNow, true)];
    p.initial = Focus::Option(0);
    p
}

/// The first page: every check on one line with its mark, then what to do.
pub fn overview_page(results: &Results) -> CheckupPage {
    let n = results.problem_count();
    let mut p = base("Checkup");
    p.paragraphs = vec![match n {
        0 => "Everything looks fine.".to_string(),
        1 => "I found 1 thing to fix.".to_string(),
        n => format!("I found {n} things to fix."),
    }];
    if n == 0 && results.items.iter().any(|(_, o)| matches!(o, Outcome::Unknown(_))) {
        p.paragraphs = vec!["Everything I could check looks fine.".to_string()];
    }
    for (check, outcome) in &results.items {
        p.fact(outcome.mark(), line(*check, outcome));
    }
    p.initial = Focus::Option(0);
    if n == 0 {
        p.options = vec![option("Done", "", Act::Done, true), option("Check again", "Look over everything once more.", Act::CheckAgain, false)];
        return p.b_means("Done");
    }
    let one = n == 1;
    p.options = vec![
        option(if one { "Fix it" } else { "Fix them one by one" }, "Each one opens on its own page and asks before it changes anything.", Act::FixAll, true),
        option("Not now", "Nothing is changed.", Act::NotNow, false),
    ];
    p.b_means("Not now")
}

fn counter(p: &mut CheckupPage, n: usize, total: usize) {
    if total > 1 {
        p.step_text = format!("Problem {n} of {total}");
        p.step_number = n;
        p.step_count = total;
    }
}

/// One problem: what it is, with the real values behind it, and the card that opens its fix.
pub fn step_page(check: Check, problem: &Problem, n: usize, total: usize) -> CheckupPage {
    let mut p = base(check.title());
    counter(&mut p, n, total);
    p.fact(problem.mark, problem.summary.clone());
    p.footnote = problem.detail.join("
");
    p.options = vec![option(&problem.fix_label, "", Act::Fix, true), option("Skip this one", "Leave it as it is for now.", Act::Skip, false)];
    p.initial = Focus::Option(0);
    p.b_means("Checkup")
}

/// A page that asks before it changes something itself (an update, a backup): the fixes that are not an existing
/// screen with its own question.
pub fn confirm_page(check: Check, question: &str, lines: Vec<String>, yes: &str, n: usize, total: usize) -> CheckupPage {
    let mut p = base(check.title());
    counter(&mut p, n, total);
    p.paragraphs = std::iter::once(question.to_string()).chain(lines).collect();
    p.options = vec![option(yes, "", Act::Yes, true), option("Not now", "Nothing is changed.", Act::Skip, false)];
    p.initial = Focus::Option(0);
    p.b_means("Not now")
}

/// After a fix: the check looked at again, its line turned green or not. `last`: no other problem is left to visit.
pub fn after_page(check: Check, outcome: &Outcome, n: usize, total: usize, last: bool) -> CheckupPage {
    let mut p = base(match outcome {
        Outcome::Ok(_) => "That is fixed",
        Outcome::Problem(_) => "Not fixed yet",
        Outcome::Unknown(_) => "Could not check again",
    });
    counter(&mut p, n, total);
    p.fact(outcome.mark(), line(check, outcome));
    let next = if last { "Finish" } else { "Next problem" };
    p.initial = Focus::Option(0);
    p.options = match outcome {
        Outcome::Ok(_) => vec![option(next, "", Act::Next, true)],
        Outcome::Problem(pr) => {
            p.footnote = pr.detail.join("
");
            vec![
                option("Try again", &pr.fix_label, Act::TryAgain, true),
                option("Check again", "Look at it once more.", Act::CheckAgain, false),
                option(next, "Leave it as it is for now.", Act::Next, false),
            ]
        }
        Outcome::Unknown(_) => vec![option("Check again", "", Act::CheckAgain, true), option(next, "", Act::Next, false)],
    };
    p.b_means(next)
}

/// The last page: how each problem the run set out to fix stands now (skipped ones count as left).
pub fn finished_page(visited: &[(Check, Outcome)]) -> CheckupPage {
    let left = visited.iter().filter(|(_, o)| o.is_problem()).count();
    let mut p = base(if left == 0 { "All done" } else { "Done for now" });
    p.paragraphs = vec![match left {
        0 => "Everything is fixed.".to_string(),
        1 => "1 thing is still left. You can come back to it from Settings, Fix problems.".to_string(),
        n => format!("{n} things are still left. You can come back to them from Settings, Fix problems."),
    }];
    for (check, outcome) in visited {
        p.fact(outcome.mark(), line(*check, outcome));
    }
    p.options = vec![option("Done", "", Act::Done, true), option("Check everything again", "", Act::CheckAgain, false)];
    p.initial = Focus::Option(0);
    p.b_means("Done")
}
