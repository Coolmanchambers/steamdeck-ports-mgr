//! Google Drive as a [`CloudService`]: the app's own folder, uploads (one request up to 5 MB, a resumable upload
//! above that), listing and downloads.
//!
//! The layout in Drive is `Deck Ports Manager/<this Deck's name>/Saves Backup/<the same folders as on the Deck>`.
//! The app asks only for the `drive.file` permission, so Drive shows it only the files and folders it made itself.
//!
//! What this file never does, and a test checks the source for: delete a file, empty or use the trash, or change
//! a file that is already there. An upload always creates a new file; the planner chose a name that is free.
//!
//! Failures are told apart by Google's code words (`reason`) and HTTP status, never by message text. A 401 makes
//! one forced refresh of the access token and one more try; only a 401 after that is "signed out".

use super::endpoints::Urls;
use super::net::{classify_status, Net, Reply, Request};
use super::oauth::{reason_of, OAuthConfig};
use super::service::{CloudService, ErrorKind, Progress, RemoteFile, ServiceError, ServiceId};
use super::store;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The folder at the top of the app's part of Drive.
pub const ROOT_FOLDER: &str = "Deck Ports Manager";
/// The folder inside a Deck's folder that mirrors the Deck's `Saves Backup`.
pub const SAVES_FOLDER: &str = "Saves Backup";
const FOLDER_TYPE: &str = "application/vnd.google-apps.folder";
const FILE_FIELDS: &str = "id,name,size,sha256Checksum,modifiedTime";

/// Uploads up to this size go in one request; bigger ones are resumable.
pub const SIMPLE_LIMIT: u64 = 5 * 1024 * 1024;
/// A resumable upload's piece: a multiple of 256 KiB, as Drive requires for every piece but the last.
pub const CHUNK: u64 = 8 * 1024 * 1024;
/// Tries for one piece (the first plus retries after a dropped connection or a 5xx).
const PIECE_TRIES: usize = 5;
/// How deep the listing goes under a Deck's `Saves Backup` (`Update Backups/<folder>/<file>` is two folders).
const MAX_DEPTH: usize = 4;

/// Where access tokens come from: the token file, under the lock `store` keeps.
#[derive(Clone)]
pub struct TokenSource {
    pub net: Net,
    pub cfg: OAuthConfig,
    pub tokens_path: PathBuf,
    pub id: ServiceId,
}

impl TokenSource {
    fn token(&self, force: bool) -> Result<String, ServiceError> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        store::access_token_with(&self.net, &self.cfg, &self.tokens_path, self.id, now, force)
    }
}

pub struct GoogleDrive {
    net: Net,
    urls: Urls,
    auth: TokenSource,
    /// Folder ids found or made in this run, by path (`Deck Ports Manager/deck/Saves Backup`).
    folders: HashMap<String, String>,
    simple_limit: u64,
    chunk: u64,
    sleep: fn(Duration),
}

impl GoogleDrive {
    pub fn new(net: Net, urls: Urls, auth: TokenSource) -> GoogleDrive {
        GoogleDrive { net, urls, auth, folders: HashMap::new(), simple_limit: SIMPLE_LIMIT, chunk: CHUNK, sleep: std::thread::sleep }
    }

    /// Smaller limits and no waiting, for tests.
    #[cfg(test)]
    pub fn for_tests(mut self, simple_limit: u64, chunk: u64) -> GoogleDrive {
        assert_eq!(chunk % (256 * 1024), 0, "a piece must be a multiple of 256 KiB");
        self.simple_limit = simple_limit;
        self.chunk = chunk;
        self.sleep = |_| {};
        self
    }

    /// One call with a good access token: a 401 refreshes the token once and tries again; a 401 after that is
    /// "signed out". Any other failure status is classified.
    fn call(&self, build: &dyn Fn(&str) -> Request) -> Result<Reply, ServiceError> {
        let mut force = false;
        loop {
            let token = self.auth.token(force)?;
            let reply = self.net.send(build(&token))?;
            if reply.status == 401 && !force {
                force = true;
                continue;
            }
            if reply.ok() {
                return Ok(reply);
            }
            return Err(failure(&reply));
        }
    }

    fn json_of(reply: &Reply) -> Result<Value, ServiceError> {
        reply.json().ok_or_else(|| ServiceError::other("Google Drive's answer could not be read"))
    }

    fn q_escape(text: &str) -> String {
        text.replace('\\', "\\\\").replace('\'', "\\'")
    }

    fn files_url(&self, q: &str, fields: &str, page: Option<&str>) -> String {
        let mut url = format!("{}/files?q={}&fields={}&pageSize=1000&spaces=drive", self.urls.drive_api, super::net::url_escape(q), super::net::url_escape(fields));
        if let Some(p) = page {
            url.push_str(&format!("&pageToken={}", super::net::url_escape(p)));
        }
        url
    }

    /// The id of the folder called `name` inside `parent` (`root` for the top), when there is one. Of several with
    /// the same name the oldest is used.
    fn find_folder(&self, name: &str, parent: &str) -> Result<Option<String>, ServiceError> {
        let q = format!("mimeType='{FOLDER_TYPE}' and name='{}' and '{}' in parents and trashed=false", GoogleDrive::q_escape(name), GoogleDrive::q_escape(parent));
        let url = self.files_url(&q, "files(id,name,createdTime)", None) + "&orderBy=createdTime";
        let reply = self.call(&|t| Request::get(url.clone()).bearer(t))?;
        let json = GoogleDrive::json_of(&reply)?;
        Ok(json.get("files").and_then(Value::as_array).and_then(|f| f.first()).and_then(|f| f.get("id")).and_then(Value::as_str).map(str::to_string))
    }

    fn create_folder(&self, name: &str, parent: &str) -> Result<String, ServiceError> {
        let body = json!({ "name": name, "mimeType": FOLDER_TYPE, "parents": [parent] }).to_string().into_bytes();
        let url = format!("{}/files?fields=id", self.urls.drive_api);
        let reply = self.call(&|t| Request::post(url.clone()).bearer(t).header("Content-Type", "application/json").bytes(body.clone()))?;
        GoogleDrive::json_of(&reply)?.get("id").and_then(Value::as_str).map(str::to_string).ok_or_else(|| ServiceError::other("Google Drive made a folder but did not say which"))
    }

    /// The folder for `path` (pieces under `root`), made piece by piece when `create` is true.
    fn folder(&mut self, pieces: &[&str], create: bool) -> Result<Option<String>, ServiceError> {
        let mut parent = "root".to_string();
        let mut so_far = String::new();
        for piece in pieces {
            if !so_far.is_empty() {
                so_far.push('/');
            }
            so_far.push_str(piece);
            if let Some(id) = self.folders.get(&so_far) {
                parent = id.clone();
                continue;
            }
            let id = match self.find_folder(piece, &parent)? {
                Some(id) => id,
                None if create => self.create_folder(piece, &parent)?,
                None => return Ok(None),
            };
            self.folders.insert(so_far.clone(), id.clone());
            parent = id;
        }
        Ok(Some(parent))
    }

    fn children(&self, parent: &str) -> Result<Vec<Value>, ServiceError> {
        let q = format!("'{}' in parents and trashed=false", GoogleDrive::q_escape(parent));
        let fields = format!("nextPageToken,files({FILE_FIELDS},mimeType)");
        let mut out = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let url = self.files_url(&q, &fields, page.as_deref());
            let reply = self.call(&|t| Request::get(url.clone()).bearer(t))?;
            let json = GoogleDrive::json_of(&reply)?;
            out.extend(json.get("files").and_then(Value::as_array).cloned().unwrap_or_default());
            match json.get("nextPageToken").and_then(Value::as_str) {
                Some(next) => page = Some(next.to_string()),
                None => return Ok(out),
            }
        }
    }

    fn walk(&self, folder: &str, prefix: &str, device: &str, depth: usize, out: &mut Vec<RemoteFile>) -> Result<(), ServiceError> {
        for item in self.children(folder)? {
            let (Some(name), Some(id)) = (item.get("name").and_then(Value::as_str), item.get("id").and_then(Value::as_str)) else { continue };
            let path = if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") };
            if item.get("mimeType").and_then(Value::as_str) == Some(FOLDER_TYPE) {
                if depth < MAX_DEPTH {
                    self.walk(id, &path, device, depth + 1, out)?;
                }
            } else {
                out.push(remote_file(&item, device, path));
            }
        }
        Ok(())
    }

    fn metadata_body(name: &str, parent: &str) -> Vec<u8> {
        json!({ "name": name, "parents": [parent] }).to_string().into_bytes()
    }

    fn upload_simple(&self, parent: &str, name: &str, file: &Path) -> Result<RemoteFile, ServiceError> {
        let bytes = fs::read(file).map_err(|e| ServiceError::other(format!("could not read the file ({})", e.kind())))?;
        let boundary = format!("dpm-boundary-{:016x}", SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64));
        let mut body = Vec::with_capacity(bytes.len() + 512);
        body.extend_from_slice(format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n").as_bytes());
        body.extend_from_slice(&GoogleDrive::metadata_body(name, parent));
        body.extend_from_slice(format!("\r\n--{boundary}\r\nContent-Type: application/zip\r\n\r\n").as_bytes());
        body.extend_from_slice(&bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let url = format!("{}/files?uploadType=multipart&fields={}", self.urls.drive_upload, super::net::url_escape(FILE_FIELDS));
        let content_type = format!("multipart/related; boundary={boundary}");
        let reply = self.call(&|t| Request::post(url.clone()).bearer(t).header("Content-Type", content_type.clone()).bytes(body.clone()))?;
        GoogleDrive::uploaded(&reply)
    }

    fn uploaded(reply: &Reply) -> Result<RemoteFile, ServiceError> {
        let json = GoogleDrive::json_of(reply)?;
        if json.get("id").and_then(Value::as_str).is_none() {
            return Err(ServiceError::other("Google Drive did not report the new file"));
        }
        Ok(remote_file(&json, "", String::new()))
    }

    /// Starts a resumable upload and gives back its session address. The address acts like a key: it is never logged.
    fn start_session(&self, parent: &str, name: &str, size: u64) -> Result<String, ServiceError> {
        let url = format!("{}/files?uploadType=resumable&fields={}", self.urls.drive_upload, super::net::url_escape(FILE_FIELDS));
        let body = GoogleDrive::metadata_body(name, parent);
        let reply = self.call(&|t| {
            Request::post(url.clone())
                .bearer(t)
                .header("Content-Type", "application/json; charset=UTF-8")
                .header("X-Upload-Content-Type", "application/zip")
                .header("X-Upload-Content-Length", size.to_string())
                .bytes(body.clone())
        })?;
        reply.header("location").map(str::to_string).filter(|l| !l.is_empty()).ok_or_else(|| ServiceError::other("Google Drive did not start the upload"))
    }

    /// Asks the session how much it has: `Ok(None)` when it has everything (the answer is the finished file),
    /// `Ok(Some(next))` with the next byte it wants, or an error.
    fn session_status(&self, session: &str, size: u64) -> Result<Result<u64, RemoteFile>, ServiceError> {
        let reply = self.net.send(Request::new("PUT", session).header("Content-Range", format!("bytes */{size}")).bytes(Vec::new()))?;
        match reply.status {
            308 => Ok(Ok(next_byte(&reply))),
            200 | 201 => Ok(Err(GoogleDrive::uploaded(&reply)?)),
            _ => Err(failure(&reply)),
        }
    }

    fn upload_resumable(&self, parent: &str, name: &str, file: &Path, size: u64, progress: Progress) -> Result<RemoteFile, ServiceError> {
        let mut input = fs::File::open(file).map_err(|e| ServiceError::other(format!("could not read the file ({})", e.kind())))?;
        let mut session = self.start_session(parent, name, size)?;
        let mut at = 0u64;
        let mut restarts = 0;
        // Tries in a row that moved nothing forward; any progress starts the count again.
        let mut failures = 0usize;
        let mut last = ServiceError::new(ErrorKind::Offline, "the upload was interrupted");
        while at < size {
            let end = (at + self.chunk).min(size) - 1;
            let mut piece = vec![0u8; (end - at + 1) as usize];
            input.seek(SeekFrom::Start(at)).and_then(|_| input.read_exact(&mut piece)).map_err(|e| ServiceError::other(format!("could not read the file ({})", e.kind())))?;
            let before = at;
            let mut retry = false;
            match self.net.send(Request::new("PUT", session.clone()).header("Content-Range", format!("bytes {at}-{end}/{size}")).bytes(piece)) {
                Ok(reply) if reply.status == 308 && end + 1 < size => at = next_byte(&reply).max(at),
                Ok(reply) if (reply.status == 200 || reply.status == 201) && end + 1 == size => return GoogleDrive::uploaded(&reply),
                // The session is gone (it lasts a week): start the whole upload again, once.
                Ok(reply) if matches!(reply.status, 404 | 410) && restarts == 0 => {
                    restarts += 1;
                    session = self.start_session(parent, name, size)?;
                    at = 0;
                    continue;
                }
                // The service is busy or the connection dropped, or the last piece was not taken whole: look again.
                Ok(reply) if reply.status >= 500 || reply.status == 429 => {
                    last = failure(&reply);
                    retry = true;
                }
                Ok(reply) if reply.status == 308 => retry = true,
                Ok(reply) => return Err(failure(&reply)),
                Err(e) if e.kind == ErrorKind::Offline => {
                    last = e;
                    retry = true;
                }
                Err(e) => return Err(e),
            }
            if retry {
                failures += 1;
                if failures >= PIECE_TRIES {
                    return Err(last);
                }
                (self.sleep)(Duration::from_secs(2 * failures as u64));
                // Ask where the upload stands: the piece may have arrived although the answer did not.
                match self.session_status(&session, size) {
                    Ok(Ok(next)) => at = next.min(size),
                    Ok(Err(done)) => return Ok(done),
                    Err(e) if e.kind.retries_by_itself() => {
                        last = e;
                        continue;
                    }
                    Err(e) => return Err(e),
                }
            }
            if at > before {
                failures = 0;
                progress(at, size);
            } else if !retry {
                // A server that keeps saying "send it again" without taking any of it must not loop for ever.
                failures += 1;
                if failures >= PIECE_TRIES {
                    return Err(ServiceError::new(ErrorKind::ServiceDown, "Google Drive kept asking for the same part"));
                }
            }
        }
        // Everything was sent but the last answer was lost: the session knows.
        match self.session_status(&session, size)? {
            Err(done) => Ok(done),
            Ok(_) => Err(ServiceError::other("Google Drive did not finish the upload")),
        }
    }
}

fn next_byte(reply: &Reply) -> u64 {
    // `Range: bytes=0-8388607` means the first 8 MiB arrived; no Range at all means nothing did.
    reply.header("range").and_then(|r| r.rsplit('-').next()).and_then(|n| n.trim().parse::<u64>().ok()).map_or(0, |last| last + 1)
}

/// Google's own code word and HTTP status turned into a failure kind.
fn failure(reply: &Reply) -> ServiceError {
    let reason = reason_of(reply);
    let reason = reason.as_deref().map(|r| if r == "dailyLimitExceeded" || r == "quotaExceeded" { "rateLimitExceeded" } else { r });
    classify_status(reply.status, reason, reply.retry_after())
}

fn remote_file(item: &Value, device: &str, path: String) -> RemoteFile {
    RemoteFile {
        device: device.to_string(),
        path,
        size: item.get("size").and_then(Value::as_str).and_then(|s| s.parse().ok()).or_else(|| item.get("size").and_then(Value::as_u64)).unwrap_or(0),
        sha256: item.get("sha256Checksum").and_then(Value::as_str).map(str::to_lowercase),
        id: item.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
        modified: item.get("modifiedTime").and_then(Value::as_str).and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|t| t.timestamp().max(0) as u64),
    }
}

impl CloudService for GoogleDrive {
    fn id(&self) -> ServiceId {
        ServiceId::Google
    }

    fn list_all(&mut self) -> Result<Vec<RemoteFile>, ServiceError> {
        let Some(root) = self.folder(&[ROOT_FOLDER], false)? else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        for deck in self.children(&root)? {
            let (Some(name), Some(id)) = (deck.get("name").and_then(Value::as_str), deck.get("id").and_then(Value::as_str)) else { continue };
            if deck.get("mimeType").and_then(Value::as_str) != Some(FOLDER_TYPE) {
                continue;
            }
            let Some(saves) = self.children(id)?.into_iter().find(|c| c.get("name").and_then(Value::as_str) == Some(SAVES_FOLDER) && c.get("mimeType").and_then(Value::as_str) == Some(FOLDER_TYPE)) else { continue };
            if let Some(saves_id) = saves.get("id").and_then(Value::as_str) {
                self.walk(saves_id, "", name, 1, &mut out)?;
            }
        }
        Ok(out)
    }

    fn upload(&mut self, device: &str, rel_path: &str, file: &Path, size: u64, progress: Progress) -> Result<RemoteFile, ServiceError> {
        let mut pieces: Vec<&str> = rel_path.split('/').collect();
        let name = pieces.pop().filter(|n| !n.is_empty()).ok_or_else(|| ServiceError::other("an upload needs a file name"))?;
        let mut path = vec![ROOT_FOLDER, device, SAVES_FOLDER];
        path.extend(pieces.iter().copied());
        let parent = self.folder(&path, true)?.ok_or_else(|| ServiceError::other("could not make the cloud folder"))?;
        let mut done = if size <= self.simple_limit { self.upload_simple(&parent, name, file)? } else { self.upload_resumable(&parent, name, file, size, progress)? };
        progress(size, size);
        done.device = device.to_string();
        done.path = rel_path.to_string();
        Ok(done)
    }

    fn download(&mut self, file: &RemoteFile, dest: &Path, progress: Progress) -> Result<(), ServiceError> {
        let url = format!("{}/files/{}?alt=media", self.urls.drive_api, super::net::url_escape(&file.id));
        let mut forced = false;
        loop {
            let token = self.auth.token(forced)?;
            let mut out = fs::OpenOptions::new().write(true).create_new(true).open(dest).map_err(|e| ServiceError::other(format!("could not write the file ({})", e.kind())))?;
            // Drive says how big the file is: more bytes than that are never written.
            let outcome = self.net.download(Request::get(url.clone()).bearer(&token), &mut out, progress, Some(file.size).filter(|s| *s > 0));
            drop(out);
            match outcome {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(reply)) if reply.status == 401 && !forced => {
                    forced = true;
                    let _ = fs::remove_file(dest);
                }
                Ok(Err(reply)) => {
                    let _ = fs::remove_file(dest);
                    return Err(failure(&reply));
                }
                Err(e) => {
                    let _ = fs::remove_file(dest);
                    return Err(e);
                }
            }
        }
    }
}
