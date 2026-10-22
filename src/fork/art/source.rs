//! What the three online sources have in common: their names, the ways a call can fail, the answer to
//! "does this key work", and a small record per source (last error, back-off) that survives restarts.
//!
//! The record lives in `cache/art/state.json`. It holds no keys and no addresses: messages are cleaned
//! with [`net::redact`] and [`net::strip_query`] before they are stored, and the error values the
//! clients build never contain either in the first place.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::net::{self, NetError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SourceId {
    SteamGridDb,
    TheGamesDb,
    ScreenScraper,
}

impl SourceId {
    pub const ALL: [SourceId; 3] = [SourceId::SteamGridDb, SourceId::TheGamesDb, SourceId::ScreenScraper];

    /// Stable lowercase id: manifest `source`, state keys, cache folders.
    pub fn id(self) -> &'static str {
        match self {
            SourceId::SteamGridDb => "steamgriddb",
            SourceId::TheGamesDb => "thegamesdb",
            SourceId::ScreenScraper => "screenscraper",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SourceId::SteamGridDb => "SteamGridDB",
            SourceId::TheGamesDb => "TheGamesDB",
            SourceId::ScreenScraper => "ScreenScraper",
        }
    }

    pub fn from_id(id: &str) -> Option<SourceId> {
        SourceId::ALL.into_iter().find(|s| s.id() == id)
    }
}

/// Why a call to a source gave nothing. Every variant's text is safe to show and to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    /// No key, or the build has no developer credentials for it.
    NotConfigured(String),
    /// The source rejected the key or login (HTTP 401 or 403 where that means a bad key).
    Invalid(String),
    /// The game, or the image list, is not there (HTTP 404).
    NotFound,
    /// Not tried: an earlier call said to wait. `secs` is how long.
    BackedOff { secs: u64, reason: String },
    /// A limit was hit (too many requests, daily quota, too many unknown ROMs).
    Limited(String),
    /// No answer from the server.
    Offline(String),
    /// An answer we could not use (not JSON, missing fields, unexpected status).
    BadResponse(String),
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::NotConfigured(why) => write!(f, "not configured: {why}"),
            SourceError::Invalid(why) => write!(f, "key rejected: {why}"),
            SourceError::NotFound => write!(f, "not found"),
            SourceError::BackedOff { secs, reason } => write!(f, "waiting {} before trying again ({reason})", human_wait(*secs)),
            SourceError::Limited(why) => write!(f, "limit reached: {why}"),
            SourceError::Offline(why) => write!(f, "offline ({why})"),
            SourceError::BadResponse(why) => write!(f, "unexpected answer: {why}"),
        }
    }
}

impl std::error::Error for SourceError {}

impl From<NetError> for SourceError {
    fn from(e: NetError) -> Self {
        match e {
            NetError::Offline(why) => SourceError::Offline(why),
            NetError::TooLarge => SourceError::BadResponse("the answer was too large".into()),
            NetError::Refused(why) => SourceError::BadResponse(why),
        }
    }
}

impl SourceError {
    /// After this error the rest of the run should leave the source alone: it will not get better in
    /// the next few seconds. `NotFound` and `BadResponse` are about one game, not the source.
    pub fn stops_the_run(&self) -> bool {
        matches!(self, SourceError::NotConfigured(_) | SourceError::Invalid(_) | SourceError::BackedOff { .. } | SourceError::Limited(_) | SourceError::Offline(_))
    }

    /// Seconds the source should be left alone for across restarts (0: not at all).
    pub fn wait_secs(&self, now: u64) -> u64 {
        match self {
            SourceError::BackedOff { secs, .. } => *secs,
            SourceError::Limited(_) => secs_until_next_utc_day(now).max(15 * 60),
            _ => 0,
        }
    }
}

pub fn human_wait(secs: u64) -> String {
    match secs {
        0..=89 => format!("{secs} s"),
        90..=5399 => format!("{} min", (secs + 30) / 60),
        _ => format!("{} h", (secs + 1800) / 3600),
    }
}

/// Seconds from `now` (Unix time) to the next 00:00 UTC.
pub fn secs_until_next_utc_day(now: u64) -> u64 {
    86_400 - now % 86_400
}

/// The answer to "does this key work", from one cheap authenticated call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyCheck {
    Ok(String),
    Invalid(String),
    Offline(String),
    NotConfigured(String),
    /// A limit or an outage on their side: the key may be fine.
    Unknown(String),
}

impl KeyCheck {
    pub fn is_ok(&self) -> bool {
        matches!(self, KeyCheck::Ok(_))
    }

    /// One short line for the screen.
    pub fn text(&self) -> String {
        match self {
            KeyCheck::Ok(detail) if detail.is_empty() => "OK".to_string(),
            KeyCheck::Ok(detail) => format!("OK ({detail})"),
            KeyCheck::Invalid(why) => format!("Invalid: {why}"),
            KeyCheck::Offline(why) => format!("Offline: {why}"),
            KeyCheck::NotConfigured(why) => format!("Not configured: {why}"),
            KeyCheck::Unknown(why) => format!("Could not tell: {why}"),
        }
    }
}

// ---- the saved record ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceState {
    pub last_error: Option<String>,
    pub last_error_at: u64,
    pub last_ok_at: u64,
    /// Calls are not made before this Unix time.
    pub blocked_until: u64,
    pub blocked_reason: String,
    /// Result of the last "Test key", as shown on the screen.
    pub last_check: Option<String>,
    pub last_check_ok: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct SourceStates {
    path: PathBuf,
    map: BTreeMap<SourceId, SourceState>,
}

const STATES_VERSION: u64 = 1;

impl SourceStates {
    pub fn in_memory() -> SourceStates {
        SourceStates::default()
    }

    /// Missing or damaged file: empty. The record only saves repeated failures.
    pub fn load(path: &Path) -> SourceStates {
        let mut states = SourceStates { path: path.to_path_buf(), map: BTreeMap::new() };
        let Ok(text) = std::fs::read_to_string(path) else { return states };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { return states };
        if v.get("version").and_then(Value::as_u64) != Some(STATES_VERSION) {
            return states;
        }
        let Some(sources) = v.get("sources").and_then(Value::as_object) else { return states };
        for (name, s) in sources {
            let Some(id) = SourceId::from_id(name) else { continue };
            let get_u = |k: &str| s.get(k).and_then(Value::as_u64).unwrap_or(0);
            let get_s = |k: &str| s.get(k).and_then(Value::as_str).map(str::to_string);
            states.map.insert(
                id,
                SourceState {
                    last_error: get_s("last_error"),
                    last_error_at: get_u("last_error_at"),
                    last_ok_at: get_u("last_ok_at"),
                    blocked_until: get_u("blocked_until"),
                    blocked_reason: get_s("blocked_reason").unwrap_or_default(),
                    last_check: get_s("last_check"),
                    last_check_ok: s.get("last_check_ok").and_then(Value::as_bool),
                },
            );
        }
        states
    }

    pub fn save(&self) -> std::io::Result<()> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut sources = serde_json::Map::new();
        for (id, s) in &self.map {
            sources.insert(
                id.id().to_string(),
                json!({
                    "last_error": s.last_error,
                    "last_error_at": s.last_error_at,
                    "last_ok_at": s.last_ok_at,
                    "blocked_until": s.blocked_until,
                    "blocked_reason": s.blocked_reason,
                    "last_check": s.last_check,
                    "last_check_ok": s.last_check_ok,
                }),
            );
        }
        let text = serde_json::to_string_pretty(&json!({ "version": STATES_VERSION, "sources": sources })).map_err(std::io::Error::other)?;
        crate::core::files::write_atomic(&self.path, text.as_bytes())
    }

    pub fn get(&self, id: SourceId) -> SourceState {
        self.map.get(&id).cloned().unwrap_or_default()
    }

    /// `Some((seconds left, why))` while the source is being left alone.
    pub fn blocked(&self, id: SourceId, now: u64) -> Option<(u64, String)> {
        let s = self.map.get(&id)?;
        (s.blocked_until > now).then(|| (s.blocked_until - now, s.blocked_reason.clone()))
    }

    pub fn record_ok(&mut self, id: SourceId, now: u64) {
        let s = self.map.entry(id).or_default();
        s.last_ok_at = now;
        s.blocked_until = 0;
        s.blocked_reason.clear();
    }

    /// Remembers a failure. `secrets` are removed from the text first.
    pub fn record_error(&mut self, id: SourceId, err: &SourceError, now: u64, secrets: &[String]) {
        if matches!(err, SourceError::NotFound) {
            return;
        }
        let text = net::redact(&net::strip_query(&err.to_string()), secrets);
        let wait = err.wait_secs(now);
        let s = self.map.entry(id).or_default();
        s.last_error = Some(text.clone());
        s.last_error_at = now;
        if wait > 0 {
            s.blocked_until = now + wait;
            s.blocked_reason = text;
        }
    }

    pub fn record_check(&mut self, id: SourceId, check: &KeyCheck, now: u64, secrets: &[String]) {
        let text = net::redact(&net::strip_query(&check.text()), secrets);
        let s = self.map.entry(id).or_default();
        s.last_check = Some(text.clone());
        s.last_check_ok = Some(check.is_ok());
        if check.is_ok() {
            s.last_ok_at = now;
            s.last_error = None;
            s.blocked_until = 0;
            s.blocked_reason.clear();
        } else {
            s.last_error = Some(text);
            s.last_error_at = now;
        }
    }

    /// Forget everything about a source, after its key was changed.
    pub fn clear(&mut self, id: SourceId) {
        self.map.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_times_read_naturally() {
        assert_eq!(human_wait(30), "30 s");
        assert_eq!(human_wait(600), "10 min");
        assert_eq!(human_wait(7200), "2 h");
    }

    #[test]
    fn a_daily_limit_blocks_until_the_next_utc_midnight_and_at_least_a_quarter_hour() {
        let noon = 1_800_000_000 - 1_800_000_000 % 86_400 + 12 * 3600;
        let err = SourceError::Limited("daily quota".into());
        assert_eq!(err.wait_secs(noon), 12 * 3600);
        let almost_midnight = noon + 12 * 3600 - 60;
        assert_eq!(err.wait_secs(almost_midnight), 15 * 60);
        assert_eq!(SourceError::Offline("x".into()).wait_secs(noon), 0);
        assert_eq!(SourceError::BackedOff { secs: 90, reason: "r".into() }.wait_secs(noon), 90);
    }

    #[test]
    fn an_error_blocks_the_source_and_is_remembered_across_a_reload_without_secrets() {
        let dir = std::env::temp_dir().join(format!("dpm_states_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.json");
        let mut states = SourceStates::load(&path);
        let err = SourceError::BackedOff { secs: 600, reason: "server busy, see https://x.example/y?apikey=SECRETKEY123".into() };
        states.record_error(SourceId::ScreenScraper, &err, 1_000, &["SECRETKEY123".to_string()]);
        states.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("SECRETKEY123") && !text.contains("apikey"), "{text}");
        let again = SourceStates::load(&path);
        assert_eq!(again.blocked(SourceId::ScreenScraper, 1_100).map(|(s, _)| s), Some(500));
        assert!(again.blocked(SourceId::ScreenScraper, 1_700).is_none());
        assert!(again.blocked(SourceId::SteamGridDb, 1_100).is_none());
        assert!(again.get(SourceId::ScreenScraper).last_error.unwrap().contains("server busy"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_good_check_clears_the_block_and_the_error() {
        let mut states = SourceStates::in_memory();
        states.record_error(SourceId::TheGamesDb, &SourceError::Limited("monthly allowance".into()), 100, &[]);
        assert!(states.blocked(SourceId::TheGamesDb, 101).is_some());
        states.record_check(SourceId::TheGamesDb, &KeyCheck::Ok("1,000 left".into()), 200, &[]);
        assert!(states.blocked(SourceId::TheGamesDb, 201).is_none());
        let s = states.get(SourceId::TheGamesDb);
        assert_eq!((s.last_error, s.last_check_ok, s.last_check.as_deref()), (None, Some(true), Some("OK (1,000 left)")));
    }

    #[test]
    fn not_found_is_not_a_failure_of_the_source() {
        let mut states = SourceStates::in_memory();
        states.record_error(SourceId::SteamGridDb, &SourceError::NotFound, 100, &[]);
        assert_eq!(states.get(SourceId::SteamGridDb), SourceState::default());
        assert!(!SourceError::NotFound.stops_the_run());
        assert!(SourceError::Invalid("x".into()).stops_the_run());
        assert!(SourceError::Offline("x".into()).stops_the_run());
        assert!(!SourceError::BadResponse("x".into()).stops_the_run());
    }

    #[test]
    fn a_damaged_or_foreign_record_loads_empty() {
        let dir = std::env::temp_dir().join(format!("dpm_states_bad_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.json"), "{ nope").unwrap();
        std::fs::write(dir.join("b.json"), r#"{"version":99,"sources":{"steamgriddb":{"blocked_until":9999999999}}}"#).unwrap();
        assert!(SourceStates::load(&dir.join("a.json")).blocked(SourceId::SteamGridDb, 1).is_none());
        assert!(SourceStates::load(&dir.join("b.json")).blocked(SourceId::SteamGridDb, 1).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn source_ids_round_trip() {
        for id in SourceId::ALL {
            assert_eq!(SourceId::from_id(id.id()), Some(id));
        }
        assert_eq!(SourceId::from_id("nope"), None);
    }
}
