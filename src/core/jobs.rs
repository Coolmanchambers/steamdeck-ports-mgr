
use super::asset_select::AssetSelectionError;
use super::installer::{self, InstallError, InstallOverrides, InstallPaths};
use super::models::{Port, SourceType};
use super::platform_resolve::resolve_preferred_asset;
use super::{github_api, gitlab_api};
use serde_json::Value;
use std::path::Path;

pub enum InstallOutcome {
    Done { tag: Option<String> },
    AssetAmbiguous { assets: Vec<Value> },
    Error(String),
}

pub fn run_install(
    port: &Port,
    paths: InstallPaths,
    github_token: Option<&str>,
    gitlab_token: Option<&str>,
    overrides: InstallOverrides,
    on_progress: &mut dyn FnMut(&str),
) -> InstallOutcome {
    match installer::install_port(port, paths, github_token, gitlab_token, overrides, on_progress) {
        Ok(tag) => InstallOutcome::Done { tag },
        Err(InstallError::Ambiguous(assets)) => InstallOutcome::AssetAmbiguous { assets },
        Err(InstallError::Message(message)) => InstallOutcome::Error(message),
    }
}

pub fn update_decision(installed_tag: Option<&str>, installed_at: &str, latest_tag: &str, latest_date: Option<&str>) -> bool {
    let tag_changed = installed_tag.is_some_and(|tag| tag != latest_tag);
    let date_newer = latest_date.is_some_and(|latest| latest > installed_at);
    tag_changed || date_newer
}

pub fn run_update_check(
    port: &Port,
    installed_tag: Option<&str>,
    installed_at: &str,
    github_token: Option<&str>,
    gitlab_token: Option<&str>,
) -> bool {
    run_update_check_result(port, installed_tag, installed_at, github_token, gitlab_token).unwrap_or(false)
}

/// Fork (home screen): [`run_update_check`] that tells a failed check (a rate limit, no network) from "no update".
pub fn run_update_check_result(
    port: &Port,
    installed_tag: Option<&str>,
    installed_at: &str,
    github_token: Option<&str>,
    gitlab_token: Option<&str>,
) -> Result<bool, String> {
    let repo = port.repo.as_deref().unwrap_or_default();
    match port.source_type {
        SourceType::Github => update_available(port, installed_tag, installed_at, |preferred| github_api::fetch_latest_tag_and_date_preferring(repo, github_token, preferred)),
        SourceType::Gitlab => update_available(port, installed_tag, installed_at, |preferred| gitlab_api::fetch_latest_tag_and_date_preferring(repo, gitlab_token, preferred)),
        SourceType::DirectUrl | SourceType::None => Ok(false),
    }
}

// The check must look at the same asset the installer picks, or a port with a "preferred_asset" is compared
// against the wrong release and gets flagged for an update every day.
fn update_available(
    port: &Port,
    installed_tag: Option<&str>,
    installed_at: &str,
    fetch_latest: impl FnOnce(Option<&str>) -> Result<(String, Option<String>), AssetSelectionError>,
) -> Result<bool, String> {
    let preferred = port.preferred_asset.as_ref().and_then(resolve_preferred_asset);
    fetch_latest(preferred.as_deref()).map(|(tag, date)| update_decision(installed_tag, installed_at, &tag, date.as_deref())).map_err(|e| e.message().to_string())
}

pub fn run_extra_install(port: &Port, library_dir: &Path, on_progress: &mut dyn FnMut(&str)) -> Result<(), String> {
    installer::install_extra(port, library_dir, on_progress).map_err(|e| match e {
        InstallError::Message(m) => m,
        InstallError::Ambiguous(_) => "This \"extra\" archive has an unexpected layout.".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_identique_sans_date_plus_recente_pas_de_maj() {
        assert!(!update_decision(Some("v1.0"), "2026-02-01T00:00:00Z", "v1.0", Some("2026-01-01T00:00:00Z")));
    }

    #[test]
    fn tag_identique_avec_date_plus_recente_signale_une_maj() {
        assert!(update_decision(Some("latest"), "2026-01-01T00:00:00Z", "latest", Some("2026-02-01T00:00:00Z")));
    }

    #[test]
    fn tag_different_signale_une_maj_meme_sans_date() {
        assert!(update_decision(Some("v1.0"), "2026-01-01T00:00:00Z", "v2.0", None));
    }

    #[test]
    fn tag_inconnu_sans_date_plus_recente_pas_de_maj() {
        assert!(!update_decision(None, "2026-01-01T00:00:00Z", "v1.0", Some("2025-12-01T00:00:00Z")));
        assert!(!update_decision(None, "2026-01-01T00:00:00Z", "v1.0", None));
    }

    #[test]
    fn tag_inconnu_avec_date_plus_recente_signale_une_maj() {
        assert!(update_decision(None, "2026-01-01T00:00:00Z", "v1.0", Some("2026-02-01T00:00:00Z")));
    }

    #[cfg(target_os = "linux")]
    fn severed_chains() -> Port {
        crate::core::models::port_from_value(&serde_json::json!({
            "name": "The Legend of Dragoon (Severed-Chains)",
            "folder": "Severed-Chains",
            "source": "https://github.com/Legend-of-Dragoon-Modding/Severed-Chains",
            "preferred_asset": {"windows": "Severed_Chains_Windows.zip", "linux": "Severed_Chains_Steam_Deck.tar.gz"}
        }))
        .unwrap()
    }

    #[cfg(target_os = "linux")]
    fn severed_chains_releases(linux_updated_at: &str) -> Vec<Value> {
        let current = |linux_updated_at: &str| {
            serde_json::json!([
                {"name": "Severed_Chains_Linux.tar.gz", "updated_at": linux_updated_at},
                {"name": "Severed_Chains_Linux_ARM64.tar.gz", "updated_at": "2026-10-17T01:16:02Z"},
                {"name": "Severed_Chains_Steam_Deck.tar.gz", "updated_at": "2026-10-17T01:15:45Z"},
                {"name": "Severed_Chains_Windows.zip", "updated_at": "2026-10-17T01:15:39Z"}
            ])
        };
        vec![
            serde_json::json!({"tag_name": "release-3.0.0", "published_at": "2026-10-17T01:16:33Z", "assets": current(linux_updated_at)}),
            serde_json::json!({"tag_name": "recommended-build-1", "published_at": "2023-03-26T20:56:34Z", "assets": [
                {"name": "Linux.zip", "updated_at": "2023-03-26T21:01:52Z"}, {"name": "Windows.zip", "updated_at": "2023-03-26T21:01:22Z"}]}),
        ]
    }

    #[cfg(target_os = "linux")]
    fn check(port: &Port, releases: &[Value]) -> bool {
        update_available(port, Some("release-3.0.0"), "2026-10-18T00:00:00Z", |preferred| github_api::latest_tag_and_date(releases.to_vec(), preferred)).unwrap_or(false)
    }

    #[test]
    fn a_failed_check_is_an_error_not_no_update() {
        let port = crate::core::models::port_from_value(&serde_json::json!({"name": "X", "folder": "x", "source": "https://github.com/a/x"})).unwrap();
        let rate = update_available(&port, Some("v1"), "2026-01-01T00:00:00Z", |_| Err(AssetSelectionError::Message("GitHub API rate limit reached. Add a token in state.json.".into())));
        assert!(rate.is_err_and(|e| e.contains("rate limit")));
        assert_eq!(update_available(&port, Some("v1"), "2026-01-01T00:00:00Z", |_| Ok(("v2".into(), None))), Ok(true));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn verification_de_maj_suit_l_asset_prefere_et_ignore_un_asset_voisin_plus_recent() {
        let releases = severed_chains_releases("2026-10-20T08:00:00Z");
        assert!(!check(&severed_chains(), &releases));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn verification_de_maj_signale_un_nouvel_asset_prefere() {
        let mut releases = severed_chains_releases("2026-10-17T01:16:08Z");
        releases[0]["assets"][2]["updated_at"] = "2026-10-19T10:00:00Z".into();
        assert!(check(&severed_chains(), &releases));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn update_check_without_preferred_asset_compares_the_newest_release_not_an_older_one() {
        // Fork (O2): upstream walked down to the 2023 release here and flagged an update every day.
        let mut port = severed_chains();
        port.preferred_asset = None;
        let mut releases = severed_chains_releases("2026-10-17T01:16:08Z");
        assert!(!check(&port, &releases));
        releases[0]["tag_name"] = "release-3.1.0".into();
        assert!(check(&port, &releases), "a new tag that needs the file picker is still offered");
        // The release's date is its newest candidate file (both Linux tarballs), not published_at.
        let replaced = severed_chains_releases("2026-10-20T08:00:00Z");
        assert!(check(&port, &replaced), "a candidate file re-uploaded after the install is an update");
    }
}
