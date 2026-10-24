//! Where the cloud calls go. In a release build: Google, over HTTPS, nothing else. In a test or debug build,
//! `DPM_CLOUD_TEST_BASE=http://127.0.0.1:<port>` sends every call to a mock server on this machine instead
//! (and supplies a dummy client when the build has none), so the screens can be driven end to end without
//! Google. A release build cannot be redirected: that code does not exist there.

use super::build::{google_client, GoogleClient};
use super::net::Net;
use super::oauth::OAuthConfig;
use super::google::{GoogleDrive, TokenSource};
use super::service::{CloudService, ServiceId};

/// Sign-in asks for these and nothing else: the Drive files this app makes, and the account email to show.
pub const GOOGLE_SCOPE: &str = "openid email https://www.googleapis.com/auth/drive.file";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Urls {
    pub device_code: String,
    pub token: String,
    pub userinfo: String,
    /// `.../drive/v3` (files, folders, downloads).
    pub drive_api: String,
    /// `.../upload/drive/v3` (uploads).
    pub drive_upload: String,
}

impl Urls {
    pub fn google() -> Urls {
        Urls {
            device_code: "https://oauth2.googleapis.com/device/code".into(),
            token: "https://oauth2.googleapis.com/token".into(),
            userinfo: "https://www.googleapis.com/oauth2/v3/userinfo".into(),
            drive_api: "https://www.googleapis.com/drive/v3".into(),
            drive_upload: "https://www.googleapis.com/upload/drive/v3".into(),
        }
    }

    /// Everything under one base address (a mock server).
    pub fn under(base: &str) -> Urls {
        let base = base.trim_end_matches('/');
        Urls { device_code: format!("{base}/device/code"), token: format!("{base}/token"), userinfo: format!("{base}/oauth2/v3/userinfo"), drive_api: format!("{base}/drive/v3"), drive_upload: format!("{base}/upload/drive/v3") }
    }
}

/// Everything a cloud call needs about where it goes and who it is.
#[derive(Clone)]
pub struct Setup {
    pub net: Net,
    pub urls: Urls,
    pub client: GoogleClient,
}

impl Setup {
    pub fn oauth(&self) -> OAuthConfig {
        OAuthConfig { device_code_url: self.urls.device_code.clone(), token_url: self.urls.token.clone(), client_id: self.client.client_id.clone(), client_secret: self.client.client_secret.clone(), scope: GOOGLE_SCOPE.into(), userinfo_url: self.urls.userinfo.clone() }
    }
}

/// The service object a pass, a listing or a download talks to: the demo folder in a debug or test run that
/// asks for one, else Google Drive.
pub fn service(setup: &Setup, tokens_path: &std::path::Path, id: ServiceId) -> Box<dyn CloudService> {
    #[cfg(any(test, debug_assertions))]
    if let Some(dir) = super::demo::demo_dir() {
        let (net, cfg, path) = (setup.net.clone(), setup.oauth(), tokens_path.to_path_buf());
        let check = move || {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            super::store::access_token(&net, &cfg, &path, id, now).map(|_| ())
        };
        return Box::new(super::demo::FolderService::new(dir).with_auth(Box::new(check)));
    }
    let auth = TokenSource { net: setup.net.clone(), cfg: setup.oauth(), tokens_path: tokens_path.to_path_buf(), id };
    Box::new(GoogleDrive::new(setup.net.clone(), setup.urls.clone(), auth))
}

#[cfg(any(test, debug_assertions))]
fn test_setup() -> Option<Setup> {
    let base = std::env::var("DPM_CLOUD_TEST_BASE").ok().filter(|b| b.starts_with("http://127.0.0.1") || b.starts_with("http://localhost"))?;
    let client = google_client().unwrap_or(GoogleClient { client_id: "test-client".into(), client_secret: "test-secret".into() });
    Some(Setup { net: Net::local_test(), urls: Urls::under(&base), client })
}

#[cfg(not(any(test, debug_assertions)))]
fn test_setup() -> Option<Setup> {
    None
}

/// The setup for this build: the mock server in a test or debug run that asks for one, else Google with the
/// client this build was compiled with, else `None` ("Cloud backup isn't set up in this build").
pub fn current() -> Option<Setup> {
    test_setup().or_else(|| Some(Setup { net: Net::production(), urls: Urls::google(), client: google_client()? }))
}

/// True when cloud backup can work in this build (the screens say "isn't set up" when it cannot).
pub fn available() -> bool {
    current().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_urls_are_all_https_and_the_scope_is_the_narrow_one() {
        let u = Urls::google();
        for url in [&u.device_code, &u.token, &u.userinfo, &u.drive_api, &u.drive_upload] {
            assert!(url.starts_with("https://"), "{url}");
        }
        assert_eq!(GOOGLE_SCOPE, "openid email https://www.googleapis.com/auth/drive.file");
        assert_eq!(GOOGLE_SCOPE.split(' ').collect::<Vec<_>>(), ["openid", "email", "https://www.googleapis.com/auth/drive.file"]);
        assert!(!GOOGLE_SCOPE.contains("auth/drive ") && !GOOGLE_SCOPE.ends_with("auth/drive"), "never the full drive scope");
    }

    #[test]
    fn the_production_sign_in_config_carries_exactly_that_scope_and_the_userinfo_address() {
        let setup = Setup { net: Net::production(), urls: Urls::google(), client: GoogleClient { client_id: "id".into(), client_secret: "secret".into() } };
        let cfg = setup.oauth();
        assert_eq!(cfg.scope, GOOGLE_SCOPE);
        assert_eq!(cfg.userinfo_url, "https://www.googleapis.com/oauth2/v3/userinfo");
        assert_eq!(cfg.device_code_url, "https://oauth2.googleapis.com/device/code");
    }

    #[test]
    fn a_mock_base_puts_every_call_under_it() {
        let u = Urls::under("http://127.0.0.1:9/");
        assert_eq!((u.device_code.as_str(), u.token.as_str(), u.drive_api.as_str(), u.drive_upload.as_str()), ("http://127.0.0.1:9/device/code", "http://127.0.0.1:9/token", "http://127.0.0.1:9/drive/v3", "http://127.0.0.1:9/upload/drive/v3"));
        assert_eq!(u.userinfo, "http://127.0.0.1:9/oauth2/v3/userinfo");
    }
}
