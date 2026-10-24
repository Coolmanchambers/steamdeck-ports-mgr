//! Signing in with a code (the OAuth "device" flow) and keeping the sign-in alive with the refresh token.
//!
//! The Deck shows a short code and a QR code; the person finishes on their phone. The app asks the service now
//! and then whether that happened ([`poll`]); when it did, it holds an access token (an hour) and a refresh
//! token (until revoked). `store::access_token` hands out a good access token, refreshing it when it is about to
//! run out; only `invalid_grant` (the refresh token is dead) means signed out. A token endpoint that refuses
//! the app itself (`invalid_client`: bad build credentials) is a plain failure that never touches the stored
//! sign-in.
//!
//! The URLs are in [`OAuthConfig`], so tests point them at a mock server. Nothing here logs a token, a code
//! or an email, and the types print no values.

use super::net::{classify_status, Net, Reply, Request};
use super::service::{ErrorKind, ServiceError};
use super::tokens::ServiceTokens;
use serde_json::Value;
use std::fmt;

#[derive(Clone)]
pub struct OAuthConfig {
    pub device_code_url: String,
    pub token_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub scope: String,
    /// Where the account email is asked for when the token answer has no id token.
    pub userinfo_url: String,
}

impl fmt::Debug for OAuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthConfig").field("scope", &self.scope).finish_non_exhaustive()
    }
}

/// What the person needs to see, plus the code the app polls with.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceCode {
    device_code: String,
    pub user_code: String,
    pub verification_url: String,
    /// Seconds since 1970.
    pub expires_at: u64,
    /// Seconds between polls.
    pub interval: u64,
}

impl fmt::Debug for DeviceCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceCode").field("expires_at", &self.expires_at).field("interval", &self.interval).finish_non_exhaustive()
    }
}

/// A fresh set of tokens from the service.
#[derive(Clone, PartialEq, Eq)]
pub struct Tokens {
    pub access_token: String,
    pub expires_in: u64,
    /// Present on the first sign-in; on a refresh only when the service rotates it.
    pub refresh_token: Option<String>,
    pub email: Option<String>,
}

impl fmt::Debug for Tokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tokens").field("expires_in", &self.expires_in).field("refresh_token", &self.refresh_token.is_some()).field("email", &self.email.is_some()).finish_non_exhaustive()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Poll {
    /// The person has not finished yet.
    Pending,
    /// Ask less often: add 5 seconds to the interval.
    SlowDown,
    Approved(Tokens),
    /// The person said no.
    Denied,
    /// The code ran out (or was used up).
    Expired,
    Failed(ServiceError),
}

/// The service's code word for an error answer (`error` as a string, or Google's nested `error.errors[0].reason`).
pub fn reason_of(reply: &Reply) -> Option<String> {
    let json = reply.json()?;
    match json.get("error")? {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => o.get("errors").and_then(|e| e.get(0)).and_then(|e| e.get("reason")).and_then(Value::as_str).map(str::to_string).or_else(|| o.get("status").and_then(Value::as_str).map(str::to_string)),
        _ => None,
    }
}

/// What the build's credentials being refused looks like to the person.
pub const BAD_BUILD: &str = "Cloud backup isn't set up correctly in this build";

/// A failed answer from the sign-in endpoints. Only `invalid_grant` is "signed out"; a 401 or `invalid_client`
/// from the token endpoint means this build's own credentials are wrong, which says nothing about the person's
/// sign-in.
fn failure(reply: &Reply) -> ServiceError {
    let reason = reason_of(reply);
    let mut e = classify_status(reply.status, reason.as_deref(), reply.retry_after());
    match reason.as_deref() {
        Some("invalid_client" | "unauthorized_client") => {
            e.kind = ErrorKind::Other;
            e.detail = BAD_BUILD.to_string();
        }
        Some("invalid_grant") => {}
        _ if e.kind == ErrorKind::SignedOut => e.kind = ErrorKind::Other,
        _ => {}
    }
    e
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string).filter(|s| !s.is_empty())
}

/// Asks for a code to show.
pub fn start(net: &Net, cfg: &OAuthConfig, now: u64) -> Result<DeviceCode, ServiceError> {
    let reply = net.send(Request::post(&cfg.device_code_url).form(&[("client_id", &cfg.client_id), ("scope", &cfg.scope)]))?;
    if !reply.ok() {
        return Err(failure(&reply));
    }
    let json = reply.json().ok_or_else(|| ServiceError::other("the sign-in answer could not be read"))?;
    let device_code = text(&json, "device_code");
    let user_code = text(&json, "user_code");
    let url = text(&json, "verification_url").or_else(|| text(&json, "verification_uri"));
    let (Some(device_code), Some(user_code), Some(verification_url)) = (device_code, user_code, url) else {
        return Err(ServiceError::other("the sign-in answer was missing its code"));
    };
    let expires_in = json.get("expires_in").and_then(Value::as_u64).unwrap_or(900).clamp(30, 1_800);
    let interval = json.get("interval").and_then(Value::as_u64).unwrap_or(5).clamp(1, 60);
    Ok(DeviceCode { device_code, user_code, verification_url, expires_at: now + expires_in, interval })
}

fn parse_tokens(reply: &Reply) -> Result<Tokens, ServiceError> {
    let json = reply.json().ok_or_else(|| ServiceError::other("the token answer could not be read"))?;
    let access_token = text(&json, "access_token").ok_or_else(|| ServiceError::other("the token answer had no access token"))?;
    // Google's device flow does not promise an id token; `poll` asks for the email when there is none.
    let email = text(&json, "id_token").and_then(|jwt| email_from_id_token(&jwt));
    Ok(Tokens { access_token, expires_in: json.get("expires_in").and_then(Value::as_u64).unwrap_or(3_600), refresh_token: text(&json, "refresh_token"), email })
}

/// One check whether the person has finished. An expired code is reported without asking the service.
pub fn poll(net: &Net, cfg: &OAuthConfig, code: &DeviceCode, now: u64) -> Poll {
    if now >= code.expires_at {
        return Poll::Expired;
    }
    let reply = match net.send(Request::post(&cfg.token_url).form(&[
        ("client_id", &cfg.client_id),
        ("client_secret", &cfg.client_secret),
        ("device_code", &code.device_code),
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
    ])) {
        Ok(r) => r,
        Err(e) => return Poll::Failed(e),
    };
    if reply.ok() {
        return match parse_tokens(&reply) {
            Ok(mut t) if t.refresh_token.is_some() => {
                if t.email.is_none() {
                    t.email = fetch_email(net, cfg, &t.access_token);
                }
                Poll::Approved(t)
            }
            Ok(_) => Poll::Failed(ServiceError::other("the sign-in gave no way to stay signed in")),
            Err(e) => Poll::Failed(e),
        };
    }
    match reason_of(&reply).as_deref() {
        Some("authorization_pending") => Poll::Pending,
        Some("slow_down") => Poll::SlowDown,
        Some("access_denied") => Poll::Denied,
        Some("expired_token" | "invalid_grant") => Poll::Expired,
        _ => Poll::Failed(failure(&reply)),
    }
}

/// The seconds to wait before the next poll, after `outcome`.
pub fn next_interval(current: u64, outcome: &Poll) -> u64 {
    match outcome {
        Poll::SlowDown => (current + 5).min(60),
        _ => current,
    }
}

/// A new access token from the refresh token. A dead refresh token is `SignedOut`.
pub fn refresh(net: &Net, cfg: &OAuthConfig, refresh_token: &str) -> Result<Tokens, ServiceError> {
    let reply = net.send(Request::post(&cfg.token_url).form(&[
        ("client_id", &cfg.client_id),
        ("client_secret", &cfg.client_secret),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ]))?;
    if !reply.ok() {
        return Err(failure(&reply));
    }
    parse_tokens(&reply)
}

/// The stored tokens with a set from the service folded in: the access token and its end time replace the old
/// ones, a new refresh token replaces the old one (a service that rotates them), a new email replaces the old.
pub fn apply(old: &ServiceTokens, new: Tokens, now: u64) -> ServiceTokens {
    ServiceTokens {
        refresh_token: new.refresh_token.or_else(|| old.refresh_token.clone()),
        access_token: Some(new.access_token),
        access_expires_at: now + new.expires_in,
        email: new.email.or_else(|| old.email.clone()),
    }
}

/// The account email, from Google's userinfo endpoint with the access token. `None` on any failure: the
/// email is only shown, never needed.
pub fn fetch_email(net: &Net, cfg: &OAuthConfig, access_token: &str) -> Option<String> {
    let reply = net.send(Request::get(&cfg.userinfo_url).bearer(access_token)).ok()?;
    if !reply.ok() {
        return None;
    }
    reply.json()?.get("email").and_then(Value::as_str).map(str::to_string).filter(|e| plausible_email(e))
}

fn plausible_email(e: &str) -> bool {
    e.contains('@') && e.len() <= 254 && !e.chars().any(|c| c.is_control() || c.is_whitespace())
}

fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.trim_end_matches('=').bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// The `email` claim of an id token. The token came straight from the service over TLS in the same answer as
/// the access token, so its signature is not checked; it is only used to show who is signed in.
pub fn email_from_id_token(jwt: &str) -> Option<String> {
    let payload = jwt.split('.').nth(1)?;
    let json: Value = serde_json::from_slice(&b64url_decode(payload)?).ok()?;
    json.get("email").and_then(Value::as_str).map(str::to_string).filter(|e| plausible_email(e))
}

/// "t***@gmail.com": enough to recognise the account, not enough to read it out loud to a stranger.
pub fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        Some((name, domain)) if !name.is_empty() => format!("{}***@{domain}", name.chars().next().unwrap_or('*')),
        _ => "your account".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_token_email_is_read_from_the_payload() {
        // {"email":"tim@example.com","sub":"1"}
        let payload = "eyJlbWFpbCI6InRpbUBleGFtcGxlLmNvbSIsInN1YiI6IjEifQ";
        assert_eq!(email_from_id_token(&format!("aaa.{payload}.sig")).as_deref(), Some("tim@example.com"));
        assert_eq!(email_from_id_token("nodots"), None);
        assert_eq!(email_from_id_token("a.!!!.c"), None);
        // {"email":"no at sign"}
        assert_eq!(email_from_id_token("a.eyJlbWFpbCI6Im5vIGF0IHNpZ24ifQ.c"), None);
    }

    #[test]
    fn emails_are_masked_for_the_screen() {
        assert_eq!(mask_email("tim@example.com"), "t***@example.com");
        assert_eq!(mask_email("@example.com"), "your account");
        assert_eq!(mask_email("nothing"), "your account");
    }

    #[test]
    fn slow_down_adds_five_seconds_up_to_a_minute() {
        assert_eq!(next_interval(5, &Poll::SlowDown), 10);
        assert_eq!(next_interval(58, &Poll::SlowDown), 60);
        assert_eq!(next_interval(5, &Poll::Pending), 5);
    }

    #[test]
    fn applying_tokens_keeps_what_the_service_did_not_replace() {
        let old = ServiceTokens { refresh_token: Some("R1".into()), access_token: Some("A1".into()), access_expires_at: 1, email: Some("a@b.c".into()) };
        let kept = apply(&old, Tokens { access_token: "A2".into(), expires_in: 100, refresh_token: None, email: None }, 1_000);
        assert_eq!((kept.refresh_token.as_deref(), kept.access_token.as_deref(), kept.access_expires_at, kept.email.as_deref()), (Some("R1"), Some("A2"), 1_100, Some("a@b.c")));
        let rotated = apply(&old, Tokens { access_token: "A3".into(), expires_in: 50, refresh_token: Some("R2".into()), email: Some("x@y.z".into()) }, 2_000);
        assert_eq!((rotated.refresh_token.as_deref(), rotated.email.as_deref()), (Some("R2"), Some("x@y.z")));
    }
}
