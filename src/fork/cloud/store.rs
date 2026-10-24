//! The two cloud files, shared between the screens (on the window thread) and the upload pass (on a worker):
//! every change is read, changed and written back under one lock, so a pass finishing at the moment the
//! person signs out cannot bring the sign-in back, and a token refresh cannot undo a sign-out.
//!
//! Network calls never happen under the lock.

use super::oauth::{self, OAuthConfig, Tokens};
use super::record::{Record, ServiceRecord, Uploaded};
use super::service::{ErrorKind, ServiceError, ServiceId};
use super::tokens::{self, CloudTokens, ServiceTokens};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

static LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// A name for a new sign-in: unique for this machine and moment, nothing to do with the account.
fn new_signin_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let seed = format!("{nanos}-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
    super::sha256::sha256_hex(seed.as_bytes())[..16].to_string()
}

/// Who is signed in, for the screens: the services with a refresh token, and each one's email. Read without
/// touching the file (the screens look every second).
pub fn signed_in(tokens_path: &Path) -> Vec<(ServiceId, Option<String>)> {
    let _g = lock();
    let tokens = CloudTokens::peek(tokens_path);
    tokens.signed_in_services().into_iter().map(|id| (id, tokens.get(id).and_then(|t| t.email.clone()))).collect()
}

/// The stored tokens of one service, or `None` (also for a damaged file).
pub fn tokens_of(tokens_path: &Path, id: ServiceId) -> Option<ServiceTokens> {
    let _g = lock();
    CloudTokens::peek(tokens_path).get(id).cloned()
}

/// The name of the current sign-in of `id` (see `ServiceTokens::signin`); a pass takes it when it starts.
///
/// A signed-in entry written by an earlier build has no name: it is given one here (and saved), so such a sign-in
/// is not left unable to upload.
pub fn signin_name(tokens_path: &Path, id: ServiceId) -> Option<String> {
    let _g = lock();
    let mut store = CloudTokens::load(tokens_path);
    let current = store.get(id).filter(|t| t.signed_in())?.clone();
    if let Some(name) = current.signin {
        return Some(name);
    }
    let name = new_signin_name();
    store.set(id, ServiceTokens { signin: Some(name.clone()), ..current }).ok()?;
    Some(name)
}

/// The service said the access is gone for good (a 401 after a forced refresh): the sign-in is marked signed out,
/// but only if it is still the sign-in the caller began with, so the status shows what the pass saw.
pub fn mark_signed_out(tokens_path: &Path, id: ServiceId, signin: &str) -> io::Result<()> {
    let _g = lock();
    if !still_that_sign_in(tokens_path, id, signin) {
        return Ok(());
    }
    CloudTokens::load(tokens_path).mark_signed_out(id)
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

/// True while `id` is signed in under the sign-in named `signin` (a pass or a copy that began under that sign-in
/// may write into its record; one that outlived a sign-out, or a sign-in as someone else, may not).
fn still_that_sign_in(tokens_path: &Path, id: ServiceId, signin: &str) -> bool {
    let tokens = CloudTokens::peek(tokens_path);
    tokens.signed_in(id) && tokens.get(id).and_then(|t| t.signin.as_deref()) == Some(signin)
}

/// Writes what a pass learned into one service's part of the record, merging with what is there (a copy back
/// from the cloud may have added entries meanwhile; they are kept), and only while the pass's own sign-in is
/// still the current one.
pub fn persist_service(record_path: &Path, tokens_path: &Path, id: ServiceId, signin: &str, part: &ServiceRecord) -> io::Result<()> {
    let _g = lock();
    if !still_that_sign_in(tokens_path, id, signin) {
        return Ok(());
    }
    let mut record = Record::load(record_path);
    let mut merged = record.service(id);
    merged.uploaded.extend(part.uploaded.iter().map(|(k, v)| (k.clone(), v.clone())));
    merged.last_upload = part.last_upload.max(merged.last_upload);
    merged.last_ok = part.last_ok;
    merged.last_error = part.last_error;
    record.set_service(id, merged);
    record.save()
}

/// Adds entries (backups just copied back from the cloud, so they are not copied up again) to the record, under
/// the same rules as [`persist_service`] and without touching its times or its other entries.
pub fn add_uploaded(record_path: &Path, tokens_path: &Path, id: ServiceId, signin: &str, entries: BTreeMap<String, Uploaded>) -> io::Result<()> {
    let _g = lock();
    if !still_that_sign_in(tokens_path, id, signin) || entries.is_empty() {
        return Ok(());
    }
    let mut record = Record::load(record_path);
    let mut part = record.service(id);
    part.uploaded.extend(entries);
    record.set_service(id, part);
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
    let mut fresh = oauth::apply(&ServiceTokens::default(), tokens, now);
    fresh.signin = Some(new_signin_name());
    store.set(id, fresh)?;
    super::log::register_secrets(&store.all_values());
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
    access_token_with(net, cfg, tokens_path, id, now, false)
}

/// [`access_token`], and with `force` the stored token is not trusted even if it looks fresh (Google answered 401
/// to it): a new one is asked for.
pub fn access_token_with(net: &super::net::Net, cfg: &OAuthConfig, tokens_path: &Path, id: ServiceId, now: u64, force: bool) -> Result<String, ServiceError> {
    let current = {
        let _g = lock();
        CloudTokens::load(tokens_path).get(id).filter(|t| t.signed_in()).cloned()
    };
    let Some(current) = current else { return Err(ServiceError::new(ErrorKind::SignedOut, "not signed in")) };
    super::log::register_secrets(&[current.refresh_token.clone(), current.access_token.clone(), current.email.clone()].into_iter().flatten().collect::<Vec<_>>());
    if let Some(token) = current.fresh_access_token(now).filter(|_| !force) {
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
        let first = signin_name(&t, ServiceId::Google).expect("a signed-in sign-in has a name");
        sign_out(&t, &r, ServiceId::Google).unwrap();
        persist_service(&r, &t, ServiceId::Google, &first, &uploaded()).unwrap();
        assert!(record_of(&r, ServiceId::Google).uploaded.is_empty());
        // Signed in again, as someone else: the pass that began under the first sign-in still may not write.
        sign_in(&t, &r, ServiceId::Google, tokens("REFRESH-2", 3600), 2_000).unwrap();
        let second = signin_name(&t, ServiceId::Google).unwrap();
        assert_ne!(first, second, "every sign-in has its own name");
        persist_service(&r, &t, ServiceId::Google, &first, &uploaded()).unwrap();
        assert!(record_of(&r, ServiceId::Google).uploaded.is_empty(), "the new account's record is untouched by the old pass");
        add_uploaded(&r, &t, ServiceId::Google, &first, uploaded().uploaded).unwrap();
        assert!(record_of(&r, ServiceId::Google).uploaded.is_empty(), "and so is a copy back that began under it");
        persist_service(&r, &t, ServiceId::Google, &second, &uploaded()).unwrap();
        assert_eq!(record_of(&r, ServiceId::Google).uploaded.len(), 1);
    }

    #[test]
    fn a_sign_in_from_an_earlier_build_without_a_name_is_given_one_and_keeps_it() {
        let dir = temp_dir("noname");
        let (t, r) = (dir.join("t.json"), dir.join("r.json"));
        fs::write(&t, r#"{"google":{"refresh_token":"REFRESH-OLD-BUILD","email":"tim@example.com"}}"#).unwrap();
        let name = signin_name(&t, ServiceId::Google).expect("the entry is signed in, so it gets a name");
        assert_eq!(signin_name(&t, ServiceId::Google), Some(name.clone()), "the same name the next time");
        assert!(fs::read_to_string(&t).unwrap().contains(&name), "and it is saved");
        persist_service(&r, &t, ServiceId::Google, &name, &uploaded()).unwrap();
        assert_eq!(record_of(&r, ServiceId::Google).uploaded.len(), 1, "such a sign-in can upload");
        let (t2, _) = (dir.join("t2.json"), 0);
        fs::write(&t2, r#"{"google":{"email":"x@y.z"}}"#).unwrap();
        assert_eq!(signin_name(&t2, ServiceId::Google), None, "a signed-out entry gets none");
    }

    #[test]
    fn marking_signed_out_after_a_401_only_applies_to_the_sign_in_the_pass_began_with() {
        let dir = temp_dir("mark");
        let (t, r) = (dir.join("t.json"), dir.join("r.json"));
        sign_in(&t, &r, ServiceId::Google, tokens("REFRESH-1", 3600), 1_000).unwrap();
        let first = signin_name(&t, ServiceId::Google).unwrap();
        mark_signed_out(&t, ServiceId::Google, "some-other-name").unwrap();
        assert!(signed_in(&t).iter().any(|(id, _)| *id == ServiceId::Google), "a stale name changes nothing");
        mark_signed_out(&t, ServiceId::Google, &first).unwrap();
        assert!(signed_in(&t).is_empty());
        assert_eq!(tokens_of(&t, ServiceId::Google).unwrap().email.as_deref(), Some("tim@example.com"), "the account is remembered for the status page");
    }

    #[test]
    fn a_refresh_keeps_the_name_of_the_sign_in() {
        let mock = answering(vec![Answer::json(200, r#"{"access_token":"NEW-ACCESS-yyyy","expires_in":3600,"refresh_token":"ROTATED-REFRESH"}"#)]);
        let dir = temp_dir("name");
        let (t, r) = (dir.join("t.json"), dir.join("r.json"));
        sign_in(&t, &r, ServiceId::Google, tokens("REFRESH-1", 100), 1_000).unwrap();
        let before = signin_name(&t, ServiceId::Google).unwrap();
        access_token(&Net::local_test(), &cfg(&mock), &t, ServiceId::Google, 9_000).unwrap();
        assert_eq!(signin_name(&t, ServiceId::Google), Some(before), "a rotated refresh token is still the same sign-in");
    }

    #[test]
    fn a_pass_and_a_copy_back_never_undo_each_other() {
        let dir = temp_dir("merge");
        let (t, r) = (dir.join("t.json"), dir.join("r.json"));
        sign_in(&t, &r, ServiceId::Google, tokens("REFRESH-1", 3600), 1_000).unwrap();
        let name = signin_name(&t, ServiceId::Google).unwrap();
        // The pass loaded the record (empty) at its start; a copy back adds an entry meanwhile; the pass then saves.
        let mut pass_part = record_of(&r, ServiceId::Google);
        let copied: BTreeMap<String, Uploaded> = [("Global Backups/copied.zip".to_string(), Uploaded { size: 5, mtime: 5, sha256: "c".repeat(64), cloud_path: "Global Backups/copied.zip".into(), at: 5 })].into();
        add_uploaded(&r, &t, ServiceId::Google, &name, copied).unwrap();
        pass_part.uploaded.insert("Global Backups/new.zip".into(), Uploaded { size: 1, mtime: 1, sha256: "d".repeat(64), cloud_path: "Global Backups/new.zip".into(), at: 9 });
        pass_part.last_upload = Some(9);
        pass_part.last_ok = Some(9);
        persist_service(&r, &t, ServiceId::Google, &name, &pass_part).unwrap();
        let rec = record_of(&r, ServiceId::Google);
        assert_eq!(rec.uploaded.keys().map(String::as_str).collect::<Vec<_>>(), ["Global Backups/copied.zip", "Global Backups/new.zip"], "both survive");
        assert_eq!((rec.last_upload, rec.last_ok), (Some(9), Some(9)));
        // A copy back never changes the times the pass keeps.
        add_uploaded(&r, &t, ServiceId::Google, &name, BTreeMap::new()).unwrap();
        assert_eq!(record_of(&r, ServiceId::Google).last_ok, Some(9));
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
