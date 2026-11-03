//! The Port page's questions and waiting pages: which version to install, whether to install the extra files, whether to
//! reset the play time. Each replaces an upstream dialog ("Loading", "Choose a version", "Error", "Install extras - X",
//! "Extras", "Reset Game Time - X") with pages in the Port page's own style: the port's name at the top left, the card's
//! name in the window title, one question, plain words, a Back that always leads somewhere.
//!
//! Pure data and words; `src/app/port_actions_screen.rs` runs them and the work behind them.

use super::{window_title, Card};
use crate::fork::page::{Focus, Page, PageButton, PageOption, LEFT};
use crate::fork::rom_screens::util::ellipsize;
use serde_json::Value;

/// What a choice on one of these pages means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The yes of a yes/no page.
    Yes,
    /// The release the user picked: its place in the list.
    Pick(usize),
    TryAgain,
    /// The no of a yes/no page, the Back button, B.
    Back,
}

/// How many characters of a full-screen page's title fit on its one line.
pub const TITLE_FIT: usize = crate::fork::install_wizard::TITLE_FIT;

fn option(title: &str, detail: impl Into<String>, action: Step) -> PageOption<Step> {
    PageOption { title: title.to_string(), detail: detail.into(), choice: None, enabled: true, recommended: false, action }
}

fn recommended(mut o: PageOption<Step>) -> PageOption<Step> {
    o.recommended = true;
    o
}

fn back_button(label: &str) -> [Option<PageButton<Step>>; 4] {
    [Some(PageButton { label: label.to_string(), action: Step::Back }), None, None, None]
}

/// A page of a card: the port's name at the top left, the card in the window title, `title` as the page's title.
fn base(name: &str, card: Card, title: &str) -> Page<Step> {
    Page {
        window_title: format!("{}: {title}", window_title(name, Some(card))),
        large: true,
        header: name.to_string(),
        title: title.to_string(),
        buttons: back_button("Back"),
        initial: Focus::Option(0),
        ..Page::default()
    }
}

/// "<rest>" with the name in it when that fits the title line, else `short`.
fn title_with_name(before: &str, name: &str, after: &str, short: &str) -> String {
    let full = format!("{before}{name}{after}");
    if full.chars().count() <= TITLE_FIT { full } else { short.to_string() }
}

// -- Select version ---------------------------------------------------------------------------------------------------

/// One release in the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub name: String,
    /// "2026-10-01", when the release says.
    pub date: Option<String>,
    pub installed: bool,
    pub newest: bool,
    /// Marked as a test release by its maker.
    pub early: bool,
}

/// The versions in a forge's answer, newest first, with the installed one marked. GitHub and GitLab name the tag
/// `tag_name`; the date is `published_at` (GitHub) or `released_at` (GitLab), read as its first ten characters.
pub fn versions_of(releases: &[Value], installed_tag: Option<&str>) -> Vec<Version> {
    releases
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let name = r.get("tag_name").and_then(Value::as_str).unwrap_or("?").to_string();
            let date = ["published_at", "released_at", "created_at"]
                .iter()
                .find_map(|k| r.get(*k).and_then(Value::as_str))
                .map(|d| d.chars().take(10).collect::<String>())
                .filter(|d| d.len() == 10 && d.as_bytes()[4] == b'-');
            Version { installed: installed_tag == Some(name.as_str()), name, date, newest: i == 0, early: r.get("prerelease").and_then(Value::as_bool).unwrap_or(false) }
        })
        .collect()
}

/// The line under a version's name: "Released 2026-10-01, newest, installed".
pub fn version_detail(v: &Version) -> String {
    let mut parts = vec![match &v.date {
        Some(d) => format!("Released {d}"),
        None => "Release date not listed".to_string(),
    }];
    if v.newest {
        parts.push("newest".to_string());
    }
    if v.installed {
        parts.push("installed".to_string());
    }
    if v.early {
        parts.push("early version, may be rough".to_string());
    }
    parts.join(", ")
}

/// While the versions are fetched: B or Cancel gives up.
pub fn versions_wait_page(name: &str) -> Page<Step> {
    let mut p = base(name, Card::Updates, "Getting the versions");
    p.paragraphs.push(format!("Looking up the newest versions of {name}. This takes a moment."));
    p.buttons = back_button("Cancel");
    p.initial = Focus::Button(LEFT);
    p.b_means("Cancel").with_progress(None)
}

/// The versions to choose from, with their dates. The one chosen is installed over the one there; update checks go off so
/// it stays (the install pins the version it was asked for).
pub fn versions_page(name: &str, versions: &[Version]) -> Page<Step> {
    let mut p = base(name, Card::Updates, "Which version?");
    p.paragraphs.push(format!("The newest versions of {name}. The one you choose is installed over the one you have; your saves are backed up first."));
    p.paragraphs.push("Update checks for this port are then switched off, so it stays on that version. Switch them on again under Updates.".to_string());
    p.options = versions.iter().enumerate().map(|(i, v)| option(&ellipsize(&v.name, 40), version_detail(v), Step::Pick(i))).collect();
    p.footnote = "Only the newest few versions are listed.".to_string();
    p.b_means("Back")
}

/// Why the versions could not be had, in plain words. The raw answer (an HTTP code, a library's text) is for the log.
pub fn versions_reason(raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.contains("rate limit") {
        "GitHub is limiting how often this Deck may ask. Try again in a while.".to_string()
    } else if lower.contains("no version history") {
        "This port does not list versions, so there is nothing to choose from.".to_string()
    } else if lower.contains("no release found") || lower.contains("http 404") {
        "No versions are listed for this port.".to_string()
    } else {
        "The server did not answer. Check the connection and try again.".to_string()
    }
}

/// The versions could not be had: why, a way to ask again, and the way back.
pub fn versions_failed_page(name: &str, raw: &str) -> Page<Step> {
    let mut p = base(name, Card::Updates, "Could not get the versions");
    p.paragraphs.push(format!("Could not get the versions of {name}. {}", versions_reason(raw)));
    p.options = vec![recommended(option("Try again", "Asks for the versions once more", Step::TryAgain))];
    p.b_means("Back")
}

/// A fetch of the version list answers with the port's key and a request number (`key` + separator + number), so an answer
/// that arrives after Cancel, or after a newer request, is recognised and ignored instead of shown or counted.
pub fn tag_request(key: &str, id: u64) -> String {
    format!("{key}\u{1f}{id}")
}

/// The port's key and the request number of a tagged answer (`None` for a key that carries none).
pub fn split_request(tagged: &str) -> (&str, Option<u64>) {
    match tagged.rsplit_once('\u{1f}') {
        Some((key, n)) => (key, n.parse().ok()),
        None => (tagged, None),
    }
}

// -- Install extras ---------------------------------------------------------------------------------------------------

/// "Install the extra files?": what extras are and that they replace copies already there. The safe answer is the one the
/// cursor starts on.
pub fn extras_ask_page(name: &str) -> Page<Step> {
    let mut p = base(name, Card::Updates, "Install the extra files?");
    p.paragraphs.push(format!("Extras are optional add-ons the catalog lists for {name}, such as extra play options or ready-made settings."));
    p.paragraphs.push("They are downloaded into its folder and replace any copies of the same files that are already there.".to_string());
    p.options = vec![
        option("Yes, install the extras", "Replaces copies of these files already in its folder", Step::Yes),
        recommended(option("No, leave it as it is", "Nothing is downloaded", Step::Back)),
    ];
    p.initial = Focus::Option(1);
    p.b_means("No")
}

/// While the extras download: nothing to choose, B waits.
pub fn extras_wait_page(name: &str) -> Page<Step> {
    let mut p = base(name, Card::Updates, "Getting the extras");
    p.paragraphs.push(format!("Downloading the extra files for {name} into its folder. This takes a moment."));
    p.buttons = [None, None, None, None];
    p.no_back = true;
    p.with_progress(None)
}

/// Why the extras could not be had, in plain words.
pub fn extras_reason(raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.contains("install this port first") {
        "Install the port first, then add its extra files.".to_string()
    } else if lower.contains("no \"extra\" files") {
        "This port has no extra files to install.".to_string()
    } else if lower.contains("unexpected layout") {
        "The extras came in a layout this app does not know how to unpack.".to_string()
    } else {
        "The server did not answer, or the download was cut off. Check the connection and try again.".to_string()
    }
}

/// Put in front of an extras failure that happened after the files started to be put in place (the install says so with
/// its "Putting the extra files in place..." progress line): some of its files may already be replaced.
pub const PARTWAY: &str = "\u{1}partway:";

/// The result: done, or why not. "Nothing was changed" is said only for a failure before the files were put in place.
pub fn extras_done_page(name: &str, result: Result<(), &str>) -> Page<Step> {
    let mut p = match result {
        Ok(()) => {
            let mut p = base(name, Card::Updates, "Extras installed");
            p.paragraphs.push(format!("The extra files for {name} are in its folder."));
            p
        }
        Err(raw) => {
            let partway = raw.starts_with(PARTWAY);
            let mut p = base(name, Card::Updates, "Could not install the extras");
            p.paragraphs.push(format!("The extra files for {name} were not {}installed. {}", if partway { "all " } else { "" }, extras_reason(raw.trim_start_matches(PARTWAY))));
            p.paragraphs.push(if partway { "Some of its files may already have been replaced." } else { "Nothing was changed." }.to_string());
            p
        }
    };
    p.buttons = [None, None, None, None];
    p.options = vec![recommended(option("Back", "Returns to the Updates page", Step::Back))];
    p.b_means("Back")
}

// -- Reset play time --------------------------------------------------------------------------------------------------

/// "Reset the play time for <name>?": the time it is now, and that this cannot be undone. Keeping it is the answer the
/// cursor starts on.
pub fn reset_ask_page(name: &str, played: &str) -> Page<Step> {
    let title = title_with_name("Reset the play time for ", name, "?", "Reset the play time?");
    let mut p = base(name, Card::More, &title);
    p.paragraphs.push(format!("The play time is {played} now for {name}. This cannot be undone."));
    p.options = vec![
        option("Yes, reset it", "The play time goes back to zero", Step::Yes),
        recommended(option("No, keep it", "Nothing is changed", Step::Back)),
    ];
    p.initial = Focus::Option(1);
    p.b_means("No")
}

/// The result: back to zero.
pub fn reset_done_page(name: &str) -> Page<Step> {
    let mut p = base(name, Card::More, "Play time reset");
    p.paragraphs.push(format!("The play time for {name} is back to zero."));
    p.options = vec![recommended(option("Back", "Returns to More", Step::Back))];
    p.buttons = [None, None, None, None];
    p.b_means("Back")
}

// -- A job that cannot start now ----------------------------------------------------------------------------------------

/// Another job holds the port (it runs, installs, updates or has its saves restored).
pub fn busy_page(name: &str, card: Card) -> Page<Step> {
    let mut p = base(name, card, "Not right now");
    p.paragraphs.push(format!("{name} is running, installing, updating or restoring right now. Try again when it has finished."));
    p.options = vec![recommended(option("Back", "Nothing was changed", Step::Back))];
    p.buttons = [None, None, None, None];
    p.b_means("Back")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::page::{action_at, initial_focus};
    use serde_json::json;

    const NAMES: [&str; 4] = ["X", "The Legend of Dragoon", "The Wind Waker HD", "The Legend of Zelda: The Wind Waker HD (ZeldaWWHDRecomp)"];

    fn versions() -> Vec<Version> {
        versions_of(
            &[
                json!({"tag_name": "v2.1.0-rc1", "published_at": "2026-10-28T10:00:00Z", "prerelease": true}),
                json!({"tag_name": "v2.0.3", "published_at": "2026-09-12T08:30:00Z"}),
                json!({"tag_name": "a-very-long-release-name-that-goes-on-and-on-past-the-limit-v1", "created_at": "2026-01-02T00:00:00Z"}),
            ],
            Some("v2.0.3"),
        )
    }

    fn all_pages() -> Vec<Page<Step>> {
        let mut pages = Vec::new();
        for name in NAMES {
            pages.extend([
                versions_wait_page(name),
                versions_page(name, &versions()),
                versions_failed_page(name, "GitHub API rate limit reached. Add a token in state.json."),
                versions_failed_page(name, "This source has no version history."),
                extras_ask_page(name),
                extras_wait_page(name),
                extras_done_page(name, Ok(())),
                extras_done_page(name, Err("error sending request for url (https://example.com/extra.zip)")),
                reset_ask_page(name, "165.5 h"),
                reset_done_page(name),
                busy_page(name, Card::Updates),
                busy_page(name, Card::More),
            ]);
        }
        pages
    }

    /// The page's title is one line (about 40 characters) whatever the name; an option's title (40) and detail (66) are one
    /// line each; no em dashes, no jargon, no double hyphen.
    #[test]
    fn every_page_fits_with_the_longest_real_names() {
        for p in all_pages() {
            assert!(p.title.chars().count() <= TITLE_FIT, "title too long ({}): {}", p.title.chars().count(), p.title);
            for o in &p.options {
                assert!(o.title.chars().count() <= 40, "{:?}: option title too long: {}", p.title, o.title);
                assert!(o.detail.chars().count() <= 66, "{:?}: detail too long ({}): {}", p.title, o.detail.chars().count(), o.detail);
                assert!(!o.title.is_empty());
            }
            for b in p.buttons.iter().flatten() {
                assert!(b.label.chars().count() <= 16, "{}", b.label);
            }
            assert!(p.window_title.starts_with("Port: "), "{}", p.window_title);
            let words: Vec<&String> = p.paragraphs.iter().chain(p.options.iter().map(|o| &o.detail)).chain(p.options.iter().map(|o| &o.title)).collect();
            for t in words {
                assert!(!t.contains('\u{2014}') && !t.contains("--"), "no em dash or double hyphen: {t}");
                for jargon in ["tag ", "asset", "executable", "upstream", "json", "HTTP", "token", "ureq"] {
                    assert!(!t.contains(jargon), "jargon {jargon:?}: {t}");
                }
            }
            assert!(!p.title.contains("Error") && !p.title.contains("Couldn't"), "{}", p.title);
        }
    }

    #[test]
    fn the_window_title_names_the_port_for_the_dim_layer() {
        for name in NAMES {
            let p = versions_page(name, &versions());
            assert_eq!(crate::fork::port_page::port_in_window_title(&p.window_title).as_deref(), Some(name), "{}", p.window_title);
        }
    }

    #[test]
    fn versions_read_from_both_forges_with_dates_newest_and_installed() {
        let v = versions();
        assert_eq!(v.iter().map(|v| v.name.as_str()).collect::<Vec<_>>()[..2], ["v2.1.0-rc1", "v2.0.3"]);
        assert_eq!(version_detail(&v[0]), "Released 2026-10-28, newest, early version, may be rough");
        assert_eq!(version_detail(&v[1]), "Released 2026-09-12, installed");
        assert_eq!(version_detail(&v[2]), "Released 2026-01-02");
        let gitlab = versions_of(&[json!({"tag_name": "1.0", "released_at": "2026-03-04T00:00:00Z"}), json!({"tag_name": "0.9"})], None);
        assert_eq!(version_detail(&gitlab[0]), "Released 2026-03-04, newest");
        assert_eq!(version_detail(&gitlab[1]), "Release date not listed");
    }

    #[test]
    fn the_version_page_lists_each_release_and_b_goes_back() {
        let p = versions_page("Dusklight", &versions());
        assert_eq!(p.title, "Which version?");
        assert_eq!(p.options.iter().map(|o| o.action).collect::<Vec<_>>(), [Step::Pick(0), Step::Pick(1), Step::Pick(2)]);
        assert_eq!(p.options[0].title, "v2.1.0-rc1");
        assert!(p.options[2].title.contains("..."), "a very long name is cut for its line");
        assert!(p.paragraphs[1].contains("switched off"), "the page says picking a version turns update checks off");
        assert_eq!(p.back_hint, "Back");
        assert_eq!(p.buttons[LEFT].as_ref().unwrap().action, Step::Back);
    }

    #[test]
    fn waiting_for_the_versions_can_be_given_up() {
        let p = versions_wait_page("Dusklight");
        assert_eq!(p.title, "Getting the versions");
        assert!(p.progress.is_some() && p.options.is_empty() && !p.no_back);
        assert_eq!((p.buttons[LEFT].as_ref().unwrap().label.as_str(), p.back_hint.as_str()), ("Cancel", "Cancel"));
    }

    #[test]
    fn a_failure_is_a_plain_sentence_never_the_raw_answer() {
        let p = versions_failed_page("Dusklight", "GitHub error (HTTP 500)");
        assert_eq!(p.title, "Could not get the versions");
        assert_eq!(p.paragraphs, ["Could not get the versions of Dusklight. The server did not answer. Check the connection and try again."]);
        assert_eq!(p.options.iter().map(|o| o.action).collect::<Vec<_>>(), [Step::TryAgain]);
        assert!(versions_reason("GitHub API rate limit reached. Add a token in state.json.").starts_with("GitHub is limiting"));
        assert_eq!(versions_reason("This source has no version history."), "This port does not list versions, so there is nothing to choose from.");
        assert_eq!(versions_reason("No release found for a/b."), "No versions are listed for this port.");
        assert_eq!(versions_reason("GitHub error (HTTP 404)"), "No versions are listed for this port.");
    }

    #[test]
    fn extras_say_what_they_are_and_start_on_no() {
        let p = extras_ask_page("Dusklight");
        assert_eq!(p.title, "Install the extra files?");
        assert!(p.paragraphs[0].starts_with("Extras are optional add-ons"));
        assert!(p.paragraphs[1].contains("replace any copies"));
        assert_eq!(p.options.iter().map(|o| o.title.as_str()).collect::<Vec<_>>(), ["Yes, install the extras", "No, leave it as it is"]);
        assert_eq!(action_at(&p, initial_focus(&p)), Some(Step::Back), "replacing files is never the answer the cursor starts on");
        assert!(p.options[1].recommended && !p.options[0].recommended);
        assert_eq!(p.back_hint, "No");
    }

    #[test]
    fn nothing_was_changed_is_only_said_when_it_is_true() {
        let before = extras_done_page("Dusklight", Err("http status: 404"));
        assert_eq!(before.paragraphs[1], "Nothing was changed.");
        let partway = extras_done_page("Dusklight", Err(&format!("{PARTWAY}Permission denied (os error 13)")));
        assert_eq!(partway.paragraphs[1], "Some of its files may already have been replaced.");
        assert!(partway.paragraphs.iter().all(|t| !t.contains("Nothing was changed") && !t.contains("os error") && !t.contains('\u{1}')), "{:?}", partway.paragraphs);
    }

    #[test]
    fn extras_end_on_a_page_never_a_raw_error() {
        let ok = extras_done_page("Dusklight", Ok(()));
        assert_eq!((ok.title.as_str(), ok.paragraphs[0].as_str()), ("Extras installed", "The extra files for Dusklight are in its folder."));
        let bad = extras_done_page("Dusklight", Err("error sending request for url (https://x/y.zip): connection refused"));
        assert_eq!(bad.title, "Could not install the extras");
        assert!(bad.paragraphs.iter().all(|t| !t.contains("https://") && !t.contains("connection refused")));
        assert_eq!(bad.paragraphs[1], "Nothing was changed.");
        assert!(extras_wait_page("X").no_back);
        assert_eq!(extras_reason("Install this port first, then add its extra files."), "Install the port first, then add its extra files.");
    }

    #[test]
    fn reset_play_time_asks_by_name_and_says_it_cannot_be_undone() {
        let p = reset_ask_page("Dusklight", "2.1 h");
        assert_eq!(p.title, "Reset the play time for Dusklight?");
        assert_eq!(p.paragraphs, ["The play time is 2.1 h now for Dusklight. This cannot be undone."]);
        assert_eq!(p.options.iter().map(|o| (o.title.as_str(), o.action)).collect::<Vec<_>>(), [("Yes, reset it", Step::Yes), ("No, keep it", Step::Back)]);
        assert_eq!(action_at(&p, initial_focus(&p)), Some(Step::Back));
        let long = reset_ask_page(NAMES[3], "2.1 h");
        assert_eq!(long.title, "Reset the play time?");
        assert!(long.paragraphs[0].contains(NAMES[3]), "the name is always in the text");
        assert_eq!(reset_done_page("Dusklight").title, "Play time reset");
    }

    #[test]
    fn a_version_answer_carries_its_request_number() {
        let tagged = tag_request("owner/repo", 7);
        assert_eq!(split_request(&tagged), ("owner/repo", Some(7)));
        assert_eq!(split_request("owner/repo"), ("owner/repo", None));
        assert_ne!(tag_request("a", 1), tag_request("a", 2), "a late answer is not taken for a newer request's");
        assert_eq!(split_request(&tag_request("a/b\u{1f}c", 3)), ("a/b\u{1f}c", Some(3)), "the last separator counts");
    }

    #[test]
    fn a_busy_port_says_so() {
        let p = busy_page("Dusklight", Card::Updates);
        assert_eq!(p.title, "Not right now");
        assert!(p.paragraphs[0].starts_with("Dusklight is running, installing"));
    }
}
