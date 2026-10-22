//! Self-update detection for this fork's rolling release (upstream bugs O4 and O5).
//!
//! Upstream compared the release date with the build date (day only), so a second build on the same day
//! was never offered, and once the "Update" flag was set nothing re-checked it. Here the running build's
//! commit (from `BUILD_INFO`, which CI ships next to the binary) is compared with the commit the rolling
//! release was built from (the "Source for this build: .../tree/<sha>" line CI writes into the release
//! notes). The date check stays as the fallback when either commit is unknown (a local build, or an
//! older release without that line). What raised the flag is saved in `fork-settings.json`, so a later
//! start can tell whether the build now running is that one (or newer) and clear a stale flag.

use super::settings::ForkSettings;
use crate::core::asset_select::{self, AssetSelectionError, ReleaseChoice};
use crate::core::version::is_newer_date;
use serde_json::{json, Value};
use std::path::Path;

pub const BUILD_INFO: &str = "BUILD_INFO";
const KEY_OFFER: &str = "self_update_offer";

/// The build that is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Running {
    pub commit: Option<String>,
    /// `YYYY-MM-DD`, compiled in (`APP_BUILD_DATE`).
    pub build_date: String,
}

/// What the rolling release offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub commit: Option<String>,
    /// The package's upload date (or the release's), RFC 3339.
    pub date: Option<String>,
}

fn is_commit(s: &str) -> bool {
    (7..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn same_commit(a: &str, b: &str) -> bool {
    let n = a.len().min(b.len());
    n >= 7 && a[..n].eq_ignore_ascii_case(&b[..n])
}

pub fn parse_build_info(text: &str) -> Option<String> {
    text.lines().find_map(|line| line.trim().strip_prefix("commit=")).map(str::trim).filter(|c| is_commit(c)).map(str::to_ascii_lowercase)
}

pub fn running(dir: &Path, build_date: &str) -> Running {
    let commit = std::fs::read_to_string(dir.join(BUILD_INFO)).ok().as_deref().and_then(parse_build_info);
    Running { commit, build_date: build_date.to_string() }
}

/// The commit in the release notes' "Source for this build: https://github.com/<repo>/tree/<sha>" line.
pub fn release_commit(body: &str) -> Option<String> {
    body.split("/tree/").skip(1).find_map(|rest| {
        let sha: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
        (sha.len() == 40).then(|| sha.to_ascii_lowercase())
    })
}

/// The offer of the newest release (stable first), from `github_api::list_releases`.
pub fn offer_from_releases(releases: Vec<Value>) -> Result<Offer, AssetSelectionError> {
    let (release, asset) = match asset_select::choose_release(releases, None, crate::core::github_api::pick_release_asset)? {
        ReleaseChoice::Picked { release, asset } => (release, Some(asset)),
        ReleaseChoice::AskForNewest { release, .. } => (release, None),
    };
    let date = asset.as_ref().and_then(|a| a.get("updated_at")).or_else(|| release.get("published_at")).and_then(Value::as_str).map(str::to_string);
    let commit = release.get("body").and_then(Value::as_str).and_then(release_commit);
    Ok(Offer { commit, date })
}

/// Whether the offer is a different, not older build than the one running.
pub fn is_update(running: &Running, offer: &Offer) -> bool {
    match (&running.commit, &offer.commit) {
        // A different commit is an update unless the release is from an earlier day than this build (a
        // branch build being tested must not be "updated" back to main).
        (Some(mine), Some(theirs)) => !same_commit(mine, theirs) && offer.date.as_deref().is_none_or(|d| d.get(..10).unwrap_or(d) >= running.build_date.as_str()),
        _ => offer.date.as_deref().is_some_and(|d| is_newer_date(d, &running.build_date)),
    }
}

pub fn saved_offer(settings: &ForkSettings) -> Option<Offer> {
    let value = settings.get(KEY_OFFER)?;
    let field = |k: &str| value.get(k).and_then(Value::as_str).map(str::to_string);
    Some(Offer { commit: field("commit"), date: field("date") })
}

pub fn save_offer(settings: &mut ForkSettings, offer: &Offer) -> std::io::Result<()> {
    settings.set(KEY_OFFER, json!({"commit": offer.commit, "date": offer.date}))
}

/// At startup, with the "Update" flag set: true when it still applies to the build now running. A flag
/// with no saved offer (set by upstream's code, or an older copy) does not count as still applying.
pub fn flag_still_applies(running: &Running, saved: Option<&Offer>) -> bool {
    saved.is_some_and(|offer| is_update(running, offer))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_A: &str = "a2ec28a0f7f3c1a1b2c3d4e5f60718293a4b5c6d";
    const SHA_B: &str = "21fc151d0e9f8a7b6c5d4e3f2a1b0c9d8e7f6a5b";

    fn running_at(commit: Option<&str>, date: &str) -> Running {
        Running { commit: commit.map(str::to_string), build_date: date.to_string() }
    }

    fn offer(commit: Option<&str>, date: Option<&str>) -> Offer {
        Offer { commit: commit.map(str::to_string), date: date.map(str::to_string) }
    }

    #[test]
    fn build_info_commit_is_read() {
        assert_eq!(parse_build_info(&format!("commit={SHA_A}\nbuild_date=2026-10-21\nrun=https://x\n")), Some(SHA_A.to_string()));
        assert_eq!(parse_build_info("build_date=2026-10-21\n"), None);
        assert_eq!(parse_build_info("commit=not-a-sha\n"), None);
        assert_eq!(parse_build_info("commit=ABC1234\r\n"), Some("abc1234".to_string()));
    }

    #[test]
    fn release_commit_comes_from_the_source_line_of_the_notes() {
        let body = format!("Rolling build of `main` at a2ec28a, built 2026-10-21 10:00 UTC.\n\nSource for this build: https://github.com/o/r/tree/{SHA_A}\n\nA testing fork.");
        assert_eq!(release_commit(&body), Some(SHA_A.to_string()));
        assert_eq!(release_commit("Rolling build of `main` at a2ec28a."), None);
        assert_eq!(release_commit("see https://github.com/o/r/tree/main"), None);
    }

    #[test]
    fn a_second_build_on_the_same_day_is_offered() {
        assert!(is_update(&running_at(Some(SHA_A), "2026-10-21"), &offer(Some(SHA_B), Some("2026-10-21T18:00:00Z"))));
    }

    #[test]
    fn the_same_commit_is_never_offered_even_a_day_later() {
        // Upstream's date check offered a build uploaded after midnight UTC again every day.
        assert!(!is_update(&running_at(Some(SHA_A), "2026-10-21"), &offer(Some(&SHA_A[..7]), Some("2026-10-22T00:05:00Z"))));
    }

    #[test]
    fn a_release_from_an_earlier_day_is_not_offered_over_a_newer_build() {
        assert!(!is_update(&running_at(Some(SHA_A), "2026-10-22"), &offer(Some(SHA_B), Some("2026-10-21T18:00:00Z"))));
        assert!(is_update(&running_at(Some(SHA_A), "2026-10-22"), &offer(Some(SHA_B), None)));
    }

    #[test]
    fn without_a_commit_on_either_side_the_date_decides() {
        assert!(is_update(&running_at(None, "2026-10-20"), &offer(Some(SHA_B), Some("2026-10-21T01:00:00Z"))));
        assert!(!is_update(&running_at(None, "2026-10-21"), &offer(Some(SHA_B), Some("2026-10-21T18:00:00Z"))));
        assert!(is_update(&running_at(Some(SHA_A), "2026-10-20"), &offer(None, Some("2026-10-21T01:00:00Z"))));
        assert!(!is_update(&running_at(Some(SHA_A), "2026-10-20"), &offer(None, None)));
    }

    #[test]
    fn offer_reads_the_package_date_and_the_notes_commit() {
        let releases = vec![json!({
            "tag_name": "rolling", "published_at": "2026-10-21T09:00:00Z",
            "body": format!("Source for this build: https://github.com/o/r/tree/{SHA_B}"),
            "assets": [
                {"name": "steamdeck-ports-mgr-linux-x86_64.tar.gz", "updated_at": "2026-10-21T10:00:00Z"},
                {"name": "steamdeck-ports-mgr-linux-x86_64.tar.gz.sha256", "updated_at": "2026-10-21T10:00:01Z"}
            ]
        })];
        assert_eq!(offer_from_releases(releases).unwrap(), offer(Some(SHA_B), Some("2026-10-21T10:00:00Z")));
    }

    #[test]
    fn a_stale_flag_is_cleared_once_the_offered_build_runs() {
        let offered = offer(Some(SHA_B), Some("2026-10-21T18:00:00Z"));
        assert!(flag_still_applies(&running_at(Some(SHA_A), "2026-10-21"), Some(&offered)));
        assert!(!flag_still_applies(&running_at(Some(SHA_B), "2026-10-21"), Some(&offered)), "updated by copying files over");
        assert!(!flag_still_applies(&running_at(None, "2026-10-21"), Some(&offered)), "no BUILD_INFO: same day counts as at least as new");
        assert!(!flag_still_applies(&running_at(Some(SHA_A), "2026-10-21"), None), "flag left by upstream's code");
    }

    #[test]
    fn the_offer_round_trips_through_fork_settings() {
        let path = std::env::temp_dir().join(format!("dpm_self_update_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut settings = ForkSettings::load(&path);
        assert_eq!(saved_offer(&settings), None);
        let offered = offer(Some(SHA_B), Some("2026-10-21T18:00:00Z"));
        save_offer(&mut settings, &offered).unwrap();
        assert_eq!(saved_offer(&ForkSettings::load(&path)), Some(offered));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn running_reads_build_info_next_to_the_binary() {
        let dir = std::env::temp_dir().join(format!("dpm_self_update_dir_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(running(&dir, "2026-10-21"), running_at(None, "2026-10-21"));
        std::fs::write(dir.join(BUILD_INFO), format!("commit={SHA_A}\n")).unwrap();
        assert_eq!(running(&dir, "2026-10-21"), running_at(Some(SHA_A), "2026-10-21"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
