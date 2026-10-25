//! The pages of the checkup as plain data: the wait while it looks, the overview (a line for each thing that needs a look),
//! one page per problem, the page after a fix that shows the line again, and the last page. `src/app/checkup_screen.rs`
//! shows them.
//!
//! One question per page, a few big cards, the recommended one tagged, short plain text (Tim's rule for every screen).

use super::{Check, Outcome, Problem, Results};
use crate::fork::page::{Focus, Mark, Page, PageButton, PageOption, LEFT, RIGHT};

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
    /// Leave this problem as it is: it stays out of the way until it changes.
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
    /// Bring the skipped problems back.
    Unskip,
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

/// The looking failed (a worker stopped, or a job is running that must finish first): say so, nothing is changed.
pub fn failed_page(why: &str) -> CheckupPage {
    let mut p = base("The checkup could not run");
    p.paragraphs = vec![why.to_string(), "Nothing was changed.".to_string()];
    p.options = vec![option("OK", "", Act::NotNow, true)];
    p.initial = Focus::Option(0);
    p
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// The first page: a line for each check that needs a look (the fine ones collapse into one line when anything does), then
/// what to do. "Check again" is always in the footer, so the cards stay in reach on a small window.
pub fn overview_page(results: &Results) -> CheckupPage {
    let n = results.problem_count();
    let skipped = results.skipped_count();
    let unknown = results.items.iter().filter(|(_, o)| matches!(o, Outcome::Unknown(_))).count();
    let mut p = base("Checkup");
    let mut text = match (n, skipped, unknown) {
        (0, 0, 0) => "Everything looks fine.".to_string(),
        (0, 0, _) => "Everything I could check looks fine.".to_string(),
        (0, s, _) => format!("Nothing new to fix. {s} skipped earlier."),
        (1, _, _) => "I found 1 thing to fix.".to_string(),
        (n, _, _) if results.to_do_now() == n => format!("I found {n} things to fix."),
        (n, _, _) => format!("I found {n} things to fix: {}.", results.split_words()),
    };
    if n > 0 && skipped > 0 {
        text.push_str(&format!(" {skipped} skipped earlier."));
    }
    p.paragraphs = vec![text];
    let all_fine = results.items.iter().all(|(_, o)| !o.is_not_fine());
    let mut fine = 0;
    for (check, outcome) in &results.items {
        if !all_fine && !outcome.is_not_fine() {
            fine += 1;
            continue;
        }
        p.fact(outcome.mark(), line(*check, outcome));
    }
    if fine > 0 {
        p.fact(Mark::Done, if fine == 1 { "1 other check is fine".to_string() } else { format!("{fine} other checks are fine") });
    }
    p.initial = Focus::Option(0);
    p.buttons[LEFT] = Some(PageButton { label: "Check again".to_string(), action: Act::CheckAgain });
    let unskip = option("Include the skipped ones", "Look at them again.", Act::Unskip, false);
    if n == 0 {
        p.options = vec![option("Done", "", Act::Done, true)];
        if skipped > 0 {
            p.options.push(unskip);
        }
        return p.b_means("Done");
    }
    p.options = vec![option(if n == 1 { "Fix it" } else { "Fix them one by one" }, "Each one opens on its own page and asks before it changes anything.", Act::FixAll, true)];
    if skipped > 0 {
        p.options.push(unskip);
    }
    // "Not now" is a footer button next to "Check again": both stay in reach however long the list is.
    p.buttons[RIGHT] = Some(PageButton { label: "Not now".to_string(), action: Act::NotNow });
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
    p.footnote = problem.detail.join("\n");
    p.options = vec![option(&problem.fix_label, "", Act::Fix, true), option("Skip this one", "It stays out of the way until something about it changes.", Act::Skip, false)];
    p.initial = Focus::Option(0);
    p.b_means("Checkup")
}

/// A page that asks before it changes something itself (an update, a backup): the fixes that are not an existing
/// screen with its own question.
pub fn confirm_page(check: Check, question: &str, lines: Vec<String>, yes: &str, n: usize, total: usize) -> CheckupPage {
    let mut p = base(check.title());
    counter(&mut p, n, total);
    p.paragraphs = std::iter::once(question.to_string()).chain(lines).collect();
    // "Not now" leaves it for now without remembering a skip (only "Skip this one" does).
    p.options = vec![option(yes, "", Act::Yes, true), option("Not now", "Nothing is changed.", Act::Next, false)];
    p.initial = Focus::Option(0);
    p.b_means("Not now")
}

/// After a fix: the check looked at again, its line turned green or not. `last`: no other problem is left to visit.
/// `progress`: the problem is still there but for fewer ports than before (one of several was done).
pub fn after_page(check: Check, outcome: &Outcome, n: usize, total: usize, last: bool, progress: bool) -> CheckupPage {
    let mut p = base(match outcome {
        Outcome::Ok(_) => "That is fixed",
        Outcome::Queued { .. } => "Queued, not finished",
        Outcome::Problem(_) if progress => "One done, more to go",
        Outcome::Problem(_) => "Not fixed yet",
        Outcome::Skipped(_) => "Skipped",
        Outcome::Unknown(_) => "Could not check again",
    });
    counter(&mut p, n, total);
    p.fact(outcome.mark(), line(check, outcome));
    let next = if last { "Finish" } else { "Next problem" };
    p.initial = Focus::Option(0);
    p.options = match outcome {
        Outcome::Ok(_) | Outcome::Skipped(_) => vec![option(next, "", Act::Next, true)],
        Outcome::Queued { fix_label, desktop_only, .. } => {
            p.paragraphs = vec!["The change is made. It is finished when Steam closes.".to_string()];
            let finish = option(fix_label, "", Act::TryAgain, !desktop_only);
            let go_on = option(next, "Leave it for now.", Act::Next, *desktop_only);
            if *desktop_only { vec![go_on, finish] } else { vec![finish, go_on] }
        }
        Outcome::Problem(pr) => {
            p.footnote = pr.detail.join("\n");
            let again = option(if progress { "Fix the next one" } else { "Try again" }, &pr.fix_label, Act::TryAgain, !pr.desktop_only);
            let recheck = option("Check again", "Look at it once more.", Act::CheckAgain, false);
            let go_on = option(next, "Leave it as it is for now.", Act::Next, pr.desktop_only);
            // In Game Mode the page that said "finish in Desktop Mode" cannot be acted on: moving on leads.
            if pr.desktop_only { vec![go_on, again, recheck] } else { vec![again, recheck, go_on] }
        }
        Outcome::Unknown(_) => vec![option("Check again", "", Act::CheckAgain, true), option(next, "", Act::Next, false)],
    };
    p.b_means(next)
}

/// The last page: how each problem the run set out to fix, and any that showed up meanwhile, stands now.
pub fn finished_page(items: &[(Check, Outcome)]) -> CheckupPage {
    let count = |f: fn(&Outcome) -> bool| items.iter().filter(|(_, o)| f(o)).count();
    let (problems, queued, unknown, skipped) = (
        count(Outcome::is_problem),
        count(|o| matches!(o, Outcome::Queued { .. })),
        count(|o| matches!(o, Outcome::Unknown(_))),
        count(|o| matches!(o, Outcome::Skipped(_))),
    );
    let clear = problems + queued + unknown + skipped == 0;
    let mut p = base(if clear { "All done" } else { "Done for now" });
    let mut left = Vec::new();
    if problems > 0 {
        left.push(format!("{} to fix", problems));
    }
    if queued > 0 {
        left.push(format!("{queued} waiting for Steam to close"));
    }
    if unknown > 0 {
        left.push(format!("{} could not be checked", plural(unknown, "check", "checks")));
    }
    if skipped > 0 {
        left.push(format!("{skipped} skipped"));
    }
    p.paragraphs = vec![if clear {
        "Everything is fixed.".to_string()
    } else {
        format!("Still left: {}. You can come back to it from Settings, Fix problems.", left.join(", "))
    }];
    for (check, outcome) in items {
        p.fact(outcome.mark(), line(*check, outcome));
    }
    p.options = vec![option("Done", "", Act::Done, true), option("Check everything again", "", Act::CheckAgain, false)];
    p.initial = Focus::Option(0);
    p.b_means("Done")
}
