//! Match scanned files to what a port needs.
//!
//! CRC-32 first: a file whose checksum (as stored, or converted to big-endian for an N64 image)
//! equals a required CRC is the file, whatever it is called. Only when nothing matches by checksum
//! does the file name get a say, and a name match is marked low-confidence. A name match whose
//! checksum disagrees with the CRC the catalog gives is reported as a wrong version, so a Europe dump
//! is never silently offered to a port that wants the USA one.

use super::requirements::{Alt, RomSpec, Slot, SlotKind};
use crate::core::models::Port;
use serde_json::Value;
use std::collections::HashMap;
use super::scan::RomFile;
use super::system::ByteOrder;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// The file's own CRC-32 equals a required one.
    Crc32,
    /// The CRC-32 equals a required one only after converting an N64 image to big-endian. The file
    /// on disk is not in the order the port expects.
    Crc32AfterByteOrderFix,
    /// The file's SHA-1 equals a required one (catalog entries that give SHA-1 instead of CRC-32).
    Sha1,
    /// The file name equals the required name.
    Name,
    /// Only the title (the name up to its first parenthesis) equals the required one.
    NameTitle,
}

impl Basis {
    pub fn is_low_confidence(self) -> bool {
        matches!(self, Basis::Name | Basis::NameTitle)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMatch {
    pub path: PathBuf,
    pub size: u64,
    /// CRC-32 of the file as it is on disk, so a screen can show "expected X, you have Y".
    pub crc32: u32,
    pub basis: Basis,
    pub byte_order: Option<ByteOrder>,
    /// Other files with exactly the same content (and any N64 byte order), so a duplicate is not
    /// mistaken for an ambiguity.
    pub also_at: Vec<PathBuf>,
    /// For a multi-file disc (`.cue` + `.bin`): the other files of the same set. `path` is the `.cue`
    /// when one matched; if only a `.bin` matched there is no `.cue` and `parts` is empty.
    pub parts: Vec<PathBuf>,
}

impl FileMatch {
    pub fn is_low_confidence(&self) -> bool {
        self.basis.is_low_confidence()
    }

    /// The file is an N64 image in a byte order a port expecting `.z64` cannot read as it is.
    pub fn needs_byte_order_conversion(&self) -> bool {
        matches!(self.byte_order, Some(ByteOrder::ByteSwapped | ByteOrder::LittleEndian))
    }

    /// The main file and the other files of its disc set: everything to connect for this slot.
    pub fn all_paths(&self) -> Vec<PathBuf> {
        std::iter::once(self.path.clone()).chain(self.parts.iter().cloned()).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotStatus {
    Found(FileMatch),
    /// Several different files match; the user picks.
    Ambiguous(Vec<FileMatch>),
    /// Files with the right name exist but their checksum is not one the catalog lists.
    WrongVersion(Vec<FileMatch>),
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotResult {
    /// `Slot::id`, stable across rewording of the catalog text: use it as `ConnectRequest::slot`.
    pub slot_id: String,
    pub label: String,
    /// What the slot asks for (names, checksums), so a screen can say what was expected.
    pub alternatives: Vec<Alt>,
    pub kind: SlotKind,
    pub optional: bool,
    pub vague: bool,
    pub status: SlotStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortStatus {
    /// Every required slot has one file.
    Found,
    /// Every required slot has a file, but at least one has several to choose from.
    Ambiguous,
    /// Some required slots have files and some do not.
    Partial,
    /// Files with the right names exist but none with a listed checksum.
    WrongVersion,
    Missing,
    /// The port states nothing at all about its files (it may need none).
    Unspecified,
    /// The port names a game but nothing a file can be checked against, and nothing matched by name.
    Unverifiable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortMatch {
    pub port_key: String,
    pub port_name: String,
    pub slots: Vec<SlotResult>,
    pub status: PortStatus,
}

/// "Legend of Zelda, The - Majora's Mask (USA)" -> "legendofzeldathemajorasmask".
fn key(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The title: everything before the first parenthesis or bracket.
fn title_key(s: &str) -> String {
    key(s.split(['(', '[']).next().unwrap_or(s))
}

fn file_stem(name: &str) -> &str {
    name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(name)
}

fn is_n64_ext(ext: &str) -> bool {
    matches!(ext, "z64" | "v64" | "n64")
}

fn extension_fits(alt: &Alt, file: &RomFile) -> bool {
    alt.exts.is_empty() || alt.exts.contains(&file.ext) || (is_n64_ext(&file.ext) && alt.exts.iter().any(|e| is_n64_ext(e)))
}

fn crc_basis(alt: &Alt, file: &RomFile) -> Option<Basis> {
    if alt.crc32.contains(&file.raw.crc32) {
        return Some(Basis::Crc32);
    }
    if file.raw.sha1.is_some_and(|h| alt.sha1.contains(&h)) {
        return Some(Basis::Sha1);
    }
    let normalized = file.normalized.as_ref()?;
    if alt.crc32.contains(&normalized.crc32) || normalized.sha1.is_some_and(|h| alt.sha1.contains(&h)) {
        return Some(Basis::Crc32AfterByteOrderFix);
    }
    None
}

/// Region words in parentheses: "(USA, Europe)" gives usa and europe.
fn regions(s: &str) -> Vec<&'static str> {
    const WORDS: [&str; 8] = ["usa", "europe", "japan", "world", "australia", "korea", "china", "brazil"];
    let lower = s.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut rest = lower.as_str();
    while let Some(open) = rest.find('(') {
        let Some(close) = rest[open..].find(')') else { break };
        let inside = &rest[open + 1..open + close];
        for w in WORDS {
            if inside.split(|c: char| !c.is_ascii_alphabetic()).any(|t| t == w) {
                out.push(w);
            }
        }
        rest = &rest[open + close + 1..];
    }
    out
}

fn name_basis(alt: &Alt, file: &RomFile) -> Option<Basis> {
    if !extension_fits(alt, file) && alt.tokens.is_empty() {
        return None;
    }
    let file_name = file.file_name();
    let stem = key(file_stem(&file_name));
    if stem.is_empty() {
        return None;
    }
    if alt.tokens.iter().any(|t| !t.is_empty() && stem.contains(t.as_str())) {
        return Some(Basis::Name);
    }
    // Declared names: the given file names, and for text without an extension the text itself. A
    // slash inside a name lists variants ("Name (USA)/Master Quest (Europe).z64"), each its own title.
    let mut declared: Vec<&str> = alt.names.iter().map(|n| file_stem(n)).collect();
    if declared.is_empty() && !alt.display.is_empty() {
        declared.push(alt.display.as_str());
    }
    if declared.iter().any(|d| key(d) == stem) {
        return Some(Basis::Name);
    }
    let title = title_key(&file_name_without_ext(&file_name));
    let titles = declared.iter().flat_map(|d| d.split('/')).map(title_key);
    if title.len() >= 4 && titles.into_iter().any(|t| t == title) {
        // The title is the same but the region tags disagree: a Europe dump is not the USA game the
        // port asked for, so it is not offered on the strength of a shared title.
        let file_regions = regions(&file_name);
        let declared_regions: Vec<&str> = declared.iter().flat_map(|d| regions(d)).collect();
        if !file_regions.is_empty() && !declared_regions.is_empty() && !file_regions.iter().any(|r| declared_regions.contains(r)) {
            return None;
        }
        return Some(Basis::NameTitle);
    }
    None
}

fn file_name_without_ext(name: &str) -> String {
    file_stem(name).to_string()
}

/// Same content, whatever the N64 byte order.
fn same_content(a: &RomFile, b: &RomFile) -> bool {
    a.size == b.size && a.canonical().crc32 == b.canonical().crc32 && a.canonical().md5 == b.canonical().md5
}

/// Collapse files with identical content into one candidate. The preferred file of a group is the one
/// already in the console's byte order, then the shortest path, then alphabetical.
fn group_candidates<'a>(hits: Vec<(&'a RomFile, Basis)>) -> Vec<FileMatch> {
    let mut groups: Vec<Vec<(&RomFile, Basis)>> = Vec::new();
    for hit in hits {
        match groups.iter_mut().find(|g| same_content(g[0].0, hit.0)) {
            Some(group) => group.push(hit),
            None => groups.push(vec![hit]),
        }
    }
    let mut out: Vec<FileMatch> = groups
        .into_iter()
        .map(|mut g| {
            g.sort_by_key(|(f, basis)| (f.needs_byte_order_conversion(), *basis as u8, f.path.as_os_str().len(), f.path.clone()));
            let (best, basis) = g[0];
            FileMatch {
                path: best.path.clone(),
                size: best.size,
                crc32: best.raw.crc32,
                basis,
                byte_order: best.byte_order,
                also_at: g[1..].iter().map(|(f, _)| f.path.clone()).collect(),
                parts: Vec::new(),
            }
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// The files of a `.cue`/`.bin` disc sit side by side with different content, and a multi-track
/// disc has a `.bin` per track ("Game (Track 1).bin" next to "Game.cue"). A hit in the same folder
/// whose name starts with a `.cue` hit's name belongs to that cue: the `.cue` is the entry point and
/// the rest are its parts. Hits with no `.cue` are grouped by folder and base name.
fn group_disc_sets<'a>(hits: Vec<(&'a RomFile, Basis)>) -> Vec<FileMatch> {
    let stem_key = |f: &RomFile| key(file_stem(&f.file_name()));
    let folder = |f: &RomFile| f.path.parent().map(PathBuf::from);
    let mut sets: Vec<((Option<PathBuf>, String), Vec<(&RomFile, Basis)>)> = Vec::new();
    // Cues first, longest name first, so a short cue name does not claim a longer cue's tracks.
    let mut cues: Vec<&(&RomFile, Basis)> = hits.iter().filter(|(f, _)| f.ext == "cue").collect();
    cues.sort_by_key(|(f, _)| std::cmp::Reverse(stem_key(f).len()));
    for (f, basis) in cues {
        sets.push(((folder(f), stem_key(f)), vec![(*f, *basis)]));
    }
    for (f, basis) in hits.iter().filter(|(f, _)| f.ext != "cue") {
        let k = (folder(f), stem_key(f));
        let owner = sets.iter_mut().find(|((dir, cue_key), g)| g[0].0.ext == "cue" && *dir == k.0 && k.1.starts_with(cue_key.as_str()));
        match owner {
            Some((_, group)) => group.push((*f, *basis)),
            None => match sets.iter_mut().find(|(sk, g)| *sk == k && g[0].0.ext != "cue") {
                Some((_, group)) => group.push((*f, *basis)),
                None => sets.push((k, vec![(*f, *basis)])),
            },
        }
    }
    let mut out: Vec<FileMatch> = sets
        .into_iter()
        .map(|(_, mut g)| {
            g.sort_by_key(|(f, basis)| (f.ext != "cue", *basis as u8, f.path.clone()));
            let (best, basis) = g[0];
            let has_cue = best.ext == "cue";
            FileMatch {
                path: best.path.clone(),
                size: best.size,
                crc32: best.raw.crc32,
                basis,
                byte_order: None,
                also_at: Vec::new(),
                parts: if has_cue { g[1..].iter().map(|(f, _)| f.path.clone()).collect() } else { Vec::new() },
            }
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn status_for_slot(slot: &Slot, files: &[RomFile]) -> SlotStatus {
    let mut crc_hits: Vec<(&RomFile, Basis)> = Vec::new();
    let mut name_hits: Vec<(&RomFile, Basis)> = Vec::new();
    let mut wrong_hits: Vec<(&RomFile, Basis)> = Vec::new();
    for alt in &slot.alternatives {
        for file in files {
            if let Some(basis) = crc_basis(alt, file) {
                crc_hits.push((file, basis));
                continue;
            }
            if let Some(basis) = name_basis(alt, file) {
                // A name match against a requirement that lists checksums, none of which the file has.
                if alt.crc32.is_empty() && alt.sha1.is_empty() {
                    name_hits.push((file, basis));
                } else {
                    wrong_hits.push((file, basis));
                }
            }
        }
    }
    let declared_disc = slot.alternatives.iter().any(|a| a.exts.iter().any(|e| e == "cue") && a.exts.iter().any(|e| e == "bin"));
    let pick = |hits: Vec<(&RomFile, Basis)>| {
        // A serial or title that matched a .cue file is a disc as well, even if the text never said bin/cue.
        let disc_set = declared_disc || hits.iter().any(|(f, _)| f.ext == "cue");
        let found = if disc_set { group_disc_sets(hits) } else { group_candidates(hits) };
        match found.len() {
            0 => None,
            1 => Some(SlotStatus::Found(found.into_iter().next().unwrap())),
            _ => Some(SlotStatus::Ambiguous(found)),
        }
    };
    if let Some(s) = pick(crc_hits) {
        return s;
    }
    if let Some(s) = pick(name_hits) {
        return s;
    }
    let wrong = if declared_disc || wrong_hits.iter().any(|(f, _)| f.ext == "cue") { group_disc_sets(wrong_hits) } else { group_candidates(wrong_hits) };
    if wrong.is_empty() {
        SlotStatus::Missing
    } else {
        SlotStatus::WrongVersion(wrong)
    }
}

/// A file satisfies at most one slot: once a slot has taken a file, later slots do not see it (or any
/// copy of it). Without this, a checksum list shared by several files ("game + IPL + disk") would let
/// the cartridge alone fill all three slots.
pub fn match_slots(spec: &RomSpec, files: &[RomFile]) -> Vec<SlotResult> {
    let mut taken: Vec<&RomFile> = Vec::new();
    let mut results = Vec::new();
    for slot in &spec.requirement.slots {
        let status = if taken.is_empty() {
            status_for_slot(slot, files)
        } else {
            let remaining: Vec<RomFile> = files.iter().filter(|f| !taken.iter().any(|t| same_content(t, f))).cloned().collect();
            status_for_slot(slot, &remaining)
        };
        if let SlotStatus::Found(m) = &status {
            taken.extend(files.iter().filter(|f| f.path == m.path));
        }
        results.push(SlotResult {
            slot_id: slot.id(),
            label: slot.label.clone(),
            alternatives: slot.alternatives.clone(),
            kind: slot.kind,
            optional: slot.optional,
            vague: slot.is_vague(),
            status,
        });
    }
    results
}

fn overall(slots: &[SlotResult]) -> PortStatus {
    // Optional slots never hold a port back, and a vague (title-only) slot that found nothing is
    // not evidence of anything missing.
    let required: Vec<&SlotResult> = slots.iter().filter(|s| !s.optional && !(s.vague && s.status == SlotStatus::Missing)).collect();
    if required.is_empty() {
        let only_vague_misses = !slots.is_empty() && slots.iter().all(|s| s.vague && s.status == SlotStatus::Missing);
        return if only_vague_misses { PortStatus::Unverifiable } else { PortStatus::Unspecified };
    }
    let count = |f: &dyn Fn(&SlotStatus) -> bool| required.iter().filter(|s| f(&s.status)).count();
    let found = count(&|s| matches!(s, SlotStatus::Found(_)));
    let ambiguous = count(&|s| matches!(s, SlotStatus::Ambiguous(_)));
    let wrong = count(&|s| matches!(s, SlotStatus::WrongVersion(_)));
    let missing = count(&|s| matches!(s, SlotStatus::Missing));
    let n = required.len();
    if found == n {
        PortStatus::Found
    } else if found + ambiguous == n {
        PortStatus::Ambiguous
    } else if missing == n {
        PortStatus::Missing
    } else if found + ambiguous == 0 && wrong > 0 {
        PortStatus::WrongVersion
    } else {
        PortStatus::Partial
    }
}

/// Match one port against a scan. `port_key` is `Port::key()`.
pub fn match_port(port_key: &str, port_name: &str, spec: &RomSpec, files: &[RomFile]) -> PortMatch {
    let slots = match_slots(spec, files);
    let status = overall(&slots);
    PortMatch { port_key: port_key.to_string(), port_name: port_name.to_string(), slots, status }
}

/// Match every port against one scan. `local_entries` is `requirements::load_local_entries`, by folder.
pub fn match_all(ports: &[Port], local_entries: &HashMap<String, Value>, files: &[RomFile]) -> Vec<PortMatch> {
    ports
        .iter()
        .map(|p| {
            let spec = RomSpec::for_port(p, local_entries.get(&p.folder));
            match_port(p.key(), &p.name, &spec, files)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::connect::Access;
    use super::super::requirements::parse_instructions;
    use super::super::scan::{scan, ScanOptions};
    use super::super::testutil::*;
    use super::*;
    use std::path::Path;
    use std::sync::atomic::AtomicBool;

    fn scan_dir(root: &Path) -> Vec<RomFile> {
        scan(root, &ScanOptions::default(), None, &AtomicBool::new(false), &mut |_| {}).unwrap().files
    }

    fn spec(instructions: &str) -> RomSpec {
        RomSpec { requirement: parse_instructions(instructions), access: Access::Link, dest_subdir: String::new() }
    }

    fn crc_text(data: &[u8]) -> String {
        format!("{:08X}", digests_of(data).crc32)
    }

    #[test]
    fn crc_match_ignores_the_file_name() {
        let dir = temp_dir();
        let rom = random_bytes(1, 5000);
        write(&dir.path().join("totally renamed.sfc"), &rom);
        write(&dir.path().join("decoy.sfc"), &random_bytes(2, 5000));
        let m = match_port("k", "Game", &spec(&format!("Required files: Real Name (USA).sfc (CRC32: {})", crc_text(&rom))), &scan_dir(dir.path()));
        assert_eq!(m.status, PortStatus::Found);
        let SlotStatus::Found(f) = &m.slots[0].status else { panic!("{:?}", m.slots) };
        assert_eq!(f.basis, Basis::Crc32);
        assert!(f.path.ends_with("totally renamed.sfc"));
        assert!(!f.is_low_confidence());
    }

    #[test]
    fn a_v64_image_matches_the_big_endian_crc_and_says_it_needs_conversion() {
        let dir = temp_dir();
        let z64 = n64_rom(3, 8192);
        write(&dir.path().join("game.v64"), &to_v64(&z64));
        let m = match_port("k", "Game", &spec(&format!("Required files: Game (USA).z64 (CRC32: {})", crc_text(&z64))), &scan_dir(dir.path()));
        let SlotStatus::Found(f) = &m.slots[0].status else { panic!("{:?}", m.slots) };
        assert_eq!(f.basis, Basis::Crc32AfterByteOrderFix);
        assert!(f.needs_byte_order_conversion());
        assert_eq!(f.byte_order, Some(ByteOrder::ByteSwapped));
    }

    #[test]
    fn the_big_endian_copy_is_preferred_over_swapped_ones_and_the_others_are_listed_not_ambiguous() {
        let dir = temp_dir();
        let z64 = n64_rom(4, 4096);
        write(&dir.path().join("a_game.v64"), &to_v64(&z64));
        write(&dir.path().join("z_game.z64"), &z64);
        write(&dir.path().join("backup/game.n64"), &to_n64(&z64));
        let m = match_port("k", "Game", &spec(&format!("Required files: Game.z64 (CRC32: {})", crc_text(&z64))), &scan_dir(dir.path()));
        assert_eq!(m.status, PortStatus::Found);
        let SlotStatus::Found(f) = &m.slots[0].status else { panic!() };
        assert!(f.path.ends_with("z_game.z64"));
        assert_eq!(f.also_at.len(), 2);
        assert!(!f.needs_byte_order_conversion());
    }

    #[test]
    fn name_only_match_is_flagged_low_confidence() {
        let dir = temp_dir();
        write(&dir.path().join("Ace Combat 5 - The Unsung War (USA) (En,Ja).iso"), &random_bytes(5, 3000));
        let m = match_port("k", "AC5", &spec("Required files: Ace Combat 5 - The Unsung War (USA) (En,Ja).iso"), &scan_dir(dir.path()));
        assert_eq!(m.status, PortStatus::Found);
        let SlotStatus::Found(f) = &m.slots[0].status else { panic!() };
        assert_eq!(f.basis, Basis::Name);
        assert!(f.is_low_confidence());
    }

    #[test]
    fn title_only_name_match_is_the_weakest() {
        let dir = temp_dir();
        write(&dir.path().join("Ace Combat 5 - The Unsung War (USA).iso"), &random_bytes(6, 3000));
        let m = match_port("k", "AC5", &spec("Required files: Ace Combat 5 - The Unsung War (USA) (En,Ja).iso"), &scan_dir(dir.path()));
        let SlotStatus::Found(f) = &m.slots[0].status else { panic!("{:?}", m.slots) };
        assert_eq!(f.basis, Basis::NameTitle);
    }

    #[test]
    fn right_name_wrong_checksum_is_a_wrong_version_not_a_match() {
        let dir = temp_dir();
        let europe = n64_rom(7, 4096);
        let usa = n64_rom(8, 4096);
        write(&dir.path().join("Mario Kart 64 (USA).z64"), &europe);
        let m = match_port("k", "MK64", &spec(&format!("Required files: Mario Kart 64 (USA).z64 (CRC32: {})", crc_text(&usa))), &scan_dir(dir.path()));
        assert_eq!(m.status, PortStatus::WrongVersion);
        assert!(matches!(&m.slots[0].status, SlotStatus::WrongVersion(v) if v.len() == 1));
        // The same port finds the right dump once it is there, even under another name.
        write(&dir.path().join("other.z64"), &usa);
        let m = match_port("k", "MK64", &spec(&format!("Required files: Mario Kart 64 (USA).z64 (CRC32: {})", crc_text(&usa))), &scan_dir(dir.path()));
        assert_eq!(m.status, PortStatus::Found);
    }

    #[test]
    fn two_different_dumps_that_both_qualify_are_ambiguous() {
        let dir = temp_dir();
        let (usa, eur) = (n64_rom(9, 4096), n64_rom(10, 4096));
        write(&dir.path().join("pd_usa.z64"), &usa);
        write(&dir.path().join("pd_eur.z64"), &eur);
        let text = format!("Required files: Perfect Dark (USA)/(Europe).z64 (CRC32: {}/{})", crc_text(&usa), crc_text(&eur));
        let m = match_port("k", "PD", &spec(&text), &scan_dir(dir.path()));
        assert_eq!(m.status, PortStatus::Ambiguous);
        assert!(matches!(&m.slots[0].status, SlotStatus::Ambiguous(v) if v.len() == 2));
    }

    #[test]
    fn two_name_matches_with_different_content_are_ambiguous() {
        let dir = temp_dir();
        write(&dir.path().join("a/Game.iso"), &random_bytes(11, 2000));
        write(&dir.path().join("b/Game.iso"), &random_bytes(12, 2000));
        let m = match_port("k", "G", &spec("Required files: Game.iso"), &scan_dir(dir.path()));
        assert_eq!(m.status, PortStatus::Ambiguous);
    }

    #[test]
    fn identical_copies_in_two_folders_are_one_file_with_a_duplicate() {
        let dir = temp_dir();
        let data = random_bytes(13, 2000);
        write(&dir.path().join("a/Game.iso"), &data);
        write(&dir.path().join("b/Game.iso"), &data);
        let m = match_port("k", "G", &spec(&format!("Required files: Game.iso (CRC32: {})", crc_text(&data))), &scan_dir(dir.path()));
        assert_eq!(m.status, PortStatus::Found);
        let SlotStatus::Found(f) = &m.slots[0].status else { panic!() };
        assert_eq!(f.also_at.len(), 1);
    }

    #[test]
    fn nothing_there_is_missing() {
        let dir = temp_dir();
        write(&dir.path().join("unrelated.gba"), &random_bytes(14, 100));
        let m = match_port("k", "G", &spec("Required files: Game (USA).z64 (CRC32: 00000001)"), &scan_dir(dir.path()));
        assert_eq!(m.status, PortStatus::Missing);
        assert_eq!(m.slots[0].status, SlotStatus::Missing);
    }

    #[test]
    fn no_requirements_is_unspecified() {
        let m = match_port("k", "G", &spec("Languages: English."), &[]);
        assert_eq!(m.status, PortStatus::Unspecified);
        assert!(m.slots.is_empty());
    }

    #[test]
    fn game_found_but_bios_missing_is_partial_unless_the_bios_is_optional() {
        let dir = temp_dir();
        let game = random_bytes(15, 4000);
        write(&dir.path().join("game.gba"), &game);
        let text = format!("Required files: Game (USA).gba (CRC32: {}) + GBA BIOS (CRC32: 81977335)", crc_text(&game));
        let m = match_port("k", "G", &spec(&text), &scan_dir(dir.path()));
        assert_eq!(m.status, PortStatus::Partial);
        assert!(matches!(m.slots[1].status, SlotStatus::Missing));

        let mut optional = spec(&text);
        optional.requirement.slots[1].optional = true;
        assert_eq!(match_port("k", "G", &optional, &scan_dir(dir.path())).status, PortStatus::Found);
    }

    #[test]
    fn a_bin_cue_pair_is_found_by_either_checksum() {
        let dir = temp_dir();
        let (bin, cue) = (random_bytes(16, 6000), b"FILE \"Game.bin\" BINARY\n  TRACK 01 MODE2/2352\n".to_vec());
        write(&dir.path().join("psx/Game (USA).bin"), &bin);
        write(&dir.path().join("psx/Game (USA).cue"), &cue);
        let text = format!("Required files: Game (USA).bin/cue (CRC32: {}/{})", crc_text(&bin), crc_text(&cue));
        let m = match_port("k", "G", &spec(&text), &scan_dir(dir.path()));
        // Both files qualify and differ in content, but they are one disc: the .cue leads, the .bin is a part.
        assert_eq!(m.status, PortStatus::Found);
        let SlotStatus::Found(f) = &m.slots[0].status else { panic!("{:?}", m.slots) };
        assert!(f.path.ends_with("Game (USA).cue"));
        assert_eq!(f.parts.len(), 1);
        assert!(f.parts[0].ends_with("Game (USA).bin"));
        assert_eq!(f.basis, Basis::Crc32);
        // Two different discs in different folders are still two answers.
        write(&dir.path().join("other/Game (USA).bin"), &bin);
        let m = match_port("k", "G", &spec(&text), &scan_dir(dir.path()));
        assert_eq!(m.status, PortStatus::Ambiguous);
    }

    #[test]
    fn serial_tokens_match_file_names() {
        let dir = temp_dir();
        write(&dir.path().join("psx/SLUS-00972.chd"), &random_bytes(17, 500));
        let m = match_port("k", "G", &spec("Required files: SLUS-00972. BIOS: Regional SCPH"), &scan_dir(dir.path()));
        assert!(matches!(m.slots[0].status, SlotStatus::Found(_)));
        assert!(matches!(m.slots[1].status, SlotStatus::Missing));
        assert_eq!(m.status, PortStatus::Partial);
    }

    #[test]
    fn extension_must_fit_a_name_match() {
        let dir = temp_dir();
        write(&dir.path().join("Game.txt.gba"), &random_bytes(18, 500));
        write(&dir.path().join("Game.sfc"), &random_bytes(19, 500));
        let m = match_port("k", "G", &spec("Required files: Game.z64"), &scan_dir(dir.path()));
        assert_eq!(m.status, PortStatus::Missing);
    }

    #[test]
    fn a_vague_title_that_matches_nothing_is_unverifiable_not_missing() {
        let dir = temp_dir();
        write(&dir.path().join("x.gba"), &random_bytes(20, 100));
        let m = match_port("k", "G", &spec("Required files: Saru Get You: Saru Saru Daisakusen"), &scan_dir(dir.path()));
        assert_eq!(m.status, PortStatus::Unverifiable);
        write(&dir.path().join("Saru Get You Saru Saru Daisakusen.iso"), &random_bytes(21, 100));
        let found = match_port("k", "G", &spec("Required files: Saru Get You: Saru Saru Daisakusen"), &scan_dir(dir.path()));
        assert_eq!(found.status, PortStatus::Found);
    }

    #[test]
    fn explicit_crcs_in_a_local_entry_match_like_catalog_ones() {
        let dir = temp_dir();
        let iso = random_bytes(22, 3000);
        write(&dir.path().join("gc/whatever.iso"), &iso);
        let entry = serde_json::json!({"name": "TP", "folder": "tp", "rom_files": [{"name": "tp.iso", "crc32": crc_text(&iso)}]});
        let port = crate::core::models::port_from_value(&entry).unwrap();
        let spec = RomSpec::for_port(&port, Some(&entry));
        let m = match_port(port.key(), &port.name, &spec, &scan_dir(dir.path()));
        assert_eq!(m.status, PortStatus::Found);
    }

    #[test]
    fn a_title_match_with_a_different_region_is_not_offered() {
        let dir = temp_dir();
        write(&dir.path().join("Ace Combat 5 - The Unsung War (Europe).iso"), &random_bytes(30, 3000));
        let m = match_port("k", "AC5", &spec("Required files: Ace Combat 5 - The Unsung War (USA) (En,Ja).iso"), &scan_dir(dir.path()));
        assert_eq!(m.status, PortStatus::Missing);
        // A shared region, or no region tag on the file, still matches by title.
        write(&dir.path().join("Ace Combat 5 - The Unsung War (USA, Europe).iso"), &random_bytes(31, 3000));
        let m = match_port("k", "AC5", &spec("Required files: Ace Combat 5 - The Unsung War (USA) (En,Ja).iso"), &scan_dir(dir.path()));
        assert_eq!(m.status, PortStatus::Found);
    }

    #[test]
    fn a_multi_track_redump_disc_is_one_answer_led_by_the_cue() {
        let dir = temp_dir();
        let tracks: Vec<Vec<u8>> = (0..3).map(|i| random_bytes(40 + i, 4000)).collect();
        let cue = b"FILE \"Tekken 3 (USA) (Track 1).bin\" BINARY\n".to_vec();
        for (i, t) in tracks.iter().enumerate() {
            write(&dir.path().join(format!("psx/Tekken 3 (USA)/Tekken 3 (USA) (Track {}).bin", i + 1)), t);
        }
        write(&dir.path().join("psx/Tekken 3 (USA)/Tekken 3 (USA).cue"), &cue);
        let crcs: Vec<String> = tracks.iter().map(|t| crc_text(t)).chain(std::iter::once(crc_text(&cue))).collect();
        let text = format!("Required files: Tekken 3 (USA) (Track 1/2/3).bin/cue (CRC32: {})", crcs.join("/"));
        let m = match_port("k", "T3", &spec(&text), &scan_dir(dir.path()));
        assert_eq!(m.status, PortStatus::Found, "{:#?}", m.slots);
        let SlotStatus::Found(f) = &m.slots[0].status else { panic!() };
        assert!(f.path.ends_with("Tekken 3 (USA).cue"));
        assert_eq!(f.parts.len(), 3);
        assert_eq!(f.all_paths().len(), 4);
    }

    #[test]
    fn a_serial_named_bin_and_cue_are_one_disc() {
        let dir = temp_dir();
        write(&dir.path().join("psx/SLUS-00972.bin"), &random_bytes(50, 3000));
        write(&dir.path().join("psx/SLUS-00972.cue"), b"FILE \"SLUS-00972.bin\" BINARY\n");
        let m = match_port("k", "G", &spec("Required files: SLUS-00972"), &scan_dir(dir.path()));
        assert_eq!(m.status, PortStatus::Found, "{:#?}", m.slots);
        let SlotStatus::Found(f) = &m.slots[0].status else { panic!() };
        assert!(f.path.ends_with("SLUS-00972.cue") && f.parts.len() == 1);
    }

    #[test]
    fn a_bin_without_its_cue_is_still_found_but_has_no_parts() {
        let dir = temp_dir();
        let bin = random_bytes(51, 3000);
        write(&dir.path().join("Game (USA).bin"), &bin);
        let text = format!("Required files: Game (USA).bin/cue (CRC32: {}/00000001)", crc_text(&bin));
        let m = match_port("k", "G", &spec(&text), &scan_dir(dir.path()));
        let SlotStatus::Found(f) = &m.slots[0].status else { panic!("{:#?}", m.slots) };
        assert!(f.path.ends_with("Game (USA).bin") && f.parts.is_empty());
    }

    #[test]
    fn a_sha1_requirement_matches_by_sha1_and_a_wrong_dump_with_the_right_name_does_not() {
        let dir = temp_dir();
        let rom = n64_rom(52, 4096);
        let sha1 = digests_of(&rom).sha1_hex().unwrap();
        write(&dir.path().join("renamed.z64"), &rom);
        write(&dir.path().join("BattleTanx - Global Assault (USA).z64"), &n64_rom(53, 4096));
        let text = format!("Required files: BattleTanx - Global Assault (USA).z64 (SHA-1: {sha1})");
        let m = match_port("k", "BT", &spec(&text), &scan_dir(dir.path()));
        assert_eq!(m.status, PortStatus::Found);
        let SlotStatus::Found(f) = &m.slots[0].status else { panic!() };
        assert_eq!(f.basis, Basis::Sha1);
        assert!(f.path.ends_with("renamed.z64"));
        let only_wrong = tempfile::tempdir().unwrap();
        write(&only_wrong.path().join("BattleTanx - Global Assault (USA).z64"), &n64_rom(53, 4096));
        assert_eq!(match_port("k", "BT", &spec(&text), &scan_dir(only_wrong.path())).status, PortStatus::WrongVersion);
    }

    #[test]
    fn f_zero_x_with_only_the_cartridge_is_partial_not_found() {
        let dir = temp_dir();
        let game = n64_rom(54, 4096);
        write(&dir.path().join("F-Zero X (USA).z64"), &game);
        let text = format!("Required files: F-Zero X (USA)/[BIOS] Nintendo 64DD IPL (Japan) (v1.2).z64/NUD-EFZE-USA_2.ndd (CRC32: {}/7F933CE2/EBDFBD2B)", crc_text(&game));
        let m = match_port("k", "FZ", &spec(&text), &scan_dir(dir.path()));
        assert_eq!(m.status, PortStatus::Partial, "{:#?}", m.slots);
    }

    #[test]
    fn slot_results_carry_an_id_and_what_was_expected_and_the_files_own_crc() {
        let dir = temp_dir();
        let rom = random_bytes(55, 2000);
        write(&dir.path().join("x.sfc"), &rom);
        let m = match_port("k", "G", &spec(&format!("Required files: Real.sfc (CRC32: {})", crc_text(&rom))), &scan_dir(dir.path()));
        assert_eq!(m.slots[0].slot_id.len(), 8);
        assert_eq!(m.slots[0].alternatives[0].names, vec!["Real.sfc"]);
        let SlotStatus::Found(f) = &m.slots[0].status else { panic!() };
        assert_eq!(f.crc32, digests_of(&rom).crc32);
    }

    #[test]
    fn match_all_uses_local_entries_by_folder() {
        let dir = temp_dir();
        let rom = random_bytes(56, 2000);
        write(&dir.path().join("x.iso"), &rom);
        let catalog = crate::core::config::parse_catalog(
            r#"{"ports":[{"name":"A","folder":"a","instructions":"Required files: nothing.iso (CRC32: 00000001)"},{"name":"B","folder":"b"}]}"#,
        )
        .unwrap();
        let mut local = HashMap::new();
        local.insert("a".to_string(), serde_json::json!({"folder": "a", "rom_files": [{"name": "x.iso", "crc32": crc_text(&rom)}]}));
        let all = match_all(&catalog, &local, &scan_dir(dir.path()));
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].status, PortStatus::Found);
        assert_eq!(all[1].status, PortStatus::Unspecified);
    }
}
