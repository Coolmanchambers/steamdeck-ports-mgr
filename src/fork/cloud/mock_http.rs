//! A small HTTP server on 127.0.0.1 for the cloud tests: it answers each request with whatever the test's
//! handler says and writes every request down, so a test can check the exact calls (and that none was a
//! delete). One request per connection, `Connection: close`.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    /// Path and query, as sent.
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }

    pub fn query(&self, key: &str) -> Option<String> {
        let q = self.target.split_once('?')?.1;
        q.split('&').filter_map(|kv| kv.split_once('=')).find(|(k, _)| *k == key).map(|(_, v)| crate::core::http::decode_url(&v.replace('+', " ")))
    }

    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The value of `key` in a form body.
    pub fn form(&self, key: &str) -> Option<String> {
        self.body_text().split('&').filter_map(|kv| kv.split_once('=')).find(|(k, _)| *k == key).map(|(_, v)| crate::core::http::decode_url(&v.replace('+', " ")))
    }
}

pub struct Answer {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Close the connection without answering (a lost reply).
    pub hang_up: bool,
}

impl Answer {
    pub fn json(status: u16, body: &str) -> Answer {
        Answer { status, headers: vec![("Content-Type".into(), "application/json".into())], body: body.as_bytes().to_vec(), hang_up: false }
    }

    pub fn bytes(status: u16, body: Vec<u8>) -> Answer {
        Answer { status, headers: Vec::new(), body, hang_up: false }
    }

    pub fn hang_up() -> Answer {
        Answer { status: 0, headers: Vec::new(), body: Vec::new(), hang_up: true }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Answer {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
}

type Handler = Box<dyn FnMut(&Seen) -> Answer + Send>;

pub struct Mock {
    pub base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    port: u16,
    thread: Option<JoinHandle<()>>,
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        308 => "Resume Incomplete",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        428 => "Precondition Required",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        507 => "Insufficient Storage",
        _ => "Status",
    }
}

fn read_request(stream: &mut TcpStream) -> Option<Seen> {
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    let mut data = Vec::new();
    let mut buf = [0u8; 8192];
    let head_end = loop {
        if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        data.extend_from_slice(&buf[..n]);
    };
    let head = String::from_utf8_lossy(&data[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, target) = (first.next()?.to_string(), first.next()?.to_string());
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
    let len = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse::<usize>().ok()).unwrap_or(0);
    let mut body = data[head_end + 4..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
    }
    body.truncate(len);
    Some(Seen { method, target, headers, body })
}

impl Mock {
    pub fn start(mut handler: impl FnMut(&Seen) -> Answer + Send + 'static) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen_t, stop_t) = (seen.clone(), stop.clone());
        let mut handler: Handler = Box::new(move |s| handler(s));
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stop_t.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut stream) = stream else { continue };
                let Some(request) = read_request(&mut stream) else { continue };
                let answer = handler(&request);
                seen_t.lock().unwrap().push(request);
                if answer.hang_up {
                    continue;
                }
                let mut out = format!("HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n", answer.status, reason(answer.status), answer.body.len());
                for (k, v) in &answer.headers {
                    out.push_str(&format!("{k}: {v}\r\n"));
                }
                out.push_str("\r\n");
                let _ = stream.write_all(out.as_bytes());
                let _ = stream.write_all(&answer.body);
                let _ = stream.flush();
            }
        });
        Mock { base: format!("http://127.0.0.1:{port}"), seen, stop, port, thread: Some(thread) }
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
