//! `cloud-backup.json`: what has been copied to each cloud service, so a backup is never uploaded twice.
//!
//! Per service, each uploaded backup is recorded by its path under `Saves Backup` with its size, modified
//! time, SHA-256 and the path it has in the cloud, plus when the last upload and the last good pass were and
//! the last failure (a code word and a time). No token, no account name, no email: the file is safe to paste
//! into a bug report. A file that cannot be read is moved aside (`.damaged`) and the record starts again;
//! the cloud listing a later pass takes recognises what is already there, so nothing is uploaded twice.

use super::service::{ErrorKind, ServiceId};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "cloud-backup.json";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uploaded {
    pub size: u64,
    /// The local file's modified time (seconds) when it was recorded.
    pub mtime: u64,
    pub sha256: String,
    /// Path under the Deck's folder in the cloud (usually the same as the local path).
    pub cloud_path: String,
    pub at: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServiceRecord {
    /// Local path under `Saves Backup` (`/` separators) -> what was copied.
    pub uploaded: BTreeMap<String, Uploaded>,
    /// The last time a file was copied.
    pub last_upload: Option<u64>,
    /// The last time a pass finished with nothing left to do.
    pub last_ok: Option<u64>,
    pub last_error: Option<(ErrorKind, u64)>,
}

impl ServiceRecord {
    fn to_json(&self) -> Value {
        let mut m = Map::new();
        let files: Map<String, Value> = self
            .uploaded
            .iter()
            .map(|(k, u)| {
                let mut f = Map::new();
                f.insert("size".into(), u.size.into());
                f.insert("mtime".into(), u.mtime.into());
                f.insert("sha256".into(), u.sha256.clone().into());
                f.insert("cloud_path".into(), u.cloud_path.clone().into());
                f.insert("at".into(), u.at.into());
                (k.clone(), Value::Object(f))
            })
            .collect();
        m.insert("uploaded".into(), Value::Object(files));
        if let Some(t) = self.last_upload {
            m.insert("last_upload".into(), t.into());
        }
        if let Some(t) = self.last_ok {
            m.insert("last_ok".into(), t.into());
        }
        if let Some((kind, at)) = self.last_error {
            m.insert("last_error".into(), serde_json::json!({ "code": kind.code(), "at": at }));
        }
        Value::Object(m)
    }

    fn from_json(v: &Value) -> ServiceRecord {
        let mut out = ServiceRecord::default();
        if let Some(files) = v.get("uploaded").and_then(Value::as_object) {
            for (path, f) in files {
                let (Some(size), Some(sha)) = (f.get("size").and_then(Value::as_u64), f.get("sha256").and_then(Value::as_str)) else { continue };
                let cloud_path = f.get("cloud_path").and_then(Value::as_str).unwrap_or(path).to_string();
                out.uploaded.insert(path.clone(), Uploaded { size, mtime: f.get("mtime").and_then(Value::as_u64).unwrap_or(0), sha256: sha.to_string(), cloud_path, at: f.get("at").and_then(Value::as_u64).unwrap_or(0) });
            }
        }
        out.last_upload = v.get("last_upload").and_then(Value::as_u64);
        out.last_ok = v.get("last_ok").and_then(Value::as_u64);
        out.last_error = v.get("last_error").and_then(|e| Some((ErrorKind::from_code(e.get("code")?.as_str()?)?, e.get("at")?.as_u64()?)));
        out
    }
}

#[derive(Debug)]
pub struct Record {
    path: PathBuf,
    services: BTreeMap<String, ServiceRecord>,
}

impl Record {
    pub fn in_memory() -> Record {
        Record { path: PathBuf::new(), services: BTreeMap::new() }
    }

    pub fn load(path: &Path) -> Record {
        let services = match fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Value>(text.strip_prefix('\u{FEFF}').unwrap_or(&text)) {
                Ok(Value::Object(map)) => map
                    .get("services")
                    .and_then(Value::as_object)
                    .map(|s| s.iter().filter(|(k, _)| ServiceId::from_key(k).is_some()).map(|(k, v)| (k.clone(), ServiceRecord::from_json(v))).collect())
                    .unwrap_or_default(),
                _ => {
                    // Kept for a look, then started again; the cloud listing recognises what is already there.
                    let mut aside = path.as_os_str().to_os_string();
                    aside.push(".damaged");
                    let _ = fs::rename(path, PathBuf::from(aside));
                    BTreeMap::new()
                }
            },
            Err(_) => BTreeMap::new(),
        };
        Record { path: path.to_path_buf(), services }
    }

    pub fn service(&self, id: ServiceId) -> ServiceRecord {
        self.services.get(id.key()).cloned().unwrap_or_default()
    }

    pub fn set_service(&mut self, id: ServiceId, rec: ServiceRecord) {
        self.services.insert(id.key().to_string(), rec);
    }

    /// Forgets everything about one service (signing out, or a different account signing in).
    pub fn forget(&mut self, id: ServiceId) {
        self.services.remove(id.key());
    }

    pub fn save(&self) -> io::Result<()> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }
        let services: Map<String, Value> = self.services.iter().map(|(k, v)| (k.clone(), v.to_json())).collect();
        let doc = serde_json::json!({ "version": 1, "services": Value::Object(services) });
        let text = serde_json::to_string_pretty(&doc).map_err(io::Error::other)?;
        let mut tmp_name = self.path.file_name().unwrap_or_default().to_os_string();
        tmp_name.push(".tmp");
        let tmp = self.path.with_file_name(tmp_name);
        let result = (|| {
            let mut file = fs::File::create(&tmp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&tmp, &self.path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dpm_cloud_record_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample() -> ServiceRecord {
        let mut r = ServiceRecord::default();
        r.uploaded.insert("Global Backups/2026-10-24.zip".into(), Uploaded { size: 10, mtime: 99, sha256: "ab".repeat(32), cloud_path: "Global Backups/2026-10-24.zip".into(), at: 1_000 });
        r.last_upload = Some(1_000);
        r.last_ok = Some(1_001);
        r.last_error = Some((ErrorKind::Offline, 1_002));
        r
    }

    #[test]
    fn a_record_round_trips() {
        let dir = temp_dir("roundtrip");
        let path = dir.join(FILE_NAME);
        let mut rec = Record::load(&path);
        rec.set_service(ServiceId::Google, sample());
        rec.save().unwrap();
        assert_eq!(Record::load(&path).service(ServiceId::Google), sample());
        assert_eq!(Record::load(&path).service(ServiceId::Google).last_error, Some((ErrorKind::Offline, 1_002)));
    }

    #[test]
    fn forgetting_a_service_empties_it() {
        let mut rec = Record::in_memory();
        rec.set_service(ServiceId::Google, sample());
        rec.forget(ServiceId::Google);
        assert_eq!(rec.service(ServiceId::Google), ServiceRecord::default());
    }

    #[test]
    fn a_damaged_file_is_set_aside_and_the_record_starts_again() {
        let dir = temp_dir("damaged");
        let path = dir.join(FILE_NAME);
        fs::write(&path, "{ nope").unwrap();
        let rec = Record::load(&path);
        assert!(rec.service(ServiceId::Google).uploaded.is_empty());
        assert_eq!(fs::read_to_string(dir.join("cloud-backup.json.damaged")).unwrap(), "{ nope");
        assert!(!path.exists());
    }

    #[test]
    fn the_file_holds_no_token_or_account_field() {
        let dir = temp_dir("fields");
        let path = dir.join(FILE_NAME);
        let mut rec = Record::load(&path);
        rec.set_service(ServiceId::Google, sample());
        rec.save().unwrap();
        let text = fs::read_to_string(&path).unwrap().to_lowercase();
        for word in ["token", "email", "account", "secret", "password", "@"] {
            assert!(!text.contains(word), "{word} in {text}");
        }
    }

    #[test]
    fn entries_without_a_size_or_hash_are_dropped_on_load() {
        let dir = temp_dir("partial");
        let path = dir.join(FILE_NAME);
        fs::write(&path, r#"{"services":{"google":{"uploaded":{"a.zip":{"size":1},"b.zip":{"size":2,"sha256":"x"}}}}}"#).unwrap();
        let rec = Record::load(&path).service(ServiceId::Google);
        assert_eq!(rec.uploaded.keys().collect::<Vec<_>>(), ["b.zip"]);
        assert_eq!(rec.uploaded["b.zip"].cloud_path, "b.zip");
    }
}
