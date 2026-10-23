//! The game file check around a move: before copying, every game file the port is known to use must
//! be there and reachable; after copying, the same must hold at the new place.
//!
//! What the check blocks on:
//! - the port's connections recorded by Settings > Game files (`rom_library.json`): links and copies
//!   inside the port folder, and "use original" files outside it;
//! - links inside the port folder that point outside it (to a ROM folder, usually).
//!
//! Files the port's entry says it needs (`Required files:` text, or `rom_files`) that are neither connected
//! nor in the folder never block a move ([`required_not_found`] lists them for the summary): many are only
//! needed once at install time. Files of that name that are in the folder are always read back by hash.
//!
//! Before the copy, anything missing or broken is reported and the move stops unless the user says
//! "Move anyway". After the copy, a mismatch always stops that port's move (the new copy is removed);
//! there is no "anyway" for it.

use super::copy::{absolute_target, Kind, Manifest};
use crate::core::models::Port;
use crate::core::path_safety::is_within;
use crate::fork::rom_library::{self, check_mapping, Health, Mapping, Method, RomSpec, RomLibraryStore};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// A required file the port's entry names, for the by-name lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredFile {
    /// The slot's label, for messages.
    pub label: String,
    /// Lowercase file names that satisfy it.
    pub names: Vec<String>,
    /// The slot's id, as recorded in a connection.
    pub slot_id: String,
    pub crc32: Vec<u32>,
}

/// What the check needs to know about one port.
#[derive(Debug, Clone, Default)]
pub struct GameFiles {
    pub mappings: Vec<Mapping>,
    pub required: Vec<RequiredFile>,
}

impl GameFiles {
    /// Reads the port's recorded connections and required files from the app folder. A missing or
    /// unreadable ROM library file gives no connections (the folder links are still checked).
    pub fn for_port(port: &Port, config_dir: &Path) -> GameFiles {
        let mappings = RomLibraryStore::load(&config_dir.join("rom_library.json"))
            .map(|s| s.mappings_for(port.key()).into_iter().cloned().collect())
            .unwrap_or_default();
        let locals = rom_library::load_local_entries(&config_dir.join("ports.local.json"));
        let spec = RomSpec::for_port(port, locals.get(&port.folder));
        let required = spec
            .requirement
            .slots
            .iter()
            .filter(|s| !s.optional && !s.is_vague())
            .filter_map(|s| {
                let names: Vec<String> = s.alternatives.iter().flat_map(|a| a.names.iter().map(|n| n.to_lowercase())).collect();
                (!names.is_empty()).then(|| RequiredFile {
                    label: s.label.clone(),
                    names,
                    slot_id: s.id(),
                    crc32: s.alternatives.iter().flat_map(|a| a.crc32.iter().copied()).collect(),
                })
            })
            .collect();
        GameFiles { mappings, required }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// The file, as the user knows it (a name or a path).
    pub file: String,
    /// Where it was expected.
    pub expected_at: PathBuf,
    pub problem: String,
}

impl Issue {
    pub fn line(&self) -> String {
        format!("{}: {} (expected at {})", self.file, self.problem, self.expected_at.display())
    }
}

fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

/// Files under `dir` (not following links into other folders), by lowercase name, a few levels deep.
fn files_by_name(dir: &Path, depth: usize, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        match e.file_type() {
            Ok(t) if t.is_dir() && depth > 0 => files_by_name(&path, depth - 1, out),
            Ok(t) if t.is_file() || t.is_symlink() => out.push((e.file_name().to_string_lossy().to_lowercase(), path)),
            _ => {}
        }
    }
}

/// Links inside `dir` that point outside it, as (link, absolute target).
pub fn outside_links(dir: &Path) -> Vec<(PathBuf, PathBuf)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, PathBuf)>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let path = e.path();
            match e.file_type() {
                Ok(t) if t.is_symlink() => {
                    if let Ok(target) = fs::read_link(&path) {
                        let abs = absolute_target(&path, &target);
                        if !is_within(&abs, root) {
                            out.push((path, abs));
                        }
                    }
                }
                Ok(t) if t.is_dir() => walk(root, &path, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn health_problem(h: Health) -> Option<&'static str> {
    match h {
        Health::Ok => None,
        Health::SourceMissing => Some("the original game file is missing (drive not plugged in, or the file was moved)"),
        Health::SourceChanged => Some("the original game file changed since it was connected"),
        Health::DestMissing => Some("its link or copy in the port folder is missing"),
        Health::DestChanged => Some("its link or copy in the port folder does not match the original any more"),
    }
}

/// The check before a move of `port_dir` (the port's folder as it is now). An adopted port (a link to
/// another folder) is checked through the link.
pub fn check_before(port_dir: &Path, files: &GameFiles) -> Vec<Issue> {
    let mut issues = Vec::new();
    for m in &files.mappings {
        // Only connections that live in this folder, or that use an original in place.
        let relevant = m.dest.as_ref().is_none_or(|d| is_within(d, port_dir));
        if !relevant {
            continue;
        }
        if let Some(problem) = health_problem(check_mapping(m)) {
            issues.push(Issue { file: name_of(&m.source), expected_at: m.dest.clone().unwrap_or_else(|| m.source.clone()), problem: problem.into() });
        }
    }
    for (link, target) in outside_links(port_dir) {
        if fs::metadata(&link).is_err() {
            issues.push(Issue { file: name_of(&link), expected_at: target, problem: "this link in the port folder points at a file that is not there".into() });
        }
    }
    issues
}

/// Files the port's entry names (`Required files:` or `rom_files`) that are neither connected nor in the port
/// folder. Information only, never a reason to stop a move: many are needed once at install time (an
/// archive, a ROM the port turned into its own data file). The move summary lists them.
pub fn required_not_found(port_dir: &Path, files: &GameFiles) -> Vec<String> {
    let mut present = Vec::new();
    files_by_name(port_dir, 3, &mut present);
    files
        .required
        .iter()
        .filter(|req| {
            let connected = files.mappings.iter().any(|m| m.slot == req.slot_id);
            let in_folder = present.iter().any(|(name, path)| req.names.contains(name) && fs::metadata(path).is_ok());
            !connected && !in_folder
        })
        .map(|req| req.names.first().cloned().unwrap_or_else(|| req.label.clone()))
        .collect()
}

/// Files (relative to the port folder) the verify step must always read back: copies of game files
/// and files named like a required file.
pub fn must_hash(port_dir: &Path, files: &GameFiles) -> BTreeSet<PathBuf> {
    let mut out = BTreeSet::new();
    for m in &files.mappings {
        if let Some(rel) = m.dest.as_ref().filter(|_| m.method == Method::Copy).and_then(|d| d.strip_prefix(port_dir).ok()) {
            out.insert(rel.to_path_buf());
        }
    }
    let mut present = Vec::new();
    files_by_name(port_dir, 3, &mut present);
    for (name, path) in present {
        if files.required.iter().any(|r| r.names.contains(&name)) {
            if let Ok(rel) = path.strip_prefix(port_dir) {
                out.insert(rel.to_path_buf());
            }
        }
    }
    out
}

/// The check after the copy, on the new folder `new_dir` (before it is used): links that reach game
/// files still reach the same file, and copies of game files have the same size and, where a CRC is
/// known, the same CRC. `manifest` is what was copied (the hashes of every file were compared already).
pub fn check_after(old_dir: &Path, new_dir: &Path, manifest: &Manifest, files: &GameFiles) -> Vec<Issue> {
    let mut issues = Vec::new();
    for entry in &manifest.entries {
        let Kind::Link { target, original } = &entry.kind else { continue };
        let old_link = old_dir.join(&entry.rel);
        let new_link = new_dir.join(&entry.rel);
        let old_abs = absolute_target(&old_link, original);
        if is_within(&old_abs, old_dir) {
            continue;
        }
        // A link that leaves the folder must reach the same file from the new place.
        let before = fs::canonicalize(&old_link).ok();
        let after = fs::canonicalize(&new_link).ok();
        let problem = match (before, after) {
            (Some(_), None) => Some("the link no longer reaches its file at the new place"),
            (Some(b), Some(a)) if a != b => Some("the link reaches a different file at the new place"),
            _ => None,
        };
        if let Some(problem) = problem {
            issues.push(Issue { file: name_of(&new_link), expected_at: absolute_target(&new_link, target), problem: problem.into() });
        }
    }
    for m in &files.mappings {
        let Some(dest) = &m.dest else { continue };
        let Ok(rel) = dest.strip_prefix(old_dir) else { continue };
        let new_dest = new_dir.join(rel);
        match m.method {
            Method::Copy => match fs::metadata(&new_dest) {
                Err(_) => issues.push(Issue { file: name_of(dest), expected_at: new_dest.clone(), problem: "the copied game file is missing at the new place".into() }),
                Ok(meta) if meta.len() != m.size => issues.push(Issue { file: name_of(dest), expected_at: new_dest.clone(), problem: "the copied game file has a different size at the new place".into() }),
                Ok(_) => {
                    if let Some(crc) = m.crc32 {
                        if crc32_of(&new_dest) != Some(crc) {
                            issues.push(Issue { file: name_of(dest), expected_at: new_dest.clone(), problem: "the copied game file does not match its known checksum at the new place".into() });
                        }
                    }
                }
            },
            Method::Symlink => {
                // An original that was already missing was reported (and accepted) before the copy.
                let same = match fs::canonicalize(&m.source) {
                    Ok(source) => fs::canonicalize(&new_dest).is_ok_and(|a| a == source),
                    Err(_) => true,
                };
                if !same {
                    issues.push(Issue { file: name_of(dest), expected_at: new_dest.clone(), problem: "the game file link does not reach the original at the new place".into() });
                }
            }
            Method::UseInPlace => {}
        }
    }
    issues
}

fn crc32_of(path: &Path) -> Option<u32> {
    use std::io::Read;
    let mut file = fs::File::open(path).ok()?;
    let mut buf = vec![0u8; 1 << 20];
    let mut crc = rom_library::hash::Crc32::new();
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            return Some(crc.finish());
        }
        crc.update(&buf[..n]);
    }
}
