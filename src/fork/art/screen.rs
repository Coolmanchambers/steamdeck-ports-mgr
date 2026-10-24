//! What the "Artwork sources" screen says and does, decided here and tested without any UI. The screen
//! itself is `src/app/art_screens.rs`; it only draws these lines and runs the slow calls on a worker.
//!
//! Rows (a row that is only information does nothing when chosen):
//!
//! ```text
//! SteamGridDB: key ****1234, test OK          (choose: paste a key)
//!    Test the SteamGridDB key
//! TheGamesDB: not set (optional)              (choose: paste a key)
//!    Test the TheGamesDB key
//! ScreenScraper: login tim, password set      (choose: enter a login)
//!    ScreenScraper is not configured in this build, so it is not used.
//!    Test the ScreenScraper login
//! Fetch artwork for installed ports now
//! Credits and what each source does
//! Back
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::cache::{confirm_in_file, ArtCache, IdTable};
use super::net::{self, Fetch};
use super::resolve::{PortRef, Session};
use super::screenscraper::{DevCreds, ScreenScraper};
use super::sgdb::SteamGridDb;
use super::source::{human_wait, KeyCheck, SourceId, SourceStates};
use super::tgdb::TheGamesDb;
use crate::fork::secrets::{mask, BuildCreds, Field, Secrets, FILE_NAME as SECRETS_FILE};

pub const TITLE: &str = "Artwork sources";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    /// Choosing it asks for the key or login.
    Edit(SourceId),
    Test(SourceId),
    /// Not clickable: says something about the source above.
    Info,
    Fetch,
    Pending,
    Credits,
    Back,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    /// Has everything it needs.
    Configured,
    /// Needs the user's key or login.
    NotConfigured,
    /// Needs developer credentials that only a build can carry, and this build has none.
    NotInThisBuild,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStatus {
    pub source: SourceId,
    pub readiness: Readiness,
    /// One short line: what is set, in masked form.
    pub summary: String,
    pub last_test: Option<String>,
    pub last_error: Option<String>,
    /// Seconds left of a back-off, if the source is being left alone.
    pub waiting: Option<(u64, String)>,
}

/// One waiting look-alike: the port, which source suggested what, and the folder to name in
/// `ports.local.json` if the user would rather choose by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingChoice {
    pub port_key: String,
    pub port_name: String,
    pub folder: String,
    pub source: SourceId,
    pub id: u64,
    pub name: String,
}

impl PendingChoice {
    /// The row that confirms this match.
    pub fn label(&self) -> String {
        format!("Use \"{}\" ({}) for {}", short(&self.name, 30), self.id, short(&self.port_name, 28))
    }

    /// The `ports.local.json` field for this source.
    fn field(&self) -> &'static str {
        if self.source == SourceId::TheGamesDb {
            "tgdb_id"
        } else {
            "sgdb_id"
        }
    }

    /// The entry to add to `ports.local.json` to choose by hand. Only these two fields: an entry with a
    /// name or a source would replace the port's catalog entry.
    pub fn json_entry(&self) -> String {
        let folder = if self.folder.is_empty() { "<folder>" } else { self.folder.as_str() };
        format!("{{\"folder\": \"{folder}\", \"{}\": {}}}", self.field(), self.id)
    }

    /// Text for the list under the "Use this match" rows.
    pub fn hint(&self) -> String {
        format!(
            "Or choose by hand: add this entry to the ports list in ports.local.json (this port is {}, folder {}), with no name and no source, because an entry with either replaces the port's catalog entry:\n{}",
            self.port_name,
            if self.folder.is_empty() { "unknown" } else { self.folder.as_str() },
            self.json_entry()
        )
    }

    /// Confirms this match in the saved id table; the waiting row clears.
    pub fn confirm(&self, cache_dir: &Path) -> std::io::Result<()> {
        confirm_in_file(cache_dir, &self.port_key, self.source, self.id, &self.name)
    }
}

pub struct ArtScreen {
    pub secrets: Secrets,
    pub states: SourceStates,
    pub build: BuildCreds,
    pending: usize,
    cache_bytes: u64,
    now: u64,
}

fn short(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    let cut: String = chars[..max.saturating_sub(3)].iter().collect();
    format!("{}...", cut.trim_end())
}

impl ArtScreen {
    pub fn load(config_dir: &std::path::Path, cache_dir: &std::path::Path, build: BuildCreds, now: u64) -> ArtScreen {
        let secrets = Secrets::load(&config_dir.join(SECRETS_FILE));
        let cache = ArtCache::new(cache_dir);
        let states = SourceStates::load(&cache.states_path());
        let pending = IdTable::load(&cache.ids_path()).ports_with_pending().len();
        ArtScreen { secrets, states, build, pending, cache_bytes: cache.image_bytes(), now }
    }

    pub fn status(&self, source: SourceId) -> SourceStatus {
        let state = self.states.get(source);
        let (readiness, summary) = match source {
            SourceId::SteamGridDb => match self.secrets.sgdb_key() {
                Some(key) => (Readiness::Configured, format!("key {}", mask(key))),
                None => (Readiness::NotConfigured, "not set (needs your free API key)".to_string()),
            },
            SourceId::TheGamesDb => match (self.secrets.tgdb_key(), self.build.tgdb_apikey.is_some()) {
                (Some(key), _) => (Readiness::Configured, format!("your key {}", mask(key))),
                (None, true) => (Readiness::Configured, "built-in key".to_string()),
                (None, false) => (Readiness::NotConfigured, "not set (optional, this build has no key of its own)".to_string()),
            },
            SourceId::ScreenScraper => {
                let login = match (self.secrets.screenscraper_user(), self.secrets.has_screenscraper_password()) {
                    (Some(user), true) => format!("login {}, password set", short(user, 20)),
                    (Some(user), false) => format!("login {}, password missing", short(user, 20)),
                    (None, _) => "no login (optional)".to_string(),
                };
                if self.build.screenscraper_dev().is_some() {
                    (Readiness::Configured, login)
                } else {
                    (Readiness::NotInThisBuild, login)
                }
            }
        };
        SourceStatus {
            source,
            readiness,
            summary,
            last_test: state.last_check.clone(),
            last_error: state.last_error.clone(),
            waiting: self.states.blocked(source, self.now),
        }
    }

    pub fn statuses(&self) -> Vec<SourceStatus> {
        SourceId::ALL.iter().map(|s| self.status(*s)).collect()
    }

    pub fn any_ready(&self) -> bool {
        self.statuses().iter().any(|s| s.readiness == Readiness::Configured)
    }

    /// The rows, each with the label to draw.
    pub fn rows(&self) -> Vec<(Line, String)> {
        let mut rows = Vec::new();
        if let Some(problem) = &self.secrets.problem {
            rows.push((Line::Info, short(problem, 70)));
        }
        if let Some(note) = self.secrets.drive_note() {
            rows.push((Line::Info, short(note, 70)));
        }
        for src in SourceId::ALL {
            let st = self.status(src);
            let mut label = format!("{}: {}", src.label(), st.summary);
            if let (Some(test), true) = (&st.last_test, st.readiness == Readiness::Configured) {
                label.push_str(&format!(", test {}", if test.starts_with("OK") { "OK" } else { "failed" }));
            }
            rows.push((Line::Edit(src), short(&label, 70)));
            if st.readiness == Readiness::NotInThisBuild {
                rows.push((Line::Info, "   ScreenScraper is not configured in this build, so it is not used.".to_string()));
            }
            rows.push((Line::Test(src), format!("   Test the {} {}", src.label(), if src == SourceId::ScreenScraper { "login" } else { "key" })));
            if let Some((secs, reason)) = &st.waiting {
                rows.push((Line::Info, short(&format!("   Waiting {}: {reason}", human_wait(*secs)), 70)));
            } else if let Some(err) = &st.last_error {
                rows.push((Line::Info, short(&format!("   Last problem: {err}"), 70)));
            }
        }
        rows.push((Line::Fetch, if self.any_ready() { "Fetch artwork for installed ports now".to_string() } else { "Fetch artwork for installed ports (set a key first)".to_string() }));
        if self.pending > 0 {
            rows.push((Line::Pending, format!("Look-alike matches waiting for you ({})", self.pending)));
        }
        rows.push((Line::Credits, "Credits and what each source does".to_string()));
        rows.push((Line::Back, "Back".to_string()));
        rows
    }

    pub fn labels(&self) -> Vec<String> {
        self.rows().into_iter().map(|(_, l)| l).collect()
    }

    pub fn lines(&self) -> Vec<Line> {
        self.rows().into_iter().map(|(l, _)| l).collect()
    }

    /// Saves a key or login part and forgets what was known about the source (a new key deserves a
    /// fresh start). A value that is not plausible is refused with the reason.
    pub fn set(&mut self, field: Field, value: Option<&str>) -> Result<(), String> {
        self.secrets.set(field, value).map_err(|e| e.to_string())?;
        let src = match field {
            Field::SteamGridDbKey => SourceId::SteamGridDb,
            Field::TheGamesDbKey => SourceId::TheGamesDb,
            Field::ScreenScraperUser | Field::ScreenScraperPassword => SourceId::ScreenScraper,
        };
        self.states.clear(src);
        Ok(())
    }

    pub fn save_states(&self) {
        let _ = self.states.save();
    }

    pub fn records_secrets_in(&self) -> Vec<String> {
        let mut all = self.secrets.all_values();
        all.extend(self.build.all_values());
        all.extend(crate::fork::cloud::tokens::values_beside(self.secrets.path()));
        all
    }

    pub fn cache_line(&self) -> String {
        format!("{:.1} MB of downloaded artwork is kept for reuse", self.cache_bytes as f64 / 1_048_576.0)
    }

    #[cfg(test)]
    fn credits_text_for_tests() -> String {
        ArtScreen { secrets: Secrets::in_memory(), states: SourceStates::in_memory(), build: BuildCreds::none(), pending: 0, cache_bytes: 0, now: 0 }.credits_text()
    }

    pub fn credits_text(&self) -> String {
        "Artwork comes from these sites. It is fetched on your device, saved in your Steam folder and in this app's cache folder, and never uploaded or shared.\n\n\
         SteamGridDB (steamgriddb.com): community capsules, heroes, logos and icons. Your own free API key; get it under Profile > Preferences > API.\n\n\
         TheGamesDB (thegamesdb.net): community box art, banners, fan art and clear logos. A key of your own is optional.\n\n\
         ScreenScraper (screenscraper.fr): box art, fan art and logos matched by the hash of your own game file. Needs a build that carries the app's credentials; your own login only raises the limits.\n\n\
         The images belong to their authors and the games' publishers. Thank you to the volunteers who make these sites."
            .to_string()
    }

    /// Look-alike matches waiting for a decision, one per candidate, for the "Use this match" rows.
    pub fn pending_choices(cache_dir: &Path) -> Vec<PendingChoice> {
        let table = IdTable::load(&ArtCache::new(cache_dir).ids_path());
        let mut out = Vec::new();
        for (port, src, candidates) in table.ports_with_pending() {
            let (name, folder) = table.identity(port);
            for c in candidates {
                out.push(PendingChoice {
                    port_key: port.to_string(),
                    port_name: name.unwrap_or(port).to_string(),
                    folder: folder.unwrap_or("").to_string(),
                    source: src,
                    id: c.id,
                    name: c.name.clone(),
                });
            }
        }
        out
    }

}

/// Breaks `text` into lines of at most `width` characters at word boundaries (paragraph breaks kept as
/// blank lines), for the picker lists that show a message as rows. A word longer than the line is cut.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let mut word = word.to_string();
            while word.chars().count() > width {
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                let head: String = word.chars().take(width).collect();
                word = word.chars().skip(width).collect();
                lines.push(head);
            }
            if line.is_empty() {
                line = word;
            } else if line.chars().count() + 1 + word.chars().count() <= width {
                line.push(' ');
                line.push_str(&word);
            } else {
                lines.push(std::mem::replace(&mut line, word));
            }
        }
        lines.push(line);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

// ---- the background jobs ------------------------------------------------------------------------------

/// What a running job tells the screen's list: a progress text (first line, second line) and whether
/// the Stop row was used. The real one is the job runner's `Ctl`; tests use a plain struct.
pub trait JobCtl {
    fn cancelled(&self) -> bool;
    fn set_progress(&self, text: String);
}

impl JobCtl for crate::fork::rom_screens::job::Ctl {
    fn cancelled(&self) -> bool {
        crate::fork::rom_screens::job::Ctl::cancelled(self)
    }

    fn set_progress(&self, text: String) {
        crate::fork::rom_screens::job::Ctl::set_progress(self, text)
    }
}

/// The rows of a job's list. A job with a Stop row (the batch fetch) has three; a one-request job (Test
/// key) has two and no Stop row: it ends by itself in seconds, and B abandons it.
pub fn job_labels(progress: &str, stoppable: bool, stopping: bool) -> Vec<String> {
    let (line1, line2) = progress.split_once('\n').unwrap_or((progress, ""));
    let mut rows = vec![if line1.is_empty() { "Working...".to_string() } else { line1.to_string() }, if line2.is_empty() { " ".to_string() } else { short(line2, 60) }];
    if stoppable {
        rows.push(if stopping { "Stopping...".to_string() } else { "Stop (what is done is kept)".to_string() });
    }
    rows
}

/// What the Test key list says while its one request runs.
pub fn test_progress(source: SourceId) -> String {
    let (what, host) = match source {
        SourceId::SteamGridDb => ("key", "steamgriddb.com"),
        SourceId::TheGamesDb => ("key", "api.thegamesdb.net"),
        SourceId::ScreenScraper => ("login", "api.screenscraper.fr"),
    };
    format!("Testing the {} {what}...\nOne request to {host}", source.label())
}

pub fn fetch_progress(index: usize, total: usize, name: &str) -> String {
    format!("Port {index} of {total}\n{name}")
}

/// The labels the Fetch list is sized from: its widest possible progress text, from the real port names.
pub fn fetch_sizing_labels(ports: &[PortJob]) -> Vec<String> {
    let longest = ports.iter().map(|p| p.name.as_str()).max_by_key(|n| n.chars().count().min(60)).unwrap_or("");
    job_labels(&fetch_progress(ports.len(), ports.len(), longest), true, true)
}

/// An installed port to fetch art for.
#[derive(Debug, Clone)]
pub struct PortJob {
    pub key: String,
    pub folder: String,
    pub name: String,
    pub instructions: String,
    pub portrait: Option<PathBuf>,
    pub icon: Option<PathBuf>,
}

/// At most this many different problems are listed in the summary.
pub const MAX_PROBLEMS: usize = 6;

#[derive(Debug, Default)]
pub struct FetchSummary {
    pub looked_up: usize,
    pub total: usize,
    pub found: BTreeMap<&'static str, usize>,
    pub problems: Vec<String>,
    pub stopped: bool,
}

/// Looks every port up on the sources through `session` (which fills the cache and the id table), one
/// after the other. Stop is checked before each port and, through the session, before each request, so
/// it is honoured within a second or so; the port it interrupts counts as looked up with what it had, the
/// rest are not touched, and everything learned so far is saved.
pub fn fetch_all(session: &Session, config_dir: &Path, ports: &[PortJob], ctl: &dyn JobCtl) -> FetchSummary {
    let total = ports.len();
    let mut summary = FetchSummary { total, ..FetchSummary::default() };
    let cancel = || ctl.cancelled();
    for (i, port) in ports.iter().enumerate() {
        if ctl.cancelled() {
            summary.stopped = true;
            break;
        }
        ctl.set_progress(fetch_progress(i + 1, total, &port.name));
        let reference = PortRef { key: &port.key, folder: &port.folder, name: &port.name, instructions: Some(&port.instructions) };
        let art = session.resolve_port_with(config_dir, reference, port.portrait.as_deref(), port.icon.as_deref(), None, &cancel);
        summary.looked_up += 1;
        for s in &art.sources {
            *summary.found.entry(s.slot.name()).or_default() += 1;
        }
        for (_, problem) in art.problems {
            if !summary.problems.contains(&problem) && summary.problems.len() < MAX_PROBLEMS {
                summary.problems.push(problem);
            }
        }
        if ctl.cancelled() && i + 1 < total {
            summary.stopped = true;
            break;
        }
    }
    session.save();
    summary
}

pub fn summary_text(s: &FetchSummary) -> String {
    let mut text = format!("Looked up {} of {} installed ports{}.", s.looked_up, s.total, if s.stopped { " (stopped early)" } else { "" });
    let found: Vec<String> = ["portrait", "wide", "hero", "logo", "icon"].iter().map(|slot| format!("{slot} {}", s.found.get(slot).copied().unwrap_or(0))).collect();
    text.push_str(&format!("\nArtwork ready to place, by slot: {}.", found.join(", ")));
    if !s.problems.is_empty() {
        text.push_str(&format!("\nProblems: {}", s.problems.join("; ")));
        if s.problems.iter().any(|p| p.contains("offline")) {
            text.push_str("\nRun Fetch again when the Deck is back online; nothing that was fetched is lost.");
        }
    }
    text.push_str("\n\nAdd to Steam places this artwork for the ports it adds. Art you chose yourself is never replaced.");
    text
}

/// One cheap authenticated call for the source, as the screen's Test rows run it. With nothing to
/// test (no key, no developer credentials) the answer says so without any network call.
pub fn check_source(source: SourceId, secrets: &Secrets, build: &BuildCreds, fetch: &dyn Fetch, sleep: &dyn Fn(Duration)) -> KeyCheck {
    match source {
        SourceId::SteamGridDb => match secrets.sgdb_key() {
            Some(key) => SteamGridDb::new(fetch, key).check_key(),
            None => KeyCheck::NotConfigured("paste your SteamGridDB key first".into()),
        },
        SourceId::TheGamesDb => match secrets.tgdb_key().or(build.tgdb_apikey.as_deref()) {
            Some(key) => TheGamesDb::new(fetch, key).check_key(),
            None => KeyCheck::NotConfigured("no TheGamesDB key: paste your own (this build has none)".into()),
        },
        SourceId::ScreenScraper => match build.screenscraper_dev() {
            Some((id, password, softname)) => {
                let dev = DevCreds { id: id.into(), password: password.into(), softname: softname.into() };
                let user = secrets.screenscraper_login().map(|(u, p)| (u.to_string(), p.to_string()));
                ScreenScraper::new(fetch, dev, user, sleep).check_login()
            }
            None => KeyCheck::NotConfigured("this build has no ScreenScraper developer credentials".into()),
        },
    }
}

/// Text for the dialog after a test; nothing in it can carry a key.
pub fn check_message(source: SourceId, check: &KeyCheck, secrets: &[String]) -> String {
    let text = net::cap(&net::redact(&net::strip_query(&check.text()), secrets), net::MAX_SERVER_TEXT);
    let hint = match check {
        KeyCheck::Invalid(_) if source == SourceId::ScreenScraper => "\n\nCheck the user name and password for typing mistakes and enter them again.",
        KeyCheck::Invalid(_) => "\n\nCheck the key for typing mistakes and paste it again.",
        KeyCheck::Offline(_) => "\n\nThe key was not checked. Try again when the Deck is online.",
        KeyCheck::Unknown(_) => "\n\nThe key may be fine; the service is limiting or failing right now.",
        _ => "",
    };
    format!("{}: {text}{hint}", source.label())
}

#[cfg(test)]
mod tests {
    use super::super::net::fake::Fake;
    use super::*;
    use std::rc::Rc;

    fn temp_dirs(name: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("dpm_artscreen_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&root);
        let (config, cache) = (root.join("app"), root.join("app").join("cache"));
        std::fs::create_dir_all(&cache).unwrap();
        (root, config, cache)
    }

    fn build_with_ss() -> BuildCreds {
        BuildCreds { ss_devid: Some("devid".into()), ss_devpassword: Some("devpass".into()), ss_softname: None, tgdb_apikey: None }
    }

    #[test]
    fn a_fresh_install_says_not_set_everywhere_and_screenscraper_not_in_this_build() {
        let (root, config, cache) = temp_dirs("fresh");
        let screen = ArtScreen::load(&config, &cache, BuildCreds::none(), 1_000);
        let st = screen.statuses();
        assert_eq!(st.iter().map(|s| s.readiness).collect::<Vec<_>>(), vec![Readiness::NotConfigured, Readiness::NotConfigured, Readiness::NotInThisBuild]);
        assert!(!screen.any_ready());
        let labels = screen.labels();
        assert!(labels[0].starts_with("SteamGridDB: not set"), "{labels:?}");
        assert!(labels.iter().any(|l| l.contains("not configured in this build")), "{labels:?}");
        assert!(labels.iter().any(|l| l.contains("set a key first")));
        assert_eq!(labels.last().map(String::as_str), Some("Back"));
        assert_eq!(screen.lines().len(), labels.len());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_saved_key_shows_masked_and_never_in_full() {
        let (root, config, cache) = temp_dirs("masked");
        let mut screen = ArtScreen::load(&config, &cache, BuildCreds::none(), 1_000);
        screen.set(Field::SteamGridDbKey, Some("0123456789abcdef0123456789abcdef")).unwrap();
        screen.set(Field::TheGamesDbKey, Some("fedcba9876543210")).unwrap();
        screen.set(Field::ScreenScraperUser, Some("tim")).unwrap();
        screen.set(Field::ScreenScraperPassword, Some("hunter22hunter")).unwrap();
        let text = screen.labels().join("\n") + &screen.credits_text() + &screen.cache_line();
        for secret in ["0123456789abcdef0123456789abcdef", "89abcdef0123", "fedcba98", "hunter22"] {
            assert!(!text.contains(secret), "{secret} leaked into the screen text");
        }
        assert!(text.contains("****cdef") && text.contains("login tim, password set"), "{text}");
        let reloaded = ArtScreen::load(&config, &cache, BuildCreds::none(), 1_000);
        assert_eq!(reloaded.status(SourceId::SteamGridDb).readiness, Readiness::Configured);
        assert!(reloaded.any_ready());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_screenscraper_login_without_the_builds_credentials_is_kept_but_marked_unused() {
        let (root, config, cache) = temp_dirs("ssnobuild");
        let mut screen = ArtScreen::load(&config, &cache, BuildCreds::none(), 1_000);
        screen.set(Field::ScreenScraperUser, Some("tim")).unwrap();
        screen.set(Field::ScreenScraperPassword, Some("pw-123456")).unwrap();
        assert_eq!(screen.status(SourceId::ScreenScraper).readiness, Readiness::NotInThisBuild);
        let screen = ArtScreen::load(&config, &cache, build_with_ss(), 1_000);
        assert_eq!(screen.status(SourceId::ScreenScraper).readiness, Readiness::Configured);
        assert!(!screen.labels().iter().any(|l| l.contains("not configured in this build")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn thegamesdb_reports_its_built_in_key_when_the_build_has_one() {
        let (root, config, cache) = temp_dirs("tgdbbuild");
        let build = BuildCreds { tgdb_apikey: Some("buildkey-1234567".into()), ..BuildCreds::none() };
        let screen = ArtScreen::load(&config, &cache, build, 1_000);
        let st = screen.status(SourceId::TheGamesDb);
        assert_eq!((st.readiness, st.summary.as_str()), (Readiness::Configured, "built-in key"));
        assert!(!screen.labels().join("\n").contains("buildkey"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn last_test_last_error_and_waiting_show_as_rows() {
        let (root, config, cache) = temp_dirs("rows");
        let mut screen = ArtScreen::load(&config, &cache, BuildCreds::none(), 5_000);
        screen.set(Field::SteamGridDbKey, Some("0123456789abcdef")).unwrap();
        screen.states.record_check(SourceId::SteamGridDb, &KeyCheck::Invalid("SteamGridDB did not accept the API key".into()), 4_000, &[]);
        let labels = screen.labels();
        assert!(labels[0].ends_with("test failed"), "{labels:?}");
        assert!(labels.iter().any(|l| l.contains("Last problem: Invalid: SteamGridDB did not accept")), "{labels:?}");
        screen.states.record_error(SourceId::SteamGridDb, &super::super::source::SourceError::BackedOff { secs: 600, reason: "busy".into() }, 4_900, &[]);
        let labels = screen.labels();
        assert!(labels.iter().any(|l| l.contains("Waiting 9 min") || l.contains("Waiting 8 min") || l.contains("Waiting 10 min")), "{labels:?}");
        screen.states.record_check(SourceId::SteamGridDb, &KeyCheck::Ok(String::new()), 5_000, &[]);
        let labels = screen.labels();
        assert!(labels[0].ends_with("test OK"), "{labels:?}");
        assert!(!labels.iter().any(|l| l.contains("Last problem") || l.contains("Waiting")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_new_key_clears_what_was_known_about_the_source() {
        let (root, config, cache) = temp_dirs("clear");
        let mut screen = ArtScreen::load(&config, &cache, BuildCreds::none(), 5_000);
        screen.states.record_check(SourceId::SteamGridDb, &KeyCheck::Invalid("x".into()), 4_000, &[]);
        screen.set(Field::SteamGridDbKey, Some("0123456789abcdef")).unwrap();
        assert!(screen.status(SourceId::SteamGridDb).last_error.is_none());
        assert!(screen.set(Field::SteamGridDbKey, Some("")).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_test_action_makes_one_call_per_source_and_reports_ok_invalid_and_offline() {
        let secrets = {
            let (_, config, _) = temp_dirs("check");
            let mut s = Secrets::load(&config.join(SECRETS_FILE));
            s.set(Field::SteamGridDbKey, Some("sgdb-test-key-0001")).unwrap();
            s.set(Field::TheGamesDbKey, Some("tgdb-test-key-0001")).unwrap();
            s
        };
        let sleep = |_d: Duration| {};
        let ok = Fake::new().on_json("/search/autocomplete/", 200, r#"{"success":true,"data":[]}"#);
        assert!(check_source(SourceId::SteamGridDb, &secrets, &BuildCreds::none(), &ok, &sleep).is_ok());
        assert_eq!(ok.requests().len(), 1);
        let bad = Fake::new().on_json("/search/autocomplete/", 401, "{}");
        assert!(matches!(check_source(SourceId::SteamGridDb, &secrets, &BuildCreds::none(), &bad, &sleep), KeyCheck::Invalid(_)));
        let offline = Fake::new();
        assert!(matches!(check_source(SourceId::TheGamesDb, &secrets, &BuildCreds::none(), &offline, &sleep), KeyCheck::Offline(_)));
        let none = Fake::new();
        let empty = Secrets::in_memory();
        assert!(matches!(check_source(SourceId::SteamGridDb, &empty, &BuildCreds::none(), &none, &sleep), KeyCheck::NotConfigured(_)));
        assert!(matches!(check_source(SourceId::ScreenScraper, &secrets, &BuildCreds::none(), &none, &sleep), KeyCheck::NotConfigured(m) if m.contains("this build")));
        assert!(none.requests().is_empty(), "nothing to test means no network call");
        let ss = Fake::new().on_json("ssinfraInfos.php", 200, r#"{"response":{}}"#);
        assert!(check_source(SourceId::ScreenScraper, &secrets, &build_with_ss(), &ss, &sleep).is_ok());
    }

    #[test]
    fn the_message_after_a_test_cannot_carry_a_key() {
        let secrets = vec!["SECRETKEY-123456".to_string()];
        let check = KeyCheck::Offline("failed for https://x.example/y?apikey=SECRETKEY-123456".into());
        let text = check_message(SourceId::TheGamesDb, &check, &secrets);
        assert!(!text.contains("SECRETKEY") && !text.contains("apikey"), "{text}");
        assert!(text.starts_with("TheGamesDB: Offline"));
        assert!(check_message(SourceId::SteamGridDb, &KeyCheck::Invalid("no".into()), &[]).contains("typing mistakes"));
        assert!(check_message(SourceId::SteamGridDb, &KeyCheck::Ok(String::new()), &[]).ends_with("OK"));
    }

    #[test]
    fn waiting_look_alikes_become_use_this_match_rows_and_confirming_one_clears_the_row() {
        let (root, config, cache) = temp_dirs("pending");
        assert!(ArtScreen::pending_choices(&cache).is_empty());
        let mut table = IdTable::load(&ArtCache::new(&cache).ids_path());
        table.set_identity("owner/port", "Doom", "doom-port");
        table.set_pending("owner/port", SourceId::SteamGridDb, vec![super::super::cache::Pending { id: 42, name: "Doom II".into(), confidence: super::super::title::Confidence::Medium }], 1);
        table.save().unwrap();
        let choices = ArtScreen::pending_choices(&cache);
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0].label(), "Use \"Doom II\" (42) for Doom");
        assert_eq!(choices[0].json_entry(), r#"{"folder": "doom-port", "sgdb_id": 42}"#);
        let hint = choices[0].hint();
        assert!(hint.contains("no name and no source") && hint.contains("doom-port") && hint.contains(&choices[0].json_entry()), "{hint}");
        let screen = ArtScreen::load(&config, &cache, BuildCreds::none(), 1);
        assert!(screen.lines().contains(&Line::Pending));
        choices[0].confirm(&cache).unwrap();
        assert!(ArtScreen::pending_choices(&cache).is_empty());
        assert!(!ArtScreen::load(&config, &cache, BuildCreds::none(), 1).lines().contains(&Line::Pending), "the row clears");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_hand_written_entry_names_the_right_field_for_each_source() {
        let c = PendingChoice { port_key: "k".into(), port_name: "P".into(), folder: String::new(), source: SourceId::TheGamesDb, id: 7, name: "N".into() };
        assert_eq!(c.json_entry(), r#"{"folder": "<folder>", "tgdb_id": 7}"#);
        assert!(c.hint().contains("folder unknown"));
    }

    struct Shared(Rc<Fake>);

    impl Fetch for Shared {
        fn get(&self, url: &str, headers: &[(&str, &str)], max_bytes: u64) -> Result<super::super::net::Response, super::super::net::NetError> {
            self.0.get(url, headers, max_bytes)
        }
    }

    struct TestCtl {
        stop: Box<dyn Fn() -> bool>,
        progress: std::cell::RefCell<Vec<String>>,
    }

    impl JobCtl for TestCtl {
        fn cancelled(&self) -> bool {
            (self.stop)()
        }
        fn set_progress(&self, text: String) {
            self.progress.borrow_mut().push(text);
        }
    }

    fn ctl(stop: impl Fn() -> bool + 'static) -> TestCtl {
        TestCtl { stop: Box::new(stop), progress: std::cell::RefCell::new(Vec::new()) }
    }

    fn jobs(n: usize) -> Vec<PortJob> {
        (0..n)
            .map(|i| PortJob { key: format!("owner/port{i}"), folder: format!("port{i}"), name: format!("Ship of Harkinian {i}"), instructions: String::new(), portrait: None, icon: None })
            .collect()
    }

    fn session_with(dir: &std::path::Path, fake: &Rc<Fake>, key: Option<&str>) -> Session {
        let mut secrets = Secrets::load(&dir.join("secrets.json"));
        if let Some(k) = key {
            secrets.set(Field::SteamGridDbKey, Some(k)).unwrap();
        }
        Session::with_parts(Box::new(Shared(fake.clone())), secrets, BuildCreds::none(), &dir.join("cache"), crate::fork::art::resolve::Mode::Online, 1_000, Box::new(|_| {}))
    }

    #[test]
    fn stop_after_the_first_port_leaves_the_rest_untouched_and_keeps_what_was_learned() {
        let dir = tempfile::tempdir().unwrap();
        let search = r#"{"success":true,"data":[{"id":900001,"name":"Ship of Harkinian 0","types":[],"verified":true}]}"#;
        let fake = Rc::new(Fake::new().on_json("/search/autocomplete/", 200, search).on_json("/heroes/", 200, r#"{"success":true,"data":[]}"#).on_json("/grids/", 200, r#"{"success":true,"data":[]}"#));
        let session = session_with(dir.path(), &fake, Some("sgdb-test-key-0001"));
        let stop_fake = fake.clone();
        let c = ctl(move || stop_fake.count("/heroes/") >= 1);
        let summary = fetch_all(&session, dir.path(), &jobs(3), &c);
        assert_eq!((summary.looked_up, summary.total, summary.stopped), (1, 3, true));
        assert_eq!(fake.count("/search/autocomplete/"), 1, "the second port was never searched");
        assert_eq!(c.progress.borrow().len(), 1);
        let table = IdTable::load(&dir.path().join("cache/art/ids.json"));
        assert_eq!(table.identity("owner/port0"), (Some("Ship of Harkinian 0"), Some("port0")));
        assert_eq!(table.usable("owner/port0", SourceId::SteamGridDb).map(|e| e.id), Some(900001), "the id found before Stop is saved");
        assert!(summary_text(&summary).contains("stopped early"));
    }

    #[test]
    fn stop_before_anything_does_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Rc::new(Fake::new());
        let session = session_with(dir.path(), &fake, Some("sgdb-test-key-0001"));
        let summary = fetch_all(&session, dir.path(), &jobs(2), &ctl(|| true));
        assert_eq!((summary.looked_up, summary.stopped), (0, true));
        assert!(fake.requests().is_empty());
    }

    #[test]
    fn a_rejected_key_gives_one_problem_however_many_ports_there_are() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Rc::new(Fake::new().on_json("/search/autocomplete/", 401, "{}"));
        let session = session_with(dir.path(), &fake, Some("sgdb-test-key-0001"));
        let summary = fetch_all(&session, dir.path(), &jobs(5), &ctl(|| false));
        assert_eq!((summary.looked_up, summary.stopped), (5, false));
        assert_eq!(summary.problems.len(), 1, "{:?}", summary.problems);
        assert!(summary.problems[0].contains("key was not accepted"));
        assert_eq!(fake.count("/search/autocomplete/"), 1);
    }

    #[test]
    fn the_problem_list_is_capped_and_never_repeats_a_line() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Rc::new(Fake::new());
        let session = session_with(dir.path(), &fake, None);
        let mut ports = jobs(10);
        for p in &mut ports {
            p.portrait = Some(dir.path().join(format!("missing-{}.png", p.folder)));
        }
        let summary = fetch_all(&session, dir.path(), &ports, &ctl(|| false));
        assert_eq!(summary.looked_up, 10);
        assert_eq!(summary.problems.len(), MAX_PROBLEMS);
        let mut unique = summary.problems.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), MAX_PROBLEMS);
    }

    #[test]
    fn an_offline_summary_tells_the_user_to_run_fetch_again() {
        let mut s = FetchSummary { looked_up: 1, total: 1, ..FetchSummary::default() };
        s.problems.push("SteamGridDB: offline (could not connect)".into());
        let text = summary_text(&s);
        assert!(text.contains("Run Fetch again when the Deck is back online; nothing that was fetched is lost."), "{text}");
        assert!(text.contains("portrait 0, wide 0, hero 0, logo 0, icon 0"));
    }

    #[test]
    fn job_rows_have_a_stop_row_only_for_a_batch_and_are_sized_from_real_text() {
        assert_eq!(job_labels("Port 1 of 3\nShip of Harkinian", true, false), vec!["Port 1 of 3", "Ship of Harkinian", "Stop (what is done is kept)"]);
        assert_eq!(job_labels("", true, true), vec!["Working...", " ", "Stopping..."]);
        let test = test_progress(SourceId::SteamGridDb);
        assert_eq!(job_labels(&test, false, false), vec!["Testing the SteamGridDB key...", "One request to steamgriddb.com"]);
        assert_eq!(job_labels(&test_progress(SourceId::ScreenScraper), false, false)[0], "Testing the ScreenScraper login...");
        let mut ports = jobs(3);
        ports[1].name = "A rather long port name that decides the width of the list".into();
        let sizing = fetch_sizing_labels(&ports);
        assert_eq!(sizing.len(), 3);
        assert!(sizing[1].starts_with("A rather long port name"), "{sizing:?}");
        assert!(!sizing.iter().any(|l| l.contains("keeps going")), "no made-up filler line");
        assert_eq!(fetch_sizing_labels(&[]).len(), 3);
    }

    #[test]
    fn wrap_breaks_at_words_keeps_paragraphs_and_cuts_long_words() {
        assert_eq!(wrap("one two three four", 9), vec!["one two", "three", "four"]);
        assert_eq!(wrap("a b\n\nc", 10), vec!["a b", "", "c"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert!(wrap("", 10).is_empty());
        assert!(wrap("word  \n", 10) == vec!["word"]);
        for line in wrap(&ArtScreen::credits_text_for_tests(), 58) {
            assert!(line.chars().count() <= 58, "{line}");
        }
    }

    #[test]
    fn credits_name_all_three_sources() {
        let (root, config, cache) = temp_dirs("credits");
        let text = ArtScreen::load(&config, &cache, BuildCreds::none(), 1).credits_text();
        for name in ["SteamGridDB", "TheGamesDB", "ScreenScraper", "steamgriddb.com", "thegamesdb.net", "screenscraper.fr"] {
            assert!(text.contains(name), "{name}");
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
