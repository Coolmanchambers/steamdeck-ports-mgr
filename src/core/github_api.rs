
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

/// Fork (O3): `releases/latest`, the newest stable release (GitHub leaves out prereleases and drafts);
/// `None` when the repository has no stable release.
fn latest_stable_release(repo: &str, token: Option<&str>) -> Result<Option<Value>, AssetSelectionError> {
    let mut req = api_agent().get(&format!("{API_BASE}/repos/{repo}/releases/latest")).header("Accept", "application/vnd.github+json");
    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    match req.call() {
        Ok(mut resp) => Ok(resp.body_mut().read_json::<Value>().ok().filter(Value::is_object)),
        Err(ureq::Error::StatusCode(404)) => Ok(None),
        Err(ureq::Error::StatusCode(403 | 429)) => Err(AssetSelectionError::Message("GitHub API rate limit reached. Add a token in state.json.".to_string())),
        Err(e) => Err(AssetSelectionError::Message(e.to_string())),
    }
}

/// Fork (O3): the releases the choice is made from: the 5 newest without drafts, plus the newest stable
/// release when all 5 are prereleases, so an older stable release still wins over betas.
pub fn candidate_releases(repo: &str, token: Option<&str>) -> Result<Vec<Value>, AssetSelectionError> {
    let mut releases: Vec<Value> = list_releases(repo, token, RECENT_RELEASES_DEPTH)?.into_iter().filter(|r| !asset_select::is_draft(r)).collect();
    if asset_select::needs_latest_stable(&releases) {
        if let Some(stable) = latest_stable_release(repo, token)? {
            releases.push(stable);
        }
    }
    if releases.is_empty() {
        return Err(AssetSelectionError::Message(format!("No release found for {repo}.")));
    }
    Ok(releases)
}

// Fork: the same release `choose_release` asks about (stable first, see `asset_select::prefer_stable`).
pub fn most_recent_release(repo: &str, token: Option<&str>) -> Result<Value, AssetSelectionError> {
    asset_select::newest_release(candidate_releases(repo, token)?).ok_or_else(|| AssetSelectionError::Message(format!("No release found for {repo}.")))
}

pub fn latest_installable_release(repo: &str, token: Option<&str>, preferred: Option<&str>) -> Result<(Value, Value), AssetSelectionError> {
    asset_select::latest_installable_release(candidate_releases(repo, token)?, preferred, pick_release_asset)
}

pub fn fetch_latest_tag_and_date_preferring(repo: &str, token: Option<&str>, preferred: Option<&str>) -> Result<(String, Option<String>), AssetSelectionError> {
    latest_tag_and_date(candidate_releases(repo, token)?, preferred)
}

/// Fork (O2): the date of a release that needs the file picker: its newest candidate file, else the release's.
pub fn ask_for_newest_date(release: &Value, candidates: &[Value]) -> Option<String> {
    candidates
        .iter()
        .filter_map(|a| a.get("updated_at").and_then(Value::as_str))
        .max()
        .or_else(|| release.get("published_at").and_then(Value::as_str))
        .map(str::to_string)
}

pub fn latest_tag_and_date(releases: Vec<Value>, preferred: Option<&str>) -> Result<(String, Option<String>), AssetSelectionError> {
    // Fork: when the newest release needs the file picker, compare against that release (its own date), so a
    // new release is still offered and Install then asks for the file, instead of comparing against an older one.
    let (release, latest_date) = match asset_select::choose_release(releases, preferred, pick_release_asset)? {
        ReleaseChoice::Picked { release, asset } => {
            let date = asset.get("updated_at").or_else(|| release.get("published_at")).and_then(Value::as_str).map(str::to_string);
            (release, date)
        }
        ReleaseChoice::AskForNewest { release, candidates } => {
            let date = ask_for_newest_date(&release, &candidates);
            (release, date)
        }
    };
    let latest_tag = release.get("tag_name").and_then(Value::as_str).unwrap_or("").to_string();
    Ok((latest_tag, latest_date))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_release_needing_the_picker_is_dated_by_its_newest_candidate_file() {
        let release = json!({"tag_name": "v2", "published_at": "2026-10-01T00:00:00Z"});
        let candidates = vec![json!({"name": "a", "updated_at": "2026-10-03T00:00:00Z"}), json!({"name": "b", "updated_at": "2026-10-05T00:00:00Z"})];
        assert_eq!(ask_for_newest_date(&release, &candidates).as_deref(), Some("2026-10-05T00:00:00Z"));
        assert_eq!(ask_for_newest_date(&release, &[json!({"name": "a"})]).as_deref(), Some("2026-10-01T00:00:00Z"));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_file_replaced_in_place_on_an_ambiguous_release_is_an_update() {
        // Same tag as installed; a candidate file re-uploaded after the install (published_at is older).
        let releases = vec![json!({"tag_name": "v2", "published_at": "2026-10-01T00:00:00Z", "assets": [
            {"name": "game-linux-a.tar.gz", "updated_at": "2026-10-01T00:00:00Z"},
            {"name": "game-linux-b.tar.gz", "updated_at": "2026-10-09T00:00:00Z"}]})];
        let (tag, date) = latest_tag_and_date(releases, None).unwrap();
        assert_eq!((tag.as_str(), date.as_deref()), ("v2", Some("2026-10-09T00:00:00Z")));
    }
}
