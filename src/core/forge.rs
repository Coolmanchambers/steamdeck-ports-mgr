use super::asset_select::AssetSelectionError;
use super::models::SourceType;
use super::{github_api, gitlab_api};
use serde_json::Value;

// Fork (upstream catch-up 2026-10-31): upstream moved its release lookups behind this enum. The fork keeps its
// own `github_api` / `gitlab_api` functions (O2 file-picker rule, O3 stable-first, the update check dated by
// the newest candidate file), so every method here delegates to them instead of upstream's versions.
#[derive(Clone, Copy)]
pub enum Forge {
    Github,
    Gitlab,
}

impl Forge {
    pub fn of(source_type: SourceType) -> Option<Forge> {
        match source_type {
            SourceType::Github => Some(Forge::Github),
            SourceType::Gitlab => Some(Forge::Gitlab),
            SourceType::DirectUrl | SourceType::None => None,
        }
    }

    pub fn token<'a>(self, github_token: Option<&'a str>, gitlab_token: Option<&'a str>) -> Option<&'a str> {
        match self {
            Forge::Github => github_token,
            Forge::Gitlab => gitlab_token,
        }
    }

    pub fn list_releases(self, repo: &str, token: Option<&str>, limit: usize) -> Result<Vec<Value>, AssetSelectionError> {
        match self {
            Forge::Github => github_api::list_releases(repo, token, limit),
            Forge::Gitlab => gitlab_api::list_releases(repo, token, limit),
        }
    }

    pub fn pick_release_asset(self, release: &Value, preferred: Option<&str>) -> Result<Value, AssetSelectionError> {
        match self {
            Forge::Github => github_api::pick_release_asset(release, preferred),
            Forge::Gitlab => gitlab_api::pick_release_asset(release, preferred),
        }
    }

    pub fn url_field(self) -> &'static str {
        match self {
            Forge::Github => "browser_download_url",
            Forge::Gitlab => "url",
        }
    }

    // Fork: the same release `choose_release` asks about (stable first), not the raw newest one.
    pub fn most_recent_release(self, repo: &str, token: Option<&str>) -> Result<Value, AssetSelectionError> {
        match self {
            Forge::Github => github_api::most_recent_release(repo, token),
            Forge::Gitlab => gitlab_api::most_recent_release(repo, token),
        }
    }

    pub fn latest_installable_release(self, repo: &str, token: Option<&str>, preferred: Option<&str>) -> Result<(Value, Value), AssetSelectionError> {
        match self {
            Forge::Github => github_api::latest_installable_release(repo, token, preferred),
            Forge::Gitlab => gitlab_api::latest_installable_release(repo, token, preferred),
        }
    }

    // Fork (O2, F5): the update check looks at the preferred file and, when the newest release needs the file
    // picker, compares against that release dated by its newest candidate file.
    pub fn fetch_latest_tag_and_date(self, repo: &str, token: Option<&str>, preferred: Option<&str>) -> Result<(String, Option<String>), AssetSelectionError> {
        match self {
            Forge::Github => github_api::fetch_latest_tag_and_date_preferring(repo, token, preferred),
            Forge::Gitlab => gitlab_api::fetch_latest_tag_and_date_preferring(repo, token, preferred),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forge_et_jeton_selon_la_source() {
        assert!(Forge::of(SourceType::DirectUrl).is_none());
        assert!(Forge::of(SourceType::None).is_none());
        assert_eq!(Forge::of(SourceType::Gitlab).map(|f| f.token(Some("gh"), Some("gl"))), Some(Some("gl")));
        assert_eq!(Forge::of(SourceType::Github).map(|f| f.token(Some("gh"), Some("gl"))), Some(Some("gh")));
    }
}
