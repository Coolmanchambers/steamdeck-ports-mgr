
use super::asset_select::{self, pick_asset, AssetSelectionError, ReleaseChoice, RECENT_RELEASES_DEPTH};
use super::http::api_agent;
use serde_json::Value;

const API_BASE: &str = "https://api.github.com";

pub fn list_releases(repo: &str, token: Option<&str>, limit: usize) -> Result<Vec<Value>, AssetSelectionError> {
    let url = format!("{API_BASE}/repos/{repo}/releases?per_page={limit}");
    let mut req = api_agent().get(&url).header("Accept", "application/vnd.github+json");
    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    let releases = match req.call() {
        Ok(mut resp) => match resp.body_mut().read_json::<Value>() {
            Ok(Value::Array(items)) => items,
            _ => Vec::new(),
        },
        Err(ureq::Error::StatusCode(403 | 429)) => {
            return Err(AssetSelectionError::Message("GitHub API rate limit reached. Add a token in state.json.".to_string()))
        }
        Err(ureq::Error::StatusCode(code)) => return Err(AssetSelectionError::Message(format!("GitHub error (HTTP {code})"))),
        Err(e) => return Err(AssetSelectionError::Message(e.to_string())),
    };
    if releases.is_empty() {
        return Err(AssetSelectionError::Message(format!("No release found for {repo}.")));
    }
    Ok(releases)
}

pub fn pick_release_asset(release: &Value, preferred: Option<&str>) -> Result<Value, AssetSelectionError> {
    let assets = release.get("assets").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    pick_asset(assets, preferred)
}

// Fork: the same release `choose_release` asks about (stable first, see `asset_select::prefer_stable`).
pub fn most_recent_release(repo: &str, token: Option<&str>) -> Result<Value, AssetSelectionError> {
    asset_select::newest_release(list_releases(repo, token, RECENT_RELEASES_DEPTH)?).ok_or_else(|| AssetSelectionError::Message(format!("No release found for {repo}.")))
}

pub fn latest_installable_release(repo: &str, token: Option<&str>, preferred: Option<&str>) -> Result<(Value, Value), AssetSelectionError> {
    let releases = list_releases(repo, token, RECENT_RELEASES_DEPTH)?;
    asset_select::latest_installable_release(releases, preferred, pick_release_asset)
}

pub fn fetch_latest_tag_and_date_preferring(repo: &str, token: Option<&str>, preferred: Option<&str>) -> Result<(String, Option<String>), AssetSelectionError> {
    latest_tag_and_date(list_releases(repo, token, RECENT_RELEASES_DEPTH)?, preferred)
}

pub fn latest_tag_and_date(releases: Vec<Value>, preferred: Option<&str>) -> Result<(String, Option<String>), AssetSelectionError> {
    // Fork: when the newest release needs the file picker, compare against that release (its own date), so a
    // new release is still offered and Install then asks for the file, instead of comparing against an older one.
    let (release, asset) = match asset_select::choose_release(releases, preferred, pick_release_asset)? {
        ReleaseChoice::Picked { release, asset } => (release, Some(asset)),
        ReleaseChoice::AskForNewest { release, .. } => (release, None),
    };
    let latest_tag = release.get("tag_name").and_then(Value::as_str).unwrap_or("").to_string();
    let latest_date = asset
        .as_ref()
        .and_then(|a| a.get("updated_at"))
        .or_else(|| release.get("published_at"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok((latest_tag, latest_date))
}
