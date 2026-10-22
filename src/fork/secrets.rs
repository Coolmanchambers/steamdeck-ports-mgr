//! The user's API keys and logins for the online art sources, in `secrets.json` next to the app.
//!
//! * A file of its own, never `state.json`: bug reports ask users to send `state.json`, and the
//!   fork's other files (`fork-settings.json`, `rom_library.json`) are meant to be shareable too.
//! * Created with mode 0600 on Linux, written through a temp file that already has that mode and
//!   then renamed over the old one, so a reader never sees a half file and the key is never briefly
//!   world-readable. An existing file with wider permissions is tightened when it is loaded.
//! * Never logged: `Debug` hides every value, and the UI only shows [`mask`]ed text.
//! * Never exported: nothing outside this module reads the file, and no report or export in the app
//!   includes it.
//! * A file that is a symbolic link is not read or written (it could point anywhere); it counts as a
//!   problem, like a damaged file.
//! * Some drives cannot restrict a file at all (exFAT and FAT on an SD card report every file as open to
//!   everyone). That is detected after the write and shown as a note, never hidden.
//! * A file that exists but cannot be read as a JSON object is left alone: it loads as empty and
//!   saving is refused until the user fixes or removes it (the same rule as `fork-settings.json`).
//!
//! The developer credentials (ScreenScraper's devid and devpassword, an app TheGamesDB key) are not
//! stored here. They exist only if a build injects them, see [`BuildCreds`].

use std::cell::Cell;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

pub const FILE_NAME: &str = "secrets.json";

const KEY_SGDB: &str = "steamgriddb_api_key";
const KEY_TGDB: &str = "thegamesdb_api_key";
const KEY_SS_USER: &str = "screenscraper_ssid";
const KEY_SS_PASSWORD: &str = "screenscraper_sspassword";

/// Longest value accepted for any field; a real key or password is far shorter.
const MAX_VALUE_LEN: usize = 256;

/// Text for the UI: the last four characters of a value, the rest hidden. A value of eight or fewer
/// characters shows nothing of itself.
pub fn mask(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 8 {
        return "****".to_string();
    }
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("****{tail}")
}

/// Trims what a paste brings along (spaces, a newline, a pair of quotes). `None` when nothing is
/// left or the value is not plausible (control characters, longer than [`MAX_VALUE_LEN`]).
pub fn clean_value(raw: &str) -> Option<String> {
    let t = raw.trim().trim_matches(|c| c == '"' || c == '\'').trim();
    if t.is_empty() || t.chars().count() > MAX_VALUE_LEN || t.chars().any(char::is_control) {
        return None;
    }
    Some(t.to_string())
}

/// A pasted password: only the line break a paste brings along is removed, since quotes and spaces can be
/// part of a real password.
pub fn clean_password(raw: &str) -> Option<String> {
    let t = raw.trim_end_matches(['\r', '\n']);
    if t.trim().is_empty() || t.chars().count() > MAX_VALUE_LEN || t.chars().any(char::is_control) {
        return None;
    }
    Some(t.to_string())
}

/// True when a Unix mode gives nothing to group or others.
pub fn mode_is_private(mode: u32) -> bool {
    mode & 0o077 == 0
}


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    SteamGridDbKey,
    TheGamesDbKey,
    ScreenScraperUser,
    ScreenScraperPassword,
}

impl Field {
    fn json_key(self) -> &'static str {
        match self {
            Field::SteamGridDbKey => KEY_SGDB,
            Field::TheGamesDbKey => KEY_TGDB,
            Field::ScreenScraperUser => KEY_SS_USER,
            Field::ScreenScraperPassword => KEY_SS_PASSWORD,
        }
    }
}

pub struct Secrets {
    path: PathBuf,
    map: Map<String, Value>,
    /// False when the file on disk could not be read as an object; saving is then refused.
    writable: bool,
    /// Set when the file exists but could not be used, so the UI can say why nothing is configured.
    pub problem: Option<String>,
    /// After a save or a load: did the drive really keep the file private? `None`: not checked (no file yet,
    /// or not a Unix drive).
    restricted: Cell<Option<bool>>,
}

/// Values never appear in debug output, so a stray `{:?}` cannot leak them into a log.
impl fmt::Debug for Secrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Secrets")
            .field("path", &self.path)
            .field("steamgriddb", &self.sgdb_key().is_some())
            .field("thegamesdb", &self.tgdb_key().is_some())
            .field("screenscraper", &self.screenscraper_login().is_some())
            .finish()
    }
}

impl Secrets {
    /// An in-memory set with nowhere to save to; for tests and for callers that only have a key.
    pub fn in_memory() -> Secrets {
        Secrets { path: PathBuf::new(), map: Map::new(), writable: false, problem: None, restricted: Cell::new(None) }
    }

    /// Reads the file. A missing file is an empty set. On Linux a file readable by group or others is
    /// set back to 0600 on the way (best effort; a failure is reported in [`Secrets::problem`]).
    pub fn load(path: &Path) -> Secrets {
        if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            let problem = Some(format!("{FILE_NAME} is a link; it is not used or changed. Replace it with a plain file."));
            return Secrets { path: path.to_path_buf(), map: Map::new(), writable: false, problem, restricted: Cell::new(None) };
        }
        let mut problem = None;
        let restricted = Cell::new(None);
        let (map, writable) = match fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Value>(text.strip_prefix('\u{FEFF}').unwrap_or(&text)) {
                Ok(Value::Object(map)) => {
                    match tighten_permissions(path) {
                        Ok(private) => restricted.set(private),
                        Err(e) => problem = Some(format!("could not restrict {FILE_NAME} to your account: {e}")),
                    }
                    (map, true)
                }
                _ => {
                    problem = Some(format!("{FILE_NAME} is not a JSON object; fix or remove it. Keys in it are not used."));
                    (Map::new(), false)
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => (Map::new(), true),
            Err(e) => {
                problem = Some(format!("could not read {FILE_NAME}: {}", e.kind()));
                (Map::new(), false)
            }
        };
        Secrets { path: path.to_path_buf(), map, writable, problem, restricted }
    }

    /// A note for the screen when the drive cannot keep the file private (exFAT or FAT on an SD card).
    pub fn drive_note(&self) -> Option<&'static str> {
        (self.restricted.get() == Some(false)).then_some("This drive cannot restrict files, so other accounts on this device could read secrets.json.")
    }


    pub fn path(&self) -> &Path {
        &self.path
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
    }

    pub fn sgdb_key(&self) -> Option<&str> {
        self.get(KEY_SGDB)
    }

    pub fn tgdb_key(&self) -> Option<&str> {
        self.get(KEY_TGDB)
    }

    /// The user's ScreenScraper login, only when both halves are set.
    pub fn screenscraper_login(&self) -> Option<(&str, &str)> {
        Some((self.get(KEY_SS_USER)?, self.get(KEY_SS_PASSWORD)?))
    }

    pub fn screenscraper_user(&self) -> Option<&str> {
        self.get(KEY_SS_USER)
    }

    pub fn has_screenscraper_password(&self) -> bool {
        self.get(KEY_SS_PASSWORD).is_some()
    }

    /// Every stored value, for redacting them out of text that might otherwise carry one.
    pub fn all_values(&self) -> Vec<String> {
        [KEY_SGDB, KEY_TGDB, KEY_SS_USER, KEY_SS_PASSWORD].iter().filter_map(|k| self.get(k)).map(str::to_string).collect()
    }

    /// Sets (or, with `None`, removes) a field and saves. The value is cleaned first; a value that is
    /// not plausible is refused and nothing changes.
    pub fn set(&mut self, field: Field, value: Option<&str>) -> io::Result<()> {
        let key = field.json_key();
        match value {
            Some(raw) => {
                let cleaned = if field == Field::ScreenScraperPassword { clean_password(raw) } else { clean_value(raw) };
                let clean = cleaned.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "that does not look like a key or login (empty, too long, or has control characters)"))?;
                self.map.insert(key.to_string(), Value::String(clean));
            }
            None => {
                self.map.remove(key);
            }
        }
        self.save()
    }

    /// Writes the file: a temp file created with mode 0600, synced, then renamed over the old file.
    pub fn save(&self) -> io::Result<()> {
        if !self.writable {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{FILE_NAME} cannot be saved here; fix or remove the file first")));
        }
        if self.path.as_os_str().is_empty() {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "no file to save to"));
        }
        if fs::symlink_metadata(&self.path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{FILE_NAME} is a link; it is not changed")));
        }
        let text = serde_json::to_string_pretty(&self.map).map_err(io::Error::other)?;
        write_private(&self.path, text.as_bytes())?;
        self.restricted.set(file_is_private(&self.path));
        Ok(())
    }
}

#[cfg(unix)]
fn file_is_private(path: &Path) -> Option<bool> {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).ok().map(|m| mode_is_private(m.permissions().mode()))
}

#[cfg(not(unix))]
fn file_is_private(_path: &Path) -> Option<bool> {
    None
}

/// Sets the file to 0600 if it is wider, then checks it took: `Some(false)` means the drive keeps every
/// file open to everyone whatever is asked of it.
#[cfg(unix)]
fn tighten_permissions(path: &Path) -> io::Result<Option<bool>> {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(path)?.permissions().mode();
    if !mode_is_private(mode) {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(file_is_private(path))
}

#[cfg(not(unix))]
fn tighten_permissions(_path: &Path) -> io::Result<Option<bool>> {
    Ok(None)
}

/// Writes `bytes` to `path` through a sibling temp file that is private from its first byte.
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    let _ = fs::remove_file(&tmp);
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

// ---- credentials that only a build can carry ----------------------------------------------------

/// Developer-side credentials injected when the app is built (`option_env!`). No build in this
/// repository sets them today, so every path that needs them reports "not configured in this build".
/// To add them later, set the variables for the compile step from CI secrets; see `FORK.md`.
#[derive(Clone, Default)]
pub struct BuildCreds {
    pub ss_devid: Option<String>,
    pub ss_devpassword: Option<String>,
    pub ss_softname: Option<String>,
    pub tgdb_apikey: Option<String>,
}

impl fmt::Debug for BuildCreds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BuildCreds")
            .field("screenscraper", &self.screenscraper_dev().is_some())
            .field("thegamesdb", &self.tgdb_apikey.is_some())
            .finish()
    }
}

/// Name sent to ScreenScraper as `softname` when the build does not set one.
pub const DEFAULT_SOFTNAME: &str = "steamdeck-ports-mgr";

impl BuildCreds {
    /// What this binary was compiled with.
    pub fn from_build() -> BuildCreds {
        fn some(v: Option<&'static str>) -> Option<String> {
            v.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
        }
        BuildCreds {
            ss_devid: some(option_env!("DPM_SS_DEVID")),
            ss_devpassword: some(option_env!("DPM_SS_DEVPASSWORD")),
            ss_softname: some(option_env!("DPM_SS_SOFTNAME")),
            tgdb_apikey: some(option_env!("DPM_TGDB_APIKEY")),
        }
    }

    pub fn none() -> BuildCreds {
        BuildCreds::default()
    }

    /// ScreenScraper developer id, password and softname, when this build carries them.
    pub fn screenscraper_dev(&self) -> Option<(&str, &str, &str)> {
        Some((self.ss_devid.as_deref()?, self.ss_devpassword.as_deref()?, self.ss_softname.as_deref().unwrap_or(DEFAULT_SOFTNAME)))
    }

    pub fn all_values(&self) -> Vec<String> {
        [&self.ss_devid, &self.ss_devpassword, &self.tgdb_apikey].into_iter().flatten().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dpm_secrets_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_missing_file_is_an_empty_set() {
        let dir = temp_dir("missing");
        let s = Secrets::load(&dir.join(FILE_NAME));
        assert!(s.sgdb_key().is_none() && s.tgdb_key().is_none() && s.screenscraper_login().is_none());
        assert!(s.problem.is_none());
        assert!(!dir.join(FILE_NAME).exists(), "loading must not create the file");
    }

    #[test]
    fn values_round_trip_and_are_cleaned() {
        let dir = temp_dir("roundtrip");
        let path = dir.join(FILE_NAME);
        let mut s = Secrets::load(&path);
        s.set(Field::SteamGridDbKey, Some("  \"0123456789abcdef0123456789abcdef\"\r\n")).unwrap();
        s.set(Field::ScreenScraperUser, Some("tim")).unwrap();
        assert_eq!(s.screenscraper_login(), None, "a login needs both halves");
        s.set(Field::ScreenScraperPassword, Some("hunter22hunter")).unwrap();
        let again = Secrets::load(&path);
        assert_eq!(again.sgdb_key(), Some("0123456789abcdef0123456789abcdef"));
        assert_eq!(again.screenscraper_login(), Some(("tim", "hunter22hunter")));
        assert!(again.tgdb_key().is_none());
        let mut again = again;
        again.set(Field::SteamGridDbKey, None).unwrap();
        assert!(Secrets::load(&path).sgdb_key().is_none());
        assert!(Secrets::load(&path).screenscraper_login().is_some(), "other fields survive a removal");
    }

    #[test]
    fn implausible_values_are_refused_and_change_nothing() {
        let dir = temp_dir("refuse");
        let path = dir.join(FILE_NAME);
        let mut s = Secrets::load(&path);
        s.set(Field::SteamGridDbKey, Some("abc123abc123")).unwrap();
        assert!(s.set(Field::SteamGridDbKey, Some("   ")).is_err());
        assert!(s.set(Field::SteamGridDbKey, Some("has\nnewline inside")).is_err());
        assert!(s.set(Field::SteamGridDbKey, Some(&"x".repeat(300))).is_err());
        assert_eq!(Secrets::load(&path).sgdb_key(), Some("abc123abc123"));
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_created_with_mode_0600_and_stays_that_way() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("mode");
        let path = dir.join(FILE_NAME);
        let mut s = Secrets::load(&path);
        s.set(Field::SteamGridDbKey, Some("abc123abc123abc1")).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        s.set(Field::TheGamesDbKey, Some("def456def456def4")).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "a rewrite keeps the mode");
        assert!(!dir.join(format!("{FILE_NAME}.tmp")).exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_was_world_readable_is_tightened_on_load() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("tighten");
        let path = dir.join(FILE_NAME);
        fs::write(&path, r#"{"steamgriddb_api_key":"abc123abc123abc1"}"#).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let s = Secrets::load(&path);
        assert_eq!(s.sgdb_key(), Some("abc123abc123abc1"));
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn a_damaged_file_is_never_overwritten() {
        let dir = temp_dir("damaged");
        let path = dir.join(FILE_NAME);
        fs::write(&path, "{ not json").unwrap();
        let mut s = Secrets::load(&path);
        assert!(s.problem.is_some());
        assert!(s.set(Field::SteamGridDbKey, Some("abc123abc123abc1")).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ not json");
    }

    #[test]
    fn masking_shows_at_most_the_last_four_characters() {
        assert_eq!(mask("0123456789abcdef"), "****cdef");
        assert_eq!(mask("short"), "****");
        assert_eq!(mask("12345678"), "****");
        assert_eq!(mask(""), "****");
        assert!(!mask("0123456789abcdef").contains("0123"));
    }

    #[test]
    fn debug_output_never_contains_a_value() {
        let dir = temp_dir("debug");
        let mut s = Secrets::load(&dir.join(FILE_NAME));
        s.set(Field::SteamGridDbKey, Some("SECRETKEY-aaaaaaaaaaaa")).unwrap();
        s.set(Field::ScreenScraperUser, Some("SECRETUSER")).unwrap();
        s.set(Field::ScreenScraperPassword, Some("SECRETPASSWORD")).unwrap();
        let text = format!("{s:?}");
        for secret in ["SECRETKEY", "SECRETUSER", "SECRETPASSWORD"] {
            assert!(!text.contains(secret), "{text}");
        }
        let build = BuildCreds { ss_devid: Some("DEVSECRET".into()), ss_devpassword: Some("DEVPASS".into()), ss_softname: None, tgdb_apikey: Some("TGDBSECRET".into()) };
        let text = format!("{build:?}");
        for secret in ["DEVSECRET", "DEVPASS", "TGDBSECRET"] {
            assert!(!text.contains(secret), "{text}");
        }
    }

    #[test]
    fn secrets_never_land_in_a_sibling_state_file() {
        let dir = temp_dir("siblings");
        fs::write(dir.join("state.json"), r#"{"installed":{}}"#).unwrap();
        let mut s = Secrets::load(&dir.join(FILE_NAME));
        s.set(Field::SteamGridDbKey, Some("KEYVALUE-abcdef123456")).unwrap();
        assert_eq!(fs::read_to_string(dir.join("state.json")).unwrap(), r#"{"installed":{}}"#);
        let others: Vec<_> = fs::read_dir(&dir).unwrap().filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(others.len(), 2, "{others:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_is_neither_read_nor_written() {
        let dir = temp_dir("symlink");
        let target = dir.join("elsewhere.json");
        fs::write(&target, r#"{"steamgriddb_api_key":"abc123abc123abc1"}"#).unwrap();
        let link = dir.join(FILE_NAME);
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let mut s = Secrets::load(&link);
        assert!(s.sgdb_key().is_none(), "a key behind a link is not used");
        assert!(s.problem.as_deref().is_some_and(|p| p.contains("link")));
        assert!(s.set(Field::SteamGridDbKey, Some("abc123abc123abc1")).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), r#"{"steamgriddb_api_key":"abc123abc123abc1"}"#);
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the link is still a link");
    }

    #[test]
    fn a_drive_that_cannot_restrict_files_is_reported_and_a_normal_one_is_not() {
        assert!(mode_is_private(0o600) && mode_is_private(0o400));
        assert!(!mode_is_private(0o644) && !mode_is_private(0o777) && !mode_is_private(0o660));
        let dir = temp_dir("drivenote");
        let mut s = Secrets::load(&dir.join(FILE_NAME));
        s.set(Field::SteamGridDbKey, Some("abc123abc123abc1")).unwrap();
        #[cfg(unix)]
        {
            assert_eq!(s.drive_note(), None, "a normal drive keeps the file private");
            s.restricted.set(Some(false));
            assert!(s.drive_note().is_some_and(|n| n.contains("cannot restrict")));
        }
    }

    #[test]
    fn passwords_keep_their_quotes_and_spaces_but_lose_the_pasted_line_break() {
        assert_eq!(clean_password("\"p w\" \r\n").as_deref(), Some("\"p w\" "));
        assert_eq!(clean_password("pw\n").as_deref(), Some("pw"));
        assert_eq!(clean_password("   ").as_deref(), None);
        assert_eq!(clean_password("a\tb"), None);
        assert_eq!(clean_value("\"key\"\n").as_deref(), Some("key"));
        let dir = temp_dir("pwq");
        let mut s = Secrets::load(&dir.join(FILE_NAME));
        s.set(Field::ScreenScraperPassword, Some("'quoted'")).unwrap();
        let text = fs::read_to_string(dir.join(FILE_NAME)).unwrap();
        assert!(text.contains("'quoted'"), "{text}");
    }

    #[test]
    fn screenscraper_dev_credentials_need_id_and_password_and_default_the_softname() {
        let mut build = BuildCreds { ss_devid: Some("id".into()), ..BuildCreds::none() };
        assert!(build.screenscraper_dev().is_none());
        build.ss_devpassword = Some("pw".into());
        assert_eq!(build.screenscraper_dev(), Some(("id", "pw", DEFAULT_SOFTNAME)));
        build.ss_softname = Some("custom".into());
        assert_eq!(build.screenscraper_dev(), Some(("id", "pw", "custom")));
    }
}
