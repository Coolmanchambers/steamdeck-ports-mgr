
use super::{config, http};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

const PORTS_RAW_URL: &str = "https://raw.githubusercontent.com/Nyaldee/Ports-Launcher/main/ports.json";
const THEMES_RAW_URL: &str = "https://raw.githubusercontent.com/Nyaldee/Ports-Launcher/main/themes.json";

pub enum CatalogUpdate {
    NotModified,
    Updated { text: String, etag: String },
}

/// What a response status means. ureq hands back a 304 as an `Ok` response (only 4xx and 5xx are
/// errors), so the status has to be read here rather than matched as an error.
fn classify(status: u16) -> Result<bool, String> {
    match status {
        304 => Ok(false),
        200..=299 => Ok(true),
        other => Err(format!("HTTP {other}")),
    }
}

/// Turns a response into the outcome. `sent_etag` is whether the request carried `If-None-Match`: a 304
/// to a request without one cannot be "unchanged" (the caller would record a check against no saved copy).
fn interpret(
    status: u16,
    sent_etag: bool,
    etag_header: &str,
    read_body: impl FnOnce() -> Result<String, String>,
    validate: impl FnOnce(&str) -> Result<(), String>,
) -> Result<CatalogUpdate, String> {
    if !classify(status)? {
        return if sent_etag { Ok(CatalogUpdate::NotModified) } else { Err("the server answered 304 to a request without an ETag".to_string()) };
    }
    let text = read_body()?;
    validate(&text)?;
    Ok(CatalogUpdate::Updated { text, etag: etag_header.to_string() })
}

fn fetch_if_changed(url: &str, known_etag: &str, validate: impl FnOnce(&str) -> Result<(), String>) -> Result<CatalogUpdate, String> {
    let agent = http::agent(Duration::from_secs(30));
    let mut req = agent.get(url);
    if !known_etag.is_empty() {
        req = req.header("If-None-Match", known_etag);
    }
    match req.call() {
        Ok(mut resp) => {
            let status = resp.status().as_u16();
            let etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            interpret(status, !known_etag.is_empty(), &etag, || resp.body_mut().read_to_string().map_err(|e| e.to_string()), validate)
        }
        // Not reached with ureq 3 (a 304 is an `Ok` response); kept in case the agent is ever set to treat 3xx as errors.
        Err(ureq::Error::StatusCode(304)) if !known_etag.is_empty() => Ok(CatalogUpdate::NotModified),
        Err(e) => Err(e.to_string()),
    }
}

pub fn validate_ports(text: &str) -> Result<(), String> {
    config::parse_catalog(text).map(|_| ()).map_err(|e| e.to_string())
}

pub fn validate_themes(text: &str) -> Result<(), String> {
    let data: Value = serde_json::from_str(config::strip_bom(text)).map_err(|e| e.to_string())?;
    let has_themes = data.get("themes").and_then(Value::as_object).is_some_and(|o| !o.is_empty());
    if has_themes { Ok(()) } else { Err("no usable \"themes\" object".to_string()) }
}

/// The ETag worth sending: only when the saved file exists and passes `validate`. A 304 means "keep what
/// you have", so it must never be asked for while the saved copy is missing or broken.
pub fn etag_for_saved_copy(known_etag: &str, saved: &Path, validate: impl FnOnce(&str) -> Result<(), String>) -> String {
    match std::fs::read_to_string(saved) {
        Ok(text) if validate(&text).is_ok() => known_etag.to_string(),
        _ => String::new(),
    }
}

/// The ETag to record after a download: the new one only when the file was saved, otherwise the old one,
/// so the next start asks again instead of being told "unchanged" about a file that was never written.
pub fn etag_to_record(saved: bool, new_etag: String, known_etag: String) -> String {
    if saved { new_etag } else { known_etag }
}

pub fn fetch_ports_if_changed(known_etag: &str) -> Result<CatalogUpdate, String> {
    fetch_if_changed(PORTS_RAW_URL, known_etag, validate_ports)
}

pub fn fetch_themes_if_changed(known_etag: &str) -> Result<CatalogUpdate, String> {
    fetch_if_changed(THEMES_RAW_URL, known_etag, validate_themes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(text: &str) -> impl FnOnce() -> Result<String, String> + '_ {
        move || Ok(text.to_string())
    }

    fn never_read() -> Result<String, String> {
        panic!("a 304 has no body to read")
    }

    #[test]
    fn a_304_is_unchanged_without_reading_a_body() {
        assert!(matches!(interpret(304, true, "", never_read, validate_ports), Ok(CatalogUpdate::NotModified)));
    }

    #[test]
    fn a_304_to_a_request_without_an_etag_is_an_error() {
        assert!(interpret(304, false, "", never_read, validate_ports).is_err());
    }

    #[test]
    fn a_200_is_validated_and_carries_its_etag_or_none() {
        let any_body = "anything the validator accepts";
        let Ok(CatalogUpdate::Updated { text, etag }) = interpret(200, true, "\"e1\"", body(any_body), |_| Ok(())) else { panic!("expected an update") };
        assert_eq!((text.as_str(), etag.as_str()), (any_body, "\"e1\""));
        let Ok(CatalogUpdate::Updated { etag, .. }) = interpret(200, false, "", body(any_body), |_| Ok(())) else { panic!("expected an update") };
        assert_eq!(etag, "", "no ETag header: nothing to remember, so the next request is unconditional");
        assert!(interpret(200, true, "\"e1\"", body("junk"), validate_ports).is_err());
        assert!(interpret(500, true, "", never_read, validate_ports).is_err());
    }

    #[test]
    fn a_body_that_cannot_be_read_is_an_error() {
        assert!(interpret(200, false, "", || Err("connection reset".into()), |_| Ok(())).is_err());
    }

    fn temp_file(name: &str, content: Option<&str>) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dpm_catalog_sync_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("saved.json");
        if let Some(c) = content {
            std::fs::write(&path, c).unwrap();
        }
        path
    }

    #[test]
    fn the_etag_is_sent_only_while_the_saved_copy_is_usable() {
        let themes_ok = temp_file("themes_ok", Some(r#"{"themes": {"dark": {}}}"#));
        assert_eq!(etag_for_saved_copy("\"e1\"", &themes_ok, validate_themes), "\"e1\"");
        let missing = temp_file("missing", None);
        assert_eq!(etag_for_saved_copy("\"e1\"", &missing, validate_themes), "");
        let empty_themes = temp_file("themes_empty", Some(r#"{"themes": {}}"#));
        assert_eq!(etag_for_saved_copy("\"e1\"", &empty_themes, validate_themes), "");
        let broken = temp_file("broken", Some("{ not json"));
        assert_eq!(etag_for_saved_copy("\"e1\"", &broken, validate_ports), "");
        for p in [themes_ok, missing, empty_themes, broken] {
            let _ = std::fs::remove_dir_all(p.parent().unwrap());
        }
    }

    #[test]
    fn a_download_that_was_not_saved_keeps_the_old_etag() {
        assert_eq!(etag_to_record(true, "new".into(), "old".into()), "new");
        assert_eq!(etag_to_record(false, "new".into(), "old".into()), "old");
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
        let CatalogUpdate::Updated { etag, .. } = fetch_ports_if_changed("").unwrap() else { panic!("a request without an ETag must return the file, not 304") };
        assert!(!etag.is_empty());
        assert!(matches!(fetch_ports_if_changed(&etag), Ok(CatalogUpdate::NotModified)));
        let CatalogUpdate::Updated { etag, .. } = fetch_themes_if_changed("").unwrap() else { panic!("a request without an ETag must return the file, not 304") };
        assert!(!etag.is_empty());
        assert!(matches!(fetch_themes_if_changed(&etag), Ok(CatalogUpdate::NotModified)));
    }
}
