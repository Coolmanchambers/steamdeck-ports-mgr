//! What a cloud service has to offer the rest of the app, and the one set of failure kinds every service
//! maps its errors onto. Google Drive is the only service built (`google.rs`); a second one (OneDrive) is
//! another implementation of [`CloudService`] plus a [`ServiceId`] entry, nothing else changes.
//!
//! There is deliberately no way to delete anything through this trait: the app uploads, lists and downloads.

use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ServiceId {
    Google,
    /// Tests only: a second service, to prove nothing assumes there is just one.
    #[cfg(test)]
    Second,
}

impl ServiceId {
    #[cfg(not(test))]
    pub const ALL: [ServiceId; 1] = [ServiceId::Google];
    #[cfg(test)]
    pub const ALL: [ServiceId; 2] = [ServiceId::Google, ServiceId::Second];

    /// The key in `cloud-tokens.json` and `cloud-backup.json`.
    pub fn key(self) -> &'static str {
        match self {
            ServiceId::Google => "google",
            #[cfg(test)]
            ServiceId::Second => "second",
        }
    }

    pub fn from_key(key: &str) -> Option<ServiceId> {
        ServiceId::ALL.into_iter().find(|s| s.key() == key)
    }

    /// The name the screens use.
    pub fn name(self) -> &'static str {
        match self {
            ServiceId::Google => "Google Drive",
            #[cfg(test)]
            ServiceId::Second => "Second Cloud",
        }
    }

    /// The address a person types on a phone to enter the sign-in code.
    pub fn device_page(self) -> &'static str {
        match self {
            ServiceId::Google => "google.com/device",
            #[cfg(test)]
            ServiceId::Second => "second.example/device",
        }
    }
}

/// Why a call failed, in the terms the screens care about. The app keys on these, never on a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// No connection, or the server did not answer in time.
    Offline,
    /// The token was revoked or expired for good: signing in again is the only fix.
    SignedOut,
    /// The account has no room left.
    StorageFull,
    /// The service asked us to slow down.
    RateLimited,
    /// A 5xx answer.
    ServiceDown,
    /// The Deck's clock is so far off that sign-in or TLS fails.
    ClockWrong,
    /// Anything else (a refusal, an answer we could not read).
    Other,
}

impl ErrorKind {
    /// The word stored in `cloud-backup.json`.
    pub fn code(self) -> &'static str {
        match self {
            ErrorKind::Offline => "offline",
            ErrorKind::SignedOut => "signed_out",
            ErrorKind::StorageFull => "storage_full",
            ErrorKind::RateLimited => "rate_limited",
            ErrorKind::ServiceDown => "service_down",
            ErrorKind::ClockWrong => "clock_wrong",
            ErrorKind::Other => "other",
        }
    }

    pub fn from_code(code: &str) -> Option<ErrorKind> {
        [ErrorKind::Offline, ErrorKind::SignedOut, ErrorKind::StorageFull, ErrorKind::RateLimited, ErrorKind::ServiceDown, ErrorKind::ClockWrong, ErrorKind::Other]
            .into_iter()
            .find(|k| k.code() == code)
    }

    /// True when trying again later can help without the person doing anything.
    pub fn retries_by_itself(self) -> bool {
        matches!(self, ErrorKind::Offline | ErrorKind::RateLimited | ErrorKind::ServiceDown | ErrorKind::Other)
    }
}

/// A failure. `detail` is for the log and the "details" line: it never holds a token or a link that acts
/// like one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceError {
    pub kind: ErrorKind,
    pub detail: String,
    /// Seconds the service asked us to wait (`Retry-After`).
    pub retry_after: Option<u64>,
}

impl ServiceError {
    pub fn new(kind: ErrorKind, detail: impl Into<String>) -> ServiceError {
        ServiceError { kind, detail: detail.into(), retry_after: None }
    }

    pub fn other(detail: impl Into<String>) -> ServiceError {
        ServiceError::new(ErrorKind::Other, detail)
    }
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.detail, self.kind.code())
    }
}

/// A file in the cloud copy: `device` is the Deck's folder name, `path` is relative to that Deck's
/// `Saves Backup` folder (`Global Backups/2026-10-24.zip`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFile {
    pub device: String,
    pub path: String,
    pub size: u64,
    pub sha256: Option<String>,
    /// The service's own handle for the file (opaque).
    pub id: String,
    /// Seconds since 1970, when the service says the file changed.
    pub modified: Option<u64>,
}

/// Progress of one transfer: bytes done, bytes in all.
pub type Progress<'a> = &'a mut dyn FnMut(u64, u64);

pub trait CloudService {
    fn id(&self) -> ServiceId;

    /// Every backup file in the cloud copy, for every Deck that has a folder there.
    fn list_all(&mut self) -> Result<Vec<RemoteFile>, ServiceError>;

    /// Copies `file` to `<device>/Saves Backup/<rel_path>`, making the folders it needs, and returns the file
    /// as the service reports it. It never replaces a file: the planner chose a name that is not taken.
    fn upload(&mut self, device: &str, rel_path: &str, file: &Path, size: u64, progress: Progress) -> Result<RemoteFile, ServiceError>;

    /// Writes the cloud file to `dest` (a path that does not exist yet).
    fn download(&mut self, file: &RemoteFile, dest: &Path, progress: Progress) -> Result<(), ServiceError>;
}
