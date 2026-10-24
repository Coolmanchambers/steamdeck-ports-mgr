//! Getting a backup back from the cloud into `Saves Backup`, and nothing more.
//!
//! The zip is downloaded into a scratch folder (`_cloud_download_*`) inside `Saves Backup`, checked (size,
//! the checksum when the service gives one, that it reads as a zip and that no name in it points outside
//! its folder or is a link, the same checks Restore saves makes before it unpacks anything), and only then
//! put in the folder that matches where it came from. It never replaces a local file: a name already taken
//! by different content gets `-cloud` added (`-cloud-2`, ...). The existing Restore saves flow then lists
//! the zip; this module has no restore engine of its own.

use super::local::{GLOBAL_DIR, RESTORE_DIR, UPDATE_DIR};
use super::record::{ServiceRecord, Uploaded};
use super::service::{CloudService, ErrorKind, RemoteFile, ServiceError};
use super::sha256::file_sha256;
use crate::fork::save_restore::zip_index::{is_safe_name, read_index};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Where a cloud file belongs, from its path in the cloud. Anything else is not a backup this app made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// `Global Backups/<name>.zip`: every port.
    Global { name: String },
    /// `Update Backups/<folder>/<name>.zip`.
    Update { folder: String, name: String },
    /// `Restore Backups/<folder>/<name>.zip`.
    Restore { folder: String, name: String },
}

impl Place {
    pub fn parse(path: &str) -> Option<Place> {
        let piece = |s: &str| (!s.is_empty() && !s.starts_with(['.', '_']) && !s.chars().any(|c| c.is_control() || c == '/' || c == '\\')).then(|| s.to_string());
        let zip = |s: &str| s.strip_suffix(".zip").and_then(piece).map(|_| s.to_string());
        let parts: Vec<&str> = path.split('/').collect();
        match parts.as_slice() {
            [GLOBAL_DIR, name] => Some(Place::Global { name: zip(name)? }),
            [UPDATE_DIR, folder, name] => Some(Place::Update { folder: piece(folder)?, name: zip(name)? }),
            [RESTORE_DIR, folder, name] => Some(Place::Restore { folder: piece(folder)?, name: zip(name)? }),
            _ => None,
        }
    }

    /// The path under `Saves Backup`, `/` separated.
    pub fn rel(&self) -> String {
        match self {
            Place::Global { name } => format!("{GLOBAL_DIR}/{name}"),
            Place::Update { folder, name } => format!("{UPDATE_DIR}/{folder}/{name}"),
            Place::Restore { folder, name } => format!("{RESTORE_DIR}/{folder}/{name}"),
        }
    }

    /// The port folder a per-port backup is for.
    pub fn folder(&self) -> Option<&str> {
        match self {
            Place::Global { .. } => None,
            Place::Update { folder, .. } | Place::Restore { folder, .. } => Some(folder),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Downloaded {
    /// Path of the zip under `Saves Backup` as it was put there (`-cloud` may have been added).
    pub rel: String,
    pub path: PathBuf,
    pub place: Place,
    /// True when the same file was already on the Deck (nothing was written).
    pub already_here: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DownloadError {
    /// The service failed (offline, signed out, ...).
    Service(ServiceError),
    /// Refused or damaged; nothing was written to the Deck. The text is for the person.
    Refused(String),
}

impl DownloadError {
    pub fn text(&self) -> String {
        match self {
            DownloadError::Service(e) => e.detail.clone(),
            DownloadError::Refused(t) => t.clone(),
        }
    }
}

/// Texts here may carry a file name from a zip or a message from the system: control characters, web addresses and
/// length are dealt with before a person sees them.
fn refused(text: impl Into<String>) -> DownloadError {
    DownloadError::Refused(super::log::scrub(&text.into()))
}

fn with_cloud_suffix(name: &str, n: usize) -> String {
    let stem = name.strip_suffix(".zip").unwrap_or(name);
    if n <= 1 {
        format!("{stem}-cloud.zip")
    } else {
        format!("{stem}-cloud-{n}.zip")
    }
}

fn mtime_secs(path: &Path) -> u64 {
    fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
}

/// Puts `from` at `to` without ever replacing something that is there: a hard link refuses an existing
/// name; where links are not possible (exFAT) the name is checked first.
fn place_without_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    match fs::hard_link(from, to) {
        // The file is in place once the link exists; the scratch copy goes with its folder if this fails.
        Ok(()) => {
            let _ = fs::remove_file(from);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(e),
        Err(_) => {
            if to.exists() {
                return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
            }
            fs::rename(from, to)
        }
    }
}

/// Downloads one cloud file into `Saves Backup`. `rec` is told about it, so the next upload pass does not copy
/// it back up. The scratch folder is always removed.
pub fn download_one(
    service: &mut dyn CloudService,
    remote: &RemoteFile,
    saves_backup_dir: &Path,
    rec: &mut ServiceRecord,
    now: u64,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Downloaded, DownloadError> {
    let place = Place::parse(&remote.path).ok_or_else(|| refused("that is not a save backup this app made"))?;
    if let Some(problem) = crate::fork::save_restore::space_problem(remote.size.saturating_mul(2), crate::fork::save_restore::free_bytes(saves_backup_dir.ancestors().find(|a| a.exists()).unwrap_or(saves_backup_dir)), saves_backup_dir) {
        return Err(refused(problem));
    }
    fs::create_dir_all(saves_backup_dir).map_err(|e| refused(format!("could not make the backup folder ({})", e.kind())))?;
    let scratch = tempfile::Builder::new().prefix("_cloud_download_").tempdir_in(saves_backup_dir).map_err(|e| refused(format!("could not make a scratch folder ({})", e.kind())))?;
    let file = scratch.path().join("download.zip");
    service.download(remote, &file, progress).map_err(DownloadError::Service)?;
    let size = fs::metadata(&file).map_or(0, |m| m.len());
    if size != remote.size {
        return Err(refused("the copy from the cloud did not arrive whole; nothing was kept"));
    }
    let sha = file_sha256(&file).map_err(|e| refused(format!("could not read the copy ({})", e.kind())))?;
    if remote.sha256.as_deref().is_some_and(|h| !h.eq_ignore_ascii_case(&sha)) {
        return Err(refused("the copy from the cloud did not match its checksum; nothing was kept"));
    }
    let entries = read_index(&file).map_err(|e| refused(format!("the file from the cloud is not a readable zip ({e}); nothing was kept")))?;
    if let Some(bad) = entries.iter().find(|e| !is_safe_name(&e.name)) {
        return Err(refused(format!("the file from the cloud holds a name that points outside its folder ({}); nothing was kept", bad.name)));
    }
    if let Some(link) = entries.iter().find(|e| e.is_symlink) {
        return Err(refused(format!("the file from the cloud holds a link instead of a file ({}); nothing was kept", link.name)));
    }
    let rel = place.rel();
    let (dir_rel, name) = rel.rsplit_once('/').map(|(d, n)| (d.to_string(), n.to_string())).unwrap_or_default();
    let dir = dir_rel.split('/').fold(saves_backup_dir.to_path_buf(), |p, part| p.join(part));
    fs::create_dir_all(&dir).map_err(|e| refused(format!("could not make the folder for it ({})", e.kind())))?;
    let mut candidate = name.clone();
    for n in 1..100 {
        let target = dir.join(&candidate);
        if target.exists() {
            if fs::metadata(&target).is_ok_and(|m| m.is_file()) && file_sha256(&target).is_ok_and(|h| h == sha) {
                let local_rel = format!("{dir_rel}/{candidate}");
                note_in_record(rec, &local_rel, &target, &sha, remote, now);
                return Ok(Downloaded { rel: local_rel, path: target, place, already_here: true });
            }
            candidate = with_cloud_suffix(&name, n);
            continue;
        }
        match place_without_replacing(&file, &target) {
            Ok(()) => {
                let local_rel = format!("{dir_rel}/{candidate}");
                note_in_record(rec, &local_rel, &target, &sha, remote, now);
                return Ok(Downloaded { rel: local_rel, path: target, place, already_here: false });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => candidate = with_cloud_suffix(&name, n),
            Err(e) => return Err(refused(format!("could not put the file in place ({})", e.kind()))),
        }
    }
    Err(refused("too many copies of that backup are already on this Deck"))
}

fn note_in_record(rec: &mut ServiceRecord, local_rel: &str, path: &Path, sha: &str, remote: &RemoteFile, now: u64) {
    rec.uploaded.insert(local_rel.to_string(), Uploaded { size: remote.size, mtime: mtime_secs(path), sha256: sha.to_string(), cloud_path: remote.path.clone(), at: now });
}

/// The cloud files for this Deck's own `Saves Backup` that are not on it yet (same path, same size counts as
/// there), newest first by the service's modified time, for "Copy all my backups to this Deck".
pub fn missing_locally<'a>(remote: &'a [RemoteFile], saves_backup_dir: &Path) -> Vec<&'a RemoteFile> {
    let mut out: Vec<&RemoteFile> = remote
        .iter()
        .filter(|f| Place::parse(&f.path).is_some())
        .filter(|f| {
            let local = f.path.split('/').fold(saves_backup_dir.to_path_buf(), |p, part| p.join(part));
            match fs::metadata(&local) {
                Ok(m) if m.is_file() && m.len() == f.size => {
                    // Same name and size: when the cloud gives a checksum, the content has to match too.
                    f.sha256.as_deref().is_some_and(|h| file_sha256(&local).map_or(true, |mine| !mine.eq_ignore_ascii_case(h)))
                }
                _ => true,
            }
        })
        .collect();
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.path.cmp(&b.path)));
    out
}

/// True for the failures a person can wait out.
pub fn will_pass(e: &DownloadError) -> bool {
    matches!(e, DownloadError::Service(s) if s.kind.retries_by_itself() || s.kind == ErrorKind::RateLimited)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::cloud::fake::FakeService;
    use crate::fork::cloud::local::testing::{tiny_zip, write_zip};
    use crate::fork::cloud::sha256::sha256_hex;
    use std::collections::BTreeSet;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dpm_cloud_restore_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn remote_of(fake: &FakeService, device: &str, path: &str) -> RemoteFile {
        let bytes = &fake.files[&(device.to_string(), path.to_string())];
        RemoteFile { device: device.into(), path: path.into(), size: bytes.len() as u64, sha256: Some(sha256_hex(bytes)), id: "x".into(), modified: Some(5) }
    }

    fn names_under(dir: &Path) -> BTreeSet<String> {
        fn walk(dir: &Path, base: &Path, out: &mut BTreeSet<String>) {
            for e in fs::read_dir(dir).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, base, out);
                } else {
                    out.insert(p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/"));
                }
            }
        }
        let mut out = BTreeSet::new();
        walk(dir, dir, &mut out);
        out
    }

    #[test]
    fn places_are_parsed_strictly() {
        assert_eq!(Place::parse("Global Backups/2026-10-24.zip"), Some(Place::Global { name: "2026-10-24.zip".into() }));
        assert_eq!(Place::parse("Update Backups/soh/20261024-101010.zip"), Some(Place::Update { folder: "soh".into(), name: "20261024-101010.zip".into() }));
        assert_eq!(Place::parse("Restore Backups/soh/x.zip").unwrap().folder(), Some("soh"));
        for bad in [
            "../x.zip",
            "Global Backups/../x.zip",
            "Update Backups/../x/y.zip",
            "Update Backups/soh/../../y.zip",
            "Global Backups/x.txt",
            "Global Backups/.zip",
            "Pending Restore/soh/x.zip",
            "Update Backups/soh/deeper/x.zip",
            "Update Backups//x.zip",
            "Update Backups/_scratch/x.zip",
            "/Global Backups/x.zip",
            "Global Backups/a\\b.zip",
            "Global Backups/x.zip/",
            "",
        ] {
            assert_eq!(Place::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_backup_lands_in_its_folder_and_is_recorded() {
        let dir = temp_dir("lands");
        let mut fake = FakeService::new();
        fake.put("deck", "Update Backups/soh/s1.zip", &tiny_zip("save_folder/a.sav", b"data"));
        let remote = remote_of(&fake, "deck", "Update Backups/soh/s1.zip");
        let mut rec = ServiceRecord::default();
        let got = download_one(&mut fake, &remote, &dir, &mut rec, 100, &mut |_, _| {}).unwrap();
        assert_eq!((got.rel.as_str(), got.already_here), ("Update Backups/soh/s1.zip", false));
        assert_eq!(fs::read(dir.join("Update Backups/soh/s1.zip")).unwrap(), tiny_zip("save_folder/a.sav", b"data"));
        assert_eq!(rec.uploaded["Update Backups/soh/s1.zip"].cloud_path, "Update Backups/soh/s1.zip");
        assert_eq!(names_under(&dir), BTreeSet::from(["Update Backups/soh/s1.zip".to_string()]), "no scratch folder left behind");
    }

    #[test]
    fn it_never_overwrites_a_local_file_with_other_content() {
        let dir = temp_dir("never");
        write_zip(&dir.join("Global Backups/day.zip"), "mine.sav", b"my own saves");
        let mine = fs::read(dir.join("Global Backups/day.zip")).unwrap();
        let mut fake = FakeService::new();
        fake.put("deck", "Global Backups/day.zip", &tiny_zip("cloud.sav", b"from the cloud"));
        let remote = remote_of(&fake, "deck", "Global Backups/day.zip");
        let mut rec = ServiceRecord::default();
        let first = download_one(&mut fake, &remote, &dir, &mut rec, 1, &mut |_, _| {}).unwrap();
        assert_eq!(first.rel, "Global Backups/day-cloud.zip");
        let second = download_one(&mut fake, &remote, &dir, &mut rec, 2, &mut |_, _| {}).unwrap();
        assert!(second.already_here && second.rel == "Global Backups/day-cloud.zip", "{second:?}");
        assert_eq!(fs::read(dir.join("Global Backups/day.zip")).unwrap(), mine, "the local file is untouched");
        fs::write(dir.join("Global Backups/day-cloud.zip"), tiny_zip("other.sav", b"changed since")).unwrap();
        let third = download_one(&mut fake, &remote, &dir, &mut rec, 3, &mut |_, _| {}).unwrap();
        assert_eq!(third.rel, "Global Backups/day-cloud-2.zip");
        assert_eq!(names_under(&dir).len(), 3);
    }

    #[test]
    fn the_same_file_already_here_writes_nothing() {
        let dir = temp_dir("same");
        let bytes = tiny_zip("a.sav", b"data");
        fs::create_dir_all(dir.join("Global Backups")).unwrap();
        fs::write(dir.join("Global Backups/day.zip"), &bytes).unwrap();
        let mut fake = FakeService::new();
        fake.put("deck", "Global Backups/day.zip", &bytes);
        let remote = remote_of(&fake, "deck", "Global Backups/day.zip");
        let got = download_one(&mut fake, &remote, &dir, &mut ServiceRecord::default(), 1, &mut |_, _| {}).unwrap();
        assert!(got.already_here);
        assert_eq!(names_under(&dir).len(), 1);
    }

    #[test]
    fn a_name_that_points_outside_is_refused_before_anything_is_downloaded() {
        let dir = temp_dir("hostile");
        let mut fake = FakeService::new();
        for path in ["../evil.zip", "Global Backups/../../evil.zip", "Update Backups/../../x/evil.zip"] {
            fake.put("deck", path, &tiny_zip("a", b"x"));
            let remote = remote_of(&fake, "deck", path);
            let err = download_one(&mut fake, &remote, &dir, &mut ServiceRecord::default(), 1, &mut |_, _| {}).unwrap_err();
            assert!(matches!(err, DownloadError::Refused(_)), "{path}: {err:?}");
        }
        assert!(fake.calls.iter().all(|c| !c.starts_with("download")), "{:?}", fake.calls);
        assert!(names_under(&dir).is_empty());
    }

    #[test]
    fn a_zip_with_an_entry_outside_its_folder_is_refused_and_leaves_nothing() {
        let dir = temp_dir("slip");
        let mut fake = FakeService::new();
        fake.put("deck", "Global Backups/slip.zip", &tiny_zip("../escape.sav", b"x"));
        let remote = remote_of(&fake, "deck", "Global Backups/slip.zip");
        let err = download_one(&mut fake, &remote, &dir, &mut ServiceRecord::default(), 1, &mut |_, _| {}).unwrap_err();
        assert!(matches!(&err, DownloadError::Refused(t) if t.contains("outside")), "{err:?}");
        assert!(names_under(&dir).is_empty(), "{:?}", names_under(&dir));
        assert!(!fs::read_dir(&dir).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("_cloud_download_")), "the scratch folder is removed");
    }

    #[test]
    fn a_short_or_altered_download_is_refused() {
        let dir = temp_dir("short");
        let bytes = tiny_zip("a.sav", b"data");
        let mut fake = FakeService::new();
        fake.put("deck", "Global Backups/a.zip", &bytes);
        let mut remote = remote_of(&fake, "deck", "Global Backups/a.zip");
        remote.size += 5;
        assert!(matches!(download_one(&mut fake, &remote, &dir, &mut ServiceRecord::default(), 1, &mut |_, _| {}), Err(DownloadError::Refused(_))));
        let mut remote = remote_of(&fake, "deck", "Global Backups/a.zip");
        remote.sha256 = Some("0".repeat(64));
        assert!(matches!(download_one(&mut fake, &remote, &dir, &mut ServiceRecord::default(), 1, &mut |_, _| {}), Err(DownloadError::Refused(t)) if t.contains("checksum")));
        fake.put("deck", "Global Backups/junk.zip", b"this is not a zip file at all, just text");
        let remote = remote_of(&fake, "deck", "Global Backups/junk.zip");
        assert!(matches!(download_one(&mut fake, &remote, &dir, &mut ServiceRecord::default(), 1, &mut |_, _| {}), Err(DownloadError::Refused(t)) if t.contains("not a readable zip")));
        assert!(names_under(&dir).is_empty());
    }

    #[test]
    fn a_service_failure_is_passed_on_and_leaves_nothing() {
        let dir = temp_dir("offline");
        let mut fake = FakeService::new().failing(ErrorKind::Offline);
        fake.put("deck", "Global Backups/a.zip", &tiny_zip("a", b"x"));
        let remote = remote_of(&fake, "deck", "Global Backups/a.zip");
        let err = download_one(&mut fake, &remote, &dir, &mut ServiceRecord::default(), 1, &mut |_, _| {}).unwrap_err();
        assert!(matches!(&err, DownloadError::Service(e) if e.kind == ErrorKind::Offline));
        assert!(will_pass(&err));
        assert!(names_under(&dir).is_empty());
    }

    #[test]
    fn same_name_and_size_with_other_content_counts_as_missing_when_the_cloud_has_a_checksum() {
        let dir = temp_dir("samesize");
        fs::create_dir_all(dir.join("Global Backups")).unwrap();
        fs::write(dir.join("Global Backups/a.zip"), b"AAAA").unwrap();
        let mut f = RemoteFile { device: "d".into(), path: "Global Backups/a.zip".into(), size: 4, sha256: Some(sha256_hex(b"BBBB")), id: "i".into(), modified: Some(1) };
        assert_eq!(missing_locally(&[f.clone()], &dir).len(), 1, "a different file of the same size is not the same file");
        f.sha256 = Some(sha256_hex(b"AAAA").to_uppercase());
        assert!(missing_locally(&[f.clone()], &dir).is_empty(), "the same content, whatever the case of the hex");
        f.sha256 = None;
        assert!(missing_locally(&[f], &dir).is_empty(), "no checksum: size and name are all there is");
    }

    #[test]
    fn refusal_texts_lose_control_characters_and_addresses() {
        let e = refused("bad name \u{7}evil\nline https://example.com/x?key=SECRET");
        assert_eq!(e.text(), "bad name  evil line [address]");
    }

    #[test]
    fn missing_locally_lists_what_the_deck_lacks_newest_first() {
        let dir = temp_dir("missing");
        let bytes = tiny_zip("a", b"x");
        fs::create_dir_all(dir.join("Global Backups")).unwrap();
        fs::write(dir.join("Global Backups/have.zip"), &bytes).unwrap();
        let file = |path: &str, size: u64, modified: u64| RemoteFile { device: "d".into(), path: path.into(), size, sha256: None, id: "i".into(), modified: Some(modified) };
        let remote = vec![
            file("Global Backups/have.zip", bytes.len() as u64, 1),
            file("Global Backups/old.zip", 10, 2),
            file("Update Backups/soh/new.zip", 10, 9),
            file("Global Backups/have.zip.partial", 10, 9),
            file("Global Backups/resized.zip", 10, 5),
        ];
        fs::write(dir.join("Global Backups/resized.zip"), b"different size on disk").unwrap();
        let missing: Vec<&str> = missing_locally(&remote, &dir).iter().map(|f| f.path.as_str()).collect();
        assert_eq!(missing, ["Update Backups/soh/new.zip", "Global Backups/resized.zip", "Global Backups/old.zip"]);
    }
}
