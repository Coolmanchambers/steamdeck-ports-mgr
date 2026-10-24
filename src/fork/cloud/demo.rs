//! A "cloud" that is a folder on this machine, for driving the screens end to end in a test or debug run
//! (`DPM_CLOUD_DEMO_DIR=<folder>`) without Google. Files go to `<folder>/<device>/Saves Backup/<path>`. Like the
//! real services it can list, upload and download, and nothing else; it does not exist in a release build.

use super::service::{CloudService, ErrorKind, Progress, RemoteFile, ServiceError, ServiceId};
use super::sha256::file_sha256;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub struct FolderService {
    root: PathBuf,
}

/// The folder a debug or test run uses as its cloud, when it asks for one.
pub fn demo_dir() -> Option<PathBuf> {
    std::env::var_os("DPM_CLOUD_DEMO_DIR").map(PathBuf::from).filter(|p| p.is_absolute())
}

impl FolderService {
    pub fn new(root: PathBuf) -> FolderService {
        FolderService { root }
    }

    fn io(e: std::io::Error) -> ServiceError {
        ServiceError::new(ErrorKind::Other, format!("the demo cloud failed ({})", e.kind()))
    }

    fn walk(dir: &Path, base: &Path, device: &str, out: &mut Vec<RemoteFile>) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                FolderService::walk(&path, base, device, out);
            } else if let Ok(meta) = entry.metadata() {
                let rel = path.strip_prefix(base).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
                let modified = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs());
                out.push(RemoteFile { device: device.to_string(), path: rel.clone(), size: meta.len(), sha256: file_sha256(&path).ok(), id: format!("{device}/{rel}"), modified });
            }
        }
    }
}

impl CloudService for FolderService {
    fn id(&self) -> ServiceId {
        ServiceId::Google
    }

    fn list_all(&mut self) -> Result<Vec<RemoteFile>, ServiceError> {
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.root).into_iter().flatten().flatten().filter(|e| e.path().is_dir()) {
            let device = entry.file_name().to_string_lossy().into_owned();
            let base = entry.path().join("Saves Backup");
            FolderService::walk(&base, &base, &device, &mut out);
        }
        Ok(out)
    }

    fn upload(&mut self, device: &str, rel_path: &str, file: &Path, size: u64, progress: Progress) -> Result<RemoteFile, ServiceError> {
        let dest = rel_path.split('/').fold(self.root.join(device).join("Saves Backup"), |p, part| p.join(part));
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(FolderService::io)?;
        }
        let mut out = fs::OpenOptions::new().write(true).create_new(true).open(&dest).map_err(FolderService::io)?;
        let mut input = fs::File::open(file).map_err(FolderService::io)?;
        std::io::copy(&mut input, &mut out).map_err(FolderService::io)?;
        progress(size, size);
        let meta = fs::metadata(&dest).map_err(FolderService::io)?;
        Ok(RemoteFile { device: device.into(), path: rel_path.into(), size: meta.len(), sha256: file_sha256(&dest).ok(), id: format!("{device}/{rel_path}"), modified: None })
    }

    fn download(&mut self, file: &RemoteFile, dest: &Path, progress: Progress) -> Result<(), ServiceError> {
        let src = file.path.split('/').fold(self.root.join(&file.device).join("Saves Backup"), |p, part| p.join(part));
        let mut out = fs::OpenOptions::new().write(true).create_new(true).open(dest).map_err(FolderService::io)?;
        let mut input = fs::File::open(&src).map_err(FolderService::io)?;
        let n = std::io::copy(&mut input, &mut out).map_err(FolderService::io)?;
        progress(n, n);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_list_download_round_trip_and_no_replacing() {
        let dir = std::env::temp_dir().join(format!("dpm_cloud_demo_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("local")).unwrap();
        fs::write(dir.join("local/a.zip"), b"zipdata").unwrap();
        let mut svc = FolderService::new(dir.join("cloud"));
        fs::create_dir_all(dir.join("cloud")).unwrap();
        let up = svc.upload("deck", "Global Backups/a.zip", &dir.join("local/a.zip"), 7, &mut |_, _| {}).unwrap();
        assert_eq!((up.size, up.path.as_str()), (7, "Global Backups/a.zip"));
        assert!(svc.upload("deck", "Global Backups/a.zip", &dir.join("local/a.zip"), 7, &mut |_, _| {}).is_err(), "never replaces");
        let all = svc.list_all().unwrap();
        assert_eq!(all.len(), 1);
        svc.download(&all[0], &dir.join("back.zip"), &mut |_, _| {}).unwrap();
        assert_eq!(fs::read(dir.join("back.zip")).unwrap(), b"zipdata");
        let _ = fs::remove_dir_all(&dir);
    }
}
