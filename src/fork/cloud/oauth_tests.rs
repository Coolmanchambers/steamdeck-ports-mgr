//! The device-code sign-in and the refresh, against a mock server on this machine.

use super::mock_http::{Answer, Mock, Seen};
use super::net::Net;
use super::oauth::{self, OAuthConfig, Poll};
use super::service::{ErrorKind, ServiceId};
use super::tokens::{CloudTokens, ServiceTokens};
use std::sync::Mutex;

const DEVICE_CODE: &str = "DEVICE-CODE-SECRET-0123456789";
const ACCESS: &str = "ACCESS-TOKEN-SECRET-abcdef";
const REFRESH: &str = "REFRESH-TOKEN-SECRET-uvwxyz";
const CLIENT_SECRET: &str = "CLIENT-SECRET-VALUE-qrstuv";
const EMAIL: &str = "tim@example.com";
// {"email":"tim@example.com","sub":"1"}
const ID_TOKEN: &str = "h.eyJlbWFpbCI6InRpbUBleGFtcGxlLmNvbSIsInN1YiI6IjEifQ.s";

fn cfg(mock: &Mock) -> OAuthConfig {
    OAuthConfig { device_code_url: mock.url("/device/code"), token_url: mock.url("/token"), client_id: "CLIENT-ID-VALUE".into(), client_secret: CLIENT_SECRET.into(), scope: "openid email drive.file".into() }
}

fn code_answer() -> Answer {
    Answer::json(200, &format!(r#"{{"device_code":"{DEVICE_CODE}","user_code":"ABCD-EFGH","verification_url":"https://www.google.com/device","expires_in":900,"interval":5}}"#))
}

fn tokens_answer(refresh: bool) -> Answer {
    let r = if refresh { format!(r#","refresh_token":"{REFRESH}""#) } else { String::new() };
    Answer::json(200, &format!(r#"{{"access_token":"{ACCESS}","expires_in":3599,"scope":"x","token_type":"Bearer","id_token":"{ID_TOKEN}"{r}}}"#))
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dpm_cloud_oauth_{}_{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn answering(answers: Vec<Answer>) -> Mock {
    let queue = Mutex::new(answers.into_iter().collect::<std::collections::VecDeque<_>>());
    Mock::start(move |_: &Seen| queue.lock().unwrap().pop_front().unwrap_or_else(|| Answer::json(500, "{}")))
}

#[test]
fn start_asks_for_a_code_and_reads_it() {
    let mock = answering(vec![code_answer()]);
    let code = oauth::start(&Net::local_test(), &cfg(&mock), 1_000).unwrap();
    assert_eq!((code.user_code.as_str(), code.verification_url.as_str(), code.expires_at, code.interval), ("ABCD-EFGH", "https://www.google.com/device", 1_900, 5));
    let seen = mock.seen();
    assert_eq!((seen[0].method.as_str(), seen[0].path()), ("POST", "/device/code"));
    assert_eq!(seen[0].form("client_id").as_deref(), Some("CLIENT-ID-VALUE"));
    assert_eq!(seen[0].form("scope").as_deref(), Some("openid email drive.file"));
    assert_eq!(seen[0].form("client_secret"), None, "the code request needs no secret");
}

#[test]
fn start_with_a_refusal_or_a_broken_answer_is_an_error() {
    let mock = answering(vec![Answer::json(400, r#"{"error":"invalid_client"}"#), Answer::json(200, "not json"), Answer::json(200, r#"{"user_code":"X"}"#)]);
    let net = Net::local_test();
    let c = cfg(&mock);
    assert_eq!(oauth::start(&net, &c, 0).unwrap_err().kind, ErrorKind::Other);
    assert!(oauth::start(&net, &c, 0).is_err());
    assert!(oauth::start(&net, &c, 0).unwrap_err().detail.contains("missing"));
}

#[test]
fn polling_walks_through_pending_slow_down_and_approval() {
    let mock = answering(vec![
        code_answer(),
        Answer::json(428, r#"{"error":"authorization_pending"}"#),
        Answer::json(403, r#"{"error":"slow_down"}"#),
        tokens_answer(true),
    ]);
    let (net, c) = (Net::local_test(), cfg(&mock));
    let code = oauth::start(&net, &c, 0).unwrap();
    assert_eq!(oauth::poll(&net, &c, &code, 5), Poll::Pending);
    let slow = oauth::poll(&net, &c, &code, 10);
    assert_eq!(slow, Poll::SlowDown);
    assert_eq!(oauth::next_interval(code.interval, &slow), 10);
    match oauth::poll(&net, &c, &code, 20) {
        Poll::Approved(t) => {
            assert_eq!((t.access_token.as_str(), t.refresh_token.as_deref(), t.expires_in, t.email.as_deref()), (ACCESS, Some(REFRESH), 3599, Some(EMAIL)));
        }
        other => panic!("{other:?}"),
    }
    let polls: Vec<Seen> = mock.seen().into_iter().filter(|s| s.path() == "/token").collect();
    assert_eq!(polls.len(), 3);
    assert_eq!(polls[0].form("device_code").as_deref(), Some(DEVICE_CODE));
    assert_eq!(polls[0].form("grant_type").as_deref(), Some("urn:ietf:params:oauth:grant-type:device_code"));
    assert_eq!(polls[0].form("client_secret").as_deref(), Some(CLIENT_SECRET));
}

#[test]
fn denied_expired_and_a_code_that_ran_out_by_the_clock() {
    let mock = answering(vec![code_answer(), Answer::json(403, r#"{"error":"access_denied"}"#), Answer::json(400, r#"{"error":"expired_token"}"#), Answer::json(400, r#"{"error":"invalid_grant"}"#)]);
    let (net, c) = (Net::local_test(), cfg(&mock));
    let code = oauth::start(&net, &c, 0).unwrap();
    assert_eq!(oauth::poll(&net, &c, &code, 5), Poll::Denied);
    assert_eq!(oauth::poll(&net, &c, &code, 5), Poll::Expired);
    assert_eq!(oauth::poll(&net, &c, &code, 5), Poll::Expired);
    let before = mock.seen().len();
    assert_eq!(oauth::poll(&net, &c, &code, 900), Poll::Expired, "after expires_at the service is not even asked");
    assert_eq!(mock.seen().len(), before);
}

#[test]
fn an_approval_without_a_refresh_token_is_a_failure_not_a_half_sign_in() {
    let mock = answering(vec![code_answer(), tokens_answer(false)]);
    let (net, c) = (Net::local_test(), cfg(&mock));
    let code = oauth::start(&net, &c, 0).unwrap();
    assert!(matches!(oauth::poll(&net, &c, &code, 5), Poll::Failed(e) if e.kind == ErrorKind::Other));
}

#[test]
fn a_busy_or_broken_service_while_polling_is_reported_by_kind() {
    let mock = answering(vec![code_answer(), Answer::json(503, "{}"), Answer::json(429, r#"{"error":"rate"}"#).with_header("Retry-After", "30")]);
    let (net, c) = (Net::local_test(), cfg(&mock));
    let code = oauth::start(&net, &c, 0).unwrap();
    assert!(matches!(oauth::poll(&net, &c, &code, 5), Poll::Failed(e) if e.kind == ErrorKind::ServiceDown));
    assert!(matches!(oauth::poll(&net, &c, &code, 5), Poll::Failed(e) if e.kind == ErrorKind::RateLimited && e.retry_after == Some(30)));
}

#[test]
fn no_connection_is_offline() {
    let dead = {
        let mock = answering(vec![]);
        cfg(&mock)
    };
    let err = oauth::start(&Net::local_test(), &dead, 0).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Offline, "{err}");
}

fn stored(access_expires_at: u64) -> ServiceTokens {
    ServiceTokens { refresh_token: Some(REFRESH.into()), access_token: Some("OLD-ACCESS-TOKEN-xxxx".into()), access_expires_at, email: Some(EMAIL.into()) }
}

#[test]
fn a_fresh_access_token_is_used_without_asking_anyone() {
    let mock = answering(vec![]);
    let dir = temp_dir("fresh");
    let mut store = CloudTokens::load(&dir.join("t.json"));
    store.set(ServiceId::Google, stored(10_000)).unwrap();
    let token = oauth::access_token(&Net::local_test(), &cfg(&mock), &mut store, ServiceId::Google, 1_000).unwrap();
    assert_eq!(token, "OLD-ACCESS-TOKEN-xxxx");
    assert!(mock.seen().is_empty());
}

#[test]
fn an_old_access_token_is_refreshed_and_the_file_is_updated() {
    let mock = answering(vec![Answer::json(200, &format!(r#"{{"access_token":"{ACCESS}","expires_in":3599,"token_type":"Bearer"}}"#))]);
    let dir = temp_dir("refresh");
    let path = dir.join("t.json");
    let mut store = CloudTokens::load(&path);
    store.set(ServiceId::Google, stored(1_020)).unwrap();
    let token = oauth::access_token(&Net::local_test(), &cfg(&mock), &mut store, ServiceId::Google, 1_000).unwrap();
    assert_eq!(token, ACCESS);
    let seen = mock.seen();
    assert_eq!(seen[0].form("refresh_token").as_deref(), Some(REFRESH));
    assert_eq!(seen[0].form("grant_type").as_deref(), Some("refresh_token"));
    let on_disk = CloudTokens::load(&path);
    let t = on_disk.get(ServiceId::Google).unwrap();
    assert_eq!((t.access_token.as_deref(), t.access_expires_at, t.refresh_token.as_deref(), t.email.as_deref()), (Some(ACCESS), 1_000 + 3599, Some(REFRESH), Some(EMAIL)));
}

#[test]
fn a_rotated_refresh_token_replaces_the_old_one() {
    let mock = answering(vec![Answer::json(200, r#"{"access_token":"NEW-ACCESS-TOKEN-1234","expires_in":3600,"refresh_token":"ROTATED-REFRESH-5678"}"#)]);
    let dir = temp_dir("rotate");
    let path = dir.join("t.json");
    let mut store = CloudTokens::load(&path);
    store.set(ServiceId::Google, stored(0)).unwrap();
    oauth::access_token(&Net::local_test(), &cfg(&mock), &mut store, ServiceId::Google, 100).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("ROTATED-REFRESH-5678") && !text.contains(REFRESH), "{text}");
}

#[test]
fn a_revoked_refresh_token_signs_out_and_keeps_only_the_email() {
    let mock = answering(vec![Answer::json(400, r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#)]);
    let dir = temp_dir("revoked");
    let path = dir.join("t.json");
    let mut store = CloudTokens::load(&path);
    store.set(ServiceId::Google, stored(0)).unwrap();
    let err = oauth::access_token(&Net::local_test(), &cfg(&mock), &mut store, ServiceId::Google, 100).unwrap_err();
    assert_eq!(err.kind, ErrorKind::SignedOut);
    let again = CloudTokens::load(&path);
    assert!(!again.signed_in(ServiceId::Google));
    assert_eq!(again.get(ServiceId::Google).unwrap().email.as_deref(), Some(EMAIL));
    assert!(!std::fs::read_to_string(&path).unwrap().contains(REFRESH));
}

#[test]
fn a_service_outage_during_refresh_keeps_the_sign_in() {
    let mock = answering(vec![Answer::json(503, "{}")]);
    let dir = temp_dir("outage");
    let mut store = CloudTokens::load(&dir.join("t.json"));
    store.set(ServiceId::Google, stored(0)).unwrap();
    let err = oauth::access_token(&Net::local_test(), &cfg(&mock), &mut store, ServiceId::Google, 100).unwrap_err();
    assert_eq!(err.kind, ErrorKind::ServiceDown);
    assert!(store.signed_in(ServiceId::Google));
}

#[test]
fn not_signed_in_is_signed_out() {
    let mock = answering(vec![]);
    let mut store = CloudTokens::in_memory();
    let err = oauth::access_token(&Net::local_test(), &cfg(&mock), &mut store, ServiceId::Google, 0).unwrap_err();
    assert_eq!(err.kind, ErrorKind::SignedOut);
}

#[test]
fn no_token_code_secret_or_email_reaches_the_log_or_a_debug_print() {
    super::log::take_captured();
    let mock = answering(vec![
        code_answer(),
        Answer::json(428, r#"{"error":"authorization_pending"}"#),
        tokens_answer(true),
        Answer::json(400, r#"{"error":"invalid_grant"}"#),
        Answer::json(503, "{}"),
    ]);
    let (net, c) = (Net::local_test(), cfg(&mock));
    let mut printed = String::new();
    let code = oauth::start(&net, &c, 0).unwrap();
    printed += &format!("{code:?} {c:?}");
    let _ = oauth::poll(&net, &c, &code, 5);
    let approved = oauth::poll(&net, &c, &code, 10);
    printed += &format!("{approved:?}");
    let dir = temp_dir("logs");
    let mut store = CloudTokens::load(&dir.join("t.json"));
    store.set(ServiceId::Google, stored(0)).unwrap();
    printed += &format!("{store:?}");
    let e1 = oauth::access_token(&net, &c, &mut store, ServiceId::Google, 100).unwrap_err();
    printed += &format!("{e1} {e1:?}");
    let mut store2 = CloudTokens::load(&dir.join("t2.json"));
    store2.set(ServiceId::Google, stored(0)).unwrap();
    let e2 = oauth::access_token(&net, &c, &mut store2, ServiceId::Google, 100).unwrap_err();
    printed += &format!("{e2} {e2:?}");
    let logged = super::log::take_captured().join("\n");
    for secret in [DEVICE_CODE, ACCESS, REFRESH, CLIENT_SECRET, EMAIL, "OLD-ACCESS-TOKEN", ID_TOKEN] {
        assert!(!logged.contains(secret), "{secret} in the log: {logged}");
        assert!(!printed.contains(secret), "{secret} in a debug print: {printed}");
    }
}

/// The source holds no token-shaped text: a Google access token, refresh token, client secret or client id
/// (the needles are built from pieces below so this file does not match itself).
#[test]
fn no_token_shaped_strings_in_the_source() {
    fn files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs" || e == "yml" || e == "md" || e == "json" || e == "slint") {
                out.push(path);
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut all = Vec::new();
    for sub in ["src", "ui", ".github", "tools"] {
        if root.join(sub).is_dir() {
            files(&root.join(sub), &mut all);
        }
    }
    all.push(root.join("PRIVACY.md"));
    all.push(root.join("README.md"));
    // Built from pieces so this file does not match itself.
    let needles: Vec<String> = vec![format!("{}{}", "ya29", "."), format!("{}{}", "GOCSPX", "-"), format!("{}{}", ".apps.google", "usercontent.com"), format!("{}{}", "\"1", "//0")];
    let long_run_after = |text: &str, needle: &str, min: usize| text.match_indices(needle).any(|(i, _)| text[i + needle.len()..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').count() >= min);
    for path in all {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        for needle in &needles {
            let hit = if needle.starts_with(".apps") { text.contains(needle.as_str()) } else { long_run_after(&text, needle, 20) };
            assert!(!hit, "token-shaped text ({needle}) in {}", path.display());
        }
    }
}
