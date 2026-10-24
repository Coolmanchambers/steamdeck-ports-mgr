//! The HTTP client of the cloud services: one request in, one reply out, every failure already turned into a
//! [`ServiceError`] with a kind the screens understand.
//!
//! A production client (`Net::production`) only speaks HTTPS. A client for tests (`Net::local_test`) also
//! speaks plain HTTP, but only to this machine (127.0.0.1 or localhost), so a mock server can stand in for
//! Google; it exists only in test and debug builds, never in a release build.

use super::log;
use super::service::{ErrorKind, ServiceError};
use serde_json::Value;
use std::io::{Read, Write};
use std::time::Duration;

pub enum Body {
    None,
    Bytes(Vec<u8>),
    /// `application/x-www-form-urlencoded`.
    Form(Vec<(String, String)>),
}

pub struct Request {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Body,
}

/// A request never prints its headers (they carry tokens) or its url (an upload-session url acts like a key).
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request").field("method", &self.method).finish_non_exhaustive()
    }
}

impl Request {
    pub fn new(method: &'static str, url: impl Into<String>) -> Request {
        Request { method, url: url.into(), headers: Vec::new(), body: Body::None }
    }

    pub fn get(url: impl Into<String>) -> Request {
        Request::new("GET", url)
    }

    pub fn post(url: impl Into<String>) -> Request {
        Request::new("POST", url)
    }

    pub fn header(mut self, name: &str, value: impl Into<String>) -> Request {
        self.headers.push((name.to_string(), value.into()));
        self
    }

    pub fn bearer(self, token: &str) -> Request {
        self.header("Authorization", format!("Bearer {token}"))
    }

    pub fn form(mut self, fields: &[(&str, &str)]) -> Request {
        self.body = Body::Form(fields.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect());
        self
    }

    pub fn bytes(mut self, bytes: Vec<u8>) -> Request {
        self.body = Body::Bytes(bytes);
        self
    }
}

#[derive(Clone, Default)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl std::fmt::Debug for Reply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reply").field("status", &self.status).field("body_bytes", &self.body.len()).finish()
    }
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> Option<Value> {
        serde_json::from_slice(&self.body).ok()
    }

    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn retry_after(&self) -> Option<u64> {
        self.header("retry-after").and_then(|v| v.trim().parse::<u64>().ok())
    }
}

/// `application/x-www-form-urlencoded` text for `fields`.
pub fn form_encode(fields: &[(String, String)]) -> String {
    fn esc(s: &str) -> String {
        let mut out = String::new();
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }
    fields.iter().map(|(k, v)| format!("{}={}", esc(k), esc(v))).collect::<Vec<_>>().join("&")
}

/// Percent-encodes one piece of a url (a query value or a path piece).
pub fn url_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// What a failed HTTP status means. `reason` is the service's own code word when its answer had one
/// (`storageQuotaExceeded`, `rateLimitExceeded`, `invalid_grant`); the text of a message is never used.
pub fn classify_status(status: u16, reason: Option<&str>, retry_after: Option<u64>) -> ServiceError {
    let kind = match (status, reason) {
        (_, Some("storageQuotaExceeded")) | (507, _) => ErrorKind::StorageFull,
        (_, Some("rateLimitExceeded" | "userRateLimitExceeded" | "slow_down")) | (429, _) => ErrorKind::RateLimited,
        (_, Some("invalid_grant")) | (401, _) => ErrorKind::SignedOut,
        (408, _) | (500..=599, _) => ErrorKind::ServiceDown,
        _ => ErrorKind::Other,
    };
    let detail = match reason {
        Some(r) => format!("HTTP {status}, {r}"),
        None => format!("HTTP {status}"),
    };
    ServiceError { kind, detail, retry_after }
}

fn classify_transport(e: &ureq::Error) -> ServiceError {
    let text = format!("{e:?}").to_ascii_lowercase();
    if text.contains("notvalidyet") || text.contains("not valid yet") || (text.contains("certificate") && text.contains("expired")) {
        return ServiceError::new(ErrorKind::ClockWrong, "the secure connection was refused because of the date and time");
    }
    match e {
        ureq::Error::Io(_) | ureq::Error::Timeout(_) | ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => ServiceError::new(ErrorKind::Offline, "no connection"),
        other => ServiceError::new(ErrorKind::Other, format!("the connection failed ({other})")),
    }
}

#[derive(Clone)]
pub struct Net {
    agent: ureq::Agent,
    plain_http_to_this_machine: bool,
}

impl Net {
    pub fn production() -> Net {
        Net { agent: crate::core::http::cloud_agent(true), plain_http_to_this_machine: false }
    }

    /// For a mock server on this machine: plain HTTP to 127.0.0.1 or localhost is allowed, HTTPS still works.
    #[cfg(any(test, debug_assertions))]
    pub fn local_test() -> Net {
        Net { agent: crate::core::http::cloud_agent(false), plain_http_to_this_machine: true }
    }

    fn check_url(&self, url: &str) -> Result<(), ServiceError> {
        if url.starts_with("https://") {
            return Ok(());
        }
        let local = ["http://127.0.0.1", "http://localhost"].iter().any(|p| url.strip_prefix(p).is_some_and(|rest| rest.starts_with([':', '/']) || rest.is_empty()));
        if self.plain_http_to_this_machine && local {
            return Ok(());
        }
        Err(ServiceError::other("a cloud address that is not secure (https) was refused"))
    }

    fn run(&self, req: Request) -> Result<ureq::http::Response<ureq::Body>, ServiceError> {
        self.check_url(&req.url)?;
        let mut builder = ureq::http::Request::builder().method(req.method).uri(&req.url);
        for (k, v) in &req.headers {
            builder = builder.header(k, v);
        }
        let bad = |e: ureq::http::Error| ServiceError::other(format!("could not build the request ({e})"));
        let result = match req.body {
            Body::None => self.agent.run(builder.body(()).map_err(bad)?),
            Body::Bytes(bytes) => self.agent.run(builder.body(bytes).map_err(bad)?),
            Body::Form(fields) => self.agent.run(builder.header("Content-Type", "application/x-www-form-urlencoded").body(form_encode(&fields).into_bytes()).map_err(bad)?),
        };
        result.map_err(|e| classify_transport(&e))
    }

    fn head(resp: &ureq::http::Response<ureq::Body>) -> (u16, Vec<(String, String)>) {
        let headers = resp.headers().iter().filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string()))).collect();
        (resp.status().as_u16(), headers)
    }

    /// The whole reply, whatever its status (a 4xx is a reply, not an error). The body is capped at 8 MB.
    pub fn send(&self, req: Request) -> Result<Reply, ServiceError> {
        let mut resp = self.run(req)?;
        let (status, headers) = Net::head(&resp);
        let body = resp.body_mut().with_config().limit(8 * 1024 * 1024).read_to_vec().map_err(|e| classify_transport(&e))?;
        Ok(Reply { status, headers, body })
    }

    /// Streams a successful body into `out`, calling `progress(bytes so far, total or 0)`. A reply that is not
    /// a success comes back as `Err(reply)` with its (small) body, for the caller to classify.
    pub fn download(&self, req: Request, out: &mut dyn Write, progress: &mut dyn FnMut(u64, u64)) -> Result<Result<(), Reply>, ServiceError> {
        let mut resp = self.run(req)?;
        let (status, headers) = Net::head(&resp);
        if !(200..300).contains(&status) {
            let body = resp.body_mut().with_config().limit(64 * 1024).read_to_vec().unwrap_or_default();
            return Ok(Err(Reply { status, headers, body }));
        }
        let total = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse::<u64>().ok()).unwrap_or(0);
        let mut reader = resp.body_mut().as_reader();
        let mut buf = vec![0u8; 256 * 1024];
        let mut done = 0u64;
        loop {
            let n = reader.read(&mut buf).map_err(|e| {
                log::note(format!("download interrupted ({})", e.kind()));
                ServiceError::new(ErrorKind::Offline, "the download was interrupted")
            })?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(|e| ServiceError::other(format!("could not write the file ({})", e.kind())))?;
            done += n as u64;
            progress(done, total);
        }
        if total > 0 && done != total {
            return Err(ServiceError::new(ErrorKind::Offline, "the download ended early"));
        }
        Ok(Ok(()))
    }
}

/// How long to wait before the next try after a failure: the service's own `Retry-After` when it gave one, else
/// 5, 15 then 60 minutes (the step is the number of failures in a row so far, starting at 0).
pub fn backoff(step: usize, retry_after: Option<u64>) -> Duration {
    if let Some(secs) = retry_after {
        return Duration::from_secs(secs.clamp(1, 3_600));
    }
    Duration::from_secs([5 * 60, 15 * 60, 60 * 60][step.min(2)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_encoding_escapes_what_it_must() {
        let f = form_encode(&[("a b".into(), "x&y=z".into()), ("k".into(), "é".into())]);
        assert_eq!(f, "a+b=x%26y%3Dz&k=%C3%A9");
    }

    #[test]
    fn statuses_map_to_kinds_by_code_not_by_message() {
        assert_eq!(classify_status(403, Some("storageQuotaExceeded"), None).kind, ErrorKind::StorageFull);
        assert_eq!(classify_status(507, None, None).kind, ErrorKind::StorageFull);
        assert_eq!(classify_status(403, Some("rateLimitExceeded"), None).kind, ErrorKind::RateLimited);
        assert_eq!(classify_status(429, None, Some(7)).retry_after, Some(7));
        assert_eq!(classify_status(400, Some("invalid_grant"), None).kind, ErrorKind::SignedOut);
        assert_eq!(classify_status(401, None, None).kind, ErrorKind::SignedOut);
        assert_eq!(classify_status(503, None, None).kind, ErrorKind::ServiceDown);
        assert_eq!(classify_status(403, Some("forbidden"), None).kind, ErrorKind::Other);
        assert_eq!(classify_status(404, None, None).kind, ErrorKind::Other);
    }

    #[test]
    fn backoff_follows_retry_after_and_then_5_15_60_minutes() {
        assert_eq!(backoff(0, None), Duration::from_secs(300));
        assert_eq!(backoff(1, None), Duration::from_secs(900));
        assert_eq!(backoff(2, None), Duration::from_secs(3_600));
        assert_eq!(backoff(9, None), Duration::from_secs(3_600));
        assert_eq!(backoff(0, Some(42)), Duration::from_secs(42));
        assert_eq!(backoff(0, Some(999_999)), Duration::from_secs(3_600));
    }

    #[test]
    fn production_refuses_anything_that_is_not_https() {
        let net = Net::production();
        for url in ["http://example.com/x", "http://127.0.0.1:1/x", "ftp://x/y", "//x"] {
            assert!(net.send(Request::get(url)).unwrap_err().detail.contains("not secure"), "{url}");
        }
    }

    #[test]
    fn the_test_client_allows_plain_http_to_this_machine_only() {
        let net = Net::local_test();
        assert!(net.check_url("http://127.0.0.1:8080/x").is_ok());
        assert!(net.check_url("http://localhost:8080/x").is_ok());
        assert!(net.check_url("http://localhost.evil.example/x").is_err());
        assert!(net.check_url("http://127.0.0.1.evil.example/x").is_err());
        assert!(net.check_url("http://example.com/x").is_err());
    }

    #[test]
    fn a_request_never_prints_its_headers_or_url() {
        let text = format!("{:?}", Request::post("https://x/upload?upload_id=SESSIONKEY").bearer("TOKENVALUE").bytes(vec![1]));
        assert!(!text.contains("TOKENVALUE") && !text.contains("SESSIONKEY"), "{text}");
    }
}
