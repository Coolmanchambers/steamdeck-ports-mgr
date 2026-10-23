//! Find the game files one port needs in the user's game files folder, for any screen that wants it.
//!
//! This is the one place that answers "is this port's game file in the ROM folder?". The app runs it
//! after a port installs (or is adopted), from the port's Info dialog, from Settings > Game files, and
//! the setup wizard and import screen can call it too. It is decisions and text only, with no UI, and
//! is safe to run on a worker thread: it reads the folder and writes nothing but the scan cache.
//!
//! # API
//!
//! - [`find_game_files_for_port`]: one port. Returns a [`FindOutcome`], or `None` if `cancel` was set.
//! - [`find_game_files_for_ports`]: several ports with one walk of the folder (an import of many).
//! - [`FindRequest::new`]: what to look for (the port, whether it is installed, its `ports.local.json`
//!   entry, the recorded connections).
//!
//! What the outcome means and what a screen should do with it:
//!
//! | Outcome | Meaning | Screen |
//! |---|---|---|
//! | [`FindOutcome::NoFilesNeeded`] | The port names no file it cannot run without | Say nothing |
//! | [`FindOutcome::NoRomFolder`] | No game files folder is set yet | Offer to choose one |
//! | [`FindOutcome::FolderUnreadable`] | The folder is set but cannot be read (card out?) | Say so, offer search again |
//! | [`FindOutcome::FolderLooksEmpty`] | No game files where the saved scan had some (card out?); the saved scan is kept | Same |
//! | [`FindOutcome::Found`] | Every required file was found, one each | Open the Connect step with `row` |
//! | [`FindOutcome::Ambiguous`] | Every required file was found, some more than once | Let the user pick, then Connect |
//! | [`FindOutcome::NotFound`] | At least one required file is not in the folder | Name the files and where it looked |
//!
//! Reusing the saved scan: the folder is always walked (directory listings and file sizes and
//! modified times only), and a file is read and checksummed again only when it is new or its size or
//! modified time changed. A folder that did not change costs one walk and no file reads, which is what
//! "fresh" means here; [`LookedIn::rescanned`] says whether anything had to be read or the saved scan
//! changed. The saved scan is rewritten only when it changed.

use super::report::{all_slots_connected, PortRow};
use super::util::{ellipsize, file_name, human_bytes};
use crate::core::models::Port;
use crate::fork::rom_library::scan::hash_file;
use crate::fork::rom_library::system::classify;
use crate::fork::rom_library::{
    match_port, scan_folder, Basis, FileMatch, Mapping, PortMatch, RomFile, RomLibraryStore, RomSpec, ScanOptions, ScanResult, SlotKind, SlotResult,
    SlotStatus, System,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::UNIX_EPOCH;

/// What to look for: one port.
#[derive(Debug, Clone)]
pub struct FindRequest {
    pub port: Port,
    /// Installed in the library. A row for a port that is not installed cannot be connected.
    pub installed: bool,
    pub spec: RomSpec,
    /// Recorded connections, so an already connected port reads as connected.
    pub mappings: Vec<Mapping>,
}

impl FindRequest {
    /// `local_entry` is the port's raw object from `ports.local.json`, if any (it can carry explicit
    /// `rom_files`); `requirements::load_local_entries` reads them by folder.
    pub fn new(port: Port, installed: bool, local_entry: Option<&Value>, mappings: Vec<Mapping>) -> FindRequest {
        let spec = RomSpec::for_port(&port, local_entry);
        FindRequest { port, installed, spec, mappings }
    }

    /// The port names at least one file it cannot run without.
    pub fn needs_files(&self) -> bool {
        needs_game_files(&self.spec)
    }

    /// Every required file has a recorded connection that still checks out on disk. Reads only the
    /// recorded files, not the game folder.
    pub fn already_connected(&self) -> bool {
        let matched = PortMatch {
            port_key: self.port.key().to_string(),
            port_name: self.port.name.clone(),
            slots: self
                .spec
                .requirement
                .slots
                .iter()
                .map(|s| SlotResult {
                    slot_id: s.id(),
                    label: s.label.clone(),
                    alternatives: s.alternatives.clone(),
                    kind: s.kind,
                    optional: s.optional,
                    vague: s.is_vague(),
                    status: SlotStatus::Missing,
                })
                .collect(),
            status: crate::fork::rom_library::PortStatus::Missing,
        };
        all_slots_connected(&matched, &self.mappings)
    }
}

/// Does the port name any file it cannot run without.
pub fn needs_game_files(spec: &RomSpec) -> bool {
    spec.requirement.slots.iter().any(|s| !s.optional)
}

/// One file the port needs, as the catalog names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Need {
    /// The file as the catalog writes it ("oot.z64", "Some Game (USA).bin/cue"), or the slot label.
    pub name: String,
    /// CRC-32 values the catalog lists for it (any one will do).
    pub crc32: Vec<u32>,
    /// The catalog lists a SHA-1 for it.
    pub has_sha1: bool,
    pub kind: SlotKind,
    /// The catalog names only a game title: nothing a file can be checked against.
    pub title_only: bool,
    /// The catalog lists this many other files that would do instead ("any one of these").
    pub other_choices: usize,
}

impl Need {
    pub fn from_slot(slot: &SlotResult) -> Need {
        let first = slot.alternatives.first();
        let name = first.map(|a| a.display.trim().to_string()).filter(|d| !d.is_empty()).unwrap_or_else(|| slot.label.clone());
        Need {
            name,
            crc32: first.map(|a| a.crc32.clone()).unwrap_or_default(),
            has_sha1: first.is_some_and(|a| !a.sha1.is_empty()),
            kind: slot.kind,
            title_only: slot.vague,
            other_choices: slot.alternatives.len().saturating_sub(1),
        }
    }

    /// "oot.z64 (CRC32 AABBCCDD)", "BIOS: scph1001.bin (CRC32 ...)", "Game X (no checksum listed)".
    pub fn line(&self) -> String {
        let bios = if self.kind == SlotKind::Bios { "BIOS: " } else { "" };
        let check = if !self.crc32.is_empty() {
            let mut list: Vec<String> = self.crc32.iter().take(2).map(|c| format!("{c:08X}")).collect();
            if self.crc32.len() > 2 {
                list.push(format!("{} more", self.crc32.len() - 2));
            }
            format!(" (CRC32 {})", list.join(" or "))
        } else if self.has_sha1 {
            " (checksum listed)".to_string()
        } else if self.title_only {
            " (only the game is named, no file or checksum)".to_string()
        } else {
            " (no checksum listed, looked for by name)".to_string()
        };
        let others = match self.other_choices {
            0 => String::new(),
            1 => ", or 1 other file it lists".to_string(),
            n => format!(", or one of {n} other files it lists"),
        };
        format!("{bios}{}{check}{others}", ellipsize(&self.name, 70))
    }
}

/// Where the search looked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookedIn {
    pub folder: PathBuf,
    /// Game files (ROM and disc files) in the folder.
    pub files: usize,
    /// Files had to be read again, or the saved scan changed (false: the saved scan was still right).
    pub rescanned: bool,
    /// Folders or files that could not be read and were skipped.
    pub problems: usize,
    /// The scan changed but could not be saved, so it will be read again next time.
    pub cache_error: Option<String>,
}

impl LookedIn {
    /// "/run/media/deck/SD/Emulation/roms (12,431 files)".
    pub fn line(&self) -> String {
        format!("{} ({})", ellipsize(&self.folder.to_string_lossy(), 70), count_files(self.files))
    }
}

#[derive(Debug, Clone)]
pub enum FindOutcome {
    /// The port names no file it cannot run without. Nothing to say.
    NoFilesNeeded,
    /// No game files folder is set yet, so there was nowhere to look.
    NoRomFolder { needs: Vec<Need> },
    /// The folder is set but could not be read (the card or drive is not plugged in, or it is gone).
    FolderUnreadable { needs: Vec<Need>, folder: PathBuf, error: String },
    /// The folder can be read but holds no game files, although the saved scan of it had `saved`:
    /// most likely the card or drive is not plugged in. The saved scan is left as it was.
    FolderLooksEmpty { needs: Vec<Need>, folder: PathBuf, saved: usize },
    /// Every required file was found, one each. `row` feeds the Connect step.
    Found { row: PortRow, looked_in: LookedIn },
    /// Every required file was found, and `slots` (indices into `row.matched.slots`) have several
    /// different matching files: the user picks.
    Ambiguous { row: PortRow, slots: Vec<usize>, looked_in: LookedIn },
    /// At least one required file is not in the folder. `needs` are the missing ones; `notes` say what
    /// was found instead (a wrong version, the files that were found).
    NotFound { needs: Vec<Need>, looked_in: LookedIn, notes: Vec<String> },
}

impl FindOutcome {
    /// A few words for a list row ("found", "no matching file found").
    pub fn short(&self) -> &'static str {
        match self {
            FindOutcome::NoFilesNeeded => "needs no game files",
            FindOutcome::NoRomFolder { .. } => "no game files folder set yet",
            FindOutcome::FolderUnreadable { .. } => "the game files folder could not be read",
            FindOutcome::FolderLooksEmpty { .. } => "the game files folder looks empty (card or drive out?)",
            FindOutcome::Found { row, .. } if row.connected => "already connected",
            FindOutcome::Found { .. } => "found, choose to connect",
            FindOutcome::Ambiguous { .. } => "several matches, choose to pick one",
            FindOutcome::NotFound { .. } => "no matching game file found",
        }
    }
}

fn mtime_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// "1 game file", "12,431 game files".
pub fn count_files(n: usize) -> String {
    let digits = n.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    if n == 1 {
        "1 game file".to_string()
    } else {
        format!("{grouped} game files")
    }
}

fn required_needs(spec: &RomSpec) -> Vec<Need> {
    spec.requirement
        .slots
        .iter()
        .filter(|s| !s.optional)
        .map(|s| {
            Need::from_slot(&SlotResult {
                slot_id: s.id(),
                label: s.label.clone(),
                alternatives: s.alternatives.clone(),
                kind: s.kind,
                optional: s.optional,
                vague: s.is_vague(),
                status: SlotStatus::Missing,
            })
        })
        .collect()
}

/// Why the folder could not be used.
enum Lookup {
    Unreadable(String),
    /// The walk found no game files although the saved scan of this folder had `saved` of them: most
    /// likely the card or drive is not mounted (an empty mount point). The saved scan is kept.
    LooksEmpty { saved: usize },
}

/// The folder's files: the saved scan where nothing changed, files read again where something did.
/// `Ok(None)` when cancelled.
fn library_files(store: &RomLibraryStore, root: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(String)) -> Result<Option<(ScanResult, LookedIn)>, Lookup> {
    let prior = store.load_scan_cache().filter(|p| p.root == root);
    let scan = scan_folder(root, &ScanOptions::default(), prior.as_ref(), cancel, &mut |p| {
        progress(format!("Checked {} files, {} read\n{}", p.files_done, human_bytes(p.bytes_hashed), file_name(p.current)));
    })
    .map_err(|e| Lookup::Unreadable(e.to_string()))?;
    if scan.cancelled || cancel.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(None);
    }
    // Never let an empty walk replace a populated saved scan: an unmounted card leaves an empty folder.
    if let Some(saved) = prior.as_ref().map(|p| p.files.len()).filter(|n| *n > 0 && scan.files.is_empty()) {
        return Err(Lookup::LooksEmpty { saved });
    }
    let rescanned = prior.as_ref().is_none_or(|p| p.files != scan.files);
    let cache_error = if rescanned { store.save_scan_cache(&scan).err().map(|e| e.to_string()) } else { None };
    let looked_in = LookedIn { folder: root.to_path_buf(), files: scan.files.len(), rescanned, problems: scan.issues.len(), cache_error };
    Ok(Some((scan, looked_in)))
}

fn row_for(req: &FindRequest, matched: PortMatch, files: &[RomFile]) -> PortRow {
    let mut system = System::Unknown;
    let mut low_confidence = false;
    for slot in &matched.slots {
        let candidates: &[FileMatch] = match &slot.status {
            SlotStatus::Found(f) => std::slice::from_ref(f),
            SlotStatus::Ambiguous(c) => c,
            _ => continue,
        };
        for f in candidates {
            low_confidence |= f.is_low_confidence();
            if system == System::Unknown && slot.kind == SlotKind::Game {
                system = files.iter().find(|r| r.path == f.path).map(|r| r.system).unwrap_or(System::Unknown);
            }
        }
    }
    let connected = all_slots_connected(&matched, &req.mappings);
    PortRow { port: req.port.clone(), installed: req.installed, status: matched.status, system, spec: req.spec.clone(), matched, connected, low_confidence }
}

/// Decide the outcome for one port against a scan of the folder.
pub fn outcome_for(req: &FindRequest, files: &[RomFile], looked_in: LookedIn) -> FindOutcome {
    if !req.needs_files() {
        return FindOutcome::NoFilesNeeded;
    }
    let matched = match_port(req.port.key(), &req.port.name, &req.spec, files);
    let required: Vec<(usize, &SlotResult)> = matched.slots.iter().enumerate().filter(|(_, s)| !s.optional).collect();
    let missing: Vec<&SlotResult> = required.iter().map(|(_, s)| *s).filter(|s| !matches!(s.status, SlotStatus::Found(_) | SlotStatus::Ambiguous(_))).collect();
    if missing.is_empty() {
        let ambiguous: Vec<usize> = required.iter().filter(|(_, s)| matches!(s.status, SlotStatus::Ambiguous(_))).map(|(i, _)| *i).collect();
        let row = row_for(req, matched, files);
        return if ambiguous.is_empty() { FindOutcome::Found { row, looked_in } } else { FindOutcome::Ambiguous { row, slots: ambiguous, looked_in } };
    }
    let mut notes = Vec::new();
    for slot in &missing {
        if let SlotStatus::WrongVersion(c) = &slot.status {
            for f in c.iter().take(2) {
                notes.push(format!(
                    "{} has the right name, but its checksum ({:08X}) is not one {} lists. It may be another version or a modified file.",
                    ellipsize(&file_name(&f.path), 50),
                    f.crc32,
                    req.port.display_name()
                ));
            }
        }
    }
    let rest = if missing.len() == 1 { "only the file above is missing" } else { "only the files above are missing" };
    for (_, slot) in &required {
        if let SlotStatus::Found(f) = &slot.status {
            notes.push(format!("Already found: {} ({rest})", ellipsize(&file_name(&f.path), 60)));
        }
    }
    FindOutcome::NotFound { needs: missing.iter().map(|s| Need::from_slot(s)).collect(), looked_in, notes }
}

/// Look for one port's game files. `progress` gets a two-line status ("Checked N files, X read" and
/// the current file) while the folder is walked. `None` when `cancel` was set; nothing is saved then.
pub fn find_game_files_for_port(store: &RomLibraryStore, req: &FindRequest, cancel: &AtomicBool, progress: &mut dyn FnMut(String)) -> Option<FindOutcome> {
    find_game_files_for_ports(store, std::slice::from_ref(req), cancel, progress).map(|mut v| v.remove(0))
}

/// Look for several ports' game files with one walk of the folder. Outcomes are in request order.
/// The folder is not touched when no request needs files or no folder is set.
pub fn find_game_files_for_ports(store: &RomLibraryStore, reqs: &[FindRequest], cancel: &AtomicBool, progress: &mut dyn FnMut(String)) -> Option<Vec<FindOutcome>> {
    if !reqs.iter().any(FindRequest::needs_files) {
        return Some(reqs.iter().map(|_| FindOutcome::NoFilesNeeded).collect());
    }
    let Some(root) = store.rom_dir().map(Path::to_path_buf) else {
        return Some(reqs.iter().map(|r| if r.needs_files() { FindOutcome::NoRomFolder { needs: required_needs(&r.spec) } } else { FindOutcome::NoFilesNeeded }).collect());
    };
    match library_files(store, &root, cancel, progress) {
        Ok(None) => None,
        Err(lookup) => Some(
            reqs.iter()
                .map(|r| {
                    if !r.needs_files() {
                        return FindOutcome::NoFilesNeeded;
                    }
                    let needs = required_needs(&r.spec);
                    match &lookup {
                        Lookup::Unreadable(error) => FindOutcome::FolderUnreadable { needs, folder: root.clone(), error: error.clone() },
                        Lookup::LooksEmpty { saved } => FindOutcome::FolderLooksEmpty { needs, folder: root.clone(), saved: *saved },
                    }
                })
                .collect(),
        ),
        Ok(Some((scan, looked_in))) => Some(reqs.iter().map(|r| outcome_for(r, &scan.files, looked_in.clone())).collect()),
    }
}

// -- Text for the prompts -------------------------------------------------------------------------

const MAX_NEEDS: usize = 4;

fn need_lines(needs: &[Need], lead: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for (i, need) in needs.iter().take(MAX_NEEDS).enumerate() {
        lines.push(if i == 0 { format!("{lead} {}", need.line()) } else { format!("  and {}", need.line()) });
    }
    if needs.len() > MAX_NEEDS {
        lines.push(format!("  and {} more files", needs.len() - MAX_NEEDS));
    }
    lines
}

/// The "not found" prompt, one line per row.
pub fn not_found_lines(port_name: &str, needs: &[Need], looked_in: &LookedIn, notes: &[String]) -> Vec<String> {
    let mut lines = vec![format!("No matching game file found for {port_name}.")];
    lines.extend(need_lines(needs, "It needs:"));
    lines.push(format!("Looked in: {}", looked_in.line()));
    lines.extend(notes.iter().take(4).cloned());
    if looked_in.problems > 0 {
        lines.push(format!("{} folders or files could not be read and were skipped.", looked_in.problems));
    }
    lines.push("Add the file to that folder and choose Search again, or Connect by hand.".to_string());
    lines
}

/// The prompt when no game files folder is set.
pub fn no_folder_lines(port_name: &str, needs: &[Need]) -> Vec<String> {
    let mut lines = vec![format!("{port_name} needs game files from your own copy of the game.")];
    lines.extend(need_lines(needs, "It needs:"));
    lines.push("No game files folder is set yet, so there is nowhere to look.".to_string());
    lines.push("Choose the folder you keep your game files in, or Connect by hand.".to_string());
    lines
}

/// The prompt when the folder cannot be read.
pub fn unreadable_lines(port_name: &str, needs: &[Need], folder: &Path, error: &str) -> Vec<String> {
    let mut lines = vec![format!("Could not look for {port_name}'s game files.")];
    lines.extend(need_lines(needs, "It needs:"));
    lines.push(format!("Could not read {}: {}", ellipsize(&folder.to_string_lossy(), 60), ellipsize(error, 60)));
    lines.push("Is the card or drive plugged in? Plug it in and choose Search again, or choose another folder.".to_string());
    lines
}

/// The prompt when the folder holds no game files although the saved scan had some.
pub fn looks_empty_lines(port_name: &str, needs: &[Need], folder: &Path, saved: usize) -> Vec<String> {
    let mut lines = vec![format!("Could not look for {port_name}'s game files.")];
    lines.extend(need_lines(needs, "It needs:"));
    lines.push(format!("{} held {} last time, but now it has none.", ellipsize(&folder.to_string_lossy(), 60), count_files(saved)));
    lines.push("The folder looks empty. Is the card or drive plugged in?".to_string());
    lines.push("Plug it in and choose Search again, or choose another folder.".to_string());
    lines
}

/// Under every prompt, so "Later" is not a dead end.
pub const LATER_HINT: &str = "Later: use Game files... in the port's Info, or Settings > Game files > Find game files for a port.";

// -- Connect by hand ------------------------------------------------------------------------------

/// What a file the user picked by hand turned out to be.
#[derive(Debug, Clone)]
pub enum HandPick {
    /// It is a file the port lists (by checksum, or by name and flagged): open the Connect step.
    Matches { row: PortRow },
    /// It is not a file the port lists. `file` is it as a low-confidence match; `slots` are the
    /// required slots (indices into `row.matched.slots`) it could stand in for if the user insists.
    NoMatch { row: PortRow, file: FileMatch, slots: Vec<usize>, wrong_version: bool },
}

/// Read and checksum one file the user picked and match it against the port's needs.
/// `Ok(None)` when cancelled. Only reads the file.
pub fn match_hand_picked(req: &FindRequest, path: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(String)) -> Result<Option<HandPick>, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("Could not read {}: {e}", file_name(path)))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file.", file_name(path)));
    }
    let rel = PathBuf::from(path.file_name().unwrap_or_default());
    let system = classify(&rel);
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let n64 = system == System::N64 || matches!(ext.as_str(), "z64" | "v64" | "n64");
    let (name, total) = (file_name(path), meta.len());
    let mut read = 0u64;
    let mut on_bytes = |n: u64| {
        read += n;
        progress(format!("Checked {} of {}\n{name}", human_bytes(read), human_bytes(total)));
    };
    let Some(h) = hash_file(path, n64, true, cancel, &mut on_bytes).map_err(|e| format!("Could not read {}: {e}", file_name(path)))? else {
        return Ok(None);
    };
    let file = RomFile { path: path.to_path_buf(), rel_path: rel, system, size: h.size, mtime_ms: mtime_ms(&meta), ext, byte_order: h.byte_order, raw: h.raw, normalized: h.normalized };
    let matched = match_port(req.port.key(), &req.port.name, &req.spec, std::slice::from_ref(&file));
    let hit = matched.slots.iter().any(|s| !s.optional && matches!(s.status, SlotStatus::Found(_)));
    let wrong_version = matched.slots.iter().any(|s| matches!(s.status, SlotStatus::WrongVersion(_)));
    let slots: Vec<usize> = matched.slots.iter().enumerate().filter(|(_, s)| !s.optional).map(|(i, _)| i).collect();
    let row = row_for(req, matched, std::slice::from_ref(&file));
    if hit {
        return Ok(Some(HandPick::Matches { row }));
    }
    let file = FileMatch { path: file.path.clone(), size: file.size, crc32: file.raw.crc32, basis: Basis::Name, byte_order: file.byte_order, also_at: Vec::new(), parts: Vec::new() };
    Ok(Some(HandPick::NoMatch { row, file, slots, wrong_version }))
}

/// The row with `file` standing in for slot `slot`, after the user said to use it anyway. It stays
/// marked "matched by name, check this" in the Connect step.
pub fn use_anyway(row: &PortRow, slot: usize, file: &FileMatch) -> PortRow {
    let mut row = row.clone();
    if let Some(s) = row.matched.slots.get_mut(slot) {
        s.status = SlotStatus::Found(file.clone());
    }
    row.low_confidence = true;
    row
}

/// The prompt for a hand-picked file that is not one the port lists.
pub fn no_match_lines(port_name: &str, row: &PortRow, file: &FileMatch, wrong_version: bool) -> Vec<String> {
    let what = if wrong_version { "has the right name, but its checksum is not one" } else { "is not a file" };
    let mut lines = vec![format!("{} (CRC32 {:08X}) {what} {port_name} lists.", ellipsize(&file_name(&file.path), 50), file.crc32)];
    let needs: Vec<Need> = row.matched.slots.iter().filter(|s| !s.optional).map(Need::from_slot).collect();
    lines.extend(need_lines(&needs, "It needs:"));
    lines.push("Using it anyway may not work. Your file is only read either way.".to_string());
    lines
}

#[cfg(test)]
mod tests {
    use super::super::report::tests_support::port;
    use super::*;
    use crate::fork::rom_library::testutil::*;
    use crate::fork::rom_library::{plan_connect, Access, Approval, ConnectRequest, Method};
    use std::fs;
    use std::sync::atomic::AtomicBool;

    fn crc(data: &[u8]) -> String {
        format!("{:08X}", digests_of(data).crc32)
    }

    struct World {
        tmp: tempfile::TempDir,
        roms: PathBuf,
        store: RomLibraryStore,
    }

    fn world() -> World {
        let tmp = temp_dir();
        let roms = tmp.path().join("roms");
        fs::create_dir_all(&roms).unwrap();
        let mut store = RomLibraryStore::load(&tmp.path().join("rom_library.json")).unwrap();
        store.set_rom_dir(Some(roms.clone())).unwrap();
        World { tmp, roms, store }
    }

    fn req(name: &str, instructions: &str) -> FindRequest {
        FindRequest::new(port(name, &name.replace(' ', ""), &["n64"], instructions), true, None, vec![])
    }

    fn find(store: &RomLibraryStore, r: &FindRequest) -> FindOutcome {
        find_game_files_for_port(store, r, &AtomicBool::new(false), &mut |_| {}).unwrap()
    }

    #[test]
    fn found_gives_a_row_the_connect_step_can_use() {
        let w = world();
        let oot = n64_rom(1, 40_000);
        write(&w.roms.join("n64/my zelda.z64"), &oot);
        write(&w.roms.join("snes/other.sfc"), &random_bytes(2, 3_000));
        let before = snapshot(&w.roms);
        let r = req("Ship of Harkinian", &format!("Required files: oot.z64 (CRC32: {})", crc(&oot)));
        let FindOutcome::Found { row, looked_in } = find(&w.store, &r) else { panic!("not found") };
        assert!(row.can_connect() && !row.connected && !row.low_confidence);
        assert_eq!(row.system, System::N64);
        assert_eq!(looked_in.files, 2);
        assert!(looked_in.rescanned, "the first look reads the folder");
        assert_eq!(snapshot(&w.roms), before, "the game folder is only read");
    }

    #[test]
    fn several_different_matches_are_ambiguous_and_name_the_slot() {
        let w = world();
        write(&w.roms.join("n64/Zelda (USA).z64"), &n64_rom(1, 9_000));
        write(&w.roms.join("backup/Zelda (USA).z64"), &n64_rom(2, 9_000));
        let r = req("Zelda Port", "Required files: Zelda (USA).z64");
        let FindOutcome::Ambiguous { row, slots, .. } = find(&w.store, &r) else { panic!("not ambiguous") };
        assert_eq!(slots, vec![0]);
        let SlotStatus::Ambiguous(c) = &row.matched.slots[0].status else { panic!() };
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn not_found_names_the_file_its_crc_and_where_it_looked() {
        let w = world();
        write(&w.roms.join("n64/unrelated.z64"), &n64_rom(7, 9_000));
        write(&w.roms.join("readme.txt"), b"not a game");
        let r = req("Ship of Harkinian", "Required files: oot.z64 (CRC32: AABBCCDD)");
        let FindOutcome::NotFound { needs, looked_in, notes } = find(&w.store, &r) else { panic!("found something") };
        assert_eq!(needs.len(), 1);
        assert_eq!(needs[0].line(), "oot.z64 (CRC32 AABBCCDD)");
        assert_eq!(looked_in.files, 1, "only game files are counted");
        let text = not_found_lines("Ship of Harkinian", &needs, &looked_in, &notes).join("\n");
        assert!(text.starts_with("No matching game file found for Ship of Harkinian."), "{text}");
        assert!(text.contains("It needs: oot.z64 (CRC32 AABBCCDD)"), "{text}");
        assert!(text.contains(&format!("Looked in: {} (1 game file)", w.roms.display())), "{text}");
        assert!(text.contains("Search again") && text.contains("Connect by hand"), "{text}");
    }

    #[test]
    fn a_wrong_version_is_not_found_with_a_note_and_a_partial_lists_only_what_is_missing() {
        let w = world();
        let game = n64_rom(3, 9_000);
        write(&w.roms.join("n64/Kart (USA).z64"), &game);
        let wrong = req("Kart Port", "Required files: Kart (USA).z64 (CRC32: 01020304)");
        let FindOutcome::NotFound { notes, .. } = find(&w.store, &wrong) else { panic!() };
        assert!(notes.iter().any(|n| n.contains("Kart (USA).z64 has the right name, but its checksum (") && n.contains("is not one Kart Port lists. It may be another version")), "{notes:?}");

        let two = req("Two Files", &format!("Required files: Kart (USA).z64 (CRC32: {}), Disk (Japan).z64 (CRC32: 0A0B0C0D)", crc(&game)));
        assert_eq!(two.spec.requirement.slots.len(), 2);
        let FindOutcome::NotFound { needs, notes, .. } = find(&w.store, &two) else { panic!() };
        assert_eq!(needs.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), vec!["Disk (Japan).z64"]);
        assert!(notes.iter().any(|n| n == "Already found: Kart (USA).z64 (only the file above is missing)"), "{notes:?}");
    }

    #[test]
    fn no_rom_folder_set_says_so_without_touching_any_folder() {
        let tmp = temp_dir();
        let store = RomLibraryStore::load(&tmp.path().join("rom_library.json")).unwrap();
        let r = req("Ship of Harkinian", "Required files: oot.z64 (CRC32: AABBCCDD)");
        let FindOutcome::NoRomFolder { needs } = find(&store, &r) else { panic!() };
        let text = no_folder_lines("Ship of Harkinian", &needs).join("\n");
        assert!(text.contains("No game files folder is set yet") && text.contains("oot.z64 (CRC32 AABBCCDD)"), "{text}");
        assert!(!store.scan_cache_path().exists());
    }

    #[test]
    fn an_unplugged_folder_is_unreadable_not_not_found() {
        let w = world();
        fs::remove_dir_all(&w.roms).unwrap();
        let r = req("Ship of Harkinian", "Required files: oot.z64 (CRC32: AABBCCDD)");
        let FindOutcome::FolderUnreadable { folder, needs, .. } = find(&w.store, &r) else { panic!() };
        assert_eq!(folder, w.roms);
        assert!(unreadable_lines("Ship of Harkinian", &needs, &folder, "gone").join("\n").contains("plugged in"));
    }

    #[test]
    fn a_fresh_cache_is_reused_and_a_stale_one_triggers_a_rescan() {
        let w = world();
        let oot = n64_rom(1, 40_000);
        let r = req("Ship of Harkinian", &format!("Required files: oot.z64 (CRC32: {})", crc(&oot)));
        write(&w.roms.join("n64/other.z64"), &n64_rom(9, 9_000));
        let FindOutcome::NotFound { looked_in, .. } = find(&w.store, &r) else { panic!() };
        assert!(looked_in.rescanned);
        let cache_time = fs::metadata(w.store.scan_cache_path()).unwrap().modified().unwrap();

        // Nothing changed: the saved scan is used as it is and not rewritten; no file is read.
        let mut reads = 0u64;
        let again = find_game_files_for_port(&w.store, &r, &AtomicBool::new(false), &mut |p| {
            if !p.contains(", 0 B read") {
                reads += 1;
            }
        })
        .unwrap();
        let FindOutcome::NotFound { looked_in, .. } = again else { panic!() };
        assert!(!looked_in.rescanned);
        assert_eq!(reads, 0, "no bytes were read for an unchanged folder");
        assert_eq!(fs::metadata(w.store.scan_cache_path()).unwrap().modified().unwrap(), cache_time);

        // The user adds the file: the saved scan is stale, the new file is read and found.
        write(&w.roms.join("n64/added.z64"), &oot);
        let FindOutcome::Found { looked_in, .. } = find(&w.store, &r) else { panic!("the new file was not picked up") };
        assert!(looked_in.rescanned);
        assert_eq!(w.store.load_scan_cache().unwrap().files.len(), 2, "the saved scan was updated");

        // A file replaced in place (same name, new content and time) is read again too.
        fs::write(w.roms.join("n64/added.z64"), n64_rom(5, 40_001)).unwrap();
        let FindOutcome::NotFound { looked_in, .. } = find(&w.store, &r) else { panic!("the stale checksum was trusted") };
        assert!(looked_in.rescanned);
    }

    #[test]
    fn an_emptied_folder_never_replaces_a_populated_saved_scan() {
        let w = world();
        let oot = n64_rom(1, 9_000);
        write(&w.roms.join("n64/oot.z64"), &oot);
        write(&w.roms.join("snes/other.sfc"), &random_bytes(2, 900));
        let r = req("Ship of Harkinian", &format!("Required files: oot.z64 (CRC32: {})", crc(&oot)));
        assert!(matches!(find(&w.store, &r), FindOutcome::Found { .. }));
        let saved = fs::read(w.store.scan_cache_path()).unwrap();
        // The card is out: the mount point is still there but empty.
        fs::remove_dir_all(&w.roms).unwrap();
        fs::create_dir_all(&w.roms).unwrap();
        let FindOutcome::FolderLooksEmpty { needs, folder, saved: n } = find(&w.store, &r) else { panic!("an empty walk read as not found") };
        assert_eq!((folder.as_path(), n), (w.roms.as_path(), 2));
        assert_eq!(fs::read(w.store.scan_cache_path()).unwrap(), saved, "the saved scan is untouched");
        let text = looks_empty_lines("Ship of Harkinian", &needs, &folder, n).join("\n");
        assert!(text.contains("The folder looks empty. Is the card or drive plugged in?") && text.contains("2 game files last time"), "{text}");
        // A folder that was empty before stays an ordinary not found.
        let tmp = temp_dir();
        let mut fresh = RomLibraryStore::load(&tmp.path().join("rom_library.json")).unwrap();
        fs::create_dir_all(tmp.path().join("roms")).unwrap();
        fresh.set_rom_dir(Some(tmp.path().join("roms"))).unwrap();
        assert!(matches!(find(&fresh, &r), FindOutcome::NotFound { .. }));
    }

    #[test]
    fn a_cancelled_hand_pick_returns_nothing() {
        let tmp = temp_dir();
        let file = tmp.path().join("big.z64");
        write(&file, &n64_rom(1, 3 << 20));
        let r = req("Ship", "Required files: oot.z64 (CRC32: AABBCCDD)");
        let mut seen = Vec::new();
        assert!(match_hand_picked(&r, &file, &AtomicBool::new(true), &mut |t| seen.push(t)).unwrap().is_none());
        let mut seen = Vec::new();
        assert!(match_hand_picked(&r, &file, &AtomicBool::new(false), &mut |t| seen.push(t)).unwrap().is_some());
        assert!(seen.last().unwrap().starts_with("Checked 3.1 MB of 3.1 MB\nbig.z64"), "{seen:?}");
    }

    #[test]
    fn a_saved_scan_of_another_folder_is_not_used() {
        let w = world();
        let oot = n64_rom(1, 9_000);
        let elsewhere = w.tmp.path().join("old");
        write(&elsewhere.join("oot.z64"), &oot);
        let mut old = w.store.clone();
        old.set_rom_dir(Some(elsewhere)).unwrap();
        let r = req("Ship of Harkinian", &format!("Required files: oot.z64 (CRC32: {})", crc(&oot)));
        assert!(matches!(find(&old, &r), FindOutcome::Found { .. }));
        // Back to the (empty) folder: the saved scan of the old one must not answer for it.
        old.set_rom_dir(Some(w.roms.clone())).unwrap();
        assert!(matches!(find(&old, &r), FindOutcome::NotFound { .. }));
        assert_eq!(old.load_scan_cache().unwrap().root, w.roms);
    }

    #[test]
    fn a_port_that_needs_no_files_gets_nothing_and_the_folder_is_not_walked() {
        let tmp = temp_dir();
        let mut store = RomLibraryStore::load(&tmp.path().join("rom_library.json")).unwrap();
        let roms = tmp.path().join("roms");
        write(&roms.join("a.z64"), &n64_rom(1, 900));
        store.set_rom_dir(Some(roms)).unwrap();
        for text in ["A fan game, no files needed.", ""] {
            let r = req("Free Game", text);
            assert!(!r.needs_files());
            assert!(matches!(find(&store, &r), FindOutcome::NoFilesNeeded));
        }
        assert!(!store.scan_cache_path().exists(), "nothing was scanned");
    }

    #[test]
    fn a_byte_swapped_n64_image_is_found_and_flagged() {
        let w = world();
        let oot = n64_rom(4, 40_000);
        write(&w.roms.join("n64/zelda.v64"), &to_v64(&oot));
        let r = req("Ship of Harkinian", &format!("Required files: oot.z64 (CRC32: {})", crc(&oot)));
        let FindOutcome::Found { row, .. } = find(&w.store, &r) else { panic!("the v64 image was not found") };
        let SlotStatus::Found(f) = &row.matched.slots[0].status else { panic!() };
        assert_eq!(f.basis, Basis::Crc32AfterByteOrderFix);
        assert!(f.needs_byte_order_conversion());
    }

    #[test]
    fn several_ports_share_one_walk_and_keep_their_order() {
        let w = world();
        let oot = n64_rom(1, 9_000);
        write(&w.roms.join("oot.z64"), &oot);
        let reqs = vec![
            req("Missing", "Required files: mm.z64 (CRC32: AABBCCDD)"),
            req("Free", "No files needed."),
            req("Ship", &format!("Required files: oot.z64 (CRC32: {})", crc(&oot))),
        ];
        let out = find_game_files_for_ports(&w.store, &reqs, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert!(matches!(out[0], FindOutcome::NotFound { .. }));
        assert!(matches!(out[1], FindOutcome::NoFilesNeeded));
        assert!(matches!(out[2], FindOutcome::Found { .. }));
    }

    #[test]
    fn cancelling_returns_nothing_and_saves_no_scan() {
        let w = world();
        write(&w.roms.join("a.z64"), &n64_rom(1, 9_000));
        let cancel = AtomicBool::new(true);
        let r = req("Ship", "Required files: oot.z64 (CRC32: AABBCCDD)");
        assert!(find_game_files_for_port(&w.store, &r, &cancel, &mut |_| {}).is_none());
        assert!(!w.store.scan_cache_path().exists());
    }

    #[cfg(unix)]
    #[test]
    fn an_already_connected_port_reads_connected() {
        let w = world();
        let oot = n64_rom(1, 9_000);
        write(&w.roms.join("oot.z64"), &oot);
        let lib = w.tmp.path().join("lib/Ship");
        fs::create_dir_all(&lib).unwrap();
        let mut r = req("Ship", &format!("Required files: oot.z64 (CRC32: {})", crc(&oot)));
        assert!(!r.already_connected());
        let FindOutcome::Found { row, .. } = find(&w.store, &r) else { panic!() };
        let SlotStatus::Found(f) = &row.matched.slots[0].status else { panic!() };
        let plan = plan_connect(&ConnectRequest {
            port_key: r.port.key().into(),
            slot: row.matched.slots[0].slot_id.clone(),
            source: f.path.clone(),
            port_dir: lib,
            dest_subdir: String::new(),
            dest_name: None,
            method: Method::Symlink,
            access: Access::Link,
            byte_order: None,
            known_crc32: Some(f.crc32),
        })
        .unwrap();
        r.mappings = vec![crate::fork::rom_library::apply(&plan, Approval::None).unwrap()];
        assert!(r.already_connected());
        let FindOutcome::Found { row, .. } = find(&w.store, &r) else { panic!() };
        assert!(row.connected);
    }

    #[test]
    fn a_hand_picked_file_that_matches_opens_connect_and_one_that_does_not_can_be_used_anyway() {
        let tmp = temp_dir();
        let oot = n64_rom(1, 9_000);
        let good = tmp.path().join("anywhere/strange name.bin");
        let bad = tmp.path().join("anywhere/other.z64");
        write(&good, &oot);
        write(&bad, &n64_rom(2, 9_000));
        let before = snapshot(tmp.path());
        let r = req("Ship", &format!("Required files: oot.z64 (CRC32: {})", crc(&oot)));
        let Some(HandPick::Matches { row }) = match_hand_picked(&r, &good, &AtomicBool::new(false), &mut |_| {}).unwrap() else { panic!("checksum match missed") };
        assert!(row.can_connect());

        let Some(HandPick::NoMatch { row, file, slots, wrong_version }) = match_hand_picked(&r, &bad, &AtomicBool::new(false), &mut |_| {}).unwrap() else { panic!() };
        assert!(!wrong_version && slots == vec![0] && !row.can_connect());
        assert!(no_match_lines("Ship", &row, &file, false)[0].contains("is not a file Ship lists"));
        let forced = use_anyway(&row, 0, &file);
        assert!(forced.can_connect() && forced.low_confidence);
        let screen = super::super::connect_screen::ConnectScreen::new(&forced, tmp.path().join("lib"), &[]);
        assert_eq!(screen.unchecked_files(), vec!["other.z64".to_string()], "the done message will say it was not checked");
        assert!(match_hand_picked(&r, tmp.path(), &AtomicBool::new(false), &mut |_| {}).is_err(), "a folder is refused");
        assert_eq!(snapshot(tmp.path()), before, "picked files are only read");
    }

    #[test]
    fn need_lines_cover_bios_sha1_titles_and_alternatives() {
        let n = |crc32: Vec<u32>, has_sha1, kind, title_only, other_choices| Need { name: "x.bin".into(), crc32, has_sha1, kind, title_only, other_choices };
        assert_eq!(n(vec![1, 2, 3], false, SlotKind::Bios, false, 0).line(), "BIOS: x.bin (CRC32 00000001 or 00000002 or 1 more)");
        assert_eq!(n(vec![], true, SlotKind::Game, false, 0).line(), "x.bin (checksum listed)");
        assert!(n(vec![], false, SlotKind::Game, true, 2).line().ends_with("no file or checksum), or one of 2 other files it lists"));
        assert_eq!(count_files(12_431), "12,431 game files");
        assert_eq!(count_files(1), "1 game file");
        assert_eq!(count_files(999), "999 game files");
        assert_eq!(count_files(1_000_000), "1,000,000 game files");
    }
}
