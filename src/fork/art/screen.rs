//! What the "Artwork sources" screen knows and does, decided here and tested without any UI: each source's status,
//! saving a key, the key test, the look-alikes waiting, fetching artwork for the installed ports. What the pages say is
//! `crate::fork::art::pages`; `src/app/art_screens.rs` shows them and runs the slow calls on a worker.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::cache::{confirm_in_file, ArtCache, IdTable};
use super::net::{self, Fetch};
use super::resolve::{PortRef, Session};
use super::screenscraper::{DevCreds, ScreenScraper};
use super::sgdb::SteamGridDb;
use super::source::{KeyCheck, SourceId, SourceStates};
use super::tgdb::TheGamesDb;
use crate::fork::secrets::{mask, BuildCreds, Field, Secrets, FILE_NAME as SECRETS_FILE};

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

/// One waiting look-alike: the port, which source suggested what. (Choosing by hand in `ports.local.json`, with
/// `sgdb_id` or `tgdb_id`, still works; the screen no longer teaches it.)
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

    /// How many ports have look-alike matches waiting for a choice.
    pub fn pending_count(&self) -> usize {
        self.pending
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
        format!("{:.1} MB of downloaded artwork is kept to use again", self.cache_bytes as f64 / 1_048_576.0)
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

/// What the fetching page reads back ("Port 2 of 5" and the port's name; `pages::fetching_page`).
pub fn fetch_progress(index: usize, total: usize, name: &str) -> String {
    format!("Port {index} of {total}\n{name}")
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

/// The test's answer for the log (the page says it in plain words); nothing in it can carry a key.
pub fn check_log_text(source: SourceId, check: &KeyCheck, secrets: &[String]) -> String {
    let text = net::cap(&net::redact(&net::strip_query(&check.text()), secrets), net::MAX_SERVER_TEXT);
    format!("{}: {text}", source.label())
}

#[cfg(test)]
mod tests {
    use super::super::net::fake::Fake;
    use super::super::source::human_wait;
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
        assert!(st[0].summary.starts_with("not set"), "{st:?}");
        assert_eq!(screen.pending_count(), 0);
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
        let text = screen.statuses().iter().map(|s| s.summary.clone()).collect::<Vec<_>>().join("\n") + &screen.cache_line();
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
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn thegamesdb_reports_its_built_in_key_when_the_build_has_one() {
        let (root, config, cache) = temp_dirs("tgdbbuild");
        let build = BuildCreds { tgdb_apikey: Some("buildkey-1234567".into()), ..BuildCreds::none() };
        let screen = ArtScreen::load(&config, &cache, build, 1_000);
        let st = screen.status(SourceId::TheGamesDb);
        assert_eq!((st.readiness, st.summary.as_str()), (Readiness::Configured, "built-in key"));
        assert!(!screen.statuses().iter().any(|s| s.summary.contains("buildkey")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn last_test_last_error_and_waiting_show_as_rows() {
        let (root, config, cache) = temp_dirs("rows");
        let mut screen = ArtScreen::load(&config, &cache, BuildCreds::none(), 5_000);
        screen.set(Field::SteamGridDbKey, Some("0123456789abcdef")).unwrap();
        screen.states.record_check(SourceId::SteamGridDb, &KeyCheck::Invalid("SteamGridDB did not accept the API key".into()), 4_000, &[]);
        let st = screen.status(SourceId::SteamGridDb);
        assert!(st.last_test.as_deref().is_some_and(|t| !t.starts_with("OK")), "{st:?}");
        assert!(st.last_error.as_deref().is_some_and(|e| e.contains("Invalid: SteamGridDB did not accept")), "{st:?}");
        screen.states.record_error(SourceId::SteamGridDb, &super::super::source::SourceError::BackedOff { secs: 600, reason: "busy".into() }, 4_900, &[]);
        let waiting = screen.status(SourceId::SteamGridDb).waiting.map(|(secs, _)| human_wait(secs)).unwrap_or_default();
        assert!(["9 min", "8 min", "10 min"].contains(&waiting.as_str()), "{waiting:?}");
        screen.states.record_check(SourceId::SteamGridDb, &KeyCheck::Ok(String::new()), 5_000, &[]);
        let st = screen.status(SourceId::SteamGridDb);
        assert!(st.last_test.as_deref().is_some_and(|t| t.starts_with("OK")), "{st:?}");
        assert!(st.last_error.is_none() && st.waiting.is_none(), "{st:?}");
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
        let text = check_log_text(SourceId::TheGamesDb, &check, &secrets);
        assert!(!text.contains("SECRETKEY") && !text.contains("apikey"), "{text}");
        assert!(text.starts_with("TheGamesDB: Offline"));
        assert!(check_log_text(SourceId::SteamGridDb, &KeyCheck::Ok(String::new()), &[]).ends_with("OK"));
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
        assert_eq!((choices[0].port_name.as_str(), choices[0].name.as_str(), choices[0].id), ("Doom", "Doom II", 42));
        let screen = ArtScreen::load(&config, &cache, BuildCreds::none(), 1);
        assert_eq!(screen.pending_count(), 1);
        choices[0].confirm(&cache).unwrap();
        assert!(ArtScreen::pending_choices(&cache).is_empty());
        assert_eq!(ArtScreen::load(&config, &cache, BuildCreds::none(), 1).pending_count(), 0, "the card clears");
        let _ = std::fs::remove_dir_all(root);
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

}
