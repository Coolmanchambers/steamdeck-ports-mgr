//! Settings > Steam > Artwork sources as pages, one question each: the sources as cards (what each one is and how it
//! stands: "Set up", "Working", or its last problem in plain words), a page per source, a text box page for a key or
//! login, a yes/no page before a saved key is removed, a waiting page and a result page for the key test, the credits as
//! a page of short facts, look-alike matches as a choice page ("None of these" last), and fetching as a progress page
//! with a Stop card, then a done page in plain sentences.
//!
//! A saved key is never shown in full: a page says one is saved and, for a long one, its last 4 characters.
//!
//! Pure data and words; `src/app/art_screens.rs` runs them. What is saved, where, and what is downloaded is
//! `crate::fork::art::screen` (unchanged).

use super::screen::{ArtScreen, FetchSummary, PendingChoice, Readiness, SourceStatus, MAX_PROBLEMS};
use super::source::{human_wait, KeyCheck, SourceId};
use crate::fork::page::{Focus, Mark, Page, PageButton, PageOption, InputBox, LEFT};
use crate::fork::rom_screens::util::ellipsize;

/// The menu's window title (the smoke test looks for it); every other page's starts with it.
pub const TITLE: &str = "Artwork sources";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtAct {
    /// Menu: a source's page.
    Source(SourceId),
    Fetch,
    Pending,
    Credits,
    /// Up one level.
    Back,
    /// A source's page: enter or change its key (or login).
    EnterKey,
    Test,
    /// The text box page.
    Save,
    Remove,
    Cancel,
    /// The remove question.
    RemoveYes,
    RemoveNo,
    /// A notice, a result, the credits, the done page.
    Done,
    /// The fetching page.
    Stop,
    /// The look-alike list: port group `n`.
    Port(usize),
    /// A look-alike page: candidate `n`.
    Pick(usize),
    NoneOfThese,
}

fn option(title: impl Into<String>, detail: impl Into<String>, action: ArtAct) -> PageOption<ArtAct> {
    PageOption { title: title.into(), detail: detail.into(), choice: None, enabled: true, recommended: false, action }
}

fn base(window: &str, title: &str) -> Page<ArtAct> {
    Page { window_title: window.to_string(), large: true, header: TITLE.to_string(), title: title.to_string(), ..Page::default() }
}

fn back_button(p: &mut Page<ArtAct>) {
    p.buttons[LEFT] = Some(PageButton { label: "Back".to_string(), action: ArtAct::Back });
}

fn window_of(src: SourceId) -> String {
    format!("{TITLE}: {}", src.label())
}

/// One option to close a page with ("Done", "OK"), the cursor on it and B doing the same.
fn done_option(mut p: Page<ArtAct>, label: &str) -> Page<ArtAct> {
    p.options.push(PageOption { recommended: true, ..option(label, "", ArtAct::Done) });
    p.initial = Focus::Option(p.options.len() - 1);
    p.b_does(&ArtAct::Done)
}

/// The last 4 characters of a long saved value; a short one gives nothing away.
fn tail(value: &str) -> Option<String> {
    let chars: Vec<char> = value.chars().collect();
    (chars.len() > 8).then(|| chars[chars.len() - 4..].iter().collect())
}

/// A problem a source had, in plain words (the raw text stays in the app's own record and the log).
pub fn plain_problem(raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.contains("invalid") || lower.contains("not accept") || lower.contains("401") || lower.contains("403") {
        "the key was not accepted".to_string()
    } else if lower.contains("offline") || lower.contains("connect") || lower.contains("timed out") || lower.contains("dns") {
        "could not reach the site".to_string()
    } else if lower.contains("busy") || lower.contains("limit") || lower.contains("429") || lower.contains("quota") {
        "the site was too busy".to_string()
    } else if lower.contains("download") {
        "a picture could not be downloaded".to_string()
    } else {
        "the site sent an answer the app did not expect".to_string()
    }
}

/// What a source card says under its name.
pub fn source_detail(st: &SourceStatus) -> String {
    if st.readiness == Readiness::NotInThisBuild {
        return "Not available in this version of the app".to_string();
    }
    if let Some((secs, _)) = &st.waiting {
        return format!("Paused for {}: the site asked the app to slow down", human_wait(*secs));
    }
    if let Some(err) = &st.last_error {
        return format!("Last problem: {}", plain_problem(err));
    }
    match (st.readiness, st.source) {
        (Readiness::Configured, _) if st.last_test.as_deref().is_some_and(|t| t.starts_with("OK")) => "Working".to_string(),
        (Readiness::Configured, _) => "Set up".to_string(),
        (_, SourceId::SteamGridDb) => "Not set up: it needs your free key".to_string(),
        _ => "Not set up (optional)".to_string(),
    }
}

/// What each source is, in a sentence.
fn source_about(src: SourceId) -> &'static str {
    match src {
        SourceId::SteamGridDb => "Community covers, banners, logos and icons. It needs a free key of your own: sign in on steamgriddb.com, open Preferences, and copy the key from there.",
        SourceId::TheGamesDb => "Community box art, banners and logos. A key of your own is optional.",
        SourceId::ScreenScraper => "Box art and logos matched to your own game files. A login of your own is optional; it lets the app ask for more at a time.",
    }
}

fn is_login(src: SourceId) -> bool {
    src == SourceId::ScreenScraper
}

/// The first page: a card per source, then Fetch artwork, the look-alikes waiting (when there are some) and the credits.
pub fn menu_page(screen: &ArtScreen) -> Page<ArtAct> {
    let mut p = base(TITLE, TITLE);
    p.header = "Settings".to_string();
    p.paragraphs.push("The pictures your ports get in Steam come from these sites.".to_string());
    if screen.secrets.problem.is_some() {
        p.fact(Mark::Problem, "The file that keeps your keys could not be used");
    }
    if screen.secrets.drive_note().is_some() {
        p.fact(Mark::Problem, "This drive cannot keep your saved keys private");
    }
    for src in SourceId::ALL {
        p.options.push(option(src.label(), source_detail(&screen.status(src)), ArtAct::Source(src)));
    }
    let fetch = if screen.any_ready() { "Get pictures for your installed ports" } else { "Set up SteamGridDB first" };
    p.options.push(option("Fetch artwork", fetch, ArtAct::Fetch));
    match screen.pending_count() {
        0 => {}
        1 => p.options.push(option("Choose between look-alikes", "1 port has more than one close match", ArtAct::Pending)),
        n => p.options.push(option("Choose between look-alikes", format!("{n} ports have more than one close match"), ArtAct::Pending)),
    }
    p.options.push(option("Where the artwork comes from", "The sites and what each one gives", ArtAct::Credits));
    back_button(&mut p);
    p.initial = Focus::Option(0);
    p
}

/// A source's page: what it is, what is saved, how the last test went, and Enter a key / Test.
pub fn source_page(screen: &ArtScreen, src: SourceId) -> Page<ArtAct> {
    let st = screen.status(src);
    let mut p = base(&window_of(src), src.label());
    p.paragraphs.push(source_about(src).to_string());
    let saved_key = match src {
        SourceId::SteamGridDb => screen.secrets.sgdb_key(),
        SourceId::TheGamesDb => screen.secrets.tgdb_key(),
        SourceId::ScreenScraper => None,
    };
    match src {
        SourceId::ScreenScraper => {
            match screen.secrets.screenscraper_user() {
                Some(user) => p.fact(Mark::Done, format!("Your login: {}", ellipsize(user, 30))),
                None => p.fact(Mark::None, "No login saved (optional)"),
            }
            if screen.secrets.screenscraper_user().is_some() {
                if screen.secrets.has_screenscraper_password() {
                    p.fact(Mark::Done, "Password saved");
                } else {
                    p.fact(Mark::ToDo, "No password saved yet");
                }
            }
        }
        _ => match (saved_key, src == SourceId::TheGamesDb && screen.build.tgdb_apikey.is_some()) {
            (Some(key), _) => p.fact(Mark::Done, tail(key).map_or_else(|| "A key is saved".to_string(), |t| format!("A key is saved, ending in {t}"))),
            (None, true) => p.fact(Mark::Done, "Uses the app's own key; yours is optional"),
            (None, false) if src == SourceId::SteamGridDb => p.fact(Mark::ToDo, "No key saved yet"),
            (None, false) => p.fact(Mark::None, "No key saved (optional)"),
        },
    }
    if st.readiness == Readiness::NotInThisBuild {
        p.fact(Mark::Problem, "This version of the app cannot use ScreenScraper");
    } else if let Some((secs, _)) = &st.waiting {
        p.fact(Mark::Waiting, format!("Paused for {}: the site asked the app to slow down", human_wait(*secs)));
    } else if let Some(err) = &st.last_error {
        p.fact(Mark::Problem, format!("Last problem: {}", plain_problem(err)));
    } else if st.last_test.as_deref().is_some_and(|t| t.starts_with("OK")) {
        p.fact(Mark::Done, "Last test: it worked");
    }
    let what = if is_login(src) { "login" } else { "key" };
    let has_one = if is_login(src) { screen.secrets.screenscraper_user().is_some() } else { saved_key.is_some() };
    let enter = match (is_login(src), has_one) {
        (true, true) => "Change your login",
        (true, false) => "Enter your login",
        (false, true) => "Change the key",
        (false, false) => "Enter a key",
    };
    p.options.push(option(enter, "Paste it or type it in", ArtAct::EnterKey));
    let can_test = st.readiness == Readiness::Configured;
    let test_detail = match st.readiness {
        Readiness::Configured => "One quick check with the site".to_string(),
        Readiness::NotInThisBuild => "Not available in this version of the app".to_string(),
        Readiness::NotConfigured => format!("Enter a {what} first"),
    };
    p.options.push(PageOption { enabled: can_test, ..option(format!("Test the {what}"), test_detail, ArtAct::Test) });
    back_button(&mut p);
    p.initial = Focus::Option(if can_test && has_one { 1 } else { 0 });
    p
}

/// Which text a key page asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyField {
    Key(SourceId),
    User,
    Password,
}

impl KeyField {
    fn words(self) -> (String, &'static str) {
        match self {
            KeyField::Key(src) => (format!("Your {} key", src.label()), "the key"),
            KeyField::User => ("Your ScreenScraper user name".to_string(), "your user name"),
            KeyField::Password => ("Your ScreenScraper password".to_string(), "your password"),
        }
    }

    pub fn source(self) -> SourceId {
        match self {
            KeyField::Key(src) => src,
            KeyField::User | KeyField::Password => SourceId::ScreenScraper,
        }
    }
}

/// The text box page. `saved` says what is saved now ("ending in 0000", or just that there is one), never the value;
/// `text` is what the box holds (kept after a refusal, so a long paste is not lost); `problem` is why the last Save was
/// refused, in plain words.
pub fn key_page(field: KeyField, saved: Option<Option<String>>, text: &str, problem: Option<&str>, game_mode: bool) -> Page<ArtAct> {
    let (title, what) = field.words();
    let mut p = base(&format!("{}: enter", window_of(field.source())), &title);
    let keyboard = if game_mode { " Steam + X brings up the keyboard." } else { "" };
    p.paragraphs.push(format!("Paste or type {what}, then choose Save.{keyboard}"));
    match &saved {
        Some(Some(t)) => p.fact(Mark::Done, format!("One is saved now, ending in {t}")),
        Some(None) => p.fact(Mark::Done, "One is saved now"),
        None => {}
    }
    if let Some(why) = problem {
        p.fact(Mark::Problem, format!("Not saved: {why}"));
    }
    p.input = Some(InputBox { placeholder: format!("Type {what} here"), text: text.to_string(), password: field != KeyField::User });
    p.options.push(PageOption { recommended: true, ..option("Save", "", ArtAct::Save) });
    if saved.is_some() {
        let remove = match field {
            KeyField::Key(_) => "Remove the saved key",
            KeyField::User => "Remove the saved login",
            KeyField::Password => "Remove the saved password",
        };
        p.options.push(option(remove, "", ArtAct::Remove));
    }
    p.options.push(option("Cancel", "Keep what is saved", ArtAct::Cancel));
    p.initial = Focus::Input;
    // Back in the text box every time, never on Save.
    p.fresh_focus = true;
    p.b_does(&ArtAct::Cancel)
}

/// Why a Save was refused, in plain words.
pub fn refused_reason(raw: &str) -> String {
    if raw.contains("does not look like") {
        "that does not look right (empty, too long, or odd characters)".to_string()
    } else {
        "the file that keeps your keys could not be written".to_string()
    }
}

/// Before a saved key (or login) is removed. No first.
pub fn remove_page(field: KeyField) -> Page<ArtAct> {
    let (thing, title) = match field {
        KeyField::Key(_) => ("key", "Remove the saved key?"),
        KeyField::User => ("login", "Remove the saved login?"),
        KeyField::Password => ("password", "Remove the saved password?"),
    };
    let mut p = base(&format!("{}: remove", window_of(field.source())), title);
    p.paragraphs.push(format!("The {} {thing} is removed from this Deck. Artwork already fetched stays.", field.source().label()));
    p.options = vec![option("No, keep it", "", ArtAct::RemoveNo), option("Yes, remove it", "", ArtAct::RemoveYes)];
    p.initial = Focus::Option(0);
    p.fresh_focus = true;
    p.b_does(&ArtAct::RemoveNo)
}

/// While the key test runs (a few seconds). B goes back to the source's page.
pub fn testing_page(src: SourceId) -> Page<ArtAct> {
    let what = if is_login(src) { "login" } else { "key" };
    let mut p = base(&format!("{}: testing", window_of(src)), &format!("Testing the {what}"));
    let host = match src {
        SourceId::SteamGridDb => "steamgriddb.com",
        SourceId::TheGamesDb => "thegamesdb.net",
        SourceId::ScreenScraper => "screenscraper.fr",
    };
    p.paragraphs.push(format!("One quick check with {host}."));
    p.with_progress(None)
}

/// How the key test went.
pub fn test_result_page(src: SourceId, check: &KeyCheck) -> Page<ArtAct> {
    let what = if is_login(src) { "login" } else { "key" };
    let site = src.label();
    let (title, text) = match check {
        KeyCheck::Ok(_) => (format!("The {what} works"), format!("{site} accepted it. Artwork can be fetched now.")),
        KeyCheck::Invalid(_) => (format!("The {what} was not accepted"), format!("{site} did not accept it. Check it for typing mistakes and enter it again.")),
        KeyCheck::Offline(_) => (format!("Could not reach {site}"), format!("The {what} was not checked. Try again when the Deck is online.")),
        KeyCheck::Unknown(_) => (format!("Could not check the {what}"), format!("{site} is busy or having trouble right now. The {what} may be fine; try again later.")),
        KeyCheck::NotConfigured(_) if src == SourceId::ScreenScraper => ("Nothing to test".to_string(), "This version of the app cannot use ScreenScraper.".to_string()),
        KeyCheck::NotConfigured(_) => ("Nothing to test yet".to_string(), format!("Enter a {what} first.")),
    };
    let mut p = base(&format!("{}: test result", window_of(src)), &title);
    p.paragraphs.push(text);
    done_option(p, "Done")
}

/// The credits: the sites, what each gives, where the pictures are kept.
pub fn credits_page(cache_line: &str) -> Page<ArtAct> {
    let mut p = base(&format!("{TITLE}: credits"), "Where the artwork comes from");
    p.paragraphs.push("Artwork is fetched on this Deck and kept in your Steam folder and in the app's own folder. Nothing is uploaded or shared.".to_string());
    p.facts = vec![
        "SteamGridDB (steamgriddb.com): covers, banners, logos, icons".to_string(),
        "TheGamesDB (thegamesdb.net): box art, banners, fan art, logos".to_string(),
        "ScreenScraper (screenscraper.fr): box art and logos".to_string(),
    ];
    if !cache_line.is_empty() {
        p.facts.push(cache_line.to_string());
    }
    p.paragraphs.push("The pictures belong to their artists and the games' makers. Thank you to the volunteers who run these sites.".to_string());
    done_option(p, "Done")
}

/// Something to say, then back (no key yet, nothing installed, a task still running).
pub fn notice_page(window: &str, title: &str, text: &str) -> Page<ArtAct> {
    let mut p = base(&format!("{TITLE}: {window}"), title);
    p.paragraphs = text.split("\n\n").map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect();
    done_option(p, "OK")
}

// -- Look-alikes -------------------------------------------------------------------------------------------------------

/// The look-alikes waiting, one group per port and source, in the order found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookAlikes {
    pub port_key: String,
    pub port_name: String,
    pub source: SourceId,
    pub choices: Vec<PendingChoice>,
}

pub fn group_lookalikes(choices: &[PendingChoice]) -> Vec<LookAlikes> {
    let mut out: Vec<LookAlikes> = Vec::new();
    for c in choices {
        match out.iter_mut().find(|g| g.port_key == c.port_key && g.source == c.source) {
            Some(g) => g.choices.push(c.clone()),
            None => out.push(LookAlikes { port_key: c.port_key.clone(), port_name: c.port_name.clone(), source: c.source, choices: vec![c.clone()] }),
        }
    }
    out
}

/// More than one port waiting: which one to look at first.
pub fn lookalike_list_page(groups: &[LookAlikes]) -> Page<ArtAct> {
    let mut p = base(&format!("{TITLE}: look-alikes"), "Choose between look-alikes");
    p.paragraphs.push("A site found more than one close match for these ports. Choose the right one for each.".to_string());
    for (i, g) in groups.iter().enumerate() {
        p.options.push(option(ellipsize(&g.port_name, 40), format!("{} from {}", many(g.choices.len(), "close match", "close matches"), g.source.label()), ArtAct::Port(i)));
    }
    back_button(&mut p);
    p.initial = Focus::Option(0);
    p
}

/// What the site says about a look-alike: the year it came out, and whether the site checked the entry itself.
pub fn lookalike_detail(c: &PendingChoice) -> String {
    match (c.year, c.verified) {
        (Some(y), Some(true)) => format!("Released {y}, checked by the site"),
        (Some(y), _) => format!("Released {y}"),
        (None, Some(true)) => "Checked by the site".to_string(),
        _ => "No more details from the site".to_string(),
    }
}

/// One port's look-alikes: one option per match, "None of these" last.
pub fn lookalike_page(group: &LookAlikes) -> Page<ArtAct> {
    let mut p = base(&format!("{TITLE}: look-alike"), &format!("Which one is {}?", ellipsize(&group.port_name, 26)));
    p.paragraphs.push(format!(
        "{} found more than one close match for {}. Choose the right one; its pictures are used the next time artwork is fetched.",
        group.source.label(),
        group.port_name
    ));
    for (i, c) in group.choices.iter().enumerate() {
        p.options.push(option(ellipsize(&c.name, 40), lookalike_detail(c), ArtAct::Pick(i)));
    }
    p.options.push(option("None of these", "Decide later; nothing is changed", ArtAct::NoneOfThese));
    p.initial = Focus::Option(0);
    p.b_does(&ArtAct::NoneOfThese)
}

/// After a look-alike was chosen (or could not be saved).
pub fn chosen_page(port_name: &str, picked: &str, problem: Option<&str>) -> Page<ArtAct> {
    let window = format!("{TITLE}: look-alike chosen");
    let p = match problem {
        None => {
            let mut p = base(&window, "Choice saved");
            p.paragraphs.push(format!("\"{picked}\" will be used for {port_name}. Fetch artwork to get its pictures."));
            p
        }
        Some(why) => {
            let mut p = base(&window, "The choice could not be saved");
            p.paragraphs.push(format!("Nothing was changed: {why}."));
            p
        }
    };
    done_option(p, "Done")
}

// -- Fetching ----------------------------------------------------------------------------------------------------------

/// While artwork is fetched: the port it is on, how far, and a Stop card (B stops too).
pub fn fetching_page(progress: &str, total: usize, stopping: bool) -> Page<ArtAct> {
    let mut p = base(&format!("{TITLE}: fetching"), "Fetching artwork");
    let (line1, line2) = progress.split_once('\n').unwrap_or((progress, ""));
    // "Port 2 of 5" and the port's name, from `screen::fetch_progress`.
    let at = line1.strip_prefix("Port ").and_then(|r| r.split_once(" of ")).and_then(|(i, _)| i.trim().parse::<usize>().ok());
    p.paragraphs.push(format!("Looking up pictures for your {} installed ports. Nothing is placed in Steam yet.", total));
    match (at, line2.trim()) {
        (Some(i), name) if !name.is_empty() => p.fact(Mark::None, format!("Port {i} of {total}: {}", ellipsize(name, 46))),
        _ => p.fact(Mark::None, "Starting..."),
    }
    let done = at.map_or(0, |i| i.saturating_sub(1));
    let fraction = if total == 0 { None } else { Some(done as f32 / total as f32) };
    let mut p = p.with_progress(fraction);
    if stopping {
        p.options.push(PageOption { enabled: false, ..option("Stopping...", "Finishing the port it is on", ArtAct::Stop) });
        p.no_back = true;
    } else {
        p.options.push(option("Stop", "What is done so far is kept", ArtAct::Stop));
        p.initial = Focus::Option(0);
        p = p.b_does(&ArtAct::Stop);
    }
    p
}

fn many(n: usize, one: &str, more: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {more}") }
}

/// "3 covers, 2 wide covers and 1 logo".
fn found_sentence(s: &FetchSummary) -> String {
    let words = [("portrait", "cover", "covers"), ("wide", "wide cover", "wide covers"), ("hero", "background", "backgrounds"), ("logo", "logo", "logos"), ("icon", "icon", "icons")];
    let parts: Vec<String> = words.iter().filter_map(|(slot, one, more)| s.found.get(slot).copied().filter(|n| *n > 0).map(|n| many(n, one, more))).collect();
    match parts.as_slice() {
        [] => "No new pictures were found.".to_string(),
        [one] => format!("Found {one}."),
        _ => format!("Found {} and {}.", parts[..parts.len() - 1].join(", "), parts[parts.len() - 1]),
    }
}

/// A fetch problem in plain words: "SteamGridDB: could not reach the site". One that names no source is the app's own
/// sentence and is shown as it is.
fn plain_fetch_problem(raw: &str) -> String {
    match raw.split_once(": ") {
        Some((site, rest)) if SourceId::ALL.iter().any(|s| s.label() == site) => format!("{site}: {}", plain_problem(rest)),
        _ => raw.to_string(),
    }
}

/// When it is over: what was looked at, what was found, the problems, and where the pictures go next.
pub fn fetched_page(s: &FetchSummary) -> Page<ArtAct> {
    let title = if s.stopped {
        "Fetching stopped"
    } else if !s.problems.is_empty() && s.found.values().all(|n| *n == 0) {
        "No artwork could be fetched"
    } else if !s.problems.is_empty() {
        "Artwork fetched, with problems"
    } else {
        "Artwork is ready"
    };
    let mut p = base(&format!("{TITLE}: fetched"), title);
    let ports = many(s.total, "installed port", "installed ports");
    p.paragraphs.push(if s.stopped { format!("Stopped after {} of your {ports}. What was fetched is kept.", s.looked_up) } else { format!("Looked at {} of your {ports}.", s.looked_up) });
    p.paragraphs.push(found_sentence(s));
    let mut shown: Vec<String> = Vec::new();
    for problem in s.problems.iter().take(MAX_PROBLEMS) {
        let line = plain_fetch_problem(problem);
        if !shown.contains(&line) {
            p.fact(Mark::Problem, ellipsize(&line, 66));
            shown.push(line);
        }
    }
    if s.problems.iter().any(|p| p.contains("offline")) {
        p.paragraphs.push("Try again when the Deck is online; nothing that was fetched is lost.".to_string());
    }
    if s.found.values().any(|n| *n > 0) {
        p.paragraphs.push("Add to Steam puts this artwork on the ports it adds. Artwork you chose yourself is never replaced.".to_string());
    }
    done_option(p, "Done")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::art::source::SourceError;
    use crate::fork::page::{action_at, initial_focus};
    use crate::fork::rom_screens::settings_pages::tests::{assert_fits, shown};
    use crate::fork::secrets::{BuildCreds, Field};

    const FAKE_KEY: &str = "test-key-0000";
    const LONG: &str = "The Legend of Zelda: The Wind Waker HD (ZeldaWWHDRecomp)";

    fn screen(name: &str) -> (tempfile::TempDir, ArtScreen) {
        let dir = tempfile::tempdir().unwrap();
        let (config, cache) = (dir.path().join(name), dir.path().join(name).join("cache"));
        std::fs::create_dir_all(&cache).unwrap();
        let s = ArtScreen::load(&config, &cache, BuildCreds::none(), 5_000);
        (dir, s)
    }

    /// No em dash, no file names, no paths, none of the words the owner does not use.
    fn assert_plain(p: &Page<ArtAct>) {
        for t in shown(p) {
            assert!(!t.contains('\u{2014}') && !t.contains("--"), "no em dash: {t}");
            let lower = t.to_lowercase();
            for word in ["json", "api key", "queue", "/run/media", "/home/", "~/", "(s)", "display()", "secrets", "ids.json", "sgdb_id", "invalid:", "offline:"] {
                assert!(!lower.contains(word), "{word:?} in {t:?} ({:?})", p.title);
            }
        }
        assert!(p.window_title.starts_with(TITLE), "{}", p.window_title);
    }

    fn check(p: &Page<ArtAct>) {
        assert_fits(p);
        assert_plain(p);
    }

    fn group(n: usize) -> LookAlikes {
        let choices = (0..n)
            .map(|i| PendingChoice { port_key: "k".into(), port_name: LONG.into(), folder: "z".into(), source: SourceId::SteamGridDb, id: 42 + i as u64, name: format!("{LONG} edition {i}"), year: Some(1995 + i as i32), verified: Some(i == 0) })
            .collect();
        LookAlikes { port_key: "k".into(), port_name: LONG.into(), source: SourceId::SteamGridDb, choices }
    }

    fn all_pages(s: &ArtScreen) -> Vec<Page<ArtAct>> {
        let mut summary = FetchSummary { looked_up: 3, total: 5, stopped: true, ..FetchSummary::default() };
        summary.found.insert("portrait", 3);
        summary.found.insert("hero", 1);
        summary.problems = (0..9).map(|i| format!("SteamGridDB: offline (could not connect) for a port with a very long name number {i}")).collect();
        let mut pages = vec![menu_page(s), credits_page(&s.cache_line()), notice_page("no key", "Set up a source first", "Fetching needs a SteamGridDB key.\n\nChoose SteamGridDB, then Enter a key.")];
        for src in SourceId::ALL {
            pages.push(source_page(s, src));
            pages.push(testing_page(src));
            for c in [KeyCheck::Ok(String::new()), KeyCheck::Invalid("x".into()), KeyCheck::Offline("x".into()), KeyCheck::Unknown("x".into()), KeyCheck::NotConfigured("x".into())] {
                pages.push(test_result_page(src, &c));
            }
        }
        for f in [KeyField::Key(SourceId::SteamGridDb), KeyField::Key(SourceId::TheGamesDb), KeyField::User, KeyField::Password] {
            pages.push(key_page(f, Some(Some("0000".into())), "abc", Some(&refused_reason("that does not look like a key")), true));
            pages.push(key_page(f, None, "", None, false));
            pages.push(remove_page(f));
        }
        pages.extend([lookalike_list_page(&[group(3), group(2)]), lookalike_page(&group(3)), chosen_page(LONG, LONG, None), chosen_page(LONG, LONG, Some("the drive is full"))]);
        pages.extend([fetching_page(&format!("Port 2 of 5\n{LONG}"), 5, false), fetching_page("", 5, true), fetched_page(&summary), fetched_page(&FetchSummary { looked_up: 5, total: 5, ..FetchSummary::default() })]);
        pages
    }

    #[test]
    fn every_page_fits_and_says_it_plainly() {
        let (_d, mut s) = screen("fit");
        for p in all_pages(&s) {
            check(&p);
        }
        s.set(Field::SteamGridDbKey, Some(FAKE_KEY)).unwrap();
        s.set(Field::ScreenScraperUser, Some("a-rather-long-user-name-for-a-login")).unwrap();
        s.states.record_check(SourceId::SteamGridDb, &KeyCheck::Invalid("SteamGridDB did not accept the API key".into()), 4_000, &[]);
        for p in all_pages(&s) {
            check(&p);
        }
    }

    #[test]
    fn a_saved_key_is_never_shown_in_full_only_its_last_four() {
        let (_d, mut s) = screen("masked");
        s.set(Field::SteamGridDbKey, Some(FAKE_KEY)).unwrap();
        s.set(Field::TheGamesDbKey, Some("test-key-1111")).unwrap();
        s.set(Field::ScreenScraperPassword, Some("test-pass-2222")).unwrap();
        let text: Vec<String> = all_pages(&s).iter().flat_map(shown).collect();
        let joined = text.join("\n");
        for secret in [FAKE_KEY, "test-key-1111", "test-pass-2222", "key-0000", "pass-2222"] {
            assert!(!joined.contains(secret), "{secret} leaked");
        }
        let sgdb = source_page(&s, SourceId::SteamGridDb);
        assert_eq!(sgdb.facts[0], "A key is saved, ending in 0000");
        let short = tail("abcd1234");
        assert_eq!(short, None, "a short value gives nothing away");
        let k = key_page(KeyField::Key(SourceId::SteamGridDb), Some(None), "", None, false);
        assert_eq!(k.facts, ["One is saved now"]);
        assert_eq!(k.input.as_ref().map(|i| i.text.as_str()), Some(""), "the box starts empty, never with the saved key");
        assert!(key_page(KeyField::Password, None, "", None, false).input.unwrap().password, "a password is typed as dots");
        assert!(key_page(KeyField::Key(SourceId::SteamGridDb), None, "", None, false).input.unwrap().password, "so is a key");
        assert!(!key_page(KeyField::User, None, "", None, false).input.unwrap().password, "a user name is not");
    }

    #[test]
    fn the_menu_has_a_card_per_source_with_where_it_stands() {
        let (_d, mut s) = screen("menu");
        let p = menu_page(&s);
        assert_eq!((p.window_title.as_str(), p.title.as_str()), (TITLE, TITLE));
        let cards: Vec<(&str, &str)> = p.options.iter().map(|o| (o.title.as_str(), o.detail.as_str())).collect();
        assert_eq!(
            cards,
            [
                ("SteamGridDB", "Not set up: it needs your free key"),
                ("TheGamesDB", "Not set up (optional)"),
                ("ScreenScraper", "Not available in this version of the app"),
                ("Fetch artwork", "Set up SteamGridDB first"),
                ("Where the artwork comes from", "The sites and what each one gives"),
            ]
        );
        assert_eq!(p.buttons[LEFT].as_ref().map(|b| b.action), Some(ArtAct::Back));
        s.set(Field::SteamGridDbKey, Some(FAKE_KEY)).unwrap();
        assert_eq!(menu_page(&s).options[0].detail, "Set up");
        s.states.record_check(SourceId::SteamGridDb, &KeyCheck::Ok(String::new()), 4_000, &[]);
        assert_eq!(menu_page(&s).options[0].detail, "Working");
        assert_eq!(menu_page(&s).options[3].detail, "Get pictures for your installed ports");
        s.states.record_check(SourceId::SteamGridDb, &KeyCheck::Invalid("SteamGridDB did not accept the API key".into()), 4_500, &[]);
        assert_eq!(menu_page(&s).options[0].detail, "Last problem: the key was not accepted");
        s.states.record_error(SourceId::SteamGridDb, &SourceError::BackedOff { secs: 600, reason: "busy".into() }, 4_900, &[]);
        assert!(menu_page(&s).options[0].detail.starts_with("Paused for "), "{}", menu_page(&s).options[0].detail);
    }

    #[test]
    fn problems_are_said_in_plain_words() {
        assert_eq!(plain_problem("Invalid: SteamGridDB did not accept the API key"), "the key was not accepted");
        assert_eq!(plain_problem("SteamGridDB: offline (could not connect)"), "could not reach the site");
        assert_eq!(plain_problem("HTTP 429 rate limit"), "the site was too busy");
        assert_eq!(plain_problem("unexpected response: <html>"), "the site sent an answer the app did not expect");
        assert_eq!(refused_reason("that does not look like a key or login (empty, too long, or has control characters)"), "that does not look right (empty, too long, or odd characters)");
        assert_eq!(refused_reason("secrets.json cannot be saved here"), "the file that keeps your keys could not be written");
    }

    #[test]
    fn the_source_page_offers_enter_and_test_and_test_waits_for_a_key() {
        let (_d, mut s) = screen("source");
        let p = source_page(&s, SourceId::SteamGridDb);
        assert_eq!((p.options[0].title.as_str(), p.options[1].title.as_str()), ("Enter a key", "Test the key"));
        assert!(!p.options[1].enabled, "nothing to test without a key");
        assert_eq!(p.options[1].detail, "Enter a key first");
        assert_eq!(action_at(&p, initial_focus(&p)), Some(ArtAct::EnterKey));
        s.set(Field::SteamGridDbKey, Some(FAKE_KEY)).unwrap();
        let p = source_page(&s, SourceId::SteamGridDb);
        assert_eq!(p.options[0].title, "Change the key");
        assert!(p.options[1].enabled);
        assert_eq!(action_at(&p, initial_focus(&p)), Some(ArtAct::Test), "with a key the cursor starts on Test");
        let ss = source_page(&s, SourceId::ScreenScraper);
        assert_eq!(ss.options[0].title, "Enter your login");
        assert!(!ss.options[1].enabled && ss.options[1].detail == "Not available in this version of the app");
        assert_eq!(p.window_title, "Artwork sources: SteamGridDB");
    }

    #[test]
    fn the_key_page_has_save_remove_cancel_and_remove_asks_with_no_first() {
        let k = key_page(KeyField::Key(SourceId::SteamGridDb), Some(Some("0000".into())), "", None, false);
        assert_eq!(k.paragraphs[0], "Paste or type the key, then choose Save.", "the keyboard is named only in Game Mode");
        assert!(key_page(KeyField::User, None, "", None, true).paragraphs[0].ends_with("Steam + X brings up the keyboard."));
        let titles: Vec<&str> = k.options.iter().map(|o| o.title.as_str()).collect();
        assert_eq!(titles, ["Save", "Remove the saved key", "Cancel"]);
        assert_eq!(k.initial, Focus::Input);
        assert_eq!(k.back_hint, "Cancel");
        let none = key_page(KeyField::Key(SourceId::SteamGridDb), None, "", None, false);
        assert_eq!(none.options.iter().map(|o| o.title.as_str()).collect::<Vec<_>>(), ["Save", "Cancel"], "nothing saved: nothing to remove");
        let kept = key_page(KeyField::Key(SourceId::SteamGridDb), None, "abc", Some("that does not look right"), false);
        assert_eq!(kept.input.unwrap().text, "abc", "the text survives a refusal");
        assert_eq!(kept.facts, ["Not saved: that does not look right"]);
        let r = remove_page(KeyField::Key(SourceId::SteamGridDb));
        assert_eq!(r.options.iter().map(|o| o.action).collect::<Vec<_>>(), [ArtAct::RemoveNo, ArtAct::RemoveYes]);
        assert_eq!(action_at(&r, initial_focus(&r)), Some(ArtAct::RemoveNo));
        assert!(r.fresh_focus && k.fresh_focus, "No (and the text box) every time");
        assert_eq!(r.back_hint, "No, keep it");
    }

    #[test]
    fn the_test_result_says_what_happened_and_has_done() {
        let ok = test_result_page(SourceId::SteamGridDb, &KeyCheck::Ok("x".into()));
        assert_eq!(ok.title, "The key works");
        assert_eq!(action_at(&ok, initial_focus(&ok)), Some(ArtAct::Done));
        assert_eq!(ok.back_hint, "Done");
        assert_eq!(test_result_page(SourceId::ScreenScraper, &KeyCheck::Invalid("x".into())).title, "The login was not accepted");
        assert_eq!(test_result_page(SourceId::TheGamesDb, &KeyCheck::Offline("x".into())).title, "Could not reach TheGamesDB");
        let server = test_result_page(SourceId::TheGamesDb, &KeyCheck::Offline("failed for https://x.example/y?apikey=SECRETKEY-123456".into()));
        assert!(!shown(&server).join(" ").contains("SECRETKEY"), "the server's own text is not shown");
        assert!(testing_page(SourceId::SteamGridDb).progress.is_some());
    }

    #[test]
    fn credits_are_short_facts_naming_all_three_sites() {
        let p = credits_page("1.0 MB of downloaded artwork is kept for reuse");
        let text = shown(&p).join("\n");
        for name in ["SteamGridDB", "TheGamesDB", "ScreenScraper", "steamgriddb.com", "thegamesdb.net", "screenscraper.fr"] {
            assert!(text.contains(name), "{name}");
        }
        assert!(p.facts.iter().all(|f| f.chars().count() <= 66), "{:?}", p.facts);
        assert_eq!(action_at(&p, initial_focus(&p)), Some(ArtAct::Done));
    }

    #[test]
    fn look_alikes_are_a_choice_page_with_none_of_these_last_and_no_hand_editing() {
        let choices = vec![
            PendingChoice { port_key: "a".into(), port_name: "Doom".into(), folder: "doom".into(), source: SourceId::SteamGridDb, id: 42, name: "Doom II".into(), year: Some(1994), verified: Some(true) },
            PendingChoice { port_key: "a".into(), port_name: "Doom".into(), folder: "doom".into(), source: SourceId::SteamGridDb, id: 43, name: "Doom 64".into(), year: Some(1997), verified: Some(false) },
            PendingChoice { port_key: "b".into(), port_name: "Quake".into(), folder: "q".into(), source: SourceId::TheGamesDb, id: 7, name: "Quake II".into(), year: None, verified: None },
        ];
        let groups = group_lookalikes(&choices);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].choices.len(), 2);
        let p = lookalike_page(&groups[0]);
        assert_eq!(p.title, "Which one is Doom?");
        let opts: Vec<(&str, &str)> = p.options.iter().map(|o| (o.title.as_str(), o.detail.as_str())).collect();
        assert_eq!(opts, [("Doom II", "Released 1994, checked by the site"), ("Doom 64", "Released 1997"), ("None of these", "Decide later; nothing is changed")]);
        assert_eq!(lookalike_detail(&groups[1].choices[0]), "No more details from the site");
        let all = shown(&p).join(" ");
        assert!(!all.contains("entry") && !all.contains("42"), "no ids: {all}");
        assert_eq!(p.back_hint, "None of these");
        let list = lookalike_list_page(&groups);
        assert_eq!(list.options[0].detail, "2 close matches from SteamGridDB");
        assert_eq!(list.options[1].detail, "1 close match from TheGamesDB");
        let text = shown(&p).join(" ").to_lowercase();
        assert!(!text.contains("ports.local") && !text.contains("folder"), "no hand-edit instructions: {text}");
    }

    #[test]
    fn fetching_shows_the_port_and_a_stop_card_and_the_done_page_counts_in_sentences() {
        let p = fetching_page("Port 2 of 5\nShip of Harkinian", 5, false);
        assert_eq!(p.facts, ["Port 2 of 5: Ship of Harkinian"]);
        assert_eq!(p.progress, Some(crate::fork::page::Progress::Part(200)));
        assert_eq!((p.options[0].title.as_str(), p.options[0].detail.as_str()), ("Stop", "What is done so far is kept"));
        assert_eq!(p.back_hint, "Stop", "B stops too");
        let stopping = fetching_page("Port 2 of 5\nShip of Harkinian", 5, true);
        assert!(!stopping.options[0].enabled && stopping.no_back);
        let mut s = FetchSummary { looked_up: 1, total: 3, stopped: true, ..FetchSummary::default() };
        s.found.insert("portrait", 3);
        s.found.insert("wide", 1);
        s.found.insert("icon", 2);
        let d = fetched_page(&s);
        assert_eq!(d.title, "Fetching stopped");
        assert_eq!(d.paragraphs[0], "Stopped after 1 of your 3 installed ports. What was fetched is kept.");
        assert_eq!(d.paragraphs[1], "Found 3 covers, 1 wide cover and 2 icons.");
        let mut off = FetchSummary { looked_up: 1, total: 1, ..FetchSummary::default() };
        off.problems.push("SteamGridDB: offline (could not connect)".into());
        let o = fetched_page(&off);
        assert_eq!(o.title, "No artwork could be fetched");
        assert_eq!(o.facts, ["SteamGridDB: could not reach the site"], "plain words, not the raw error");
        assert!(!o.paragraphs.iter().any(|t| t.starts_with("Add to Steam puts")), "nothing found: no word about placing it");
        assert!(d.paragraphs.iter().any(|t| t.starts_with("Add to Steam puts")), "something found: where it goes next");
        assert!(o.paragraphs.iter().any(|t| t.contains("Try again when the Deck is online")));
        assert_eq!(o.mark_of(0), Mark::Problem);
        assert_eq!(fetched_page(&FetchSummary { looked_up: 2, total: 2, ..FetchSummary::default() }).paragraphs[1], "No new pictures were found.");
    }
}
