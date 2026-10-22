//! The one door every art source talks through: [`Fetch`].
//!
//! The real implementation, [`Net`], goes through the app's own http helper (`core::http::agent`,
//! https only), reads at most a given number of bytes, and turns every outcome into a status code or a
//! coarse [`NetError`]. Tests plug in a fake that replays saved responses, so no test touches the
//! network and no fixture holds a key.
//!
//! Secrets travel in URLs (TheGamesDB and ScreenScraper want them as query parameters). Two rules keep
//! them out of logs and messages: the error values built here never contain the URL (they are fixed
//! phrases chosen by the kind of failure), and [`redact`] / [`strip_query`] clean any text that might.

use std::time::Duration;

/// Largest JSON answer read from an API.
pub const MAX_JSON_BYTES: u64 = 8 * 1024 * 1024;
/// Largest image read from a CDN.
pub const MAX_IMAGE_BYTES: u64 = 24 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    /// Where the answer really came from, after any redirects. Clients refuse an answer from a host the
    /// source does not own.
    pub final_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetError {
    /// No answer at all: no network, DNS, TLS, timeout. The text is a fixed phrase, never the URL.
    Offline(String),
    /// The answer was bigger than the limit given to `get`.
    TooLarge,
    /// The request itself was refused before it was sent (not https, empty).
    Refused(String),
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetError::Offline(why) => write!(f, "no answer from the server ({why})"),
            NetError::TooLarge => write!(f, "the answer was too large"),
            NetError::Refused(why) => write!(f, "request refused: {why}"),
        }
    }
}

/// An HTTP GET. A 4xx or 5xx answer is `Ok` with its status (the clients decide what it means); only a
/// missing answer is an `Err`.
pub trait Fetch {
    fn get(&self, url: &str, headers: &[(&str, &str)], max_bytes: u64) -> Result<Response, NetError>;
}

/// The real thing.
pub struct Net {
    pub timeout: Duration,
}

impl Net {
    pub fn new() -> Net {
        Net { timeout: Duration::from_secs(20) }
    }
}

impl Default for Net {
    fn default() -> Self {
        Net::new()
    }
}

impl Fetch for Net {
    fn get(&self, url: &str, headers: &[(&str, &str)], max_bytes: u64) -> Result<Response, NetError> {
        if !url.starts_with("https://") {
            return Err(NetError::Refused("only https addresses are used".into()));
        }
        let agent = crate::core::http::agent(self.timeout);
        // A few redirects are normal for image hosts; credentials never follow them (they are in the
        // address, which the redirect replaces), and the clients check where the answer came from.
        let mut request = agent.get(url).config().max_redirects(3).redirect_auth_headers(ureq::config::RedirectAuthHeaders::Never).build();
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        match request.call() {
            Ok(mut response) => {
                let status = response.status().as_u16();
                let final_url = ureq::ResponseExt::get_uri(&response).to_string();
                match response.body_mut().with_config().limit(max_bytes).read_to_vec() {
                    Ok(body) => Ok(Response { status, body, final_url }),
                    Err(ureq::Error::BodyExceedsLimit(_)) => Err(NetError::TooLarge),
                    Err(e) => Err(offline(&e)),
                }
            }
            Err(ureq::Error::StatusCode(status)) => Ok(Response { status, body: Vec::new(), final_url: url.to_string() }),
            Err(e) => Err(offline(&e)),
        }
    }
}

/// A fixed phrase per kind of failure: `ureq`'s own text can carry the address.
fn offline(e: &ureq::Error) -> NetError {
    let why = match e {
        ureq::Error::Timeout(_) => "timed out",
        ureq::Error::HostNotFound => "could not find the server",
        ureq::Error::ConnectionFailed => "could not connect",
        ureq::Error::Io(_) => "network error",
        _ => "request failed",
    };
    NetError::Offline(why.to_string())
}

/// Percent-encodes everything but the unreserved characters, for one path segment or query value.
pub fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The host of an `https://` address, lowercased; `None` for anything else.
pub fn https_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.contains('@') {
        return None;
    }
    let host = authority.rsplit_once(':').map_or(authority, |(h, port)| if port.bytes().all(|b| b.is_ascii_digit()) { h } else { authority });
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// True when `url` is https and its host is one of `suffixes` or a subdomain of one. API answers carry
/// image addresses; they are data, not instructions, so only the source's own hosts are followed.
pub fn host_allowed(url: &str, suffixes: &[&str]) -> bool {
    let Some(host) = https_host(url) else { return false };
    suffixes.iter().any(|s| host == *s || host.ends_with(&format!(".{s}")))
}

/// Replaces every occurrence of each secret with `***`. Values shorter than 4 characters are ignored
/// (replacing them would mangle ordinary words).
pub fn redact(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for s in secrets.iter().filter(|s| s.chars().count() >= 4) {
        out = out.replace(s.as_str(), "***");
    }
    out
}

/// Cuts the `?query` off every web address in `text`, since queries are where keys travel. A question
/// mark in ordinary prose is left alone: only a word that holds `://` is cut.
pub fn strip_query(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for piece in text.split_inclusive(char::is_whitespace) {
        let word = piece.trim_end();
        match (word.contains("://"), word.find('?')) {
            (true, Some(at)) => {
                out.push_str(&word[..at]);
                out.push_str("?...");
                out.push_str(&piece[word.len()..]);
            }
            _ => out.push_str(piece),
        }
    }
    out
}

/// Longest server text kept in a message or in the saved state; the rest is cut.
pub const MAX_SERVER_TEXT: usize = 200;

/// `text` cut to `max` characters, with `...` when something was removed.
pub fn cap(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(3)).collect();
    format!("{}...", cut.trim_end())
}

#[cfg(test)]
pub(crate) mod fake {
    //! A scripted [`Fetch`] for tests: answers by the first rule whose pattern the URL contains and
    //! records every request it saw.

    use super::*;
    use std::cell::RefCell;

    pub enum Reply {
        Ok(u16, Vec<u8>),
        /// A 200 that came from another address after a redirect.
        Redirected(String, Vec<u8>),
        Offline,
        TooLarge,
    }

    pub struct Fake {
        rules: Vec<(String, Reply)>,
        pub seen: RefCell<Vec<(String, Vec<(String, String)>)>>,
    }

    impl Fake {
        pub fn new() -> Fake {
            Fake { rules: Vec::new(), seen: RefCell::new(Vec::new()) }
        }

        /// Answers any URL containing `pattern` (every time).
        pub fn on(mut self, pattern: &str, status: u16, body: impl Into<Vec<u8>>) -> Fake {
            self.rules.push((pattern.to_string(), Reply::Ok(status, body.into())));
            self
        }

        pub fn on_json(self, pattern: &str, status: u16, body: &str) -> Fake {
            self.on(pattern, status, body.as_bytes().to_vec())
        }

        /// Answers 200 as if the request had been redirected to `final_url`.
        pub fn redirected_on(mut self, pattern: &str, final_url: &str, body: impl Into<Vec<u8>>) -> Fake {
            self.rules.push((pattern.to_string(), Reply::Redirected(final_url.to_string(), body.into())));
            self
        }

        pub fn offline_on(mut self, pattern: &str) -> Fake {
            self.rules.push((pattern.to_string(), Reply::Offline));
            self
        }

        pub fn too_large_on(mut self, pattern: &str) -> Fake {
            self.rules.push((pattern.to_string(), Reply::TooLarge));
            self
        }

        /// Everything not matched is offline.
        pub fn requests(&self) -> Vec<String> {
            self.seen.borrow().iter().map(|(u, _)| u.clone()).collect()
        }

        pub fn count(&self, pattern: &str) -> usize {
            self.requests().iter().filter(|u| u.contains(pattern)).count()
        }

        pub fn header_of(&self, pattern: &str, name: &str) -> Option<String> {
            let seen = self.seen.borrow();
            let (_, headers) = seen.iter().find(|(u, _)| u.contains(pattern))?;
            headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone())
        }
    }

    impl Fetch for Fake {
        fn get(&self, url: &str, headers: &[(&str, &str)], _max_bytes: u64) -> Result<Response, NetError> {
            if !url.starts_with("https://") {
                return Err(NetError::Refused("only https addresses are used".into()));
            }
            self.seen.borrow_mut().push((url.to_string(), headers.iter().map(|(n, v)| (n.to_string(), v.to_string())).collect()));
            for (pattern, reply) in &self.rules {
                if url.contains(pattern.as_str()) {
                    return match reply {
                        Reply::Ok(status, body) => Ok(Response { status: *status, body: body.clone(), final_url: url.to_string() }),
                        Reply::Redirected(to, body) => Ok(Response { status: 200, body: body.clone(), final_url: to.clone() }),
                        Reply::Offline => Err(NetError::Offline("could not connect".into())),
                        Reply::TooLarge => Err(NetError::TooLarge),
                    };
                }
            }
            Err(NetError::Offline("could not connect".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_are_encoded_but_unreserved_characters_are_not() {
        assert_eq!(encode_component("Ship of Harkinian"), "Ship%20of%20Harkinian");
        assert_eq!(encode_component("a&b=c/d?e#f+g"), "a%26b%3Dc%2Fd%3Fe%23f%2Bg");
        assert_eq!(encode_component("Zelda: Majora's Mask"), "Zelda%3A%20Majora%27s%20Mask");
        assert_eq!(encode_component("abc-DEF_1.2~"), "abc-DEF_1.2~");
        assert_eq!(encode_component("Pokémon"), "Pok%C3%A9mon");
    }

    #[test]
    fn hosts_are_read_and_checked_against_the_allowed_suffixes() {
        assert_eq!(https_host("https://cdn2.steamgriddb.com/grid/a.png").as_deref(), Some("cdn2.steamgriddb.com"));
        assert_eq!(https_host("https://User@evil.example/x"), None);
        assert_eq!(https_host("http://cdn2.steamgriddb.com/x"), None);
        assert_eq!(https_host("https://cdn.thegamesdb.net:443/x").as_deref(), Some("cdn.thegamesdb.net"));
        let allowed = ["steamgriddb.com"];
        assert!(host_allowed("https://cdn2.steamgriddb.com/grid/a.png", &allowed));
        assert!(host_allowed("https://steamgriddb.com/a.png", &allowed));
        assert!(!host_allowed("https://evilsteamgriddb.com/a.png", &allowed));
        assert!(!host_allowed("https://steamgriddb.com.evil.example/a.png", &allowed));
        assert!(!host_allowed("http://cdn2.steamgriddb.com/a.png", &allowed));
        assert!(!host_allowed("file:///etc/passwd", &allowed));
    }

    #[test]
    fn redact_hides_every_secret_and_leaves_short_ones_alone() {
        let text = "GET https://api.example/x?apikey=SECRET12345&u=bob failed for SECRET12345";
        let clean = redact(text, &["SECRET12345".to_string(), "bo".to_string()]);
        assert!(!clean.contains("SECRET12345"), "{clean}");
        assert!(clean.contains("***"));
        assert!(clean.contains("u=bob"), "short secrets are not replaced: {clean}");
    }

    #[test]
    fn strip_query_cuts_the_query_off_addresses_only() {
        assert_eq!(strip_query("failed: https://a.example/x?apikey=K&b=2 (retrying)"), "failed: https://a.example/x?... (retrying)");
        assert_eq!(strip_query("no queries here"), "no queries here");
        assert!(!strip_query("https://a/x?devpassword=P").contains('P'));
    }

    #[test]
    fn a_question_mark_in_prose_survives_and_long_text_is_capped() {
        assert_eq!(strip_query("Is it ok? yes"), "Is it ok? yes");
        assert_eq!(strip_query("see https://x.example/y?k=1
next line?"), "see https://x.example/y?...
next line?");
        assert_eq!(cap("short", 200), "short");
        let long = "x".repeat(500);
        assert!(cap(&long, 200).chars().count() <= 200 && cap(&long, 200).ends_with("..."));
    }

    #[test]
    fn the_real_client_refuses_plain_http_without_any_network() {
        let net = Net::new();
        assert!(matches!(net.get("http://example.invalid/x", &[], 10), Err(NetError::Refused(_))));
        assert!(matches!(net.get("ftp://example.invalid/x", &[], 10), Err(NetError::Refused(_))));
    }

    #[test]
    fn the_fake_replays_by_pattern_and_records_what_it_saw() {
        let fake = fake::Fake::new().on_json("/a", 200, "{}").offline_on("/b");
        assert_eq!(fake.get("https://x/a", &[("Authorization", "Bearer t")], 10).unwrap().status, 200);
        assert!(matches!(fake.get("https://x/b", &[], 10), Err(NetError::Offline(_))));
        assert!(matches!(fake.get("https://x/unknown", &[], 10), Err(NetError::Offline(_))));
        assert_eq!(fake.count("/a"), 1);
        assert_eq!(fake.header_of("/a", "authorization").as_deref(), Some("Bearer t"));
    }
}
