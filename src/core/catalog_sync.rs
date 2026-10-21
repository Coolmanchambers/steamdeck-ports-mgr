
use super::{config, http};
use serde_json::Value;
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

fn fetch_if_changed(url: &str, known_etag: &str, validate: impl FnOnce(&str) -> Result<(), String>) -> Result<CatalogUpdate, String> {
    let agent = http::agent(Duration::from_secs(30));
    let mut req = agent.get(url);
    if !known_etag.is_empty() {
        req = req.header("If-None-Match", known_etag);
    }
    match req.call() {
        Ok(mut resp) => {
            if !classify(resp.status().as_u16())? {
                return Ok(CatalogUpdate::NotModified);
            }
            let etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            let text = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
            validate(&text)?;
            Ok(CatalogUpdate::Updated { text, etag })
        }
        // Not reached with ureq 3 (a 304 is an `Ok` response); kept in case the agent is ever set to treat 3xx as errors.
        Err(ureq::Error::StatusCode(304)) => Ok(CatalogUpdate::NotModified),
        Err(e) => Err(e.to_string()),
    }
}

pub fn fetch_ports_if_changed(known_etag: &str) -> Result<CatalogUpdate, String> {
    fetch_if_changed(PORTS_RAW_URL, known_etag, |text| config::parse_catalog(text).map(|_| ()).map_err(|e| e.to_string()))
}

pub fn fetch_themes_if_changed(known_etag: &str) -> Result<CatalogUpdate, String> {
    fetch_if_changed(THEMES_RAW_URL, known_etag, |text| {
        let data: Value = serde_json::from_str(config::strip_bom(text)).map_err(|e| e.to_string())?;
        let has_themes = data.get("themes").and_then(Value::as_object).is_some_and(|o| !o.is_empty());
        if has_themes { Ok(()) } else { Err("no usable \"themes\" object".to_string()) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
