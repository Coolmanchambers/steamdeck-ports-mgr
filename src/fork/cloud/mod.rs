//! Cloud backup of the save backups (`docs/design/cloud-backup.md` in the project notes).
//!
//! The app copies the zips it already makes in `Saves Backup` to the user's own Google Drive, and can copy one
//! back. Nothing here runs until the user signs in, and nothing here deletes anything in the cloud: the
//! service trait ([`service::CloudService`]) can list, upload and download, and that is all.
//!
//! - `sha256`, `log`: a file checksum and the one place diagnostics are written (never a token).
//! - `tokens`: `cloud-tokens.json` (mode 0600), the only file that holds sign-in tokens and the account email.
//! - `record`: `cloud-backup.json`, what was uploaded where (no secrets), safe in a bug report.
//! - `local`, `plan`, `sync`: which zips are candidates, what to upload, and one pass that does it.
//! - `service`: the trait every cloud service implements, and the failure kinds the screens speak of.
//! - `net`, `oauth`: the HTTPS client and the device-code sign-in with refresh.
//! - `build`: the Google client id and secret a build was compiled with (CI secrets), or "not set up".
//!
//! Google Drive itself, the screens and the hooks that start a pass come in the next modules.

pub mod build;
pub mod log;
pub mod local;
pub mod net;
pub mod oauth;
pub mod plan;
pub mod record;
pub mod service;
pub mod sha256;
pub mod sync;
pub mod tokens;

#[cfg(test)]
pub mod fake;
#[cfg(test)]
mod mock_http;
#[cfg(test)]
mod oauth_tests;
#[cfg(test)]
mod sync_tests;

/// The folder name this Deck gets in the cloud: its network name, cleaned so it is a plain folder name.
pub fn clean_device_name(raw: &str) -> String {
    let cleaned: String = raw.trim().chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-') { c } else { '-' }).collect();
    let cleaned = cleaned.trim_matches(|c: char| c == '.' || c == '-' || c == ' ').chars().take(40).collect::<String>();
    if cleaned.is_empty() {
        "Steam Deck".to_string()
    } else {
        cleaned
    }
}

/// This Deck's cloud folder name (`/etc/hostname`, then `HOSTNAME`).
pub fn device_name() -> String {
    let raw = std::fs::read_to_string("/etc/hostname").ok().filter(|s| !s.trim().is_empty()).or_else(|| std::env::var("HOSTNAME").ok()).unwrap_or_default();
    clean_device_name(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_names_are_plain_folder_names() {
        assert_eq!(clean_device_name("steamdeck\n"), "steamdeck");
        assert_eq!(clean_device_name("  "), "Steam Deck");
        assert_eq!(clean_device_name("a/b\\c:d"), "a-b-c-d");
        assert_eq!(clean_device_name("../.."), "Steam Deck");
        assert_eq!(clean_device_name(&"x".repeat(100)).len(), 40);
        assert_eq!(clean_device_name("Tim's Deck"), "Tim-s Deck");
    }
}
