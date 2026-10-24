//! Sign-in tokens for the cloud services, in `cloud-tokens.json` next to the app.
//!
//! Same rules as `secrets.json` (see `crate::fork::secrets`): mode 0600 on Linux, written through a private
//! temp file and renamed, a symbolic link is neither read nor written, an unreadable file is left alone
//! and saving is refused, `Debug` hides every value, and a drive that cannot restrict files (exFAT, FAT on
//! an SD card) is reported. It is a file of its own because a refresh token can change on every use and
//! a damaged token file must never block the artwork keys. It is where the signed-in account's email is
//! kept too; no other file the app writes holds it.

use super::service::ServiceId;
use crate::fork::secrets::{file_is_private, tighten_permissions, write_private};
use serde_json::{Map, Value};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "cloud-tokens.json";

#[derive(Clone, PartialEq, Eq, Default)]
pub struct ServiceTokens {
    /// `None`: signed out (the entry stays only to remember which account it was).
    pub refresh_token: Option<String>,
    pub access_token: Option<String>,
    /// Seconds since 1970 when `access_token` stops working.
    pub access_expires_at: u64,
    pub email: Option<String>,
}

impl fmt::Debug for ServiceTokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceTokens")
            .field("refresh_token", &self.refresh_token.is_some())
            .field("access_token", &self.access_token.is_some())
            .field("access_expires_at", &self.access_expires_at)
            .field("email", &self.email.is_some())
            .finish()
    }
}

impl ServiceTokens {
    pub fn signed_in(&self) -> bool {
        self.refresh_token.is_some()
    }

    /// The access token, when it is still good for at least a minute.
    pub fn fresh_access_token(&self, now: u64) -> Option<&str> {
        self.access_token.as_deref().filter(|_| self.access_expires_at > now + 60)
    }

    fn to_json(&self) -> Value {
        let mut m = Map::new();
        if let Some(v) = &self.refresh_token {
            m.insert("refresh_token".into(), v.clone().into());
        }
        if let Some(v) = &self.access_token {
            m.insert("access_token".into(), v.clone().into());
            m.insert("access_expires_at".into(), self.access_expires_at.into());
        }
        if let Some(v) = &self.email {
            m.insert("email".into(), v.clone().into());
        }
        Value::Object(m)
    }

    fn from_json(v: &Value) -> ServiceTokens {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string).filter(|s| !s.is_empty());
        ServiceTokens { refresh_token: text("refresh_token"), access_token: text("access_token"), access_expires_at: v.get("access_expires_at").and_then(Value::as_u64).unwrap_or(0), email: text("email") }
    }
}

pub struct CloudTokens {
    path: PathBuf,
    services: BTreeMap<String, ServiceTokens>,
    /// False when the file on disk could not be read as an object; saving is then refused.
    writable: bool,
    pub problem: Option<String>,
    restricted: Cell<Option<bool>>,
}

impl fmt::Debug for CloudTokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloudTokens").field("path", &self.path).field("services", &self.services).finish()
    }
}

impl CloudTokens {
    pub fn in_memory() -> CloudTokens {
        CloudTokens { path: PathBuf::new(), services: BTreeMap::new(), writable: false, problem: None, restricted: Cell::new(None) }
    }

    pub fn load(path: &Path) -> CloudTokens {
        let blank = |problem: String| CloudTokens { path: path.to_path_buf(), services: BTreeMap::new(), writable: false, problem: Some(problem), restricted: Cell::new(None) };
        if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            return blank(format!("{FILE_NAME} is a link; it is not used or changed. Replace it with a plain file."));
        }
        let restricted = Cell::new(None);
        match fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Value>(text.strip_prefix('\u{FEFF}').unwrap_or(&text)) {
                Ok(Value::Object(map)) => {
                    let mut problem = None;
                    match tighten_permissions(path) {
                        Ok(private) => restricted.set(private),
                        Err(e) => problem = Some(format!("could not restrict {FILE_NAME} to your account: {e}")),
                    }
                    let services = map.iter().filter(|(k, _)| ServiceId::from_key(k).is_some()).map(|(k, v)| (k.clone(), ServiceTokens::from_json(v))).collect();
                    CloudTokens { path: path.to_path_buf(), services, writable: true, problem, restricted }
                }
                _ => blank(format!("{FILE_NAME} is not a JSON object; fix or remove it. Nothing in it is used.")),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => CloudTokens { path: path.to_path_buf(), services: BTreeMap::new(), writable: true, problem: None, restricted },
            Err(e) => blank(format!("could not read {FILE_NAME}: {}", e.kind())),
        }
    }

    pub fn drive_note(&self) -> Option<&'static str> {
        (self.restricted.get() == Some(false)).then_some("This drive cannot restrict files, so other accounts on this device could read cloud-tokens.json.")
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self, id: ServiceId) -> Option<&ServiceTokens> {
        self.services.get(id.key())
    }

    pub fn signed_in(&self, id: ServiceId) -> bool {
        self.get(id).is_some_and(ServiceTokens::signed_in)
    }

    pub fn signed_in_services(&self) -> Vec<ServiceId> {
        ServiceId::ALL.into_iter().filter(|id| self.signed_in(*id)).collect()
    }

    pub fn set(&mut self, id: ServiceId, tokens: ServiceTokens) -> io::Result<()> {
        let before = self.services.insert(id.key().to_string(), tokens);
        self.save().inspect_err(|_| {
            match before {
                Some(b) => self.services.insert(id.key().to_string(), b),
                None => self.services.remove(id.key()),
            };
        })
    }

    /// Signs out: every token goes and the entry with it (the account is forgotten).
    pub fn remove(&mut self, id: ServiceId) -> io::Result<()> {
        let before = self.services.remove(id.key());
        if before.is_none() {
            return Ok(());
        }
        self.save().inspect_err(|_| {
            if let Some(b) = before {
                self.services.insert(id.key().to_string(), b);
            }
        })
    }

    /// The tokens are no good any more (revoked): they are dropped but the email stays, so a later sign-in
    /// can tell whether it is the same account.
    pub fn mark_signed_out(&mut self, id: ServiceId) -> io::Result<()> {
        let Some(current) = self.get(id).cloned() else { return Ok(()) };
        self.set(id, ServiceTokens { email: current.email, ..ServiceTokens::default() })
    }

    /// Every stored secret, for removing them from text that might otherwise carry one.
    pub fn all_values(&self) -> Vec<String> {
        self.services.values().flat_map(|t| [t.refresh_token.clone(), t.access_token.clone(), t.email.clone()]).flatten().collect()
    }

    fn save(&self) -> io::Result<()> {
        if !self.writable {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{FILE_NAME} cannot be saved here; fix or remove the file first")));
        }
        if self.path.as_os_str().is_empty() {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "no file to save to"));
        }
        if fs::symlink_metadata(&self.path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{FILE_NAME} is a link; it is not changed")));
        }
        let map: Map<String, Value> = self.services.iter().map(|(k, v)| (k.clone(), v.to_json())).collect();
        let text = serde_json::to_string_pretty(&map).map_err(io::Error::other)?;
        write_private(&self.path, text.as_bytes())?;
        self.restricted.set(file_is_private(&self.path));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dpm_cloud_tokens_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample() -> ServiceTokens {
        ServiceTokens { refresh_token: Some("REFRESH-aaaaaaaaaaaa".into()), access_token: Some("ACCESS-bbbbbbbbbbbb".into()), access_expires_at: 5_000, email: Some("someone@example.com".into()) }
    }

    #[test]
    fn a_missing_file_is_an_empty_set_and_loading_creates_nothing() {
        let dir = temp_dir("missing");
        let t = CloudTokens::load(&dir.join(FILE_NAME));
        assert!(t.signed_in_services().is_empty() && t.problem.is_none());
        assert!(!dir.join(FILE_NAME).exists());
    }

    #[test]
    fn tokens_round_trip_and_sign_out_removes_the_entry() {
        let dir = temp_dir("roundtrip");
        let path = dir.join(FILE_NAME);
        let mut t = CloudTokens::load(&path);
        t.set(ServiceId::Google, sample()).unwrap();
        let again = CloudTokens::load(&path);
        assert_eq!(again.get(ServiceId::Google), Some(&sample()));
        assert!(again.signed_in(ServiceId::Google));
        let mut again = again;
        again.remove(ServiceId::Google).unwrap();
        assert!(CloudTokens::load(&path).get(ServiceId::Google).is_none());
    }

    #[test]
    fn a_revoked_sign_in_keeps_only_the_account_email() {
        let dir = temp_dir("revoked");
        let path = dir.join(FILE_NAME);
        let mut t = CloudTokens::load(&path);
        t.set(ServiceId::Google, sample()).unwrap();
        t.mark_signed_out(ServiceId::Google).unwrap();
        let again = CloudTokens::load(&path);
        assert!(!again.signed_in(ServiceId::Google));
        let kept = again.get(ServiceId::Google).unwrap();
        assert_eq!(kept.email.as_deref(), Some("someone@example.com"));
        assert!(kept.access_token.is_none() && kept.refresh_token.is_none());
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("REFRESH") && !text.contains("ACCESS"), "{text}");
    }

    #[test]
    fn the_access_token_counts_as_fresh_only_with_a_minute_to_spare() {
        let t = sample();
        assert_eq!(t.fresh_access_token(4_000), Some("ACCESS-bbbbbbbbbbbb"));
        assert_eq!(t.fresh_access_token(4_950), None);
        assert_eq!(ServiceTokens::default().fresh_access_token(0), None);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_private_from_its_first_byte_and_stays_that_way() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("mode");
        let path = dir.join(FILE_NAME);
        let mut t = CloudTokens::load(&path);
        t.set(ServiceId::Google, sample()).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let mut next = sample();
        next.access_token = Some("ACCESS-cccccccccccc".into());
        t.set(ServiceId::Google, next).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "a rewrite keeps the mode");
        assert!(!dir.join(format!("{FILE_NAME}.tmp")).exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_was_world_readable_is_tightened_on_load() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("tighten");
        let path = dir.join(FILE_NAME);
        fs::write(&path, r#"{"google":{"refresh_token":"REFRESH-aaaaaaaaaaaa"}}"#).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let t = CloudTokens::load(&path);
        assert!(t.signed_in(ServiceId::Google));
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn a_damaged_file_is_never_overwritten() {
        let dir = temp_dir("damaged");
        let path = dir.join(FILE_NAME);
        fs::write(&path, "{ not json").unwrap();
        let mut t = CloudTokens::load(&path);
        assert!(t.problem.is_some());
        assert!(t.set(ServiceId::Google, sample()).is_err());
        assert!(t.get(ServiceId::Google).is_none(), "a failed save is rolled back in memory");
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ not json");
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_is_neither_read_nor_written() {
        let dir = temp_dir("symlink");
        let target = dir.join("elsewhere.json");
        fs::write(&target, r#"{"google":{"refresh_token":"REFRESH-aaaaaaaaaaaa"}}"#).unwrap();
        let link = dir.join(FILE_NAME);
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let mut t = CloudTokens::load(&link);
        assert!(!t.signed_in(ServiceId::Google));
        assert!(t.problem.as_deref().is_some_and(|p| p.contains("link")));
        assert!(t.set(ServiceId::Google, sample()).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), r#"{"google":{"refresh_token":"REFRESH-aaaaaaaaaaaa"}}"#);
    }

    #[test]
    fn debug_output_never_contains_a_value() {
        let dir = temp_dir("debug");
        let mut t = CloudTokens::load(&dir.join(FILE_NAME));
        t.set(ServiceId::Google, sample()).unwrap();
        let text = format!("{t:?} {:?}", sample());
        for secret in ["REFRESH", "ACCESS", "someone@example.com"] {
            assert!(!text.contains(secret), "{text}");
        }
        assert!(t.all_values().iter().any(|v| v.starts_with("REFRESH")));
    }

    #[test]
    fn tokens_never_land_in_a_sibling_file() {
        let dir = temp_dir("siblings");
        fs::write(dir.join("state.json"), r#"{"installed":{}}"#).unwrap();
        let mut t = CloudTokens::load(&dir.join(FILE_NAME));
        t.set(ServiceId::Google, sample()).unwrap();
        assert_eq!(fs::read_to_string(dir.join("state.json")).unwrap(), r#"{"installed":{}}"#);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
    }

    #[test]
    fn unknown_services_in_the_file_are_ignored() {
        let dir = temp_dir("unknown");
        let path = dir.join(FILE_NAME);
        fs::write(&path, r#"{"onedrive":{"refresh_token":"x-xxxxxxxx"},"google":{}}"#).unwrap();
        let t = CloudTokens::load(&path);
        assert!(t.signed_in_services().is_empty());
    }
}
