//! The questions and notices of Settings > Add to Steam as pages: which Steam account (a choice, the one in use marked),
//! the Steam changes waiting (one card per change in plain words, then "Clear all waiting changes"), a yes/no page
//! before a change is dropped or all are cleared (No first), the review before adding ("Add these to Steam?"), the
//! question before changes are left waiting for Steam to close, and notices ("Steam changes waiting", "Added to
//! Steam"). The main list stays a list on purpose (`fork::screens`).
//!
//! The words: "Add to Steam" for the action, "your Steam shortcuts" for the thing, "your Steam shortcuts list" for the
//! file Steam keeps them in, "waiting" for changes that wait for Steam to close.
//!
//! Pure data and words; `src/app/steam_screens.rs` runs them. What is written to Steam, and when, is unchanged.

use crate::fork::page::{Focus, Mark, Page, PageButton, PageOption, LEFT};
use crate::fork::rom_screens::util::ellipsize;
use crate::fork::steam_queue::{QueuedChange, QueuedKind, SteamQueue};

pub const TITLE: &str = "Add to Steam";
/// The window title (and page title) of the waiting changes and the watch's notices.
pub const WAITING: &str = "Steam changes waiting";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteamAct {
    /// The account choice: account `n`.
    Account(usize),
    /// The waiting changes: change `n`.
    Change(usize),
    ClearAll,
    Yes,
    No,
    Done,
    Back,
}

fn option(title: impl Into<String>, detail: impl Into<String>, action: SteamAct) -> PageOption<SteamAct> {
    PageOption { title: title.into(), detail: detail.into(), choice: None, enabled: true, recommended: false, action }
}

fn base(window: &str, title: &str) -> Page<SteamAct> {
    Page { window_title: window.to_string(), large: true, header: TITLE.to_string(), title: title.to_string(), ..Page::default() }
}

fn many(n: usize, one: &str, more: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {more}") }
}

/// A yes/no question; `no_first` puts No first and the cursor on it (B is always No).
fn yes_no(mut p: Page<SteamAct>, yes: &str, no: &str, no_first: bool) -> Page<SteamAct> {
    let (y, n) = (option(yes, "", SteamAct::Yes), option(no, "", SteamAct::No));
    p.options = if no_first { vec![n, y] } else { vec![PageOption { recommended: true, ..y }, n] };
    p.initial = Focus::Option(0);
    // Every time on No: after "Yes, drop it" the next "Drop this waiting change?" is the same page to the window.
    p.fresh_focus = no_first;
    p.b_does(&SteamAct::No)
}

/// Which Steam account the shortcuts are for. `current` is the one in use (marked).
pub fn account_page(labels: &[String], current: Option<usize>) -> Page<SteamAct> {
    let mut p = base(&format!("{TITLE}: account"), "Which Steam account?");
    p.paragraphs.push("This Deck has more than one Steam account. Your ports are added to the one you choose.".to_string());
    for (i, label) in labels.iter().enumerate() {
        let detail = if current == Some(i) { "In use now" } else { "" };
        p.options.push(PageOption { choice: Some(current == Some(i)), ..option(ellipsize(label, 40), detail, SteamAct::Account(i)) });
    }
    p.buttons[LEFT] = Some(PageButton { label: "Back".to_string(), action: SteamAct::Back });
    p.initial = Focus::Option(current.unwrap_or(0));
    p
}

/// What a waiting change will do, in plain words.
pub fn change_words(kind: QueuedKind) -> &'static str {
    match kind {
        QueuedKind::Add => "Will be added to Steam",
        QueuedKind::Update => "Its Steam shortcut will be updated",
        QueuedKind::Repoint => "Its Steam shortcut will follow its new folder",
        QueuedKind::Icon => "Its Steam icon will be repaired",
    }
}

/// What a waiting change will do, as a sentence about the port: "Dusklight will be added to Steam".
pub fn change_sentence(name: &str, kind: QueuedKind) -> String {
    match kind {
        QueuedKind::Add => format!("{name} will be added to Steam"),
        QueuedKind::Update => format!("The Steam shortcut of {name} will be updated"),
        QueuedKind::Repoint => format!("The Steam shortcut of {name} will follow its new folder"),
        QueuedKind::Icon => format!("The Steam icon of {name} will be repaired"),
    }
}

/// The name a Steam account goes by on this Deck (`accounts`: account id and the name the account page shows); a
/// change for an account the Deck no longer has is named by its number.
pub fn account_name(accounts: &[(u32, String)], account_id: u32) -> String {
    accounts.iter().find(|(id, _)| *id == account_id).map_or_else(|| format!("Steam account {account_id}"), |(_, label)| label.clone())
}

/// The Steam changes waiting for Steam to close: one card each (choosing one asks to drop it), then Clear all. With
/// more than one Steam account on the Deck (`accounts`), the changes are grouped under a line naming each account.
pub fn waiting_page(queue: &SteamQueue, accounts: &[(u32, String)], game_mode: bool) -> Page<SteamAct> {
    let mut p = base(WAITING, WAITING);
    if queue.is_empty() {
        p.paragraphs.push("Nothing is waiting.".to_string());
    } else {
        p.paragraphs.push(if game_mode {
            "These are made the next time the app runs in Desktop Mode with Steam closed. Choose one to drop it.".to_string()
        } else {
            "These are made once Steam is closed. Choose one to drop it.".to_string()
        });
        if accounts.len() > 1 || queue.accounts().len() > 1 {
            for account in queue.accounts() {
                // The account on a line of its own: a dimmed line that cannot be chosen, the account's changes under it.
                p.options.push(PageOption { enabled: false, ..option(ellipsize(&account_name(accounts, account), 40), "Changes for this Steam account", SteamAct::Back) });
                for (i, c) in queue.entries.iter().enumerate().filter(|(_, c)| c.account_id == account) {
                    p.options.push(option(ellipsize(&c.name, 40), change_words(c.kind), SteamAct::Change(i)));
                }
            }
        } else {
            for (i, c) in queue.entries.iter().enumerate() {
                p.options.push(option(ellipsize(&c.name, 40), change_words(c.kind), SteamAct::Change(i)));
            }
        }
        p.options.push(option("Clear all waiting changes", format!("Drop all {}", many(queue.len(), "change", "changes")), SteamAct::ClearAll));
        p.initial = Focus::Option(p.options.iter().position(|o| o.enabled).unwrap_or(0));
    }
    p.buttons[LEFT] = Some(PageButton { label: "Back".to_string(), action: SteamAct::Back });
    p
}

/// Before one waiting change is dropped. No first.
pub fn drop_page(c: &QueuedChange) -> Page<SteamAct> {
    let mut p = base(&format!("{WAITING}: drop one"), "Drop this waiting change?");
    p.paragraphs.push(format!("{}.", change_sentence(&c.name, c.kind)));
    p.paragraphs.push("Dropping it means it is not made. Your Steam shortcuts list is not touched.".to_string());
    yes_no(p, "Yes, drop it", "No, keep it", true)
}

/// Before every waiting change is cleared. No first.
pub fn clear_page(count: usize) -> Page<SteamAct> {
    let mut p = base(&format!("{WAITING}: clear"), "Clear all waiting changes?");
    p.paragraphs.push(format!("{} {} dropped and not made. Your Steam shortcuts list is not touched.", if count == 1 { "The waiting change".to_string() } else { format!("All {count} waiting changes") }, if count == 1 { "is" } else { "are" }));
    yes_no(p, "Yes, clear them", "No, keep them", true)
}

/// Before the chosen ports are added to Steam (Steam is closed): what is added, what is updated.
pub fn review_page(added: &[String], updated: &[String]) -> Page<SteamAct> {
    let total = added.len() + updated.len();
    let mut p = base(&format!("{TITLE}: review"), "Add these to Steam?");
    let mut lines: Vec<String> = added.iter().map(|n| format!("Add {n}")).chain(updated.iter().map(|n| format!("Update {n}"))).collect();
    if lines.len() > 6 {
        let more = lines.len() - 5;
        lines.truncate(5);
        lines.push(format!("And {more} more"));
    }
    for l in lines {
        p.fact(Mark::None, ellipsize(&l, 66));
    }
    if !updated.is_empty() {
        p.paragraphs.push("An updated shortcut keeps its play time.".to_string());
    }
    p.paragraphs.push("Your Steam shortcuts list is backed up first.".to_string());
    yes_no(p, &format!("Yes, add {}", if total == 1 { "it" } else { "them" }), "Not now", false)
}

/// Steam is running: leave the chosen changes waiting until it is closed (or, in Game Mode, until Desktop Mode)?
/// `account`: the Steam account they are for, named when the Deck has more than one.
pub fn wait_page(changes: &[QueuedChange], game_mode: bool, account: Option<&str>) -> Page<SteamAct> {
    let title = if game_mode { "Add to Steam in Desktop Mode?" } else { "Add when Steam is closed?" };
    let mut p = base(&format!("{TITLE}: wait"), title);
    p.paragraphs.push(if game_mode {
        "Steam cannot be closed in Game Mode, so nothing is written now. These wait, and the app asks to make them the next time it runs in Desktop Mode with Steam closed.".to_string()
    } else {
        "Steam is running, so nothing is written now. These wait until Steam is closed (Steam menu, Exit).".to_string()
    });
    if let Some(name) = account {
        p.fact(Mark::None, ellipsize(&format!("For the Steam account {name}"), 66));
    }
    for c in changes.iter().take(5) {
        p.fact(Mark::Waiting, ellipsize(&change_sentence(&c.name, c.kind), 66));
    }
    if changes.len() > 5 {
        p.fact(Mark::Waiting, format!("And {} more", changes.len() - 5));
    }
    yes_no(p, "Yes, let them wait", "Not now", false)
}

/// Something to say, then back (OK). `window` names it for the window title.
pub fn notice_page(window: &str, title: &str, text: &str) -> Page<SteamAct> {
    let mut p = base(window, title);
    p.paragraphs = text.split("\n\n").map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect();
    p.options.push(PageOption { recommended: true, ..option("OK", "", SteamAct::Done) });
    p.initial = Focus::Option(0);
    p.b_does(&SteamAct::Done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::page::{action_at, initial_focus};
    use crate::fork::rom_screens::settings_pages::tests::{assert_fits, shown};
    use crate::fork::steam_queue::ArtChoice;

    const LONG: &str = "The Legend of Zelda: The Wind Waker HD (ZeldaWWHDRecomp)";

    fn change(name: &str, kind: QueuedKind, account: u32) -> QueuedChange {
        QueuedChange {
            account_id: account,
            account_label: format!("A rather long Steam persona name {account}"),
            name: name.into(),
            kind,
            repoint_by_name: false,
            port_key: name.to_lowercase(),
            art: ArtChoice::Catalog,
            from_game_mode: true,
            queued_at: "20261101-100000".into(),
            repoint: None,
            icon: None,
        }
    }

    fn accounts() -> Vec<(u32, String)> {
        vec![(1, "Deck Player (deckplayer)".to_string()), (7, "Guest (guest)".to_string())]
    }

    fn queue() -> SteamQueue {
        SteamQueue { entries: vec![change(LONG, QueuedKind::Add, 1), change("SoH", QueuedKind::Update, 2), change("Dusk", QueuedKind::Repoint, 1), change("2S2H", QueuedKind::Icon, 1)] }
    }

    /// No em dash, no file names, none of the words the owner does not use.
    pub(crate) fn assert_plain_steam<A>(p: &Page<A>) {
        for t in shown(p) {
            assert!(!t.contains('\u{2014}') && !t.contains("--"), "no em dash: {t}");
            let lower = t.to_lowercase();
            for word in ["queue", "shortcuts.vdf", "repoint", "json", "/home/", "~/", "(s)", "display()", " art ", "apply"] {
                assert!(!lower.contains(word), "{word:?} in {t:?}");
            }
        }
    }

    fn all() -> Vec<Page<SteamAct>> {
        let q = queue();
        let names: Vec<String> = (0..9).map(|i| format!("{LONG} {i}")).collect();
        vec![
            account_page(&[LONG.to_string(), "Tim".to_string()], Some(1)),
            account_page(&["Tim".to_string()], None),
            waiting_page(&q, &[], true),
            waiting_page(&q, &accounts(), false),
            waiting_page(&SteamQueue::default(), &[], false),
            drop_page(&q.entries[0]),
            clear_page(4),
            clear_page(1),
            review_page(&names, &names[..2]),
            review_page(&["Dusk".to_string()], &[]),
            wait_page(&q.entries, true, Some(LONG)),
            wait_page(&q.entries[..1], false, None),
            notice_page(WAITING, WAITING, "Nothing is waiting."),
        ]
    }

    #[test]
    fn every_page_fits_and_says_it_plainly() {
        for p in all() {
            assert_fits(&p);
            assert_plain_steam(&p);
            assert!(p.window_title.starts_with(TITLE) || p.window_title.starts_with(WAITING), "{}", p.window_title);
        }
    }

    #[test]
    fn the_account_page_marks_the_one_in_use_and_starts_on_it() {
        let p = account_page(&["Tim".to_string(), "Guest".to_string()], Some(1));
        assert_eq!(p.title, "Which Steam account?");
        assert_eq!(p.options.iter().map(|o| o.choice).collect::<Vec<_>>(), [Some(false), Some(true)]);
        assert_eq!(p.options[1].detail, "In use now");
        assert_eq!(action_at(&p, initial_focus(&p)), Some(SteamAct::Account(1)));
    }

    #[test]
    fn waiting_changes_are_one_card_each_in_plain_words_then_clear_all() {
        // Two accounts on the Deck: each account on a line of its own (dimmed), its changes under it; one the Deck
        // no longer has is named by its number.
        let p = waiting_page(&queue(), &accounts(), false);
        assert_eq!(p.title, WAITING);
        let rows: Vec<(&str, &str, bool)> = p.options.iter().map(|o| (o.title.as_str(), o.detail.as_str(), o.enabled)).collect();
        assert_eq!(rows[0], ("Deck Player (deckplayer)", "Changes for this Steam account", false));
        assert_eq!(rows[1].1, "Will be added to Steam");
        assert_eq!(rows[2], ("Dusk", "Its Steam shortcut will follow its new folder", true));
        assert_eq!(rows[3], ("2S2H", "Its Steam icon will be repaired", true));
        assert_eq!((rows[4].0, rows[4].2), ("Steam account 2", false));
        assert_eq!(p.options[5].action, SteamAct::Change(1), "a card still drops the change it shows");
        assert_eq!(action_at(&p, initial_focus(&p)), Some(SteamAct::Change(0)), "the cursor starts on a change, not a name");
        let last = p.options.last().unwrap();
        assert_eq!((last.title.as_str(), last.detail.as_str(), last.action), ("Clear all waiting changes", "Drop all 4 changes", SteamAct::ClearAll));
        // One account left on the Deck, but changes still wait for one it no longer has: the lines stay, so the owner
        // can see which changes will not be made.
        let orphan = waiting_page(&queue(), &accounts()[..1], false);
        assert_eq!((orphan.options[0].title.as_str(), orphan.options[0].enabled), ("Deck Player (deckplayer)", false));
        assert!(orphan.options.iter().any(|o| o.title == "Steam account 2" && !o.enabled));
        let mine = SteamQueue { entries: queue().entries.into_iter().filter(|c| c.account_id == 1).collect() };
        let one = waiting_page(&mine, &accounts()[..1], false);
        assert!(one.options.iter().all(|o| o.enabled), "one account on the Deck and only its changes: no account lines");
        assert_eq!(one.options[0].detail, "Will be added to Steam");
        assert_eq!(account_name(&accounts(), 9), "Steam account 9");
    }

    #[test]
    fn dropping_and_clearing_ask_first_with_no_first() {
        for p in [clear_page(3), drop_page(&queue().entries[1])] {
            assert!(p.fresh_focus, "starts on No every time, not where the last one was left");
            assert_eq!(p.options[0].action, SteamAct::No);
            assert_eq!(action_at(&p, initial_focus(&p)), Some(SteamAct::No));
            assert!(p.back_hint.starts_with("No"), "{}", p.back_hint);
            assert!(p.paragraphs.iter().any(|t| t.contains("Your Steam shortcuts list is not touched")));
        }
        assert_eq!(clear_page(3).paragraphs[0], "All 3 waiting changes are dropped and not made. Your Steam shortcuts list is not touched.");
    }

    #[test]
    fn the_review_lists_what_is_added_and_says_the_list_is_backed_up() {
        let p = review_page(&["Dusklight".to_string()], &["Ship of Harkinian".to_string()]);
        assert_eq!(p.title, "Add these to Steam?");
        assert_eq!(p.facts, ["Add Dusklight", "Update Ship of Harkinian"]);
        assert!(p.paragraphs.iter().any(|t| t == "Your Steam shortcuts list is backed up first."));
        assert_eq!(p.options[0].title, "Yes, add them");
        assert_eq!(p.back_hint, "Not now");
        let many: Vec<String> = (0..9).map(|i| format!("P{i}")).collect();
        assert_eq!(review_page(&many, &[]).facts.last().map(String::as_str), Some("And 4 more"));
    }

    #[test]
    fn the_wait_question_says_when_they_are_made() {
        let q = queue();
        let g = wait_page(&q.entries, true, None);
        assert_eq!(g.title, "Add to Steam in Desktop Mode?");
        assert_eq!(g.options[0].title, "Yes, let them wait");
        assert_eq!(wait_page(&q.entries, false, None).title, "Add when Steam is closed?");
        assert!(g.facts[0].ends_with("will be added to Steam") && g.facts[0].chars().count() <= 66, "{}", g.facts[0]);
        assert_eq!(drop_page(&q.entries[2]).paragraphs[0], "The Steam shortcut of Dusk will follow its new folder.");
        assert_eq!(drop_page(&change("Dusklight", QueuedKind::Add, 1)).paragraphs[0], "Dusklight will be added to Steam.");
    }
}
