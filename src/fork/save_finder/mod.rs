//! Save finder: what the app knows about where a port keeps its saves, and a way for the user to fix it.
//!
//! Recomps that do not follow the catalog's save field (Severed Chains added from the Deck-ready list has
//! none; Dusklight's names only a Windows path) leave the save features blind: the update backup, Save Backup,
//! Restore saves and the uninstall preserve all skip a port whose save folder is unknown. This module
//!
//! - says, per installed port, whether its saves are found ([`state_of`]);
//! - stores the user's answer as a per-port override in `fork-settings.json` ([`Override`], key
//!   `save_overrides`, keyed by the port's catalog folder) and applies it to the catalog when it loads
//!   ([`apply_overrides`]), so every save feature reads the same value through `platform_resolve::save_location`;
//! - searches likely places, read only, for folders that look like this port's saves ([`search`]);
//! - holds the pages of the wizard as plain data ([`view`]); `src/app/save_finder_screen.rs` draws them.
//!
//! Nothing is moved, copied or written into a save folder here. A folder is stored in a portable form: a
//! folder inside the game folder as a relative path (it follows the game when the ports folder moves), a
//! folder in the home folder as `%HOME%/...`, anything else as the absolute path.

pub mod search;
pub mod view;

use crate::core::models::Port;
use crate::core::path_safety::is_within;
use crate::core::platform_resolve::{save_location, SaveLocation};
use crate::fork::save_restore::{save_targets, Target};
use crate::fork::settings::ForkSettings;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const KEY: &str = "save_overrides";
const KEY_PATH: &str = "path";
const KEY_NO_SAVES: &str = "no_saves";

/// How deep and how many entries [`has_files`] looks at before saying "yes" or giving up.
const PROBE_DEPTH: usize = 4;
const PROBE_ENTRIES: usize = 2000;

/// What the user told the app about one port's saves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Override {
    /// The saves are in this folder, in the stored (portable) form.
    Folder(String),
    /// This game has no saves; remembered so the warning stays away.
    NoSaves,
}

/// The overrides in the settings, by catalog folder. An entry that is neither a path nor "no saves" is ignored.
pub fn overrides(settings: &ForkSettings) -> BTreeMap<String, Override> {
    let mut out = BTreeMap::new();
    let Some(Value::Object(map)) = settings.get(KEY) else { return out };
    for (folder, entry) in map {
        let Some(entry) = entry.as_object() else { continue };
        if let Some(path) = entry.get(KEY_PATH).and_then(Value::as_str).filter(|p| !p.is_empty()) {
            out.insert(folder.clone(), Override::Folder(path.to_string()));
        } else if entry.get(KEY_NO_SAVES).and_then(Value::as_bool) == Some(true) {
            out.insert(folder.clone(), Override::NoSaves);
        }
    }
    out
}

fn write(settings: &mut ForkSettings, map: Map<String, Value>) -> io::Result<()> {
    if map.is_empty() {
        settings.remove(KEY)
    } else {
        settings.set(KEY, Value::Object(map))
    }
}

fn current_map(settings: &ForkSettings) -> Map<String, Value> {
    match settings.get(KEY) {
        Some(Value::Object(map)) => map.clone(),
        _ => Map::new(),
    }
}

pub fn set_override(settings: &mut ForkSettings, folder: &str, choice: &Override) -> io::Result<()> {
    let mut map = current_map(settings);
    let entry = match choice {
        Override::Folder(path) => json!({ KEY_PATH: path }),
        Override::NoSaves => json!({ KEY_NO_SAVES: true }),
    };
    map.insert(folder.to_string(), entry);
    write(settings, map)
}

pub fn clear_override(settings: &mut ForkSettings, folder: &str) -> io::Result<()> {
    let mut map = current_map(settings);
    if map.remove(folder).is_none() {
        return Ok(());
    }
    write(settings, map)
}

/// Puts each port's override into the port, so the save features see it. A folder replaces both save fields
/// (the user named where all the saves are); "no saves" empties both. Ports without an override are untouched.
pub fn apply_overrides(catalog: &mut [Port], map: &BTreeMap<String, Override>) {
    if map.is_empty() {
        return;
    }
    for port in catalog.iter_mut() {
        match map.get(&port.folder) {
            Some(Override::Folder(path)) => {
                port.save = Some(Value::String(path.clone()));
                port.save2 = None;
            }
            Some(Override::NoSaves) => {
                port.save = None;
                port.save2 = None;
            }
            None => {}
        }
    }
}

/// [`apply_overrides`] with the overrides read from the settings file.
pub fn apply_from_file(catalog: &mut [Port], settings_path: &Path) {
    apply_overrides(catalog, &overrides(&ForkSettings::load(settings_path)));
}

/// The catalog entry for `folder` as it reads from the files on disk, overrides applied: used after the user sets
/// or clears an override, so a cleared one brings back what the catalog says.
pub fn fresh_entry(config_dir: &Path, folder: &str) -> Option<Port> {
    let mut catalog = if config_dir.join("ports.json").exists() {
        crate::core::config::load_config(&config_dir.join("ports.json")).ok()?
    } else {
        Vec::new()
    };
    catalog = crate::core::config::merge_local_catalog(catalog, crate::core::config::load_local_config(&config_dir.join("ports.local.json")));
    apply_from_file(&mut catalog, &config_dir.join(crate::fork::settings::FILE_NAME));
    catalog.into_iter().find(|p| p.folder == folder)
}

// -- State ---------------------------------------------------------------------------------------------

/// What the app knows about a port's saves right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveState {
    /// A known folder that holds files.
    Found,
    /// The user said this game has no saves.
    NoSaves,
    /// A known folder that exists and holds nothing.
    Empty,
    /// A known folder that does not exist.
    Missing,
    /// No save folder is known on Linux (no save field, or only a Windows path).
    Unknown,
}

impl SaveState {
    /// Whether the port shows the "Saves not found" badge. An unknown folder always does: backups skip such a
    /// port, and saves of a game that was installed some other way may be at risk. A known folder that is empty
    /// or missing does only once the game was played, since a game that never ran has nothing to save yet.
    pub fn warns(self, played: bool) -> bool {
        match self {
            SaveState::Unknown => true,
            SaveState::Empty | SaveState::Missing => played,
            SaveState::Found | SaveState::NoSaves => false,
        }
    }
}

/// Whether `dir` holds at least one regular file, looking a few levels down at most. Links are not followed.
pub fn has_files(dir: &Path) -> bool {
    let mut seen = 0usize;
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > PROBE_ENTRIES {
                return false;
            }
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_file() {
                return true;
            }
            if kind.is_dir() && depth + 1 < PROBE_DEPTH {
                stack.push((entry.path(), depth + 1));
            }
        }
    }
    false
}

/// The state of an installed port whose game folder is `game_dir`. `choice` is the port's override, if any.
pub fn state_of(port: &Port, game_dir: &Path, choice: Option<&Override>) -> SaveState {
    if matches!(choice, Some(Override::NoSaves)) {
        return SaveState::NoSaves;
    }
    let (mut found, mut empty, mut missing) = (false, false, false);
    for value in [port.save.as_ref(), port.save2.as_ref()].into_iter().flatten() {
        let SaveLocation::Known(path) = save_location(value, game_dir) else { continue };
        // The game folder itself (or a folder holding it) is not a save folder: no feature backs it up.
        if is_within(game_dir, &path) {
            continue;
        }
        if has_files(&path) {
            found = true;
        } else if path.is_dir() {
            empty = true;
        } else {
            missing = true;
        }
    }
    if found {
        SaveState::Found
    } else if empty {
        SaveState::Empty
    } else if missing {
        SaveState::Missing
    } else {
        SaveState::Unknown
    }
}

/// The line under the Info dialog's status lines, or `None` when there is nothing to say.
pub fn info_line(state: SaveState, choice: Option<&Override>, played: bool) -> Option<String> {
    if !state.warns(played) && !matches!(state, SaveState::NoSaves) && choice.is_none() {
        return None;
    }
    Some(
        match (state, choice) {
            (SaveState::NoSaves, _) => "Saves: you said this game has none",
            (SaveState::Unknown, _) => "Saves not found: unknown on Linux",
            (SaveState::Empty, _) => "Saves not found: folder is empty",
            (SaveState::Missing, _) => "Saves not found: folder is missing",
            (SaveState::Found, Some(Override::Folder(_))) => "Saves: using the folder you chose",
            (SaveState::Found, _) => return None,
        }
        .to_string(),
    )
}

// -- Storing a chosen folder ----------------------------------------------------------------------------

fn plain(rel: &Path) -> String {
    rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

/// The form a chosen folder is stored in: relative to the game folder when inside it, `%HOME%/...` when inside
/// the home folder, else absolute. A name holding `%` in the stored part cannot be stored (the resolver would
/// read it as a variable).
pub fn portable_form(chosen: &Path, game_dir: &Path, home: Option<&Path>) -> Result<String, String> {
    let stored = if chosen == game_dir {
        ".".to_string()
    } else if chosen.starts_with(game_dir) {
        plain(chosen.strip_prefix(game_dir).map_err(|e| e.to_string())?)
    } else if let Some(home) = home.filter(|h| chosen.starts_with(h) && chosen != *h) {
        format!("%HOME%/{}", plain(chosen.strip_prefix(home).map_err(|e| e.to_string())?))
    } else if chosen.is_absolute() {
        chosen.to_string_lossy().into_owned()
    } else {
        return Err("That is not a full folder path.".into());
    };
    let variable_part = stored.strip_prefix("%HOME%").unwrap_or(&stored);
    if variable_part.contains('%') {
        return Err("That folder has a % in its name, which the app cannot store. Rename it, or pick another folder.".into());
    }
    Ok(stored)
}

/// The most a save folder may hold before the finder refuses it as "very large for saves".
pub const MAX_SAVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_SIZE_ENTRIES: usize = 200_000;

/// The size of a folder's files, and whether it was cut short because it is past [`MAX_SAVE_BYTES`] or huge.
pub fn folder_size(dir: &Path) -> (u64, bool) {
    crate::fork::save_restore::tree_size_capped(dir, MAX_SAVE_BYTES, MAX_SIZE_ENTRIES)
}

/// The bare reason (see `refusal`) a link at `chosen` cannot be used, from where it leads.
fn link_reason(chosen: &Path, game_dir: &Path, saves_backup_dir: &Path, home: Option<&Path>) -> Option<String> {
    let real = fs::canonicalize(chosen).ok().filter(|r| r != chosen)?;
    let game_real = fs::canonicalize(game_dir).unwrap_or_else(|_| game_dir.to_path_buf());
    let home_real = home.map(|h| fs::canonicalize(h).unwrap_or_else(|_| h.to_path_buf()));
    crate::fork::save_restore::folder_problem(&real, &game_real, game_real.parent(), saves_backup_dir, home_real.as_deref())
        .or_else(|| crate::fork::save_restore::folder_problem(&real, game_dir, game_dir.parent(), saves_backup_dir, home))
}

const BROAD_MESSAGE: &str = "That folder holds many apps' files, not this game's. Open the game's own folder inside it.";

fn refusal(why: &str) -> String {
    if why == crate::fork::save_restore::broad::BROAD_REASON {
        BROAD_MESSAGE.to_string()
    } else {
        format!("That folder cannot be used for saves: it {why}.")
    }
}

/// Checks a folder the user chose and gives the override to store, or the reason it cannot be used. The same
/// guards as Restore saves apply (not the game folder, not the home folder, not near the top of the drive, not
/// inside Saves Backup), so a choice that would be refused later is refused now.
pub fn check_choice(port: &Port, chosen: &Path, game_dir: &Path, saves_backup_dir: &Path, home: Option<&Path>) -> Result<Override, String> {
    if !chosen.is_dir() {
        return Err("That is not a folder.".into());
    }
    let stored = portable_form(chosen, game_dir, home)?;
    let mut probe = port.clone();
    // The folder is judged as an absolute path (what the stored form resolves to), whatever HOME is.
    probe.save = Some(Value::String(chosen.to_string_lossy().into_owned()));
    probe.save2 = None;
    match save_targets(&probe, game_dir, saves_backup_dir, home).into_iter().next().map(|t| t.target) {
        Some(Target::Folder(_)) => {}
        Some(Target::Refused(why)) => return Err(refusal(&why)),
        _ => return Err("That folder cannot be used for saves.".into()),
    }
    // A link is judged by where it leads, as a restore would.
    if let Some(why) = link_reason(chosen, game_dir, saves_backup_dir, home) {
        return Err(refusal(&why));
    }
    if folder_size(chosen).1 {
        return Err("That folder is very large for saves. Open the game's own save folder inside it.".into());
    }
    Ok(Override::Folder(stored))
}

/// The folder an override points at, for showing it and for the folder picker to start in.
pub fn folder_of(choice: &Override, game_dir: &Path) -> Option<PathBuf> {
    match choice {
        Override::Folder(value) => match save_location(&Value::String(value.clone()), game_dir) {
            SaveLocation::Known(path) => Some(path),
            _ => None,
        },
        Override::NoSaves => None,
    }
}

#[cfg(test)]
mod tests;
