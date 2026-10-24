//! An in-memory cloud service for tests (and nothing else): files live in a map, failures can be queued, and
//! every call is written down. It has no delete of any kind, like the real trait.

use super::service::{CloudService, ErrorKind, Progress, RemoteFile, ServiceError, ServiceId};
use super::sha256::sha256_hex;
use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

pub struct FakeService {
    pub id: ServiceId,
    /// (device, path) -> bytes.
    pub files: BTreeMap<(String, String), Vec<u8>>,
    /// Each queued error fails the next call that reaches the service, whatever it is.
    pub fail: VecDeque<ServiceError>,
    /// Report a wrong size for the next upload.
    pub corrupt_next_upload: bool,
    /// Hide the checksum, like a service that does not report one.
    pub report_sha256: bool,
    pub calls: Vec<String>,
}

impl FakeService {
    pub fn new() -> FakeService {
        FakeService { id: ServiceId::Google, files: BTreeMap::new(), fail: VecDeque::new(), corrupt_next_upload: false, report_sha256: true, calls: Vec::new() }
    }

    pub fn put(&mut self, device: &str, path: &str, bytes: &[u8]) {
        self.files.insert((device.to_string(), path.to_string()), bytes.to_vec());
    }

    pub fn failing(mut self, kind: ErrorKind) -> FakeService {
        self.fail.push_back(ServiceError::new(kind, "injected"));
        self
    }

    fn entry(&self, device: &str, path: &str, bytes: &[u8]) -> RemoteFile {
        RemoteFile {
            device: device.to_string(),
            path: path.to_string(),
            size: bytes.len() as u64,
            sha256: self.report_sha256.then(|| sha256_hex(bytes)),
            id: format!("{device}/{path}"),
            modified: None,
        }
    }

    fn check(&mut self, call: String) -> Result<(), ServiceError> {
        self.calls.push(call);
        match self.fail.pop_front() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    pub fn uploads(&self) -> Vec<&str> {
        self.calls.iter().filter_map(|c| c.strip_prefix("upload ")).collect()
    }
}

impl CloudService for FakeService {
    fn id(&self) -> ServiceId {
        self.id
    }

    fn list_all(&mut self) -> Result<Vec<RemoteFile>, ServiceError> {
        self.check("list".into())?;
        Ok(self.files.iter().map(|((d, p), b)| self.entry(d, p, b)).collect())
    }

    fn upload(&mut self, device: &str, rel_path: &str, file: &Path, size: u64, progress: Progress) -> Result<RemoteFile, ServiceError> {
        self.check(format!("upload {device}/{rel_path}"))?;
        let bytes = std::fs::read(file).map_err(|e| ServiceError::other(e.to_string()))?;
        assert_eq!(bytes.len() as u64, size, "the caller's size matches the file");
        let key = (device.to_string(), rel_path.to_string());
        assert!(!self.files.contains_key(&key), "an upload must never replace a cloud file: {rel_path}");
        progress(size, size);
        let mut shown = self.entry(device, rel_path, &bytes);
        if std::mem::take(&mut self.corrupt_next_upload) {
            shown.size += 1;
        }
        self.files.insert(key, bytes);
        Ok(shown)
    }

    fn download(&mut self, file: &RemoteFile, dest: &Path, progress: Progress) -> Result<(), ServiceError> {
        self.check(format!("download {}/{}", file.device, file.path))?;
        let bytes = self.files.get(&(file.device.clone(), file.path.clone())).ok_or_else(|| ServiceError::other("no such file"))?;
        assert!(!dest.exists(), "a download must not write over an existing file");
        std::fs::write(dest, bytes).map_err(|e| ServiceError::other(e.to_string()))?;
        progress(bytes.len() as u64, bytes.len() as u64);
        Ok(())
    }
}
