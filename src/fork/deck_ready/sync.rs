//! Fetching the Deck-ready list from this fork's repository and keeping a saved copy.
//!
//! Same pattern as upstream's catalog sync: an `If-None-Match` request with the last ETag, at most once
//! every 12 hours. The saved copy is `cache/deck-ready.json`; `cache/deck-ready.meta.json` holds the
//! ETag and when the app last checked. A download that doesn't parse never replaces the saved copy.

use super::index::{self, Index};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const INDEX_URL: &str = "https://raw.githubusercontent.com/Coolmanchambers/steamdeck-ports-mgr/main/index/deck-ready.json";
const CACHE_FILE: &str = "deck-ready.json";
const META_FILE: &str = "deck-ready.meta.json";
const REFRESH_AFTER_HOURS: i64 = 12;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meta {
    pub etag: String,
    /// RFC 3339, UTC.
    pub checked_at: String,
}

pub enum Fetched {
    NotModified,
    Updated { text: String, etag: String },
}

#[derive(Debug)]
pub struct Outcome {
    /// The list to show: the new download, or the saved copy when nothing changed or the fetch failed.
    pub index: Option<Index>,
    pub meta: Meta,
    /// Why the fetch failed; the saved copy (if any) is still in `index`.
    pub error: Option<String>,
}

fn cache_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join(CACHE_FILE)
}

fn meta_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join(META_FILE)
}

pub fn load_meta(cache_dir: &Path) -> Meta {
    let Ok(text) = fs::read_to_string(meta_path(cache_dir)) else { return Meta::default() };
    let Ok(v) = serde_json::from_str::<Value>(&text) else { return Meta::default() };
    let field = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    Meta { etag: field("etag"), checked_at: field("checked_at") }
}

fn save_meta(cache_dir: &Path, meta: &Meta) {
    let text = json!({"etag": meta.etag, "checked_at": meta.checked_at}).to_string();
    let _ = crate::core::files::write_atomic(&meta_path(cache_dir), text.as_bytes());
}

pub fn load_cached(cache_dir: &Path) -> Option<Index> {
    index::parse(&fs::read_to_string(cache_path(cache_dir)).ok()?).ok()
}

pub fn is_due(meta: &Meta, now: chrono::DateTime<chrono::Utc>) -> bool {
    match chrono::DateTime::parse_from_rfc3339(&meta.checked_at) {
        Ok(at) => now.signed_duration_since(at) >= chrono::Duration::hours(REFRESH_AFTER_HOURS) || at > now,
        Err(_) => true,
    }
}

/// Checks for a newer list with `fetch` (given the ETag to send, empty for none) and returns what to
/// show. The ETag is only sent when a usable saved copy exists: a 304 must never leave the app with
/// nothing to show.
pub fn refresh(cache_dir: &Path, now: chrono::DateTime<chrono::Utc>, fetch: impl FnOnce(&str) -> Result<Fetched, String>) -> Outcome {
    let cached = load_cached(cache_dir);
    let mut meta = load_meta(cache_dir);
    let etag = if cached.is_some() { meta.etag.clone() } else { String::new() };
    let checked_at = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    match fetch(&etag) {
        Ok(Fetched::NotModified) if cached.is_some() => {
            meta.checked_at = checked_at;
            save_meta(cache_dir, &meta);
            Outcome { index: cached, meta, error: None }
        }
        Ok(Fetched::NotModified) => Outcome { index: None, meta, error: Some("the server sent no list".to_string()) },
        Ok(Fetched::Updated { text, etag }) => match index::parse(&text) {
            Ok(fresh) => {
                let _ = fs::create_dir_all(cache_dir);
                if crate::core::files::write_atomic(&cache_path(cache_dir), text.as_bytes()).is_ok() {
                    meta = Meta { etag, checked_at };
                } else {
                    meta.checked_at = checked_at;
                }
                save_meta(cache_dir, &meta);
                Outcome { index: Some(fresh), meta, error: None }
            }
            Err(e) => Outcome { index: cached, meta, error: Some(format!("the downloaded list was not usable ({e})")) },
        },
        Err(e) => Outcome { index: cached, meta, error: Some(e) },
    }
}

/// What a response status means. ureq hands back a 304 as an `Ok` response (only 4xx and 5xx are
/// errors), so the status has to be read here rather than matched as an error.
fn classify(status: u16) -> Result<bool, String> {
    match status {
        304 => Ok(false),
        200..=299 => Ok(true),
        other => Err(format!("GitHub answered HTTP {other}")),
    }
}

/// The real request: `If-None-Match` when there is an ETag, 304 means unchanged.
pub fn fetch_from_github(known_etag: &str) -> Result<Fetched, String> {
    fetch_from_github_within(known_etag, Duration::from_secs(30))
}

/// `fetch_from_github` with its own time limit (Import ports waits at most 10 s on its scan).
pub fn fetch_from_github_within(known_etag: &str, limit: Duration) -> Result<Fetched, String> {
    let agent = crate::core::http::agent(limit);
    let mut req = agent.get(INDEX_URL);
    if !known_etag.is_empty() {
        req = req.header("If-None-Match", known_etag);
    }
    match req.call() {
        Ok(mut resp) => {
            if !classify(resp.status().as_u16())? {
                return Ok(Fetched::NotModified);
            }
            let etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            let text = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
            Ok(Fetched::Updated { text, etag })
        }
        Err(ureq::Error::StatusCode(304)) => Ok(Fetched::NotModified),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::deck_ready::index::tests::{file, project};
    use chrono::TimeZone;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dpm_deck_ready_sync_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(2026, 10, 21, 9, 14, 0).unwrap()
    }

    fn one_project() -> String {
        file(vec![project("a", "linux_build", "game")])
    }

    #[test]
    fn first_fetch_saves_the_list_and_its_etag() {
        let dir = temp_dir("first");
        let out = refresh(&dir, now(), |etag| {
            assert_eq!(etag, "");
            Ok(Fetched::Updated { text: one_project(), etag: "\"e1\"".into() })
        });
        assert!(out.error.is_none());
        assert_eq!(out.index.unwrap().entries.len(), 1);
        assert_eq!(load_meta(&dir), Meta { etag: "\"e1\"".into(), checked_at: "2026-10-21T09:14:00Z".into() });
        assert!(load_cached(&dir).is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unchanged_list_keeps_the_saved_copy_and_records_the_check() {
        let dir = temp_dir("unchanged");
        refresh(&dir, now(), |_| Ok(Fetched::Updated { text: one_project(), etag: "\"e1\"".into() }));
        let later = now() + chrono::Duration::hours(13);
        let out = refresh(&dir, later, |etag| {
            assert_eq!(etag, "\"e1\"");
            Ok(Fetched::NotModified)
        });
        assert!(out.error.is_none() && out.index.is_some());
        assert_eq!(out.meta.checked_at, "2026-10-21T22:14:00Z");
        assert_eq!(load_meta(&dir).etag, "\"e1\"");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_etag_is_sent_without_a_usable_saved_copy() {
        let dir = temp_dir("no_copy");
        save_meta(&dir, &Meta { etag: "\"stale\"".into(), checked_at: String::new() });
        fs::write(dir.join(CACHE_FILE), "broken").unwrap();
        let out = refresh(&dir, now(), |etag| {
            assert_eq!(etag, "", "a 304 here would leave nothing to show");
            Ok(Fetched::Updated { text: one_project(), etag: "\"e2\"".into() })
        });
        assert!(out.index.is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_304_with_nothing_saved_is_an_error_not_an_empty_list() {
        let dir = temp_dir("304_empty");
        let out = refresh(&dir, now(), |_| Ok(Fetched::NotModified));
        assert!(out.index.is_none() && out.error.is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn failures_keep_the_saved_copy_untouched() {
        let dir = temp_dir("fail");
        refresh(&dir, now(), |_| Ok(Fetched::Updated { text: one_project(), etag: "\"e1\"".into() }));
        let before = fs::read(dir.join(CACHE_FILE)).unwrap();

        let offline = refresh(&dir, now(), |_| Err("offline".into()));
        assert_eq!(offline.error.as_deref(), Some("offline"));
        assert!(offline.index.is_some());

        let junk = refresh(&dir, now(), |_| Ok(Fetched::Updated { text: "{\"schema\": 9}".into(), etag: "\"e9\"".into() }));
        assert!(junk.error.unwrap().contains("not usable"));
        assert!(junk.index.is_some());

        assert_eq!(fs::read(dir.join(CACHE_FILE)).unwrap(), before);
        assert_eq!(load_meta(&dir).etag, "\"e1\"");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_304_is_unchanged_not_an_unusable_download() {
        assert_eq!(classify(200), Ok(true));
        assert_eq!(classify(304), Ok(false));
        assert!(classify(500).is_err() && classify(404).is_err() && classify(302).is_err());
    }

    /// Against the real server: a second request with the ETag of the first must come back unchanged.
    /// Needs the network, so it only runs on request: `cargo test -- --ignored live_etag`.
    #[test]
    #[ignore]
    fn live_etag_round_trip() {
        let Fetched::Updated { etag, .. } = fetch_from_github("").unwrap() else { panic!("first request had no body") };
        assert!(!etag.is_empty());
        assert!(matches!(fetch_from_github(&etag).unwrap(), Fetched::NotModified));
    }

    #[test]
    fn checks_at_most_every_12_hours() {
        let at = |h: i64| Meta { etag: String::new(), checked_at: (now() - chrono::Duration::hours(h)).to_rfc3339() };
        assert!(!is_due(&at(1), now()));
        assert!(is_due(&at(12), now()));
        assert!(is_due(&Meta::default(), now()));
        assert!(is_due(&at(-5), now()), "a check stamped in the future (clock moved back) must not block checks");
    }
}
