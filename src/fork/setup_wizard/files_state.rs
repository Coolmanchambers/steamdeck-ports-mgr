//! Whether one port has its game files, the one answer Home, the setup wizard and the game files counts share.
//!
//! A port's entry can name its game files three ways: by file name (`oot.z64`: checkable against the port's
//! folder), by something a scan can use (a checksum or disc serial with no file name), or only by game title
//! (Dusklight: `Legend of Zelda, The - Twilight Princess PAL or NTSC`). A title cannot be checked against a file, so
//! for such a slot the app asks two cheap questions instead: was a connection recorded for it, and does a game or disc
//! file in the port's folder carry every significant word of the title. With neither, the state is
//! [`FilesState::Unverifiable`]: something to do, not a failure.
//!
//! What the move, verify and check-after code treats as required (`GameFiles::required`, `required_not_found`,
//! `must_hash`) is not changed by this: it still ignores title-only slots.

use crate::core::models::Port;
use crate::fork::port_mover::game_files::{files_by_name, required_not_found, GameFiles};
use crate::fork::rom_library::requirements::generated_archives;
use crate::fork::rom_library::system::is_rom_extension;
use crate::fork::rom_library::{load_local_entries, RomSpec, Slot};
use std::path::{Path, PathBuf};

/// Whether a port has its game files, as far as the app can tell from the port's folder and the game files records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesState {
    /// Every file it needs is in its folder or connected (or it made its own game archive).
    Have,
    /// A file the catalog names (by file name) is neither in the folder nor connected: the game will not start.
    Missing,
    /// The catalog names only a game title, and nothing in the folder or the records says it is there.
    Unverifiable,
}

/// Words in a title that say nothing about the game: regions, formats, filler.
const TITLE_FILLER: [&str; 24] = [
    "pal", "ntsc", "ntscu", "ntscj", "usa", "us", "europe", "eur", "japan", "jpn", "world", "australia", "korea", "or", "and", "the", "of", "iso", "disc",
    "dump", "image", "version", "region", "an",
];

/// The words of a game title worth looking for in a file name: lowercase, no parenthesised tags, no region or filler
/// words, no one-letter words ("Majora's" is `majoras`).
pub fn significant_words(title: &str) -> Vec<String> {
    let mut plain = String::new();
    let mut depth = 0usize;
    for c in title.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            '\'' | '\u{2019}' => {}
            c if depth == 0 => plain.push(c),
            _ => {}
        }
    }
    plain
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 2 && !TITLE_FILLER.contains(&w.as_str()))
        .collect()
}

fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// A game or disc file (by extension) within a few levels of the port's folder whose name has every significant word
/// of one of the slot's titles.
fn title_in_folder(slot: &Slot, present: &[(String, PathBuf)]) -> bool {
    slot.alternatives.iter().filter(|a| a.title_only).any(|alt| {
        let words = significant_words(&alt.display);
        !words.is_empty()
            && present.iter().any(|(name, path)| {
                let Some((stem, ext)) = name.rsplit_once('.') else { return false };
                let key = squash(stem);
                is_rom_extension(ext) && words.iter().all(|w| key.contains(&squash(w))) && std::fs::metadata(path).is_ok()
            })
    })
}

fn state_for(spec: &RomSpec, files: &GameFiles, dir: &Path) -> FilesState {
    let connected = |s: &Slot| files.mappings.iter().any(|m| m.slot == s.id());
    let needed: Vec<&Slot> = spec.requirement.slots.iter().filter(|s| !s.optional).collect();
    let vague: Vec<&Slot> = needed.iter().copied().filter(|s| s.is_vague()).collect();
    if files.required.is_empty() && vague.is_empty() {
        // Nothing here can be checked by file name: the port has its game only when a connection says so.
        let unnamed: Vec<&Slot> = needed.iter().copied().filter(|s| !s.is_vague()).collect();
        return if !unnamed.is_empty() && unnamed.iter().all(|s| connected(s)) { FilesState::Have } else { FilesState::Missing };
    }
    if !required_not_found(dir, files).is_empty() {
        return FilesState::Missing;
    }
    let mut present = Vec::new();
    files_by_name(dir, 3, &mut present);
    if vague.iter().all(|s| connected(s) || title_in_folder(s, &present)) {
        FilesState::Have
    } else {
        FilesState::Unverifiable
    }
}

/// One port's state. `port_dir` is the port's folder, `config_dir` the app's (the game files records and
/// `ports.local.json` are read from it).
pub fn files_state(port: &Port, port_dir: &Path, config_dir: &Path) -> FilesState {
    if generated_archives(&port.folder).iter().any(|name| port_dir.join(name).is_file()) {
        return FilesState::Have;
    }
    let files = GameFiles::for_port(port, config_dir);
    let locals = load_local_entries(&config_dir.join("ports.local.json"));
    let spec = RomSpec::for_port(port, locals.get(&port.folder));
    state_for(&spec, &files, port_dir)
}

#[cfg(test)]
mod tests;
