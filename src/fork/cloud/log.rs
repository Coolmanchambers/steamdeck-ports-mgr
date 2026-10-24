//! The one place the cloud code writes diagnostics. A line carries the service, an HTTP status, an error
//! code, a file name and a size, never a token, a device code, an upload-session link, an Authorization
//! header or an account email (`redact` removes known secrets from text that came from a server).

#[cfg(test)]
thread_local! {
    static CAPTURED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub fn note(line: impl AsRef<str>) {
    let line = line.as_ref();
    #[cfg(test)]
    CAPTURED.with(|c| c.borrow_mut().push(line.to_string()));
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
    fn known_secrets_are_hidden_and_short_values_are_left_alone() {
        assert_eq!(redact("bad token ya29.AAAA here", &["ya29.AAAA"]), "bad token [hidden] here");
        assert_eq!(redact("a b", &["a"]), "a b");
    }
}
