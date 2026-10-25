//! Listing, adding and updating shortcuts inside a parsed `shortcuts.vdf`.
//!
//! Only the fields an operation names are touched. Everything else in an entry, including fields this
//! library does not model, stays as read. Field names are matched ignoring case because real files
//! mix `appid`, `appname`, `AppName` and `exe`; an existing key keeps the casing it has.

use super::app_id;
use super::vdf::{Document, Entry, Value};
use std::fmt;

const SHORTCUTS_KEY: &str = "shortcuts";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutError {
    NotFound(String),
    AmbiguousAppId(u32),
    DuplicateAppId(u32),
    /// The entry has no `appid`, so Steam derives one from the path. Changing the path would change
    /// the ID. Ask for `derive_missing_app_id` to pin it first.
    MissingAppId(String),
    BadField { field: &'static str, reason: String },
    InvalidValue { field: &'static str, reason: String },
}

impl fmt::Display for ShortcutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShortcutError::NotFound(what) => write!(f, "no shortcut {what}"),
            ShortcutError::AmbiguousAppId(id) => write!(f, "more than one shortcut has app ID {id}"),
            ShortcutError::DuplicateAppId(id) => write!(f, "a shortcut with app ID {id} already exists"),
            ShortcutError::MissingAppId(key) => {
                write!(f, "shortcut {key} has no stored app ID; changing its path would change the ID Steam derives")
            }
            ShortcutError::BadField { field, reason } => write!(f, "field {field} is not usable: {reason}"),
            ShortcutError::InvalidValue { field, reason } => write!(f, "invalid {field}: {reason}"),
        }
    }
}

impl std::error::Error for ShortcutError {}

/// A read-only copy of one entry, for display and matching. Text is converted lossily, so a value that
/// is not valid UTF-8 shows replacement characters; do not write these strings back with `update`.
/// `exe`, `start_dir` and `icon` are as stored, quotes included (use the `*_path` methods for plain paths; an icon
/// is normally stored without quotes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    pub key: String,
    pub app_id: Option<u32>,
    pub app_name: String,
    pub exe: String,
    pub start_dir: String,
    pub icon: String,
    pub launch_options: String,
    pub shortcut_path: String,
    pub tags: Vec<String>,
    /// When Steam last started it (seconds since 1970, UTC); 0 = never, or not recorded.
    pub last_play_time: u32,
}

impl Shortcut {
    pub fn exe_path(&self) -> &str {
        unquote(&self.exe)
    }

    pub fn start_dir_path(&self) -> &str {
        unquote(&self.start_dir)
    }

    /// The icon as a plain path: Steam stores it without quotes, but a value that came in with them (written by
    /// another tool) is unquoted the same way as `exe`, so every comparison sees the same path.
    pub fn icon_path(&self) -> &str {
        unquote(&self.icon)
    }
}

/// `exe` and `start_dir` are plain paths here; the library adds the quotes Steam stores for them.
#[derive(Debug, Clone, Default)]
pub struct NewShortcut {
    pub app_name: String,
    pub exe: String,
    pub start_dir: String,
    /// Plain path of an icon file, stored unquoted. Steam only shows an icon when this is set; a file
    /// in `config/grid/` alone is not enough.
    pub icon: Option<String>,
    pub launch_options: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    pub key: String,
    pub app_id: u32,
}

#[derive(Debug, Clone, Copy)]
pub enum Target<'a> {
    Key(&'a str),
    AppId(u32),
}

/// Fields to change; `None` leaves the field alone. `exe` and `start_dir` are plain paths (quotes are
/// added); `icon` is stored as a plain path, and an empty one clears it. The name is not here on purpose:
/// renaming would change what Steam derives for an entry without a stored ID, and Steam Input layouts
/// follow the name.
#[derive(Debug, Clone, Default)]
pub struct ShortcutUpdate {
    pub exe: Option<String>,
    pub start_dir: Option<String>,
    pub launch_options: Option<String>,
    pub icon: Option<String>,
    /// For an entry with no stored `appid`: compute what Steam derives from its current exe and name
    /// and store it before changing anything. Unverified against a real Steam (see `app_id`), so off
    /// by default; a wrong guess orphans the art and play time tied to the real ID. The safer route is
    /// to open and quit Steam once (it stores the ID it uses on exit) and load the file again.
    pub derive_missing_app_id: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Updated {
    pub key: String,
    pub app_id: u32,
    pub app_id_written: bool,
}

pub fn quote_path(path: &str) -> Result<String, ShortcutError> {
    check_text("path", path)?;
    if path.is_empty() {
        return Err(ShortcutError::InvalidValue { field: "path", reason: "empty".into() });
    }
    if path.contains('"') {
        return Err(ShortcutError::InvalidValue { field: "path", reason: "contains a double quote".into() });
    }
    Ok(format!("\"{path}\""))
}

/// A path stored without quotes (the `icon` field). Only values that would corrupt the file are refused.
fn plain_path(path: &str) -> Result<String, ShortcutError> {
    check_text("path", path)?;
    if path.is_empty() {
        return Err(ShortcutError::InvalidValue { field: "path", reason: "empty".into() });
    }
    Ok(path.to_string())
}

pub fn unquote(stored: &str) -> &str {
    stored.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(stored)
}

fn check_text(field: &'static str, text: &str) -> Result<(), ShortcutError> {
    if text.contains('\0') {
        return Err(ShortcutError::InvalidValue { field, reason: "contains a NUL byte".into() });
    }
    Ok(())
}

fn shortcuts_entries(doc: &Document) -> Option<&Vec<Entry>> {
    doc.root.iter().find(|e| e.key_matches(SHORTCUTS_KEY)).and_then(|e| match &e.value {
        Value::Object(entries) => Some(entries),
        _ => None,
    })
}

fn shortcuts_entries_mut(doc: &mut Document) -> Option<&mut Vec<Entry>> {
    doc.root.iter_mut().find(|e| e.key_matches(SHORTCUTS_KEY)).and_then(|e| match &mut e.value {
        Value::Object(entries) => Some(entries),
        _ => None,
    })
}

fn field<'a>(fields: &'a [Entry], name: &str) -> Option<&'a Entry> {
    fields.iter().find(|e| e.key_matches(name))
}

fn text(fields: &[Entry], name: &str) -> String {
    match field(fields, name).map(|e| &e.value) {
        Some(Value::String(bytes)) => String::from_utf8_lossy(bytes).into_owned(),
        _ => String::new(),
    }
}

fn stored_app_id(fields: &[Entry]) -> Option<u32> {
    match field(fields, "appid").map(|e| &e.value) {
        Some(Value::Int(v)) => Some(*v),
        _ => None,
    }
}

fn read_shortcut(entry: &Entry) -> Option<Shortcut> {
    let Value::Object(fields) = &entry.value else { return None };
    let tags = match field(fields, "tags").map(|e| &e.value) {
        Some(Value::Object(tags)) => tags
            .iter()
            .filter_map(|t| match &t.value {
                Value::String(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Some(Shortcut {
        key: String::from_utf8_lossy(&entry.key).into_owned(),
        app_id: stored_app_id(fields),
        app_name: text(fields, "AppName"),
        exe: text(fields, "Exe"),
        start_dir: text(fields, "StartDir"),
        icon: text(fields, "icon"),
        launch_options: text(fields, "LaunchOptions"),
        shortcut_path: text(fields, "ShortcutPath"),
        tags,
        last_play_time: match field(fields, "LastPlayTime").map(|e| &e.value) {
            Some(Value::Int(v)) => *v,
            _ => 0,
        },
    })
}

pub fn list(doc: &Document) -> Vec<Shortcut> {
    shortcuts_entries(doc).map(|entries| entries.iter().filter_map(read_shortcut).collect()).unwrap_or_default()
}

fn next_key(entries: &[Entry]) -> Result<String, ShortcutError> {
    let max = entries.iter().filter_map(|e| std::str::from_utf8(&e.key).ok()?.parse::<u64>().ok()).max();
    let next = match max {
        None => 0,
        Some(max) => max
            .checked_add(1)
            .ok_or_else(|| ShortcutError::BadField { field: "shortcuts", reason: "no free numeric key".into() })?,
    };
    Ok(next.to_string())
}

pub fn add(doc: &mut Document, new: &NewShortcut) -> Result<Added, ShortcutError> {
    check_text("app name", &new.app_name)?;
    check_text("launch options", &new.launch_options)?;
    if new.app_name.is_empty() {
        return Err(ShortcutError::InvalidValue { field: "app name", reason: "empty".into() });
    }
    let exe = quote_path(&new.exe)?;
    let start_dir = quote_path(&new.start_dir)?;
    let icon = match &new.icon {
        Some(path) => plain_path(path)?,
        None => String::new(),
    };
    for tag in &new.tags {
        check_text("tag", tag)?;
    }

    let app_id = app_id::shortcut_app_id(&exe, &new.app_name);
    if list(doc).iter().any(|s| s.app_id == Some(app_id)) {
        return Err(ShortcutError::DuplicateAppId(app_id));
    }

    let tags = new.tags.iter().enumerate().map(|(i, t)| Entry::string(&i.to_string(), t)).collect();
    // The field order Steam itself writes.
    let fields = vec![
        Entry::int("appid", app_id),
        Entry::string("AppName", &new.app_name),
        Entry::string("Exe", &exe),
        Entry::string("StartDir", &start_dir),
        Entry::string("icon", &icon),
        Entry::string("ShortcutPath", ""),
        Entry::string("LaunchOptions", &new.launch_options),
        Entry::int("IsHidden", 0),
        Entry::int("AllowDesktopConfig", 1),
        Entry::int("AllowOverlay", 1),
        Entry::int("OpenVR", 0),
        Entry::int("Devkit", 0),
        Entry::string("DevkitGameID", ""),
        Entry::int("DevkitOverrideAppID", 0),
        Entry::int("LastPlayTime", 0),
        Entry::string("FlatpakAppID", ""),
        Entry::new("tags", Value::Object(tags)),
    ];

    if doc.root.iter().any(|e| e.key_matches(SHORTCUTS_KEY) && !matches!(e.value, Value::Object(_))) {
        return Err(ShortcutError::BadField { field: "shortcuts", reason: "is not an object".into() });
    }
    // Everything that can fail comes before the first change to the document.
    let key = match shortcuts_entries(doc) {
        Some(entries) => next_key(entries)?,
        None => "0".to_string(),
    };
    if shortcuts_entries(doc).is_none() {
        doc.root.push(Entry::new(SHORTCUTS_KEY, Value::Object(Vec::new())));
    }
    doc.terminated = true;
    let entries = shortcuts_entries_mut(doc).expect("section exists");
    entries.push(Entry::new(&key, Value::Object(fields)));
    Ok(Added { key, app_id })
}

fn locate(entries: &[Entry], target: Target) -> Result<usize, ShortcutError> {
    let hits: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| match (target, &e.value) {
            (Target::Key(key), Value::Object(_)) => e.key == key.as_bytes(),
            (Target::AppId(id), Value::Object(fields)) => stored_app_id(fields) == Some(id),
            _ => false,
        })
        .map(|(i, _)| i)
        .collect();
    match (hits.as_slice(), target) {
        ([], Target::Key(key)) => Err(ShortcutError::NotFound(format!("with key {key}"))),
        ([], Target::AppId(id)) => Err(ShortcutError::NotFound(format!("with app ID {id}"))),
        ([one], _) => Ok(*one),
        (_, Target::AppId(id)) => Err(ShortcutError::AmbiguousAppId(id)),
        (_, Target::Key(key)) => Err(ShortcutError::NotFound(format!("with a single key {key}"))),
    }
}

fn ensure_string_field(fields: &[Entry], name: &'static str) -> Result<(), ShortcutError> {
    match field(fields, name) {
        Some(Entry { value: Value::String(_), .. }) | None => Ok(()),
        Some(_) => Err(ShortcutError::BadField { field: name, reason: "is not a string".into() }),
    }
}

/// Sets the first field with this name (Steam reads the first when a key is duplicated).
fn set_text(fields: &mut Vec<Entry>, name: &'static str, value: String) -> Result<(), ShortcutError> {
    match fields.iter_mut().find(|e| e.key_matches(name)) {
        Some(entry) => match &mut entry.value {
            Value::String(bytes) => *bytes = value.into_bytes(),
            _ => return Err(ShortcutError::BadField { field: name, reason: "is not a string".into() }),
        },
        None => fields.push(Entry::string(name, &value)),
    }
    Ok(())
}

/// Changes fields of one entry in place. The app ID, name, tags and every field not named stay as
/// they were. Nothing changes if any requested value is invalid.
pub fn update(doc: &mut Document, target: Target, change: &ShortcutUpdate) -> Result<Updated, ShortcutError> {
    let exe = change.exe.as_deref().map(quote_path).transpose()?;
    let start_dir = change.start_dir.as_deref().map(quote_path).transpose()?;
    let icon = match change.icon.as_deref() {
        Some("") => Some(String::new()),
        Some(path) => Some(plain_path(path)?),
        None => None,
    };
    if let Some(options) = &change.launch_options {
        check_text("launch options", options)?;
    }

    let entries = shortcuts_entries_mut(doc).ok_or_else(|| ShortcutError::NotFound("(no shortcuts section)".into()))?;
    let index = locate(entries, target)?;
    let key = String::from_utf8_lossy(&entries[index].key).into_owned();
    let Value::Object(fields) = &mut entries[index].value else { unreachable!("locate returns objects") };

    // Validate the existing field types before touching anything, so a refusal leaves the entry whole.
    let wanted = [
        ("Exe", exe.is_some()),
        ("StartDir", start_dir.is_some()),
        ("icon", icon.is_some()),
        ("LaunchOptions", change.launch_options.is_some()),
    ];
    for (name, wanted) in wanted {
        if wanted {
            ensure_string_field(fields, name)?;
        }
    }
    if field(fields, "appid").is_some_and(|e| !matches!(e.value, Value::Int(_))) {
        return Err(ShortcutError::BadField { field: "appid", reason: "is not an int".into() });
    }

    let (app_id, app_id_written) = match stored_app_id(fields) {
        Some(id) => (id, false),
        None if change.derive_missing_app_id => {
            let bytes = |name: &str| match field(fields, name).map(|e| &e.value) {
                Some(Value::String(b)) => b.clone(),
                _ => Vec::new(),
            };
            let id = app_id::shortcut_app_id_bytes(&bytes("Exe"), &bytes("AppName"));
            fields.insert(0, Entry::int("appid", id));
            (id, true)
        }
        None => return Err(ShortcutError::MissingAppId(key)),
    };

    if let Some(exe) = exe {
        set_text(fields, "Exe", exe)?;
    }
    if let Some(dir) = start_dir {
        set_text(fields, "StartDir", dir)?;
    }
    if let Some(options) = &change.launch_options {
        set_text(fields, "LaunchOptions", options.clone())?;
    }
    if let Some(icon) = icon {
        set_text(fields, "icon", icon)?;
    }
    doc.terminated = true;
    Ok(Updated { key, app_id, app_id_written })
}

#[cfg(test)]
mod tests {
    use super::super::vdf::fixtures::*;
    use super::*;

    fn parse(bytes: &[u8]) -> Document {
        Document::parse(bytes).unwrap()
    }

    fn new_port() -> NewShortcut {
        NewShortcut {
            app_name: "2 Ship 2 Harkinian".into(),
            exe: "/home/deck/Ports/2s2h/2s2h.AppImage".into(),
            start_dir: "/home/deck/Ports/2s2h".into(),
            icon: None,
            launch_options: String::new(),
            tags: vec!["Ports".into()],
        }
    }

    #[test]
    fn lists_the_fixture_shortcuts() {
        let shortcuts = list(&parse(&plain_file()));
        assert_eq!(shortcuts.len(), 2);
        assert_eq!(shortcuts[0].key, "0");
        assert_eq!(shortcuts[0].app_id, Some(0xF154_8865));
        assert_eq!(shortcuts[0].app_name, "Zelda Ship");
        assert_eq!(shortcuts[0].exe, "\"/home/deck/Ports/soh/soh.AppImage\"");
        assert_eq!(shortcuts[0].exe_path(), "/home/deck/Ports/soh/soh.AppImage");
        assert_eq!(shortcuts[0].tags, ["Ports", "Favorites"]);
        assert_eq!(shortcuts[1].app_name, "Chiaki");
        assert!(shortcuts[1].tags.is_empty());
    }

    #[test]
    fn lists_nothing_for_an_empty_or_missing_section() {
        assert!(list(&parse(EMPTY_FILE)).is_empty());
        assert!(list(&Document::empty()).is_empty());
    }

    #[test]
    fn lists_mixed_case_fields_and_lossy_text() {
        let shortcuts = list(&parse(&odd_file()));
        assert_eq!(shortcuts[0].app_name, "Odd \u{FFFD}\u{FFFD} name");
        assert_eq!(shortcuts[0].exe_path(), "/opt/odd/run");
        assert_eq!(shortcuts[0].start_dir_path(), "/opt/odd/");
        assert_eq!(shortcuts[1].app_id, Some(5));
    }

    #[test]
    fn add_to_an_empty_file_writes_the_fields_in_steams_order() {
        let mut doc = parse(EMPTY_FILE);
        let added = add(&mut doc, &new_port()).unwrap();
        let exe = "\"/home/deck/Ports/2s2h/2s2h.AppImage\"";
        assert_eq!(added, Added { key: "0".into(), app_id: app_id::shortcut_app_id(exe, "2 Ship 2 Harkinian") });

        let expected = file_of(&[object(
            "0",
            &[
                int_field("appid", added.app_id),
                str_field("AppName", b"2 Ship 2 Harkinian"),
                str_field("Exe", exe.as_bytes()),
                str_field("StartDir", b"\"/home/deck/Ports/2s2h\""),
                str_field("icon", b""),
                str_field("ShortcutPath", b""),
                str_field("LaunchOptions", b""),
                int_field("IsHidden", 0),
                int_field("AllowDesktopConfig", 1),
                int_field("AllowOverlay", 1),
                int_field("OpenVR", 0),
                int_field("Devkit", 0),
                str_field("DevkitGameID", b""),
                int_field("DevkitOverrideAppID", 0),
                int_field("LastPlayTime", 0),
                str_field("FlatpakAppID", b""),
                object("tags", &[str_field("0", b"Ports")]),
            ],
        )]);
        assert_eq!(doc.encode(), expected);
    }

    #[test]
    fn add_appends_after_existing_entries_and_leaves_them_byte_identical() {
        let before = plain_file();
        let mut doc = parse(&before);
        let added = add(&mut doc, &new_port()).unwrap();
        assert_eq!(added.key, "2");
        let after = doc.encode();
        // Everything up to the old `shortcuts` end marker is unchanged.
        let old_body = &before[..before.len() - 2];
        assert_eq!(&after[..old_body.len()], old_body);
        assert_eq!(list(&parse(&after)).len(), 3);
    }

    #[test]
    fn add_picks_the_next_key_after_gaps_and_ignores_non_numeric_keys() {
        let mut doc = parse(&odd_file());
        assert_eq!(add(&mut doc, &new_port()).unwrap().key, "8");
    }

    #[test]
    fn add_refuses_instead_of_overflowing_the_key_counter() {
        let bytes = file_of(&[object("18446744073709551615", &[int_field("appid", 3)])]);
        let mut doc = parse(&bytes);
        assert!(matches!(add(&mut doc, &new_port()), Err(ShortcutError::BadField { field: "shortcuts", .. })));
        assert_eq!(doc.encode(), bytes);
    }

    #[test]
    fn a_refused_add_leaves_even_an_unterminated_document_as_it_was() {
        let mut bytes = file_of(&[object("18446744073709551615", &[])]);
        bytes.pop();
        let mut doc = parse(&bytes);
        assert!(!doc.terminated);
        assert!(add(&mut doc, &new_port()).is_err());
        assert_eq!(doc.encode(), bytes);
    }

    #[test]
    fn add_stores_the_icon_as_a_plain_path() {
        let mut doc = parse(EMPTY_FILE);
        let mut port = new_port();
        port.icon = Some("/home/deck/.steam/grid/77_icon.png".into());
        add(&mut doc, &port).unwrap();
        assert_eq!(list(&doc)[0].icon, "/home/deck/.steam/grid/77_icon.png");
    }

    #[test]
    fn add_creates_the_section_when_the_file_has_none() {
        let mut doc = parse(&[]);
        add(&mut doc, &new_port()).unwrap();
        assert!(doc.terminated);
        assert_eq!(list(&parse(&doc.encode())).len(), 1);
    }

    #[test]
    fn add_refuses_a_duplicate_app_id() {
        let mut doc = parse(EMPTY_FILE);
        add(&mut doc, &new_port()).unwrap();
        let before = doc.encode();
        assert!(matches!(add(&mut doc, &new_port()), Err(ShortcutError::DuplicateAppId(_))));
        assert_eq!(doc.encode(), before);
    }

    #[test]
    fn add_rejects_values_that_would_corrupt_the_file() {
        let mut doc = parse(EMPTY_FILE);
        let mut bad = new_port();
        bad.exe = "/opt/with\"quote".into();
        assert!(matches!(add(&mut doc, &bad), Err(ShortcutError::InvalidValue { .. })));
        bad = new_port();
        bad.app_name = "nul\0name".into();
        assert!(matches!(add(&mut doc, &bad), Err(ShortcutError::InvalidValue { .. })));
        bad = new_port();
        bad.app_name = String::new();
        assert!(matches!(add(&mut doc, &bad), Err(ShortcutError::InvalidValue { .. })));
        assert_eq!(doc.encode(), EMPTY_FILE);
    }

    #[test]
    fn update_repoints_exe_and_start_dir_and_keeps_the_app_id_and_everything_else() {
        let before = plain_file();
        let mut doc = parse(&before);
        let result = update(
            &mut doc,
            Target::AppId(0xF154_8865),
            &ShortcutUpdate {
                exe: Some("/run/media/mmcblk0p1/Ports/soh/soh.AppImage".into()),
                start_dir: Some("/run/media/mmcblk0p1/Ports/soh".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result, Updated { key: "0".into(), app_id: 0xF154_8865, app_id_written: false });

        let expected = file_of(&[
            steam_entry(
                0,
                0xF154_8865,
                "Zelda Ship",
                "\"/run/media/mmcblk0p1/Ports/soh/soh.AppImage\"",
                "\"/run/media/mmcblk0p1/Ports/soh\"",
                &["Ports", "Favorites"],
            ),
            steam_entry(1, 0x9962_3236, "Chiaki", "\"/usr/bin/chiaki\"", "\"/usr/bin/\"", &[]),
        ]);
        assert_eq!(doc.encode(), expected);
    }

    #[test]
    fn update_keeps_fields_it_does_not_model_and_the_original_key_casing() {
        let mut doc = parse(&odd_file());
        update(&mut doc, Target::Key("0"), &ShortcutUpdate { exe: Some("/new/run".into()), ..Default::default() }).unwrap();
        let bytes = doc.encode();
        let original = odd_file();
        let old_exe: &[u8] = b"\x01exe\0\"/opt/odd/run\"\0";
        let new_exe: &[u8] = b"\x01exe\0\"/new/run\"\0";
        let at = original.windows(old_exe.len()).position(|w| w == old_exe).unwrap();
        let mut expected = original.clone();
        expected.splice(at..at + old_exe.len(), new_exe.iter().copied());
        assert_eq!(bytes, expected);
    }

    #[test]
    fn update_can_set_launch_options_and_icon_and_clear_the_icon() {
        let mut doc = parse(&plain_file());
        update(
            &mut doc,
            Target::Key("1"),
            &ShortcutUpdate { launch_options: Some("--fullscreen".into()), icon: Some("/grid/1_icon.png".into()), ..Default::default() },
        )
        .unwrap();
        let s = &list(&doc)[1];
        assert_eq!(s.launch_options, "--fullscreen");
        assert_eq!(s.icon, "/grid/1_icon.png");
        update(&mut doc, Target::Key("1"), &ShortcutUpdate { icon: Some(String::new()), ..Default::default() }).unwrap();
        assert_eq!(list(&doc)[1].icon, "");
    }

    #[test]
    fn update_with_nothing_to_change_changes_nothing() {
        let before = plain_file();
        let mut doc = parse(&before);
        update(&mut doc, Target::Key("0"), &ShortcutUpdate::default()).unwrap();
        assert_eq!(doc.encode(), before);
    }

    #[test]
    fn update_refuses_an_entry_without_an_app_id_unless_asked_to_pin_it() {
        let entry = object(
            "0",
            &[str_field("AppName", b"Old"), str_field("Exe", b"\"/old/run\""), str_field("StartDir", b"\"/old/\"")],
        );
        let bytes = file_of(&[entry]);
        let mut doc = parse(&bytes);
        let change = ShortcutUpdate { exe: Some("/new/run".into()), ..Default::default() };
        assert_eq!(update(&mut doc, Target::Key("0"), &change), Err(ShortcutError::MissingAppId("0".into())));
        assert_eq!(doc.encode(), bytes);

        let pinned = update(&mut doc, Target::Key("0"), &ShortcutUpdate { derive_missing_app_id: true, ..change }).unwrap();
        assert!(pinned.app_id_written);
        assert_eq!(pinned.app_id, app_id::shortcut_app_id("\"/old/run\"", "Old"));
        let s = &list(&doc)[0];
        assert_eq!(s.app_id, Some(pinned.app_id));
        assert_eq!(s.exe, "\"/new/run\"");
        assert_eq!(s.app_name, "Old");
    }

    #[test]
    fn update_errors_leave_the_document_unchanged() {
        let before = plain_file();
        let mut doc = parse(&before);
        let ok = ShortcutUpdate { exe: Some("/x".into()), ..Default::default() };
        assert!(matches!(update(&mut doc, Target::Key("9"), &ok), Err(ShortcutError::NotFound(_))));
        assert!(matches!(update(&mut doc, Target::AppId(1), &ok), Err(ShortcutError::NotFound(_))));
        let bad = ShortcutUpdate { exe: Some("/has\"quote".into()), ..Default::default() };
        assert!(matches!(update(&mut doc, Target::Key("0"), &bad), Err(ShortcutError::InvalidValue { .. })));
        assert_eq!(doc.encode(), before);
        assert!(matches!(update(&mut Document::empty(), Target::Key("0"), &ok), Err(ShortcutError::NotFound(_))));
    }

    #[test]
    fn update_refuses_when_two_entries_share_an_app_id() {
        let bytes = file_of(&[
            steam_entry(0, 77, "A", "\"/a\"", "\"/\"", &[]),
            steam_entry(1, 77, "B", "\"/b\"", "\"/\"", &[]),
        ]);
        let mut doc = parse(&bytes);
        let ok = ShortcutUpdate { exe: Some("/x".into()), ..Default::default() };
        assert_eq!(update(&mut doc, Target::AppId(77), &ok), Err(ShortcutError::AmbiguousAppId(77)));
        assert_eq!(doc.encode(), bytes);
        assert!(update(&mut doc, Target::Key("1"), &ok).is_ok());
    }

    #[test]
    fn update_refuses_a_field_of_the_wrong_type() {
        let entry = object("0", &[int_field("appid", 5), int_field("Exe", 1)]);
        let bytes = file_of(&[entry]);
        let mut doc = parse(&bytes);
        let change = ShortcutUpdate { exe: Some("/x".into()), ..Default::default() };
        assert!(matches!(update(&mut doc, Target::Key("0"), &change), Err(ShortcutError::BadField { field: "Exe", .. })));
        assert_eq!(doc.encode(), bytes);
    }
}
