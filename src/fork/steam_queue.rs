//! Steam changes queued from Game Mode, applied later in Desktop Mode with Steam closed.
//!
//! `shortcuts.vdf` is only written while Steam is closed (Steam rewrites it from memory when it exits),
//! and in Game Mode Steam is always running. So the Add to Steam screen can save the chosen adds and
//! updates to `steam-queue.json` next to the app instead. When the app sees Steam closed with a non-empty
//! queue it offers to apply it, through the same safe path as the screen (`screens::apply_selected_with`:
//! refuse while Steam runs, re-checked before the rename, verified backup, atomic write, read-back).
//!
//! The queue stores what the user chose, not the plan: the port key, the Steam account, whether a
//! shortcut found by name only may be repointed, and whether catalog artwork is placed. The plan is made
//! again from the current `shortcuts.vdf` when applying, so applying twice never adds a port twice, and
//! a port that left the catalog (or was uninstalled) since queuing is reported and dropped.
//!
//! The file is versioned JSON written atomically. A file that cannot be read is moved aside to
//! `steam-queue.json.corrupt-<time>` (never deleted) and the caller is told; the queue then starts empty.

use super::screens::{self, listed, SteamScreen};
use super::settings::ForkSettings;
use super::steam_sync::Action;
use super::store::{backup_stamp, SteamProcessCheck};
use crate::core::models::Port;
use serde_json::{json, Map, Value};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const FILE_NAME: &str = "steam-queue.json";
pub const VERSION: u64 = 1;
/// How often the app looks again while it is open with a non-empty queue.
pub const RECHECK_SECONDS: u64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueuedKind {
    Add,
    Update,
}

impl QueuedKind {
    fn as_str(self) -> &'static str {
        match self {
            QueuedKind::Add => "add",
            QueuedKind::Update => "update",
        }
    }
}

/// Which artwork to place when the change is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtChoice {
    /// The catalog portrait and icon the app has cached, as the screen does.
    Catalog,
    None,
}

impl ArtChoice {
    fn as_str(self) -> &'static str {
        match self {
            ArtChoice::Catalog => "catalog",
            ArtChoice::None => "none",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedChange {
    /// The Steam account folder under `userdata/`.
    pub account_id: u32,
    pub port_key: String,
    /// The port's name when it was queued, for messages (the catalog may have dropped it since).
    pub name: String,
    /// What it was when queued. Informational: applying plans again from the file as it is then.
    pub kind: QueuedKind,
    /// The user switched on repointing a shortcut that matched by name only. Without it such a match
    /// found at apply time is skipped, never repointed silently.
    pub repoint_by_name: bool,
    pub art: ArtChoice,
    pub queued_at: String,
}

impl QueuedChange {
    fn to_json(&self) -> Value {
        json!({
            "account_id": self.account_id,
            "port_key": self.port_key,
            "name": self.name,
            "kind": self.kind.as_str(),
            "repoint_by_name": self.repoint_by_name,
            "art": self.art.as_str(),
            "queued_at": self.queued_at,
        })
    }

    fn from_json(value: &Value) -> Result<QueuedChange, String> {
        let obj = value.as_object().ok_or("an entry is not an object")?;
        let account_id = obj
            .get("account_id")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or("an entry has no usable account_id")?;
        let port_key = obj.get("port_key").and_then(Value::as_str).filter(|k| !k.is_empty()).ok_or("an entry has no port_key")?.to_string();
        let kind = match obj.get("kind").and_then(Value::as_str) {
            Some("add") => QueuedKind::Add,
            Some("update") => QueuedKind::Update,
            _ => return Err("an entry has no usable kind".into()),
        };
        let art = match obj.get("art").and_then(Value::as_str) {
            None | Some("catalog") => ArtChoice::Catalog,
            Some("none") => ArtChoice::None,
            Some(_) => return Err("an entry has an unknown art choice".into()),
        };
        let text = |key: &str| obj.get(key).and_then(Value::as_str).map(str::to_string);
        Ok(QueuedChange {
            account_id,
            name: text("name").filter(|n| !n.is_empty()).unwrap_or_else(|| port_key.clone()),
            port_key,
            kind,
            repoint_by_name: obj.get("repoint_by_name").and_then(Value::as_bool).unwrap_or(false),
            art,
            queued_at: text("queued_at").unwrap_or_default(),
        })
    }

    /// `  Dusklight (add)`, for lists in dialogs.
    pub fn line(&self) -> String {
        format!("  {} ({})", self.name, self.kind.as_str())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SteamQueue {
    pub entries: Vec<QueuedChange>,
}

impl SteamQueue {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains(&self, account_id: u32, port_key: &str) -> bool {
        self.entries.iter().any(|e| e.account_id == account_id && e.port_key == port_key)
    }

    /// Adds a change, replacing one already queued for the same port and account (it keeps its place).
    pub fn upsert(&mut self, change: QueuedChange) {
        match self.entries.iter_mut().find(|e| e.account_id == change.account_id && e.port_key == change.port_key) {
            Some(existing) => *existing = change,
            None => self.entries.push(change),
        }
    }

    pub fn remove(&mut self, index: usize) -> Option<QueuedChange> {
        (index < self.entries.len()).then(|| self.entries.remove(index))
    }

    fn accounts(&self) -> Vec<u32> {
        let mut out: Vec<u32> = Vec::new();
        for e in &self.entries {
            if !out.contains(&e.account_id) {
                out.push(e.account_id);
            }
        }
        out
    }

    fn to_json(&self) -> Value {
        json!({ "version": VERSION, "entries": self.entries.iter().map(QueuedChange::to_json).collect::<Vec<_>>() })
    }

    fn from_json(value: &Value) -> Result<SteamQueue, String> {
        let obj: &Map<String, Value> = value.as_object().ok_or("it is not a JSON object")?;
        match obj.get("version").and_then(Value::as_u64) {
            Some(VERSION) => {}
            Some(v) => return Err(format!("it is version {v}, made by another version of this app (this one reads {VERSION})")),
            None => return Err("it has no version number".into()),
        }
        let entries = obj.get("entries").and_then(Value::as_array).ok_or("it has no entries list")?;
        let mut queue = SteamQueue::default();
        for entry in entries {
            queue.upsert(QueuedChange::from_json(entry)?);
        }
        Ok(queue)
    }
}

pub fn queue_path(config_dir: &Path) -> PathBuf {
    config_dir.join(FILE_NAME)
}

#[derive(Debug)]
pub struct LoadOutcome {
    pub queue: SteamQueue,
    /// Where an unreadable file was moved, if it was.
    pub set_aside: Option<PathBuf>,
    /// What to tell the user when the file could not be read.
    pub problem: Option<String>,
}

/// Reads the queue. A missing file is an empty queue. A file that cannot be read or understood is moved
/// aside (`.corrupt-<stamp>`, never over another file) and the queue starts empty; `problem` says so.
pub fn load(path: &Path) -> LoadOutcome {
    let empty = |set_aside, problem| LoadOutcome { queue: SteamQueue::default(), set_aside, problem };
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return empty(None, None),
        Err(e) => return empty(None, Some(format!("Could not read the queue of Steam changes ({}): {e}. It was left as it is.", path.display()))),
    };
    let parsed = std::str::from_utf8(&bytes)
        .map_err(|_| "it is not text".to_string())
        .and_then(|t| serde_json::from_str::<Value>(t.strip_prefix('\u{FEFF}').unwrap_or(t)).map_err(|e| format!("it is not valid JSON ({e})")))
        .and_then(|v| SteamQueue::from_json(&v));
    let why = match parsed {
        Ok(queue) => return LoadOutcome { queue, set_aside: None, problem: None },
        Err(why) => why,
    };
    match set_aside(path) {
        Ok(aside) => {
            let name = aside.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            empty(
                Some(aside),
                Some(format!("The queue of Steam changes could not be read: {why}.\nIt was kept as {name} and the queue starts empty.\nQueue the changes again from Settings > Add to Steam.")),
            )
        }
        Err(e) => empty(None, Some(format!("The queue of Steam changes could not be read: {why}.\nIt could not be moved aside either ({e}), so it is ignored."))),
    }
}

fn set_aside(path: &Path) -> io::Result<PathBuf> {
    let stamp = backup_stamp();
    for attempt in 0..1000u32 {
        let suffix = if attempt == 0 { format!(".corrupt-{stamp}") } else { format!(".corrupt-{stamp}-{attempt}") };
        let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        name.push(&suffix);
        let aside = path.with_file_name(name);
        if aside.exists() {
            continue;
        }
        fs::rename(path, &aside)?;
        return Ok(aside);
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "too many set-aside copies"))
}

/// Writes the queue atomically (temp file in the same folder, synced, renamed over). An empty queue
/// removes the file, so "is anything queued" is a cheap existence check.
pub fn save(path: &Path, queue: &SteamQueue) -> io::Result<()> {
    if queue.is_empty() {
        return match fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let text = serde_json::to_string_pretty(&queue.to_json()).map_err(io::Error::other)?;
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".dpm-tmp");
    let temp = path.with_file_name(name);
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create(true).truncate(true).open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

// ---------------------------------------------------------------------------------------------
// Game Mode and wording
// ---------------------------------------------------------------------------------------------

/// Game Mode runs the app under gamescope, which names itself in `XDG_CURRENT_DESKTOP` (the same signal
/// the tray check uses) and sets `GAMESCOPE_WAYLAND_DISPLAY`. Unverified on a real Deck.
pub fn is_game_mode(current_desktop: Option<&str>, gamescope_display: Option<&str>) -> bool {
    current_desktop.is_some_and(|d| d.to_ascii_lowercase().contains("gamescope")) || gamescope_display.is_some_and(|d| !d.is_empty())
}

pub fn game_mode_from_env() -> bool {
    let var = |name| std::env::var(name).ok();
    is_game_mode(var("XDG_CURRENT_DESKTOP").as_deref(), var("GAMESCOPE_WAYLAND_DISPLAY").as_deref())
}

/// Why nothing is written now and when the `count` queued changes will be, for the given mode.
pub fn when_applied_text(game_mode: bool, count: usize) -> String {
    let what = if count == 1 { "The change".to_string() } else { format!("The {count} changes") };
    if game_mode {
        format!(
            "Steam can't be closed in Game Mode, so nothing is written now.\n\
             {what} will apply next time you open this app in Desktop Mode with Steam closed (it asks first)."
        )
    } else {
        format!(
            "Steam is running, so nothing is written now.\n\
             {what} will apply once Steam is closed (Steam menu, Exit): this app asks within 30 seconds, \
             or the next time it starts with Steam closed."
        )
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The question asked at startup, after Steam closes, or from the screen's first row.
pub fn offer_text(queue: &SteamQueue) -> String {
    let lines: Vec<String> = queue.entries.iter().map(QueuedChange::line).collect();
    format!(
        "{} queued from Game Mode: apply now?\n\n{}\n\nSteam is closed, so {} can be written now. shortcuts.vdf is backed up first.",
        plural(queue.len(), "Steam change", "Steam changes"),
        listed(&lines),
        if queue.len() == 1 { "it" } else { "they" }
    )
}

// ---------------------------------------------------------------------------------------------
// Queuing from the screen
// ---------------------------------------------------------------------------------------------

/// The changes the screen would write now, as queue entries for its account.
pub fn changes_from_screen(screen: &SteamScreen, queued_at: &str) -> Vec<QueuedChange> {
    let Some(account) = screen.active_account() else { return Vec::new() };
    let art = if screen.cache_dir.is_some() { ArtChoice::Catalog } else { ArtChoice::None };
    screen
        .selected_items()
        .into_iter()
        .map(|item| QueuedChange {
            account_id: account.account_id,
            name: item.wanted.app_name.clone(),
            kind: if matches!(item.action, Action::Add) { QueuedKind::Add } else { QueuedKind::Update },
            repoint_by_name: matches!(item.action, Action::Update { by_name: true, .. }),
            port_key: item.wanted.port_key,
            art,
            queued_at: queued_at.to_string(),
        })
        .collect()
}

/// Text of the confirmation before queuing.
pub fn queue_confirm_text(changes: &[QueuedChange], game_mode: bool) -> String {
    let lines: Vec<String> = changes.iter().map(QueuedChange::line).collect();
    format!("{}\n\nQueue for Desktop Mode:\n{}", when_applied_text(game_mode, changes.len()), listed(&lines))
}

/// Adds the changes to the queue file. Returns the message to show.
pub fn queue_changes(path: &Path, changes: Vec<QueuedChange>, game_mode: bool) -> Result<String, String> {
    if changes.is_empty() {
        return Err("Nothing is selected.".into());
    }
    let loaded = load(path);
    let mut queue = loaded.queue;
    let added = changes.len();
    for change in changes {
        queue.upsert(change);
    }
    save(path, &queue).map_err(|e| format!("Could not save the queue ({}): {e}\nNothing was queued.", path.display()))?;
    let mut text = format!("Queued {} ({} waiting in all).\n\n{}", plural(added, "change", "changes"), queue.len(), when_applied_text(game_mode, queue.len()));
    if let Some(problem) = loaded.problem {
        text = format!("{problem}\n\n{text}");
    }
    Ok(text)
}

// ---------------------------------------------------------------------------------------------
// Managing the queue
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManageLine {
    Entry(usize),
    Clear,
    Back,
}

pub fn manage_lines(queue: &SteamQueue) -> Vec<ManageLine> {
    let mut lines: Vec<ManageLine> = (0..queue.len()).map(ManageLine::Entry).collect();
    if !queue.is_empty() {
        lines.push(ManageLine::Clear);
    }
    lines.push(ManageLine::Back);
    lines
}

pub fn manage_labels(queue: &SteamQueue) -> Vec<String> {
    let several_accounts = queue.accounts().len() > 1;
    manage_lines(queue)
        .into_iter()
        .map(|line| match line {
            ManageLine::Entry(i) => {
                let e = &queue.entries[i];
                let account = if several_accounts { format!(", account {}", e.account_id) } else { String::new() };
                format!("Remove: {} ({}{account})", e.name, e.kind.as_str())
            }
            ManageLine::Clear => format!("Clear the queue ({})...", plural(queue.len(), "change", "changes")),
            ManageLine::Back => "Back".to_string(),
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Applying
// ---------------------------------------------------------------------------------------------

/// What applying needs from the app, so tests can pass fixtures.
pub struct ApplyInputs<'a> {
    pub home: &'a Path,
    pub library_dir: &'a Path,
    pub catalog: &'a [Port],
    pub favorite_exe: &'a dyn Fn(&str) -> Option<String>,
    /// Where catalog images are cached; `None` places no artwork.
    pub cache_dir: Option<PathBuf>,
    pub settings: &'a ForkSettings,
}

#[derive(Debug)]
pub struct QueueApplyOutcome {
    pub text: String,
    /// What is still queued: entries of an account whose write was refused or failed.
    pub remaining: SteamQueue,
    pub wrote: bool,
    /// The write was refused or failed (Steam came back, the file could not be read, ...).
    pub refused: bool,
}

/// Applies every queued change, one write per Steam account. An entry that is already in Steam counts as
/// done; one that cannot be applied any more (port left the catalog, uninstalled, blocked, a name-only
/// match that was not approved) is reported and dropped. If a write is refused (Steam started again) or
/// fails, that account's entries and those of any later account stay queued.
pub fn apply_queue(queue: &SteamQueue, inputs: &ApplyInputs, steam: &dyn SteamProcessCheck, now: SystemTime) -> QueueApplyOutcome {
    let mut remaining = SteamQueue::default();
    let mut sections: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut already: Vec<String> = Vec::new();
    let mut wrote = false;
    let mut refused = false;

    for account_id in queue.accounts() {
        let entries: Vec<&QueuedChange> = queue.entries.iter().filter(|e| e.account_id == account_id).collect();
        if refused {
            remaining.entries.extend(entries.into_iter().cloned());
            continue;
        }
        let mut screen = SteamScreen::load(inputs.home, inputs.library_dir, inputs.catalog, inputs.settings, inputs.favorite_exe);
        screen.cache_dir = inputs.cache_dir.clone();
        let Some(index) = screen.accounts.iter().position(|a| a.account_id == account_id) else {
            skipped.extend(entries.iter().map(|e| format!("  {}: Steam account {account_id} is not on this device any more", e.name)));
            continue;
        };
        screen.set_account(index, inputs.library_dir, inputs.catalog, inputs.favorite_exe);
        if let Some(e) = &screen.load_error {
            sections.push(format!("Could not read shortcuts.vdf for account {account_id}: {e}"));
            remaining.entries.extend(entries.into_iter().cloned());
            refused = true;
            continue;
        }
        for row in &mut screen.rows {
            row.selected = false;
        }
        let mut no_art: Vec<String> = Vec::new();
        let mut selected = 0;
        for entry in &entries {
            if !inputs.catalog.iter().any(|p| p.key() == entry.port_key) {
                skipped.push(format!("  {}: no longer in the catalog", entry.name));
                continue;
            }
            let Some(row) = screen.rows.iter_mut().find(|r| r.key == entry.port_key) else {
                skipped.push(format!("  {}: no longer installed", entry.name));
                continue;
            };
            match (&row.wanted, row.plan.as_ref().map(|p| &p.action)) {
                (Err(reason), _) => skipped.push(format!("  {}: {reason}", entry.name)),
                (_, Some(Action::Blocked(reason))) => skipped.push(format!("  {}: {reason}", entry.name)),
                (_, Some(Action::UpToDate { .. })) => already.push(format!("  {}", entry.name)),
                (_, Some(Action::Update { by_name: true, .. })) if !entry.repoint_by_name => skipped.push(format!(
                    "  {}: now matches another program's shortcut by name only; check it on the Add to Steam screen",
                    entry.name
                )),
                (_, Some(_)) => {
                    row.selected = true;
                    selected += 1;
                    if entry.art == ArtChoice::None {
                        no_art.push(entry.port_key.clone());
                    }
                }
                (_, None) => skipped.push(format!("  {}: skipped", entry.name)),
            }
        }
        if selected == 0 {
            continue;
        }
        match screens::apply_selected_with(&screen, steam, inputs.settings, now, &no_art) {
            Ok(done) => {
                wrote |= done.wrote;
                sections.push(done.text);
            }
            Err(message) => {
                sections.push(message);
                remaining.entries.extend(entries.into_iter().cloned());
                refused = true;
            }
        }
    }

    let mut text = String::new();
    if refused {
        text.push_str(&format!("{} still queued.\n\n", plural(remaining.len(), "change is", "changes are")));
    }
    text.push_str(&sections.join("\n\n"));
    if !already.is_empty() {
        push_block(&mut text, &format!("Already in Steam, nothing to do ({}):\n{}", already.len(), listed(&already)));
    }
    if !skipped.is_empty() {
        push_block(&mut text, &format!("Dropped from the queue ({}):\n{}", skipped.len(), listed(&skipped)));
    }
    if text.is_empty() {
        text.push_str("Nothing was queued.");
    }
    QueueApplyOutcome { text, remaining, wrote, refused }
}

fn push_block(text: &mut String, block: &str) {
    if !text.is_empty() {
        text.push_str("\n\n");
    }
    text.push_str(block);
}

/// Loads, applies and saves what is left. The caller shows `text`.
pub fn apply_queue_file(path: &Path, inputs: &ApplyInputs, steam: &dyn SteamProcessCheck, now: SystemTime) -> QueueApplyOutcome {
    let loaded = load(path);
    let mut outcome = apply_queue(&loaded.queue, inputs, steam, now);
    // An unchanged queue is not rewritten, so a refusal leaves the file exactly as it was.
    if outcome.remaining != loaded.queue {
        if let Err(e) = save(path, &outcome.remaining) {
            push_block(&mut outcome.text, &format!("Could not update the queue file ({e}); applied changes may be offered again (they would be found already in Steam)."));
        }
    }
    if let Some(problem) = loaded.problem {
        outcome.text = format!("{problem}\n\n{}", outcome.text);
    }
    outcome
}

// ---------------------------------------------------------------------------------------------
// Watching for Steam to close
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchAction {
    Nothing,
    Offer,
}

/// One look by the startup/30-second check. `offered` remembers that the offer was made during this
/// stretch of Steam being closed, so a declined offer is not repeated every 30 seconds; it comes back
/// once Steam has been seen running and then closed again (or when the queue empties and refills).
/// The caller skips the look entirely while another dialog is open.
pub fn watch_step(queue_len: usize, steam: &io::Result<Vec<String>>, offered: &mut bool) -> WatchAction {
    if queue_len == 0 {
        *offered = false;
        return WatchAction::Nothing;
    }
    match steam {
        Ok(procs) if procs.is_empty() => {
            if *offered {
                WatchAction::Nothing
            } else {
                *offered = true;
                WatchAction::Offer
            }
        }
        Ok(_) => {
            *offered = false;
            WatchAction::Nothing
        }
        Err(_) => WatchAction::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::super::backups;
    use super::super::shortcuts;
    use super::super::store::ShortcutsFile;
    use super::super::vdf::fixtures::*;
    use super::*;
    use crate::core::models::port_from_value;
    use std::cell::Cell;

    fn temp_dir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("dpm_steam_queue_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn no_steam() -> io::Result<Vec<String>> {
        Ok(Vec::new())
    }

    fn running() -> io::Result<Vec<String>> {
        Ok(vec!["77 steam".into()])
    }

    fn catalog() -> Vec<Port> {
        [
            json!({"name": "Ship of Harkinian", "folder": "SoH", "source": "https://github.com/a/soh", "executable": "soh.appimage"}),
            json!({"name": "Dusklight", "folder": "dusklight", "source": "https://github.com/a/dusk", "executable": "dusk"}),
        ]
        .iter()
        .map(|v| port_from_value(v).unwrap())
        .collect()
    }

    fn put_exe(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// A fake home: Steam account 1234 with a hand-made "Ship of Harkinian" at an old path, and a library
    /// with Ship of Harkinian and Dusklight installed. Nothing here is the real Steam folder.
    struct Fixture {
        home: PathBuf,
        library: PathBuf,
        vdf: PathBuf,
        queue: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Fixture {
            let home = temp_dir(name);
            let user = home.join(".local/share/Steam/userdata/1234");
            fs::create_dir_all(user.join("config")).unwrap();
            let vdf = user.join("config").join("shortcuts.vdf");
            fs::write(&vdf, file_of(&[steam_entry(0, 0xAAAA_0001, "Ship of Harkinian", "\"/old/soh.appimage\"", "\"/old/\"", &[])])).unwrap();
            let library = home.join("App").join("Library");
            put_exe(&library.join("SoH").join("soh.appimage"));
            put_exe(&library.join("dusklight").join("dusk"));
            let queue = home.join("App").join(FILE_NAME);
            Fixture { home, library, vdf, queue }
        }

        fn settings(&self) -> ForkSettings {
            ForkSettings::load(&self.home.join("App").join("fork-settings.json"))
        }

        fn screen(&self, catalog: &[Port]) -> SteamScreen {
            SteamScreen::load(&self.home, &self.library, catalog, &self.settings(), &|_| None)
        }

        fn apply(&self, catalog: &[Port], steam: &dyn SteamProcessCheck) -> QueueApplyOutcome {
            let settings = self.settings();
            let inputs = ApplyInputs { home: &self.home, library_dir: &self.library, catalog, favorite_exe: &|_| None, cache_dir: None, settings: &settings };
            apply_queue_file(&self.queue, &inputs, steam, SystemTime::now())
        }

        fn saved(&self) -> Vec<shortcuts::Shortcut> {
            shortcuts::list(&ShortcutsFile::load(&self.vdf).unwrap().doc)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.home);
        }
    }

    fn change(account_id: u32, key: &str, kind: QueuedKind) -> QueuedChange {
        QueuedChange {
            account_id,
            port_key: key.into(),
            name: key.rsplit('/').next().unwrap().into(),
            kind,
            repoint_by_name: false,
            art: ArtChoice::Catalog,
            queued_at: "20261021-120000".into(),
        }
    }

    // -- the file -------------------------------------------------------------------------------

    #[test]
    fn the_queue_round_trips_through_the_file() {
        let dir = temp_dir("roundtrip");
        let path = dir.join(FILE_NAME);
        let mut queue = SteamQueue::default();
        queue.upsert(change(1234, "a/dusk", QueuedKind::Add));
        let mut soh = change(1234, "a/soh", QueuedKind::Update);
        soh.repoint_by_name = true;
        soh.art = ArtChoice::None;
        queue.upsert(soh);
        queue.upsert(change(5678, "a/dusk", QueuedKind::Add));
        save(&path, &queue).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"version\": 1"), "{text}");
        let loaded = load(&path);
        assert!(loaded.problem.is_none() && loaded.set_aside.is_none());
        assert_eq!(loaded.queue, queue);
        assert!(!dir.join(format!("{FILE_NAME}.dpm-tmp")).exists(), "no temp file left behind");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn queuing_the_same_port_again_replaces_it_and_an_empty_queue_removes_the_file() {
        let dir = temp_dir("upsert");
        let path = dir.join(FILE_NAME);
        let mut queue = SteamQueue::default();
        queue.upsert(change(1234, "a/dusk", QueuedKind::Add));
        queue.upsert(change(1234, "a/soh", QueuedKind::Add));
        queue.upsert(change(1234, "a/dusk", QueuedKind::Update));
        assert_eq!(queue.len(), 2);
        assert_eq!((queue.entries[0].port_key.as_str(), queue.entries[0].kind), ("a/dusk", QueuedKind::Update));
        save(&path, &queue).unwrap();
        assert!(queue.remove(5).is_none());
        queue.remove(0);
        queue.remove(0);
        save(&path, &queue).unwrap();
        assert!(!path.exists());
        save(&path, &queue).unwrap();
        assert!(load(&path).queue.is_empty() && load(&path).problem.is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_corrupt_queue_file_is_kept_aside_and_the_user_is_told() {
        let dir = temp_dir("corrupt");
        let path = dir.join(FILE_NAME);
        for (bad, why) in [
            (&b"{\"version\": 1, \"entries\": [ {\"port_k"[..], "not valid JSON"),
            (&b"\xFF\xFE garbage"[..], "not text"),
            (&b"{\"version\": 9, \"entries\": []}"[..], "version 9"),
            (&b"{\"entries\": []}"[..], "no version"),
            (&b"{\"version\": 1, \"entries\": [{\"account_id\": 1, \"port_key\": \"x\", \"kind\": \"delete\"}]}"[..], "no usable kind"),
            (&b"[1, 2]"[..], "not a JSON object"),
        ] {
            fs::write(&path, bad).unwrap();
            let loaded = load(&path);
            assert!(loaded.queue.is_empty());
            let problem = loaded.problem.unwrap();
            assert!(problem.contains(why), "{why}: {problem}");
            assert!(problem.contains("kept as steam-queue.json.corrupt-"), "{problem}");
            let aside = loaded.set_aside.unwrap();
            assert_eq!(fs::read(&aside).unwrap(), bad, "the bad file is kept byte for byte");
            assert!(!path.exists());
            // Loading again is quiet: the bad file is out of the way.
            assert!(load(&path).problem.is_none());
        }
        let kept = fs::read_dir(&dir).unwrap().filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().contains(".corrupt-")).count();
        assert_eq!(kept, 6, "no set-aside copy overwrote another");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn queuing_on_top_of_a_corrupt_file_keeps_it_aside_and_says_so() {
        let f = Fixture::new("queue_over_corrupt");
        fs::create_dir_all(f.queue.parent().unwrap()).unwrap();
        fs::write(&f.queue, "not json").unwrap();
        let text = queue_changes(&f.queue, vec![change(1234, "a/dusk", QueuedKind::Add)], true).unwrap();
        assert!(text.starts_with("The queue of Steam changes could not be read"), "{text}");
        assert!(text.contains("Queued 1 change (1 waiting in all)"), "{text}");
        assert_eq!(load(&f.queue).queue.len(), 1);
    }

    // -- queuing from the screen ------------------------------------------------------------------

    #[test]
    fn queuing_from_the_screen_records_ports_account_and_choices() {
        let f = Fixture::new("from_screen");
        let mut screen = f.screen(&catalog());
        screen.cache_dir = Some(f.home.join("cache"));
        assert_eq!(changes_from_screen(&screen, "T").len(), 1, "only the add is on by default");
        // Switch on the name-only repoint of the hand-made Ship of Harkinian.
        let soh = screen.rows.iter().position(|r| r.key == "a/soh").unwrap();
        screen.toggle(soh);
        let changes = changes_from_screen(&screen, "T");
        assert_eq!(changes.len(), 2);
        let dusk = changes.iter().find(|c| c.port_key == "a/dusk").unwrap();
        assert_eq!((dusk.account_id, dusk.kind, dusk.repoint_by_name, dusk.art), (1234, QueuedKind::Add, false, ArtChoice::Catalog));
        let soh = changes.iter().find(|c| c.port_key == "a/soh").unwrap();
        assert_eq!((soh.kind, soh.repoint_by_name), (QueuedKind::Update, true));

        let before = fs::read(&f.vdf).unwrap();
        let text = queue_changes(&f.queue, changes, true).unwrap();
        assert!(text.contains("Queued 2 changes"), "{text}");
        assert!(text.contains("can't be closed in Game Mode") && text.contains("next time you open this app in Desktop Mode"), "{text}");
        assert_eq!(fs::read(&f.vdf).unwrap(), before, "queuing never touches shortcuts.vdf");
        assert_eq!(load(&f.queue).queue.len(), 2);
    }

    #[test]
    fn the_wording_follows_the_mode() {
        let game = when_applied_text(true, 1);
        assert!(game.contains("Steam can't be closed in Game Mode"));
        assert!(game.contains("The change will apply next time you open this app in Desktop Mode with Steam closed"), "{game}");
        assert!(when_applied_text(true, 3).contains("The 3 changes will apply next time"));
        let desk = when_applied_text(false, 2);
        assert!(desk.contains("once Steam is closed (Steam menu, Exit)") && desk.contains("within 30 seconds"), "{desk}");
        assert!(!desk.contains("Game Mode"));
        assert!(is_game_mode(Some("gamescope"), None));
        assert!(is_game_mode(Some("KDE"), Some("gamescope-0")));
        assert!(!is_game_mode(Some("KDE"), None));
        assert!(!is_game_mode(None, Some("")));
    }

    // -- applying ---------------------------------------------------------------------------------

    fn queue_two(f: &Fixture) {
        let mut soh = change(1234, "a/soh", QueuedKind::Update);
        soh.repoint_by_name = true;
        let mut q = SteamQueue::default();
        q.upsert(change(1234, "a/dusk", QueuedKind::Add));
        q.upsert(soh);
        save(&f.queue, &q).unwrap();
    }

    #[test]
    fn applying_with_steam_closed_writes_through_the_safe_path_and_empties_the_queue() {
        let f = Fixture::new("apply");
        queue_two(&f);
        let before = fs::read(&f.vdf).unwrap();
        let out = f.apply(&catalog(), &no_steam);
        assert!(out.wrote && !out.refused, "{}", out.text);
        assert!(out.text.contains("Added 1, updated 1"), "{}", out.text);
        assert!(out.text.contains("Backup of the old list: shortcuts.vdf.bak-"), "{}", out.text);
        assert!(out.remaining.is_empty());
        assert!(!f.queue.exists(), "the queue file is gone once applied");
        let list = backups::list_backups(&f.vdf).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(fs::read(&list[0].path).unwrap(), before, "verified backup of the old file");
        let saved = f.saved();
        assert_eq!(saved.len(), 2);
        let soh = saved.iter().find(|s| s.app_name == "Ship of Harkinian").unwrap();
        assert_eq!(soh.app_id, Some(0xAAAA_0001), "repointed in place, app ID kept");
        assert_eq!(soh.exe_path(), f.library.join("SoH").join("soh.appimage").to_str().unwrap());
    }

    #[test]
    fn applying_while_steam_runs_writes_nothing_and_keeps_the_queue() {
        let f = Fixture::new("apply_running");
        queue_two(&f);
        let (vdf, queue) = (fs::read(&f.vdf).unwrap(), fs::read(&f.queue).unwrap());
        let out = f.apply(&catalog(), &running);
        assert!(out.refused && !out.wrote);
        assert!(out.text.starts_with("2 changes are still queued."), "{}", out.text);
        assert!(out.text.contains("Still running: 77 steam"), "{}", out.text);
        assert_eq!(fs::read(&f.vdf).unwrap(), vdf);
        assert_eq!(fs::read(&f.queue).unwrap(), queue, "the queue file is untouched");
        assert!(backups::list_backups(&f.vdf).unwrap().is_empty());
    }

    /// Steam is closed for the first `clear_calls` checks and running after that.
    fn steam_after(calls: &Cell<usize>, clear_calls: usize) -> impl Fn() -> io::Result<Vec<String>> + '_ {
        move || {
            calls.set(calls.get() + 1);
            if calls.get() <= clear_calls { Ok(Vec::new()) } else { Ok(vec!["88 steamwebhelper".into()]) }
        }
    }

    #[test]
    fn steam_reappearing_before_the_write_refuses_it_and_keeps_the_queue() {
        // How many times one clean apply looks for Steam; the last look is the one right before the rename.
        let probe = Fixture::new("probe");
        queue_two(&probe);
        let calls = Cell::new(0);
        assert!(probe.apply(&catalog(), &steam_after(&calls, usize::MAX)).wrote);
        let total = calls.get();
        assert!(total >= 3, "{total}");
        drop(probe);

        // Steam back after the first look, and again only at the very last look.
        for (name, clear) in [("reappear_early", 1), ("reappear_at_rename", total - 1)] {
            let f = Fixture::new(name);
            queue_two(&f);
            let (vdf, queue) = (fs::read(&f.vdf).unwrap(), fs::read(&f.queue).unwrap());
            let calls = Cell::new(0);
            let out = f.apply(&catalog(), &steam_after(&calls, clear));
            assert!(out.refused && !out.wrote, "{name}: {}", out.text);
            assert!(out.text.contains("88 steamwebhelper"), "{name}: {}", out.text);
            assert_eq!(fs::read(&f.vdf).unwrap(), vdf, "{name}: shortcuts.vdf unchanged");
            assert_eq!(fs::read(&f.queue).unwrap(), queue, "{name}: queue kept");
            assert_eq!(out.remaining.len(), 2);
            let leftovers: Vec<_> = fs::read_dir(f.vdf.parent().unwrap())
                .unwrap()
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.contains("dpm-tmp"))
                .collect();
            assert!(leftovers.is_empty(), "{name}: {leftovers:?}");
        }
    }

    #[test]
    fn applying_the_same_queue_twice_never_duplicates_a_shortcut() {
        let f = Fixture::new("idempotent");
        queue_two(&f);
        let saved_queue = fs::read(&f.queue).unwrap();
        assert!(f.apply(&catalog(), &no_steam).wrote);
        let first = fs::read(&f.vdf).unwrap();
        // The same queue again (say the queue file could not be updated last time).
        fs::write(&f.queue, &saved_queue).unwrap();
        let out = f.apply(&catalog(), &no_steam);
        assert!(!out.wrote && !out.refused, "{}", out.text);
        assert!(out.text.contains("Already in Steam, nothing to do (2)"), "{}", out.text);
        assert_eq!(fs::read(&f.vdf).unwrap(), first, "nothing rewritten");
        assert_eq!(f.saved().len(), 2);
        assert!(!f.queue.exists());
        assert_eq!(backups::list_backups(&f.vdf).unwrap().len(), 1, "no second backup for a no-op");
    }

    #[test]
    fn a_port_queued_as_an_add_that_is_in_steam_by_then_is_updated_in_place() {
        let f = Fixture::new("update_in_place");
        // Queued as an add in Game Mode; by the time Steam is closed a shortcut for the same program exists
        // (added by hand, say) with another start folder.
        let dusk = f.library.join("dusklight").join("dusk");
        let entry = steam_entry(0, 0xBBBB_0002, "Dusklight", &format!("\"{}\"", dusk.to_str().unwrap()), "\"/somewhere/else/\"", &["Mine"]);
        fs::write(&f.vdf, file_of(&[entry])).unwrap();
        let mut q = SteamQueue::default();
        q.upsert(change(1234, "a/dusk", QueuedKind::Add));
        save(&f.queue, &q).unwrap();
        let out = f.apply(&catalog(), &no_steam);
        assert!(out.text.contains("Added 0, updated 1"), "{}", out.text);
        let saved = f.saved();
        assert_eq!(saved.len(), 1, "not duplicated");
        assert_eq!(saved[0].app_id, Some(0xBBBB_0002));
        assert_eq!(saved[0].start_dir_path(), f.library.join("dusklight").to_str().unwrap());
        assert_eq!(saved[0].tags, ["Mine"]);
    }

    #[test]
    fn a_port_removed_from_the_catalog_since_queuing_is_reported_and_skipped() {
        let f = Fixture::new("stale");
        queue_two(&f);
        let mut q = load(&f.queue).queue;
        q.upsert(change(1234, "gone/port", QueuedKind::Add));
        save(&f.queue, &q).unwrap();
        let out = f.apply(&catalog(), &no_steam);
        assert!(out.wrote, "the others still go through: {}", out.text);
        assert!(out.text.contains("Dropped from the queue (1):\n  port: no longer in the catalog"), "{}", out.text);
        assert!(!f.queue.exists(), "the stale entry is not offered again");
        assert_eq!(f.saved().len(), 2);
    }

    #[test]
    fn uninstalled_ports_unknown_accounts_and_unapproved_name_matches_are_dropped_with_reasons() {
        let f = Fixture::new("reasons");
        let mut q = SteamQueue::default();
        // Ship of Harkinian matches the hand-made shortcut by name only, and was not approved for that.
        q.upsert(change(1234, "a/soh", QueuedKind::Add));
        q.upsert(change(1234, "a/dusk", QueuedKind::Add));
        q.upsert(change(999, "a/dusk", QueuedKind::Add));
        save(&f.queue, &q).unwrap();
        fs::remove_dir_all(f.library.join("dusklight")).unwrap();
        let before = fs::read(&f.vdf).unwrap();
        let out = f.apply(&catalog(), &no_steam);
        assert!(!out.wrote && !out.refused, "{}", out.text);
        assert!(out.text.contains("soh: now matches another program's shortcut by name only"), "{}", out.text);
        assert!(out.text.contains("dusk: no longer installed"), "{}", out.text);
        assert!(out.text.contains("dusk: Steam account 999 is not on this device any more"), "{}", out.text);
        assert_eq!(fs::read(&f.vdf).unwrap(), before);
        assert!(!f.queue.exists());
    }

    #[test]
    fn an_unreadable_shortcuts_file_keeps_the_queue() {
        let f = Fixture::new("garbled_vdf");
        queue_two(&f);
        fs::write(&f.vdf, b"\x00shortcuts\x00\x09bad\x00").unwrap();
        let out = f.apply(&catalog(), &no_steam);
        assert!(out.refused && !out.wrote);
        assert!(out.text.contains("Could not read shortcuts.vdf for account 1234"), "{}", out.text);
        assert_eq!(load(&f.queue).queue.len(), 2);
    }

    #[test]
    fn applying_a_corrupt_queue_does_not_crash_and_says_it_was_kept_aside() {
        let f = Fixture::new("apply_corrupt");
        fs::create_dir_all(f.queue.parent().unwrap()).unwrap();
        fs::write(&f.queue, "{ \"version\": 1, \"entries\": [").unwrap();
        let before = fs::read(&f.vdf).unwrap();
        let out = f.apply(&catalog(), &no_steam);
        assert!(!out.wrote && !out.refused);
        assert!(out.text.starts_with("The queue of Steam changes could not be read"), "{}", out.text);
        assert_eq!(fs::read(&f.vdf).unwrap(), before);
        assert!(fs::read_dir(f.queue.parent().unwrap()).unwrap().filter_map(Result::ok).any(|e| e.file_name().to_string_lossy().starts_with("steam-queue.json.corrupt-")));
    }

    #[test]
    fn art_choice_none_places_no_artwork() {
        let f = Fixture::new("art_none");
        let cache = f.home.join("cache");
        fs::create_dir_all(&cache).unwrap();
        image::RgbaImage::from_pixel(600, 900, image::Rgba([200, 30, 30, 255])).save(cache.join("dusklight.png")).unwrap();
        let settings = f.settings();
        let cat = catalog();
        let run = |art: ArtChoice, vdf_name: &str| {
            let mut q = SteamQueue::default();
            let mut c = change(1234, "a/dusk", QueuedKind::Add);
            c.art = art;
            q.upsert(c);
            save(&f.queue, &q).unwrap();
            let inputs = ApplyInputs { home: &f.home, library_dir: &f.library, catalog: &cat, favorite_exe: &|_| None, cache_dir: Some(cache.clone()), settings: &settings };
            let out = apply_queue_file(&f.queue, &inputs, &no_steam, SystemTime::now());
            assert!(out.wrote, "{vdf_name}: {}", out.text);
        };
        run(ArtChoice::None, "none");
        let grid = f.vdf.parent().unwrap().join("grid");
        assert!(!grid.exists(), "no artwork for art: none");
        // Start from an empty shortcuts file and queue the same add with catalog art.
        fs::write(&f.vdf, file_of(&[])).unwrap();
        run(ArtChoice::Catalog, "catalog");
        assert!(grid.is_dir(), "catalog art placed");
    }

    // -- watching ---------------------------------------------------------------------------------

    #[test]
    fn the_offer_comes_once_per_stretch_of_steam_being_closed() {
        let mut offered = false;
        let closed: io::Result<Vec<String>> = Ok(Vec::new());
        let up: io::Result<Vec<String>> = Ok(vec!["1 steam".into()]);
        let broken: io::Result<Vec<String>> = Err(io::Error::other("no /proc"));
        assert_eq!(watch_step(0, &closed, &mut offered), WatchAction::Nothing, "empty queue");
        assert_eq!(watch_step(2, &up, &mut offered), WatchAction::Nothing, "Game Mode: Steam up");
        assert_eq!(watch_step(2, &broken, &mut offered), WatchAction::Nothing, "cannot tell");
        assert_eq!(watch_step(2, &closed, &mut offered), WatchAction::Offer, "Steam closed");
        assert_eq!(watch_step(2, &closed, &mut offered), WatchAction::Nothing, "declined: not every 30 s");
        assert_eq!(watch_step(2, &up, &mut offered), WatchAction::Nothing);
        assert_eq!(watch_step(2, &closed, &mut offered), WatchAction::Offer, "closed again");
        assert_eq!(watch_step(0, &closed, &mut offered), WatchAction::Nothing);
        assert_eq!(watch_step(1, &closed, &mut offered), WatchAction::Offer, "queue refilled");
    }

    #[test]
    fn manage_labels_list_entries_then_clear_and_back() {
        let mut q = SteamQueue::default();
        assert_eq!(manage_labels(&q), ["Back"]);
        q.upsert(change(1234, "a/dusk", QueuedKind::Add));
        q.upsert(change(1234, "a/soh", QueuedKind::Update));
        assert_eq!(manage_labels(&q), ["Remove: dusk (add)", "Remove: soh (update)", "Clear the queue (2 changes)...", "Back"]);
        q.upsert(change(5678, "a/dusk", QueuedKind::Add));
        assert_eq!(manage_labels(&q)[2], "Remove: dusk (add, account 5678)");
        assert_eq!(manage_lines(&q)[3], ManageLine::Clear);
        assert!(offer_text(&q).starts_with("3 Steam changes queued from Game Mode: apply now?"));
    }
}
