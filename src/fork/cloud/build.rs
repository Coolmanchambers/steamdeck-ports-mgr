//! The Google client id and secret a build was compiled with.
//!
//! They come from the release build's environment (`DPM_GOOGLE_CLIENT_ID`, `DPM_GOOGLE_CLIENT_SECRET`, set
//! from GitHub Actions secrets in `.github/workflows/ci.yml`) through `option_env!`, exactly like the art
//! sources' `BuildCreds`. A build without them (a local build, a fork, a pull request) has no client: cloud
//! backup then says "isn't set up in this build" and nothing else changes.
//!
//! Google documents that the secret of an installed app (a "TV and Limited Input" client) "is obviously not
//! treated as a secret", so compiling it into a public release is expected. It still stays out of the
//! repository, the logs and the tests.

use std::fmt;

#[derive(Clone, PartialEq, Eq)]
pub struct GoogleClient {
    pub client_id: String,
    pub client_secret: String,
}

impl fmt::Debug for GoogleClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GoogleClient").finish_non_exhaustive()
    }
}

/// A value that is plausibly a client id or secret: not empty, no spaces or control characters.
fn plausible(v: Option<&str>) -> Option<String> {
    v.map(str::trim).filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(|c| c.is_whitespace() || c.is_control())).map(str::to_string)
}

pub fn client_from(id: Option<&str>, secret: Option<&str>) -> Option<GoogleClient> {
    Some(GoogleClient { client_id: plausible(id)?, client_secret: plausible(secret)? })
}

/// What this binary was compiled with, or `None`.
pub fn google_client() -> Option<GoogleClient> {
    client_from(option_env!("DPM_GOOGLE_CLIENT_ID"), option_env!("DPM_GOOGLE_CLIENT_SECRET"))
}

/// The line the screens show when there is no client.
pub const NOT_SET_UP: &str = "Cloud backup isn't set up in this build.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_halves_are_needed() {
        assert!(client_from(Some("id-1"), Some("secret-1")).is_some());
        assert!(client_from(Some("id-1"), None).is_none());
        assert!(client_from(None, Some("secret-1")).is_none());
        assert!(client_from(None, None).is_none());
    }

    #[test]
    fn empty_and_odd_values_count_as_not_set() {
        assert!(client_from(Some(""), Some("s")).is_none(), "an unset CI secret arrives as an empty string");
        assert!(client_from(Some("  "), Some("s")).is_none());
        assert!(client_from(Some("a b"), Some("s")).is_none());
        assert!(client_from(Some("a\nb"), Some("s")).is_none());
        assert!(client_from(Some(&"x".repeat(300)), Some("s")).is_none());
        assert_eq!(client_from(Some(" id \n"), Some("s")).unwrap().client_id, "id");
    }

    #[test]
    fn debug_hides_the_values() {
        let text = format!("{:?}", client_from(Some("ID-VALUE-123"), Some("SECRET-VALUE-456")).unwrap());
        assert!(!text.contains("VALUE"), "{text}");
    }

    #[test]
    fn a_build_without_the_variables_has_no_client() {
        // The test environment does not set them; a build that does is a release build with real values.
        if option_env!("DPM_GOOGLE_CLIENT_ID").is_none() {
            assert!(google_client().is_none());
        }
    }
}
