//! Google Drive against a mock of the parts of the Drive API the app uses (folders, multipart and resumable
//! uploads, listing with pages, downloads), with faults: dropped connections, 5xx, an expired session, a 401.

use super::endpoints::{Urls, GOOGLE_SCOPE};
use super::google::{GoogleDrive, TokenSource, ROOT_FOLDER, SAVES_FOLDER};
use super::mock_http::{Answer, Mock, Seen};
use super::net::Net;
use super::oauth::{OAuthConfig, Tokens};
use super::service::{CloudService, ErrorKind, RemoteFile, ServiceId};
use super::sha256::{sha256_hex, file_sha256};
use super::store;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const GOOD: &str = "GOOD-TOKEN-0123456789";
const FOLDER: &str = "application/vnd.google-apps.folder";

#[derive(Clone)]
struct Item {
    id: String,
    name: String,
    parent: String,
    folder: bool,
    bytes: Vec<u8>,
    created: u32,
}

struct Session {
    name: String,
    parent: String,
    total: usize,
    got: Vec<u8>,
}

enum Fault {
    /// Answer this status without taking the piece.
    Status(u16),
    /// Take the piece, then drop the connection before answering.
    LostAnswer,
}

#[derive(Default)]
struct Drive {
    next: u32,
    items: Vec<Item>,
    sessions: HashMap<String, Session>,
    faults: VecDeque<Fault>,
    /// The next API calls answer 401 this many times.
    unauthorized: usize,
    /// The next resumable pieces and status questions answer 401 this many times.
    unauthorized_pieces: usize,
    /// The next resumable session lookup answers 404 once.
    expire_session: bool,
    /// Answer every call with this status and reason (a Drive error).
    refuse: Option<(u16, &'static str)>,
    page_size: Option<usize>,
    refreshes: usize,
}

type Shared = Arc<Mutex<Drive>>;

fn unquote(q: &str, key: &str) -> Option<String> {
    let at = q.find(&format!("{key}='"))? + key.len() + 2;
    let mut out = String::new();
    let mut chars = q[at..].chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push(chars.next()?),
            '\'' => return Some(out),
            other => out.push(other),
        }
    }
    None
}

fn item_json(i: &Item) -> String {
    if i.folder {
        format!(r#"{{"id":"{}","name":{},"mimeType":"{FOLDER}","createdTime":"2026-10-24T10:00:{:02}Z"}}"#, i.id, serde_json::to_string(&i.name).unwrap(), i.created % 60)
    } else {
        format!(
            r#"{{"id":"{}","name":{},"mimeType":"application/zip","size":"{}","sha256Checksum":"{}","modifiedTime":"2026-10-24T10:00:00.000Z"}}"#,
            i.id,
            serde_json::to_string(&i.name).unwrap(),
            i.bytes.len(),
            sha256_hex(&i.bytes)
        )
    }
}

impl Drive {
    fn add(&mut self, name: &str, parent: &str, folder: bool, bytes: Vec<u8>) -> Item {
        self.next += 1;
        let item = Item { id: format!("id{}", self.next), name: name.to_string(), parent: parent.to_string(), folder, bytes, created: self.next };
        self.items.push(item.clone());
        item
    }

    /// Makes `Deck Ports Manager/<device>/Saves Backup/<dirs...>` and a file in it, as an earlier run would have.
    fn seed(&mut self, device: &str, rel: &str, bytes: &[u8]) {
        let mut parent = "root".to_string();
        let mut pieces = vec![ROOT_FOLDER, device, SAVES_FOLDER];
        let mut parts: Vec<&str> = rel.split('/').collect();
        let name = parts.pop().unwrap();
        pieces.extend(parts);
        for p in pieces {
            parent = match self.items.iter().find(|i| i.folder && i.name == p && i.parent == parent) {
                Some(f) => f.id.clone(),
                None => self.add(p, &parent, true, Vec::new()).id,
            };
        }
        self.add(name, &parent, false, bytes.to_vec());
    }

    fn files_of(&self, device: &str) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let root = self.items.iter().find(|i| i.folder && i.name == ROOT_FOLDER && i.parent == "root");
        let Some(root) = root else { return out };
        let Some(deck) = self.items.iter().find(|i| i.folder && i.name == device && i.parent == root.id) else { return out };
        fn walk(d: &Drive, folder: &str, prefix: &str, out: &mut Vec<(String, Vec<u8>)>) {
            for i in d.items.iter().filter(|i| i.parent == folder) {
                let path = if prefix.is_empty() { i.name.clone() } else { format!("{prefix}/{}", i.name) };
                if i.folder {
                    walk(d, &i.id, &path, out);
                } else {
                    out.push((path, i.bytes.clone()));
                }
            }
        }
        if let Some(saves) = self.items.iter().find(|i| i.folder && i.name == SAVES_FOLDER && i.parent == deck.id) {
            walk(self, &saves.id, "", &mut out);
        }
        out.sort();
        out
    }

    fn handle(&mut self, r: &Seen) -> Answer {
        let path = r.path().to_string();
        if r.method == "DELETE" || r.method == "PATCH" {
            return Answer::json(405, "{}");
        }
        if path == "/token" {
            self.refreshes += 1;
            return Answer::json(200, &format!(r#"{{"access_token":"{GOOD}","expires_in":3600,"token_type":"Bearer"}}"#));
        }
        let is_session = path.starts_with("/upload/session/");
        if is_session && self.unauthorized_pieces > 0 {
            self.unauthorized_pieces -= 1;
            return Answer::json(401, r#"{"error":{"code":401,"errors":[{"reason":"authError"}]}}"#);
        }
        {
            if !is_session && self.unauthorized > 0 {
                self.unauthorized -= 1;
                return Answer::json(401, r#"{"error":{"code":401,"message":"Invalid Credentials","errors":[{"reason":"authError"}]}}"#);
            }
            if r.header("authorization") != Some(&format!("Bearer {GOOD}")) {
                return Answer::json(401, r#"{"error":{"code":401,"errors":[{"reason":"authError"}]}}"#);
            }
            if let (false, Some((status, reason))) = (is_session, self.refuse) {
                return Answer::json(status, &format!(r#"{{"error":{{"code":{status},"errors":[{{"reason":"{reason}"}}]}}}}"#)).with_header("Retry-After", "12");
            }
        }
        match (r.method.as_str(), path.as_str()) {
            ("GET", "/drive/v3/files") => self.list(r),
            ("POST", "/drive/v3/files") => {
                let meta: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
                let parent = meta["parents"][0].as_str().unwrap().to_string();
                let item = self.add(meta["name"].as_str().unwrap(), &parent, meta["mimeType"] == FOLDER, Vec::new());
                Answer::json(200, &format!(r#"{{"id":"{}"}}"#, item.id))
            }
            ("POST", "/upload/drive/v3/files") if r.query("uploadType").as_deref() == Some("multipart") => self.multipart(r),
            ("POST", "/upload/drive/v3/files") if r.query("uploadType").as_deref() == Some("resumable") => {
                let meta: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
                let total: usize = r.header("x-upload-content-length").unwrap().parse().unwrap();
                self.next += 1;
                let id = format!("s{}", self.next);
                self.sessions.insert(id.clone(), Session { name: meta["name"].as_str().unwrap().into(), parent: meta["parents"][0].as_str().unwrap().into(), total, got: Vec::new() });
                let host = r.header("host").unwrap();
                Answer::json(200, "{}").with_header("Location", &format!("http://{host}/upload/session/{id}?upload_id=SECRET-SESSION-KEY"))
            }
            ("PUT", p) if p.starts_with("/upload/session/") => self.piece(r, p.trim_start_matches("/upload/session/")),
            ("GET", p) if p.starts_with("/drive/v3/files/") && r.query("alt").as_deref() == Some("media") => {
                let id = p.trim_start_matches("/drive/v3/files/");
                match self.items.iter().find(|i| i.id == id && !i.folder) {
                    Some(i) => Answer::bytes(200, i.bytes.clone()),
                    None => Answer::json(404, r#"{"error":{"code":404,"errors":[{"reason":"notFound"}]}}"#),
                }
            }
            _ => Answer::json(404, "{}"),
        }
    }

    fn list(&self, r: &Seen) -> Answer {
        let q = r.query("q").unwrap_or_default();
        let parent = q.find("' in parents").and_then(|end| q[..end].rfind(char::from(39u8)).map(|start| q[start + 1..end].to_string())).unwrap_or_default();
        let name = unquote(&q, "name");
        let want_folder = q.contains(&format!("mimeType='{FOLDER}'"));
        let mut found: Vec<&Item> = self.items.iter().filter(|i| i.parent == parent && name.as_ref().is_none_or(|n| &i.name == n) && (!want_folder || i.folder)).collect();
        if r.query("orderBy").as_deref() == Some("createdTime") {
            found.sort_by_key(|i| i.created);
        }
        let start: usize = r.query("pageToken").and_then(|t| t.parse().ok()).unwrap_or(0);
        let size = self.page_size.unwrap_or(1000);
        let page: Vec<String> = found.iter().skip(start).take(size).map(|i| item_json(i)).collect();
        let next = if start + size < found.len() { format!(r#","nextPageToken":"{}""#, start + size) } else { String::new() };
        Answer::json(200, &format!(r#"{{"files":[{}]{next}}}"#, page.join(",")))
    }

    fn multipart(&mut self, r: &Seen) -> Answer {
        let ct = r.header("content-type").unwrap();
        let boundary = ct.split("boundary=").nth(1).unwrap().to_string();
        let marker = format!("--{boundary}");
        let body = &r.body;
        let find = |from: usize, needle: &[u8]| body[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from);
        let first = find(0, marker.as_bytes()).unwrap();
        let meta_start = find(first, b"\r\n\r\n").unwrap() + 4;
        let second = find(meta_start, marker.as_bytes()).unwrap();
        let meta: serde_json::Value = serde_json::from_slice(&body[meta_start..second - 2]).unwrap();
        let file_start = find(second, b"\r\n\r\n").unwrap() + 4;
        let end = find(file_start, format!("\r\n{marker}--").as_bytes()).unwrap();
        let item = self.add(meta["name"].as_str().unwrap(), meta["parents"][0].as_str().unwrap(), false, body[file_start..end].to_vec());
        Answer::json(200, &item_json(&item))
    }

    fn piece(&mut self, r: &Seen, id: &str) -> Answer {
        if std::mem::take(&mut self.expire_session) {
            return Answer::json(404, "{}");
        }
        let range = r.header("content-range").unwrap().trim_start_matches("bytes ").to_string();
        let (span, total) = range.split_once('/').unwrap();
        let total: usize = total.parse().unwrap();
        if span != "*" {
            match self.faults.pop_front() {
                Some(Fault::Status(s)) => return Answer::json(s, "{}"),
                Some(Fault::LostAnswer) => {
                    self.take(id, span, r);
                    return Answer::hang_up();
                }
                None => {}
            }
            if let Some(bad) = self.take(id, span, r) {
                return bad;
            }
        }
        let s = &self.sessions[id];
        if s.got.len() == total {
            let (name, parent, bytes) = (s.name.clone(), s.parent.clone(), s.got.clone());
            let item = self.add(&name, &parent, false, bytes);
            return Answer::json(200, &item_json(&item));
        }
        let mut a = Answer { status: 308, headers: Vec::new(), body: Vec::new(), hang_up: false };
        if !s.got.is_empty() {
            a = a.with_header("Range", &format!("bytes=0-{}", s.got.len() - 1));
        }
        a
    }

    /// Adds a piece to its session; `Some(answer)` when the piece does not continue where the session is.
    fn take(&mut self, id: &str, span: &str, r: &Seen) -> Option<Answer> {
        let (a, _) = span.split_once('-').unwrap();
        let a: usize = a.parse().unwrap();
        let s = self.sessions.get_mut(id).unwrap();
        if a != s.got.len() {
            return Some(Answer::json(400, r#"{"error":{"errors":[{"reason":"badRequest"}]}}"#));
        }
        s.got.extend_from_slice(&r.body);
        None
    }
}

struct Rig {
    mock: Mock,
    drive: GoogleDrive,
    state: Shared,
    dir: PathBuf,
    tokens_path: PathBuf,
}

fn rig(name: &str, simple_limit: u64, chunk: u64) -> Rig {
    let state: Shared = Arc::new(Mutex::new(Drive::default()));
    let handler_state = state.clone();
    let mock = Mock::start(move |r: &Seen| handler_state.lock().unwrap().handle(r));
    let dir = std::env::temp_dir().join(format!("dpm_cloud_google_{}_{}", std::process::id(), name));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let (tokens_path, record_path) = (dir.join("t.json"), dir.join("r.json"));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    store::sign_in(&tokens_path, &record_path, ServiceId::Google, Tokens { access_token: GOOD.into(), expires_in: 100_000, refresh_token: Some("REFRESH-TOKEN-0123".into()), email: None }, now).unwrap();
    let urls = Urls::under(&mock.base);
    let cfg = OAuthConfig { device_code_url: urls.device_code.clone(), token_url: urls.token.clone(), client_id: "id".into(), client_secret: "secret".into(), scope: GOOGLE_SCOPE.into(), userinfo_url: urls.userinfo.clone() };
    let auth = TokenSource { net: Net::local_test(), cfg, tokens_path: tokens_path.clone(), id: ServiceId::Google };
    let drive = GoogleDrive::new(Net::local_test(), urls, auth).for_tests(simple_limit, chunk);
    Rig { mock, drive, state, dir, tokens_path }
}

fn file(dir: &std::path::Path, name: &str, len: usize) -> (PathBuf, Vec<u8>) {
    let bytes: Vec<u8> = (0..len).map(|i| ((i * 31 + len) % 251) as u8).collect();
    let path = dir.join(name);
    fs::write(&path, &bytes).unwrap();
    (path, bytes)
}

fn requests_to(rig: &Rig, method: &str, path: &str) -> usize {
    rig.mock.seen().iter().filter(|s| s.method == method && s.path() == path).count()
}

#[test]
fn a_small_file_goes_up_in_one_request_into_the_apps_own_folders() {
    let mut r = rig("small", 256 * 1024, 256 * 1024);
    let (path, bytes) = file(&r.dir, "a.zip", 100_000);
    let mut seen_progress = Vec::new();
    let up = r.drive.upload("deck", "Global Backups/a.zip", &path, 100_000, &mut |d, t| seen_progress.push((d, t))).unwrap();
    assert_eq!((up.size, up.device.as_str(), up.path.as_str()), (100_000, "deck", "Global Backups/a.zip"));
    assert_eq!(up.sha256.as_deref(), Some(sha256_hex(&bytes).as_str()));
    assert_eq!(seen_progress.last(), Some(&(100_000, 100_000)));
    let state = r.state.lock().unwrap();
    assert_eq!(state.files_of("deck"), vec![("Global Backups/a.zip".to_string(), bytes)]);
    drop(state);
    assert_eq!(requests_to(&r, "POST", "/upload/drive/v3/files"), 1, "one request");
    assert_eq!(requests_to(&r, "POST", "/drive/v3/files"), 4, "Deck Ports Manager, the Deck, Saves Backup, Global Backups");
    let upload = r.mock.seen().into_iter().find(|s| s.path() == "/upload/drive/v3/files").unwrap();
    assert_eq!(upload.query("uploadType").as_deref(), Some("multipart"));
    assert_eq!(upload.header("authorization"), Some(format!("Bearer {GOOD}").as_str()));
}

#[test]
fn folders_are_found_not_made_twice_and_are_remembered_within_a_run() {
    let mut r = rig("folders", 256 * 1024, 256 * 1024);
    let (p1, _) = file(&r.dir, "a.zip", 1_000);
    let (p2, _) = file(&r.dir, "b.zip", 2_000);
    r.drive.upload("deck", "Global Backups/a.zip", &p1, 1_000, &mut |_, _| {}).unwrap();
    r.drive.upload("deck", "Global Backups/b.zip", &p2, 2_000, &mut |_, _| {}).unwrap();
    assert_eq!(requests_to(&r, "POST", "/drive/v3/files"), 4, "the second upload reused the folders");
    // A new run (a new service object) finds what is there instead of making a second copy.
    let urls = Urls::under(&r.mock.base);
    let cfg = OAuthConfig { device_code_url: urls.device_code.clone(), token_url: urls.token.clone(), client_id: "id".into(), client_secret: "s".into(), scope: GOOGLE_SCOPE.into(), userinfo_url: urls.userinfo.clone() };
    let auth = TokenSource { net: Net::local_test(), cfg, tokens_path: r.tokens_path.clone(), id: ServiceId::Google };
    let mut again = GoogleDrive::new(Net::local_test(), urls, auth).for_tests(256 * 1024, 256 * 1024);
    let (p3, _) = file(&r.dir, "c.zip", 3_000);
    again.upload("deck", "Update Backups/soh/c.zip", &p3, 3_000, &mut |_, _| {}).unwrap();
    let state = r.state.lock().unwrap();
    assert_eq!(state.items.iter().filter(|i| i.folder && i.name == ROOT_FOLDER).count(), 1);
    assert_eq!(state.items.iter().filter(|i| i.folder && i.name == "Saves Backup").count(), 1);
    assert_eq!(state.files_of("deck").len(), 3);
}

#[test]
fn folder_names_with_quotes_are_escaped_in_the_search() {
    let mut r = rig("quote", 256 * 1024, 256 * 1024);
    let (p, _) = file(&r.dir, "a.zip", 500);
    r.drive.upload("Tim's Deck", "Global Backups/a.zip", &p, 500, &mut |_, _| {}).unwrap();
    r.drive.upload("Tim's Deck", "Global Backups/b.zip", &p, 500, &mut |_, _| {}).unwrap_or_else(|e| panic!("{e}"));
    let q: Vec<String> = r.mock.seen().iter().filter(|s| s.method == "GET").filter_map(|s| s.query("q")).collect();
    assert!(q.iter().any(|q| q.contains(r"name='Tim\'s Deck'")), "{q:?}");
}

#[test]
fn listing_walks_every_deck_and_folder_and_pages() {
    let mut r = rig("list", 256 * 1024, 256 * 1024);
    {
        let mut s = r.state.lock().unwrap();
        s.seed("deck", "Global Backups/2026-10-20.zip", b"one");
        s.seed("deck", "Update Backups/soh/20261021-101010.zip", b"two");
        s.seed("old deck", "Global Backups/2026-09-01.zip", b"three");
        for n in 0..5 {
            s.seed("deck", &format!("Restore Backups/soh/r{n}.zip"), b"x");
        }
        s.page_size = Some(2);
    }
    let all = r.drive.list_all().unwrap();
    let mut got: Vec<(String, String)> = all.iter().map(|f| (f.device.clone(), f.path.clone())).collect();
    got.sort();
    assert_eq!(got.len(), 8, "{got:?}");
    assert!(got.contains(&("deck".into(), "Update Backups/soh/20261021-101010.zip".into())));
    assert!(got.contains(&("old deck".into(), "Global Backups/2026-09-01.zip".into())));
    let one = all.iter().find(|f| f.path == "Global Backups/2026-10-20.zip").unwrap();
    assert_eq!((one.size, one.sha256.as_deref(), one.modified.is_some()), (3, Some(sha256_hex(b"one").as_str()), true));
    let asked: Vec<String> = r.mock.seen().iter().filter(|s| s.method == "GET" && s.path() == "/drive/v3/files").filter_map(|s| s.query("fields")).collect();
    assert!(asked.iter().any(|f| f.contains("sha256Checksum")), "the checksum is always asked for: {asked:?}");
}

#[test]
fn listing_an_empty_drive_makes_nothing() {
    let mut r = rig("empty", 256 * 1024, 256 * 1024);
    assert!(r.drive.list_all().unwrap().is_empty());
    assert_eq!(requests_to(&r, "POST", "/drive/v3/files"), 0, "listing never creates a folder");
}

#[test]
fn a_big_file_goes_up_in_pieces_by_a_resumable_session_with_the_token_on_every_piece() {
    let mut r = rig("resumable", 256 * 1024, 512 * 1024);
    let (path, bytes) = file(&r.dir, "big.zip", 1_300_000);
    let mut progress = Vec::new();
    let up = r.drive.upload("deck", "Update Backups/soh/big.zip", &path, 1_300_000, &mut |d, t| progress.push((d, t))).unwrap();
    assert_eq!(up.size, 1_300_000);
    assert_eq!(r.state.lock().unwrap().files_of("deck"), vec![("Update Backups/soh/big.zip".to_string(), bytes)]);
    let seen = r.mock.seen();
    let start = seen.iter().find(|s| s.query("uploadType").as_deref() == Some("resumable")).unwrap();
    assert_eq!(start.header("x-upload-content-length"), Some("1300000"));
    let pieces: Vec<&Seen> = seen.iter().filter(|s| s.method == "PUT").collect();
    let ranges: Vec<&str> = pieces.iter().map(|s| s.header("content-range").unwrap()).collect();
    assert_eq!(ranges, ["bytes 0-524287/1300000", "bytes 524288-1048575/1300000", "bytes 1048576-1299999/1300000"]);
    assert!(pieces.iter().all(|s| s.header("authorization") == Some(format!("Bearer {GOOD}").as_str())), "every piece and status question carries the token too");
    assert!(progress.windows(2).all(|w| w[0].0 <= w[1].0) && progress.last() == Some(&(1_300_000, 1_300_000)), "{progress:?}");
    assert_eq!(requests_to(&r, "POST", "/upload/drive/v3/files"), 1);
}

#[test]
fn a_busy_service_midway_is_retried_and_the_file_still_arrives_whole() {
    let mut r = rig("busy", 256 * 1024, 512 * 1024);
    r.state.lock().unwrap().faults = VecDeque::from([Fault::Status(503), Fault::Status(500)]);
    let (path, bytes) = file(&r.dir, "big.zip", 1_300_000);
    r.drive.upload("deck", "Global Backups/big.zip", &path, 1_300_000, &mut |_, _| {}).unwrap();
    assert_eq!(r.state.lock().unwrap().files_of("deck")[0].1, bytes);
}

#[test]
fn a_lost_answer_resumes_from_what_the_session_has_and_never_sends_a_piece_twice() {
    let mut r = rig("lost", 256 * 1024, 512 * 1024);
    // The 2nd piece arrives but its answer is lost: the app asks where the upload stands and carries on.
    r.state.lock().unwrap().faults = VecDeque::from([Fault::Status(503), Fault::LostAnswer]);
    let (path, bytes) = file(&r.dir, "big.zip", 1_300_000);
    // Piece 1 meets the 503, is sent again, and the lost answer comes with it; the stored bytes must still be exact.
    r.drive.upload("deck", "Global Backups/big.zip", &path, 1_300_000, &mut |_, _| {}).unwrap();
    assert_eq!(r.state.lock().unwrap().files_of("deck")[0].1, bytes, "exact bytes, no gap, no repeat");
    let status_queries = r.mock.seen().iter().filter(|s| s.method == "PUT" && s.header("content-range").is_some_and(|c| c.starts_with("bytes */"))).count();
    assert!(status_queries >= 1, "the session was asked where it stood");
}

#[test]
fn a_service_that_stays_busy_gives_up_with_a_plain_kind_and_makes_no_file() {
    let mut r = rig("down", 256 * 1024, 512 * 1024);
    r.state.lock().unwrap().faults = (0..20).map(|_| Fault::Status(503)).collect();
    let (path, _) = file(&r.dir, "big.zip", 1_300_000);
    let err = r.drive.upload("deck", "Global Backups/big.zip", &path, 1_300_000, &mut |_, _| {}).unwrap_err();
    assert_eq!(err.kind, ErrorKind::ServiceDown, "{err}");
    assert!(r.state.lock().unwrap().files_of("deck").is_empty());
}

#[test]
fn an_expired_session_starts_the_upload_again_once() {
    let mut r = rig("expired", 256 * 1024, 512 * 1024);
    r.state.lock().unwrap().expire_session = true;
    let (path, bytes) = file(&r.dir, "big.zip", 900_000);
    r.drive.upload("deck", "Global Backups/big.zip", &path, 900_000, &mut |_, _| {}).unwrap();
    assert_eq!(r.state.lock().unwrap().files_of("deck")[0].1, bytes);
    let starts = r.mock.seen().iter().filter(|s| s.query("uploadType").as_deref() == Some("resumable")).count();
    assert_eq!(starts, 2);
}

#[test]
fn a_401_makes_one_forced_refresh_and_one_more_try() {
    let mut r = rig("401", 256 * 1024, 256 * 1024);
    r.state.lock().unwrap().unauthorized = 1;
    assert!(r.drive.list_all().unwrap().is_empty(), "the call worked after the refresh");
    assert_eq!(r.state.lock().unwrap().refreshes, 1, "one forced refresh, though the stored token looked fresh");
    let gets: Vec<Seen> = r.mock.seen().into_iter().filter(|s| s.path() == "/drive/v3/files").collect();
    assert_eq!(gets.len(), 2);
}

#[test]
fn a_401_on_a_piece_goes_through_the_one_refresh_rule_too() {
    let mut r = rig("401piece", 256 * 1024, 512 * 1024);
    r.state.lock().unwrap().unauthorized_pieces = 1;
    let (path, bytes) = file(&r.dir, "big.zip", 900_000);
    r.drive.upload("deck", "Global Backups/big.zip", &path, 900_000, &mut |_, _| {}).unwrap();
    assert_eq!(r.state.lock().unwrap().files_of("deck")[0].1, bytes);
    assert_eq!(r.state.lock().unwrap().refreshes, 1, "one forced refresh for the piece's 401");
    // Two 401s in a row on pieces: signed out, and no file.
    let mut r = rig("401piece2", 256 * 1024, 512 * 1024);
    r.state.lock().unwrap().unauthorized_pieces = 5;
    let (path, _) = file(&r.dir, "big.zip", 900_000);
    let err = r.drive.upload("deck", "Global Backups/big.zip", &path, 900_000, &mut |_, _| {}).unwrap_err();
    assert_eq!(err.kind, ErrorKind::SignedOut, "{err}");
    assert!(r.state.lock().unwrap().files_of("deck").is_empty());
    assert_eq!(r.state.lock().unwrap().refreshes, 1);
}

#[test]
fn two_roots_in_drive_are_noticed_and_the_oldest_is_used() {
    let mut r = rig("tworoots", 256 * 1024, 256 * 1024);
    {
        let mut s = r.state.lock().unwrap();
        s.seed("deck", "Global Backups/old.zip", b"old");
        s.add(ROOT_FOLDER, "root", true, Vec::new());
    }
    super::log::take_captured();
    let all = r.drive.list_all().unwrap();
    assert_eq!(all.len(), 1, "the oldest root's files");
    let logged = super::log::take_captured().join("\n");
    assert!(logged.contains("more than one \"Deck Ports Manager\" folder"), "{logged}");
}

#[test]
fn a_401_after_the_refresh_is_signed_out() {
    let mut r = rig("401b", 256 * 1024, 256 * 1024);
    r.state.lock().unwrap().unauthorized = 5;
    let err = r.drive.list_all().unwrap_err();
    assert_eq!(err.kind, ErrorKind::SignedOut, "{err}");
    assert_eq!(r.state.lock().unwrap().refreshes, 1, "only one refresh is tried");
}

#[test]
fn drive_errors_are_told_apart_by_their_code_words() {
    let cases: [(u16, &str, ErrorKind); 5] = [
        (403, "storageQuotaExceeded", ErrorKind::StorageFull),
        (403, "rateLimitExceeded", ErrorKind::RateLimited),
        (403, "userRateLimitExceeded", ErrorKind::RateLimited),
        (403, "dailyLimitExceeded", ErrorKind::RateLimited),
        (403, "forbidden", ErrorKind::Other),
    ];
    for (status, reason, kind) in cases {
        let mut r = rig("errs", 256 * 1024, 256 * 1024);
        r.state.lock().unwrap().refuse = Some((status, reason));
        let err = r.drive.list_all().unwrap_err();
        assert_eq!(err.kind, kind, "{reason}: {err}");
        if kind == ErrorKind::RateLimited {
            assert_eq!(err.retry_after, Some(12), "Retry-After is honoured");
        }
    }
    let mut r = rig("errs5", 256 * 1024, 256 * 1024);
    r.state.lock().unwrap().refuse = Some((503, "backendError"));
    assert_eq!(r.drive.list_all().unwrap_err().kind, ErrorKind::ServiceDown);
    r.state.lock().unwrap().refuse = Some((429, "rateLimitExceeded"));
    assert_eq!(r.drive.list_all().unwrap_err().kind, ErrorKind::RateLimited);
}

#[test]
fn no_connection_is_offline() {
    let mut r = rig("offline", 256 * 1024, 256 * 1024);
    let dead = Mock::start(|_: &Seen| Answer::json(200, "{}"));
    let urls = Urls::under(&dead.base);
    drop(dead);
    let mut drive = GoogleDrive::new(Net::local_test(), urls, TokenSource { net: Net::local_test(), cfg: OAuthConfig { device_code_url: String::new(), token_url: String::new(), client_id: String::new(), client_secret: String::new(), scope: String::new(), userinfo_url: String::new() }, tokens_path: r.tokens_path.clone(), id: ServiceId::Google });
    assert_eq!(drive.list_all().unwrap_err().kind, ErrorKind::Offline);
    let _ = &mut r;
}

#[test]
fn a_download_streams_the_file_and_makes_a_new_one_only() {
    let mut r = rig("download", 256 * 1024, 256 * 1024);
    let bytes: Vec<u8> = (0..700_000u32).map(|i| (i % 253) as u8).collect();
    r.state.lock().unwrap().seed("deck", "Global Backups/a.zip", &bytes);
    let all = r.drive.list_all().unwrap();
    let dest = r.dir.join("out.zip");
    let mut last = (0, 0);
    r.drive.download(&all[0], &dest, &mut |d, t| last = (d, t)).unwrap();
    assert_eq!(fs::read(&dest).unwrap(), bytes);
    assert_eq!(last, (700_000, 700_000));
    assert_eq!(file_sha256(&dest).unwrap(), sha256_hex(&bytes));
    let again = r.drive.download(&all[0], &dest, &mut |_, _| {});
    assert!(again.is_err(), "an existing file is never written over");
    assert_eq!(fs::read(&dest).unwrap(), bytes);
}

#[test]
fn a_download_larger_than_the_size_drive_reported_is_stopped_and_leaves_nothing() {
    let mut r = rig("toobig", 256 * 1024, 256 * 1024);
    r.state.lock().unwrap().seed("deck", "Global Backups/a.zip", &vec![7u8; 50_000]);
    let all = r.drive.list_all().unwrap();
    // Drive said the file is 10 bytes; the body that arrives is 50,000: nothing of it is written.
    let liar = RemoteFile { size: 10, ..all[0].clone() };
    let dest = r.dir.join("liar.zip");
    let err = r.drive.download(&liar, &dest, &mut |_, _| {}).unwrap_err();
    assert!(err.detail.contains("larger than it should be"), "{err}");
    assert!(!dest.exists(), "the partial file is removed");
    // The right size still downloads.
    r.drive.download(&all[0], &r.dir.join("ok.zip"), &mut |_, _| {}).unwrap();
}

#[test]
fn no_upload_session_address_reaches_an_error_a_detail_or_a_log_line() {
    let mut r = rig("redact", 256 * 1024, 512 * 1024);
    r.state.lock().unwrap().faults = (0..20).map(|_| Fault::Status(503)).collect();
    super::log::take_captured();
    let (path, _) = file(&r.dir, "big.zip", 900_000);
    let err = r.drive.upload("deck", "Global Backups/big.zip", &path, 900_000, &mut |_, _| {}).unwrap_err();
    let shown = format!("{err} {err:?} {}", super::log::scrub(&err.detail));
    let logged = super::log::take_captured().join("\n");
    for text in [&shown, &logged] {
        assert!(!text.contains("SECRET-SESSION-KEY") && !text.contains("upload_id") && !text.contains("/upload/session/"), "{text}");
    }
}

#[test]
fn a_download_after_a_401_refreshes_once_and_a_missing_file_leaves_nothing() {
    let mut r = rig("download401", 256 * 1024, 256 * 1024);
    r.state.lock().unwrap().seed("deck", "Global Backups/a.zip", b"hello zip");
    let all = r.drive.list_all().unwrap();
    r.state.lock().unwrap().unauthorized = 1;
    let dest = r.dir.join("out.zip");
    r.drive.download(&all[0], &dest, &mut |_, _| {}).unwrap();
    assert_eq!(fs::read(&dest).unwrap(), b"hello zip");
    let gone = RemoteFile { id: "nope".into(), ..all[0].clone() };
    let dest2 = r.dir.join("gone.zip");
    let err = r.drive.download(&gone, &dest2, &mut |_, _| {}).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Other);
    assert!(!dest2.exists());
}

#[test]
fn nothing_the_service_does_ever_deletes_trashes_or_changes_a_file() {
    let mut r = rig("nodelete", 256 * 1024, 512 * 1024);
    let (small, _) = file(&r.dir, "s.zip", 5_000);
    let (big, _) = file(&r.dir, "b.zip", 900_000);
    r.drive.upload("deck", "Global Backups/s.zip", &small, 5_000, &mut |_, _| {}).unwrap();
    r.drive.upload("deck", "Update Backups/x/b.zip", &big, 900_000, &mut |_, _| {}).unwrap();
    let all = r.drive.list_all().unwrap();
    r.drive.download(&all[0], &r.dir.join("d.zip"), &mut |_, _| {}).unwrap();
    for s in r.mock.seen() {
        assert!(matches!(s.method.as_str(), "GET" | "POST" | "PUT"), "{} {}", s.method, s.path());
        // `trashed=false` in a search only leaves the bin out of the answer; nothing is ever put in it.
        assert!(!s.target.contains("trashed=true") && !s.target.to_lowercase().contains("/trash") && !s.body_text().contains("trashed") && !s.target.contains("delete"), "{}", s.path());
        // Only the app's own files and folders are named; the API is the Drive v3 one and nothing else.
        assert!(s.path().starts_with("/drive/v3/") || s.path().starts_with("/upload/drive/v3/") || s.path().starts_with("/upload/session/") || s.path() == "/token", "{}", s.path());
    }
    // Every PUT is a piece of a new file's session, never an update of an existing file.
    assert!(r.mock.seen().iter().filter(|s| s.method == "PUT").all(|s| s.path().starts_with("/upload/session/")));
}

#[test]
fn the_source_has_no_delete_trash_or_update_call() {
    let src = include_str!("google.rs");
    let code: String = src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    for forbidden in ["\"DELETE\"", "\"PATCH\"", "trashed\": true", "\"trashed\"", "/trash", "emptyTrash", "files.delete", "files.update", ".delete(", "remove_file(&path"] {
        // `fs::remove_file(dest)` of our own partial download is the one deletion there is, on the Deck.
        if forbidden == "remove_file(&path" {
            continue;
        }
        assert!(!code.contains(forbidden), "{forbidden} found in google.rs");
    }
    assert!(!code.contains("drive/v3/files/\" + ") && code.contains("alt=media"));
}

/// Needs a real sign-in; not run by default. Point `DPM_LIVE_CONFIG_DIR` at an app folder that holds a real
/// `cloud-tokens.json` and build with the real client in the environment.
#[test]
#[ignore]
fn live_list_with_a_real_sign_in() {
    let dir = PathBuf::from(std::env::var("DPM_LIVE_CONFIG_DIR").expect("DPM_LIVE_CONFIG_DIR"));
    let client = super::build::google_client().expect("the build needs DPM_GOOGLE_CLIENT_ID and DPM_GOOGLE_CLIENT_SECRET");
    let setup = super::endpoints::Setup { net: Net::production(), urls: Urls::google(), client };
    let mut drive = super::endpoints::service(&setup, &dir.join("cloud-tokens.json"), ServiceId::Google);
    let files = drive.list_all().expect("a listing");
    eprintln!("{} files", files.len());
}
