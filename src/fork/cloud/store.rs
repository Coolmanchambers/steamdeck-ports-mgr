//! The two cloud files, shared between the screens (on the window thread) and the upload pass (on a worker):
//! every change is read, changed and written back under one lock, so a pass finishing at the moment the
//! person signs out cannot bring the sign-in back, and a token refresh cannot undo a sign-out.
//!
//! Network calls never happen under the lock.

use super::oauth::{self, OAuthConfig, Tokens};
use super::record::{Record, ServiceRecord};
use super::service::{ErrorKind, ServiceError, ServiceId};
use super::tokens::{self, CloudTokens, ServiceTokens};
use std::io;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

static LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Who is signed in, for the screens: the services with a refresh token, and each one's email.
pub fn signed_in(tokens_path: &Path) -> Vec<(ServiceId, Option<String>)> {
    let _g = lock();
    let tokens = CloudTokens::load(tokens_path);
    tokens.signed_in_services().into_iter().map(|id| (id, tokens.get(id).and_then(|t| t.email.clone()))).collect()
}

/// The stored tokens of one service, or `None` (also for a damaged file).
pub fn tokens_of(tokens_path: &Path, id: ServiceId) -> Option<ServiceTokens> {
    let _g = lock();
    CloudTokens::load(tokens_path).get(id).cloned()
}

pub fn record_of(record_path: &Path, id: ServiceId) -> ServiceRecord {
    let _g = lock();
    Record::load(record_path).service(id)
}

/// Reads the record, lets `change` edit it, writes it back.
pub fn with_record(record_path: &Path, change: impl FnOnce(&mut Record)) -> io::Result<()> {
    let _g = lock();
    let mut record = Record::load(record_path);
    change(&mut record);
    record.save()
}

/// Writes one service's part of the record, only while that service is still signed in.
pub fn persist_service(record_path: &Path, tokens_path: &Path, id: ServiceId, part: &ServiceRecord) -> io::Result<()> {
    let _g = lock();
    if !CloudTokens::load(tokens_path).signed_in(id) {
        return Ok(());
    }
    let mut record = Record::load(record_path);
    record.set_service(id, part.clone());
    record.save()
}

/// A finished sign-in: the tokens are stored (replacing a revoked or older sign-in) and what was recorded
/// as uploaded under any earlier sign-in is forgotten, because it may have been another account (the next
/// pass recognises what the cloud already holds by name and size).
pub fn sign_in(tokens_path: &Path, record_path: &Path, id: ServiceId, tokens: Tokens, now: u64) -> io::Result<()> {
    let _g = lock();
    if let Some(dir) = tokens_path.parent().filter(|d| !d.as_os_str().is_empty()) {
        tokens::private_check(dir).map_err(|why| io::Error::new(io::ErrorKind::PermissionDenied, why))?;
    }
    let mut store = CloudTokens::load(tokens_path);
    let fresh = oauth::apply(&ServiceTokens::default(), tokens, now);
    store.set(id, fresh)?;
    let mut record = Record::load(record_path);
    record.forget(id);
    record.save()
}

/// Signing out: the tokens go (and the account with them) and the record of what was uploaded. Nothing in
/// the cloud is touched.
pub fn sign_out(tokens_path: &Path, record_path: &Path, id: ServiceId) -> io::Result<()> {
    let _g = lock();
    let mut store = CloudTokens::load(tokens_path);
    store.remove(id)?;
    let mut record = Record::load(record_path);
    record.forget(id);
    record.save()
}

/// A good access token for `id`. A refresh that is already in the file's hands is used as is; otherwise the
/// service is asked (without the lock held) and the answer is stored only if the person is still signed in
/// with the same refresh token. A refresh token the service has revoked marks the sign-in as signed out.
pub fn access_token(net: &super::net::Net, cfg: &OAuthConfig, tokens_path: &Path, id: ServiceId, now: u64) -> Result<String, ServiceError> {
    let current = {
        let _g = lock();
        CloudTokens::load(tokens_path).get(id).filter(|t| t.signed_in()).cloned()
    };
    let Some(current) = current else { return Err(ServiceError::new(ErrorKind::SignedOut, "not signed in")) };
    if let Some(token) = current.fresh_access_token(now) {
        return Ok(token.to_string());
    }
    let refresh_token = current.refresh_token.clone().unwrap_or_default();
    let answer = oauth::refresh(net, cfg, &refresh_token);
    let _g = lock();
    let mut store = CloudTokens::load(tokens_path);
    let same_sign_in = store.get(id).is_some_and(|t| t.refresh_token.as_deref() == Some(refresh_token.as_str()));
    match answer {
        Ok(new) => {
            if !same_sign_in {
                return Err(ServiceError::new(ErrorKind::SignedOut, "signed out while the pass was running"));
            }
            let updated = oauth::apply(&current, new, now);
            let token = updated.access_token.clone().unwrap_or_default();
            if let Err(e) = store.set(id, updated) {
                super::log::note(format!("{}: could not save the new token ({})", id.key(), e.kind()));
            }
            Ok(token)
        }
        Err(e) => {
            if e.kind == ErrorKind::SignedOut && same_sign_in {
                if let Err(io) = store.mark_signed_out(id) {
                    super::log::note(format!("{}: could not record the sign-out ({})", id.key(), io.kind()));
                }
            }
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::cloud::mock_http::{Answer, Mock, Seen};
    use crate::fork::cloud::net::Net;
    use crate::fork::cloud::record::Uploaded;
    use std::collections::VecDeque;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dpm_cloud_store_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cfg(mock: &Mock) -> OAuthConfig {
        OAuthConfig { device_code_url: mock.url("/device/code"), token_url: mock.url("/token"), client_id: "id".into(), client_secret: "secret".into(), scope: super::super::endpoints::GOOGLE_SCOPE.into(), userinfo_url: mock.url("/userinfo") }
    }

    fn answering(answers: Vec<Answer>) -> Mock {
        let queue = Mutex::new(answers.into_iter().collect::<VecDeque<_>>());
        Mock::start(move |_: &Seen| queue.lock().unwrap().pop_front().unwrap_or_else(|| Answer::json(500, "{}")))
    }

    fn tokens(refresh: &str, access_expires_at: u64) -> Tokens {
        Tokens { access_token: "ACCESS-xxxx".into(), expires_in: access_expires_at, refresh_token: Some(refresh.into()), email: Some("tim@example.com".into()) }
    }

    fn uploaded() -> ServiceRecord {
        let mut r = ServiceRecord::default();
        r.uploaded.insert("a.zip".into(), Uploaded { size: 1, mtime: 1, sha256: "x".into(), cloud_path: "a.zip".into(), at: 1 });
        r
    }

    #[test]
    fn signing_in_stores_tokens_and_forgets_an_earlier_record() {
        let dir = temp_dir("signin");
        let (t, r) = (dir.join("t.json"), dir.join("r.json"));
        with_record(&r, |rec| rec.set_service(ServiceId::Google, uploaded())).unwrap();
        sign_in(&t, &r, ServiceId::Google, tokens("REFRESH-1", 3600), 1_000).unwrap();
        assert_eq!(signed_in(&t), vec![(ServiceId::Google, Some("tim@example.com".to_string()))]);
        assert!(record_of(&r, ServiceId::Google).uploaded.is_empty(), "it may have been another account");
    }

    #[test]
    fn signing_out_removes_the_tokens_the_account_and_the_record() {
        let dir = temp_dir("signout");
        let (t, r) = (dir.join("t.json"), dir.join("r.json"));
        sign_in(&t, &r, ServiceId::Google, tokens("REFRESH-1", 3600), 1_000).unwrap();
        with_record(&r, |rec| rec.set_service(ServiceId::Google, uploaded())).unwrap();
        sign_out(&t, &r, ServiceId::Google).unwrap();
        assert!(signed_in(&t).is_empty());
        assert!(tokens_of(&t, ServiceId::Google).is_none(), "the email goes too");
        assert!(record_of(&r, ServiceId::Google).uploaded.is_empty());
    }

    #[test]
    fn a_pass_cannot_write_its_record_back_after_sign_out() {
        let dir = temp_dir("late");
        let (t, r) = (dir.join("t.json"), dir.join("r.json"));
        sign_in(&t, &r, ServiceId::Google, tokens("REFRESH-1", 3600), 1_000).unwrap();
        sign_out(&t, &r, ServiceId::Google).unwrap();
        persist_service(&r, &t, ServiceId::Google, &uploaded()).unwrap();
        assert!(record_of(&r, ServiceId::Google).uploaded.is_empty());
        sign_in(&t, &r, ServiceId::Google, tokens("REFRESH-2", 3600), 2_000).unwrap();
        persist_service(&r, &t, ServiceId::Google, &uploaded()).unwrap();
        assert_eq!(record_of(&r, ServiceId::Google).uploaded.len(), 1);
    }

    #[test]
    fn a_fresh_token_is_returned_and_an_old_one_is_refreshed_and_stored() {
        let mock = answering(vec![Answer::json(200, r#"{"access_token":"NEW-ACCESS-yyyy","expires_in":3600}"#)]);
        let dir = temp_dir("access");
        let (t, r) = (dir.join("t.json"), dir.join("r.json"));
        sign_in(&t, &r, ServiceId::Google, tokens("REFRESH-1", 3600), 1_000).unwrap();
        let net = Net::local_test();
        assert_eq!(access_token(&net, &cfg(&mock), &t, ServiceId::Google, 1_500).unwrap(), "ACCESS-xxxx");
        assert!(mock.seen().is_empty());
        assert_eq!(access_token(&net, &cfg(&mock), &t, ServiceId::Google, 9_000).unwrap(), "NEW-ACCESS-yyyy");
        assert_eq!(tokens_of(&t, ServiceId::Google).unwrap().access_token.as_deref(), Some("NEW-ACCESS-yyyy"));
        assert_eq!(tokens_of(&t, ServiceId::Google).unwrap().refresh_token.as_deref(), Some("REFRESH-1"));
    }

    #[test]
    fn a_refresh_that_finishes_after_sign_out_does_not_bring_the_sign_in_back() {
        // The mock answers only after the person has signed out (done from inside the handler).
        let dir = temp_dir("race");
        let (t, r) = (dir.join("t.json"), dir.join("r.json"));
        sign_in(&t, &r, ServiceId::Google, tokens("REFRESH-1", 100), 1_000).unwrap();
        let (t2, r2) = (t.clone(), r.clone());
        let mock = Mock::start(move |_: &Seen| {
            sign_out(&t2, &r2, ServiceId::Google).unwrap();
            Answer::json(200, r#"{"access_token":"NEW-ACCESS-yyyy","expires_in":3600}"#)
        });
        let err = access_token(&Net::local_test(), &cfg(&mock), &t, ServiceId::Google, 9_000).unwrap_err();
        assert_eq!(err.kind, ErrorKind::SignedOut);
        assert!(signed_in(&t).is_empty() && tokens_of(&t, ServiceId::Google).is_none());
    }

    #[test]
    fn a_revoked_refresh_token_marks_signed_out_but_only_for_the_same_sign_in() {
        let mock = answering(vec![Answer::json(400, r#"{"error":"invalid_grant"}"#)]);
        let dir = temp_dir("revoked");
        let (t, r) = (dir.join("t.json"), dir.join("r.json"));
        sign_in(&t, &r, ServiceId::Google, tokens("REFRESH-1", 100), 1_000).unwrap();
        let err = access_token(&Net::local_test(), &cfg(&mock), &t, ServiceId::Google, 9_000).unwrap_err();
        assert_eq!(err.kind, ErrorKind::SignedOut);
        assert!(signed_in(&t).is_empty());
        assert_eq!(tokens_of(&t, ServiceId::Google).unwrap().email.as_deref(), Some("tim@example.com"));
        // A newer sign-in is not undone by a stale failure.
        let (t2, r2) = (t.clone(), r.clone());
        let mock = Mock::start(move |_: &Seen| {
            sign_in(&t2, &r2, ServiceId::Google, Tokens { access_token: "A".into(), expires_in: 3600, refresh_token: Some("REFRESH-NEW".into()), email: None }, 9_000).unwrap();
            Answer::json(400, r#"{"error":"invalid_grant"}"#)
        });
        sign_in(&t, &r, ServiceId::Google, tokens("REFRESH-OLD", 100), 1_000).unwrap();
        let _ = access_token(&Net::local_test(), &cfg(&mock), &t, ServiceId::Google, 9_000);
        assert_eq!(tokens_of(&t, ServiceId::Google).unwrap().refresh_token.as_deref(), Some("REFRESH-NEW"));
    }
}
