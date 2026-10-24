//! The one place the cloud code writes diagnostics. A line carries the service, an HTTP status, an error
//! code, a file name and a size, never a token, a device code, an upload-session link, an Authorization
//! header or an account email (`redact` removes known secrets from text that came from a server).

use std::sync::Mutex;

#[cfg(test)]
thread_local! {
    static CAPTURED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Secrets this process has held (tokens, the account email): every line `note` writes is cleaned of them,
/// whatever text it was built from.
static SECRETS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Remembers values to hide from every later log line (short values and repeats are ignored; the oldest go first).
pub fn register_secrets(values: &[String]) {
    let mut known = SECRETS.lock().unwrap_or_else(|e| e.into_inner());
    for v in values.iter().filter(|v| v.len() >= 4) {
        if !known.contains(v) {
            known.push(v.clone());
        }
    }
    let extra = known.len().saturating_sub(32);
    known.drain(..extra);
}

/// Text that is safe to show or write: control characters gone, any web address replaced (a resumable upload's
/// session address acts like a key), and no longer than a short line.
pub fn scrub(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("http://").into_iter().chain(rest.find("https://")).min() {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let end = tail.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | '>' | ',')).unwrap_or(tail.len());
        out.push_str("[address]");
        rest = &tail[end..];
    }
    out.push_str(rest);
    let cleaned: String = out.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    if cleaned.chars().count() > 240 {
        let cut: String = cleaned.chars().take(240).collect();
        format!("{cut}...")
    } else {
        cleaned
    }
}

pub fn note(line: impl AsRef<str>) {
    let known = SECRETS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let refs: Vec<&str> = known.iter().map(String::as_str).collect();
    let line = redact(&scrub(line.as_ref()), &refs);
    #[cfg(test)]
    CAPTURED.with(|c| c.borrow_mut().push(line.clone()));
    eprintln!("cloud backup: {line}");
}

/// What `note` wrote on this thread since the last call (tests only).
#[cfg(test)]
pub fn take_captured() -> Vec<String> {
    CAPTURED.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

/// `text` with every non-empty secret replaced by `[hidden]`.
pub fn redact(text: &str, secrets: &[&str]) -> String {
    let mut out = text.to_string();
    for secret in secrets.iter().filter(|s| s.len() >= 4) {
        out = out.replace(secret, "[hidden]");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_addresses_and_control_characters_never_get_through() {
        assert_eq!(scrub("could not send to https://www.googleapis.com/upload/session/x?upload_id=SECRET-KEY (reset)"), "could not send to [address] (reset)");
        assert_eq!(scrub("a http://127.0.0.1:1/x and \"https://b.example/y\""), "a [address] and \"[address]\"");
        assert_eq!(scrub("line\nbreak\u{7}"), "line break ");
        assert!(scrub(&"x".repeat(1000)).chars().count() <= 243);
        assert_eq!(scrub("nothing to hide"), "nothing to hide");
    }

    #[test]
    fn a_registered_secret_is_hidden_from_every_note() {
        register_secrets(&["TOKEN-ABCDEFGH".to_string(), "ab".to_string()]);
        take_captured();
        note("the server said TOKEN-ABCDEFGH is bad ab");
        let lines = take_captured();
        assert_eq!(lines, ["the server said [hidden] is bad ab"]);
    }

    #[test]
    fn known_secrets_are_hidden_and_short_values_are_left_alone() {
        assert_eq!(redact("bad token ya29.AAAA here", &["ya29.AAAA"]), "bad token [hidden] here");
        assert_eq!(redact("a b", &["a"]), "a b");
    }
}
