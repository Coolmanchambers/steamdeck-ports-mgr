//! What game files a port needs, read from the catalog's free text or from explicit fields.
//!
//! The catalog states it in an `instructions` string, by hand, so the shapes vary. The ones that
//! occur in `ports.json` (checked by tests that parse every entry and pin the hard ones):
//!
//! ```text
//! Required files: Bomberman Hero (USA).z64 (CRC32: 2CC2E634)
//! Required files: Perfect Dark (USA)/(Japan)/(Europe) (En,Fr,De,Es,It).z64 (CRC32: 68446AD4/639C0DA9/7718A714)
//! Required files: Bloody Roar II (USA).bin/cue (CRC32: F5EC130A/5679034E)
//! Required files: Parasite Eve (USA) (Disc 1/2).bin (CRC32: 488641E0/226EE2EE)
//! Required files: Mega Man Zero (USA, Europe).gba + GBA BIOS (CRC32: 9707D2A1/81977335)
//! Required files: F-Zero X (USA)/[BIOS] Nintendo 64DD IPL (Japan) (v1.2).z64/NUD-EFZE-USA_2.ndd (CRC32: ...)
//! Required files: Super Smash Bros. (USA).z64 (CRC32: EB97929E) or Nintendo All-Star! ... (Japan).z64 (CRC32: 04C9D3B1)
//! Required files: Spider-Man (USA).bin/cue (CRC32: BF1C7EFF/5B840D78), Spider-Man 2 ... .bin/cue (CRC32: ...)
//! Required files: BattleTanx - Global Assault (USA).z64 (SHA-1: 805248FB...)
//! Required files: SLUS-00972. BIOS: Regional SCPH
//! Required files:            (a bulleted list of files with CRCs on the following lines)
//! Required files: USA game ISO            (nothing checkable)
//! ```
//!
//! The model is a list of slots (files that must all be present). A slot has one or more
//! alternatives (any one will do), and an alternative has file names, checksums that identify it,
//! and extra name tokens (disc serials, BIOS names). A checksum list inside one clause is "any of
//! these" when it lists regions or revisions or the parts of one disc (`bin/cue`, `Track 1/2/3`);
//! it is split into one slot per file when the text names several files (`Disc 1/2`, a game plus
//! its BIOS, `Doom/2.wad`).
//!
//! Local entries (`ports.local.json`) can state this explicitly and skip the guessing:
//!
//! ```json
//! "rom_files": [ {"name": "tp.iso", "crc32": ["1A2B3C4D"]}, {"name": "scph1001.bin", "bios": true, "optional": true} ],
//! "rom_access": "link",
//! "rom_dest": "isos"
//! ```

use super::connect::Access;
use super::hash::{crc32_of, from_hex, to_hex};
use super::system::is_rom_extension;
use crate::core::models::Port;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    Game,
    /// A console BIOS or boot ROM: needed, but not the game itself.
    Bios,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Alt {
    /// The text as written, minus the checksum clause.
    pub display: String,
    /// Full file names when the text gave an extension (`bin/cue` gives two).
    pub names: Vec<String>,
    /// Lowercase extensions the text gave, no dot.
    pub exts: Vec<String>,
    pub crc32: Vec<u32>,
    pub sha1: Vec<[u8; 20]>,
    /// Lowercase alphanumeric fragments a file name should contain (disc serials, BIOS names).
    pub tokens: Vec<String>,
    /// The text named no file, checksum or serial: a game title and nothing more to check against.
    pub title_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub label: String,
    pub kind: SlotKind,
    pub optional: bool,
    pub alternatives: Vec<Alt>,
}

impl Slot {
    /// A slot with nothing to check a file against beyond a title.
    pub fn is_vague(&self) -> bool {
        self.alternatives.iter().all(|a| a.title_only)
    }

    /// A short stable id for recording a connection. It is derived from what the slot asks for (names,
    /// checksums, tokens), not from the catalog's wording, so rewording the prose around it does not
    /// orphan a saved mapping.
    pub fn id(&self) -> String {
        let mut text = String::new();
        for alt in &self.alternatives {
            for n in &alt.names {
                text.push_str(&squash(n));
                text.push('|');
            }
            for c in &alt.crc32 {
                text.push_str(&format!("{c:08x}|"));
            }
            for s in &alt.sha1 {
                text.push_str(&to_hex(s));
                text.push('|');
            }
            for t in &alt.tokens {
                text.push_str(t);
                text.push('|');
            }
            if alt.names.is_empty() && alt.crc32.is_empty() && alt.tokens.is_empty() {
                text.push_str(&squash(&alt.display));
            }
            text.push(';');
        }
        format!("{:08x}", crc32_of(text.as_bytes()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Requirement {
    pub slots: Vec<Slot>,
    /// True when it came from explicit `rom_files` fields instead of the instructions text.
    pub explicit: bool,
}

/// Everything the library needs to know about one port's game files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomSpec {
    pub requirement: Requirement,
    /// The most economical way this port can reach a file.
    pub access: Access,
    /// Folder inside the port's folder where a link or copy goes ("" = the port folder itself).
    pub dest_subdir: String,
}

impl RomSpec {
    /// `local_entry` is the port's raw object from `ports.local.json`, when it has one: it is the only
    /// place explicit `rom_*` fields can live, because the app's own parser drops unknown fields.
    pub fn for_port(port: &Port, local_entry: Option<&Value>) -> RomSpec {
        let requirement = local_entry
            .and_then(explicit_requirement)
            .unwrap_or_else(|| parse_instructions(&port.instructions));
        let access = local_entry
            .and_then(|e| e.get("rom_access"))
            .and_then(Value::as_str)
            .and_then(Access::from_id)
            .unwrap_or_else(|| builtin_access(&port.folder));
        let dest_subdir = local_entry.and_then(|e| e.get("rom_dest")).and_then(Value::as_str).unwrap_or("").trim_matches('/').to_string();
        RomSpec { requirement, access, dest_subdir }
    }
}

/// The raw port objects of a `ports.local.json`, by `folder`, for `RomSpec::for_port`. A missing or
/// unreadable file gives an empty map (the same leniency as the app's own loader).
pub fn load_local_entries(path: &Path) -> HashMap<String, Value> {
    let Ok(text) = std::fs::read_to_string(path) else { return HashMap::new() };
    let Ok(data) = serde_json::from_str::<Value>(text.trim_start_matches('\u{FEFF}')) else { return HashMap::new() };
    data.get("ports")
        .and_then(Value::as_array)
        .map(|ports| {
            ports
                .iter()
                .filter_map(|p| Some((p.get("folder")?.as_str()?.to_string(), p.clone())))
                .collect()
        })
        .unwrap_or_default()
}

/// Ports known to read the game file once and build their own assets from it. From the findings
/// document, not yet confirmed on a Deck; a local entry's `rom_access` overrides it.
/// Files a port makes from its game file on first start, in its own folder (the HarbourMasters ports turn the
/// ROM into an `.o2r` archive and do not need the ROM again). One of them there means the port has its game.
pub fn generated_archives(folder: &str) -> &'static [&'static str] {
    match folder.to_ascii_lowercase().as_str() {
        "shipwright" => &["oot.o2r", "oot-mq.o2r"],
        "2ship2harkinian" => &["mm.o2r"],
        _ => &[],
    }
}

fn builtin_access(folder: &str) -> Access {
    match folder.to_ascii_lowercase().as_str() {
        "shipwright" | "2ship2harkinian" => Access::InPlace,
        _ => Access::Link,
    }
}

fn parse_crc(token: &str) -> Option<u32> {
    let t = token.trim();
    (t.len() == 8 && t.bytes().all(|b| b.is_ascii_hexdigit())).then(|| u32::from_str_radix(t, 16).ok()).flatten()
}

fn parse_sha1(token: &str) -> Option<[u8; 20]> {
    let t = token.trim();
    if t.len() != 40 {
        return None;
    }
    from_hex(t)?.try_into().ok()
}

fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Split on `sep` where it is not inside parentheses or brackets.
fn top_level_split<'a>(s: &'a str, sep: &str) -> Vec<&'a str> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let c = rest.chars().next().unwrap();
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 && rest.starts_with(sep) => {
                parts.push(s[start..i].trim());
                i += sep.len();
                start = i;
                continue;
            }
            _ => {}
        }
        i += c.len_utf8();
    }
    parts.push(s[start..].trim());
    parts.retain(|p| !p.is_empty());
    parts
}

#[derive(Default)]
struct Clauses {
    crc32: Vec<u32>,
    sha1: Vec<[u8; 20]>,
}

impl Clauses {
    fn any(&self) -> bool {
        !self.crc32.is_empty() || !self.sha1.is_empty()
    }
}

/// Cut `(CRC32: AAAAAAAA/BBBBBBBB)` and `(SHA-1: ...)` clauses out of a piece. Returns the text before
/// the first one and the values. Anything after a clause (" for all language.") is dropped, and a
/// clause with no valid value (`(CRC32: )`) counts as no clause.
fn split_clauses(piece: &str) -> (&str, Clauses) {
    const MARKERS: [&str; 3] = ["(crc32", "(sha-1", "(sha1"];
    let lower = piece.to_ascii_lowercase();
    let first = MARKERS.iter().filter_map(|m| lower.find(m)).min();
    let Some(first) = first else { return (piece.trim(), Clauses::default()) };
    let mut clauses = Clauses::default();
    for marker in MARKERS {
        let mut from = 0;
        while let Some(at) = lower[from..].find(marker) {
            let open = from + at;
            let close = piece[open..].find(')').map(|c| open + c).unwrap_or(piece.len());
            let inner = piece[open + marker.len()..close].trim_start_matches([':', ' ']);
            let tokens = inner.split(['/', ',', ' ']);
            if marker == "(crc32" {
                clauses.crc32.extend(tokens.filter_map(parse_crc));
            } else {
                clauses.sha1.extend(tokens.filter_map(parse_sha1));
            }
            from = close.max(open + marker.len());
        }
    }
    (piece[..first].trim(), clauses)
}

/// `Name (USA).z64` gives (`Name (USA)`, [z64]); `Name.bin/cue` and `Name.bin/.cue` give two
/// extensions; `Name (Rev 1)bin/cue` (a typo in the catalog) is accepted too. Extensions keep the
/// case they were written in.
fn extract_exts(name: &str) -> Option<(&str, Vec<String>)> {
    let name = name.trim().trim_end_matches('.');
    let lower = name.to_ascii_lowercase();
    if lower.ends_with("bin/cue") && !lower.ends_with(".bin/cue") {
        let cut = name.len() - "bin/cue".len();
        return Some((name[..cut].trim_end(), vec![name[cut..cut + 3].to_string(), name[cut + 4..].to_string()]));
    }
    let valid = |e: &str| !e.is_empty() && e.len() <= 5 && e.bytes().all(|b| b.is_ascii_alphanumeric()) && is_rom_extension(e);
    let dot = name.rfind('.')?;
    let tail = &name[dot + 1..];
    if tail.is_empty() || tail.contains(' ') {
        return None;
    }
    // "Name.bin/.cue": the last dot belongs to the second extension, so look one dot further back.
    if name[..dot].ends_with('/') {
        let head = &name[..dot - 1];
        if let Some(d) = head.rfind('.') {
            let first = &head[d + 1..];
            if valid(first) && valid(tail) {
                return Some((head[..d].trim_end(), vec![first.to_string(), tail.to_string()]));
            }
        }
    }
    let exts: Vec<String> = tail.split('/').map(|t| t.trim_start_matches('.').to_string()).collect();
    exts.iter().all(|e| valid(e)).then(|| (name[..dot].trim_end(), exts))
}

fn is_file_like(piece: &str) -> bool {
    let (name, clauses) = split_clauses(piece);
    clauses.any() || extract_exts(name).is_some()
}

/// Split only when every part looks like a file; otherwise the separator was part of a title
/// ("Legend of Zelda, The - Majora's Mask").
fn split_if_file_like<'a>(piece: &'a str, sep: &str) -> Vec<&'a str> {
    let parts = top_level_split(piece, sep);
    if parts.len() > 1 && parts.iter().all(|p| is_file_like(p)) {
        parts
    } else {
        vec![piece]
    }
}

/// "A.z64/B.ndd (CRC32: ...)": a slash right after an extension, followed by another file name, joins
/// separate files. `.bin/cue` is one disc and stays whole; `Name (USA)/(Japan).z64` has no extension
/// before its slash and stays whole too.
fn split_after_ext_slash(s: &str) -> Vec<&str> {
    for (i, c) in s.char_indices() {
        if c != '/' {
            continue;
        }
        let before = &s[..i];
        let after = s[i + 1..].trim_start();
        let ext_before = before.rfind('.').map(|d| &before[d + 1..]).is_some_and(|e| !e.is_empty() && e.bytes().all(|b| b.is_ascii_alphanumeric()) && is_rom_extension(e));
        let first_word = after.split(|ch: char| !ch.is_ascii_alphanumeric()).next().unwrap_or("");
        let bare_ext = is_rom_extension(first_word) && !after[first_word.len()..].starts_with('.');
        if ext_before && !after.starts_with('.') && after.len() > 4 && !bare_ext && is_file_like(after) {
            let mut out = vec![before.trim()];
            out.extend(split_after_ext_slash(after));
            return out;
        }
    }
    vec![s]
}

/// "A/B.z64/C.ndd (CRC32: x/y/z)": after `split_after_ext_slash`, the slash-separated names are as many
/// as the checksums, so every name is its own file with its own checksum, in order. A name without an
/// extension takes the one that ends its part ("A/B.z64" is "A.z64" and "B.z64"). Variants like
/// "(USA)/(Japan)" are not files, so their presence cancels this.
fn split_each_segment(parts: &[&str]) -> Option<Vec<Vec<(Alt, bool)>>> {
    let (last_name, clauses) = split_clauses(parts.last()?);
    let wanted = clauses.crc32.len();
    let mut names: Vec<String> = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        let text = if i + 1 == parts.len() { last_name } else { part };
        let segments = top_level_split(text, "/");
        let (_, exts) = extract_exts(segments.last()?)?;
        if segments.iter().any(|s| s.starts_with('(')) {
            return None;
        }
        for seg in segments {
            names.push(if extract_exts(seg).is_some() { seg.to_string() } else { format!("{seg}.{}", exts[0]) });
        }
    }
    if wanted < 2 || names.len() != wanted {
        return None;
    }
    Some(
        names
            .iter()
            .zip(&clauses.crc32)
            .map(|(name, crc)| {
                let (mut alt, _) = parse_alt(name);
                alt.crc32 = vec![*crc];
                vec![(alt, true)]
            })
            .collect(),
    )
}

fn kind_of(display: &str) -> SlotKind {
    let l = display.to_ascii_lowercase();
    if l.contains("bios") || l.contains("scph") || l.contains("boot.bin") || l.contains("boot.rom") {
        SlotKind::Bios
    } else {
        SlotKind::Game
    }
}

/// Parse one alternative. The bool says whether it carried a checksum clause of its own.
fn parse_alt(piece: &str) -> (Alt, bool) {
    let (name_part, clauses) = split_clauses(piece);
    let had_clause = clauses.any();
    let name_part = name_part.trim().trim_end_matches('.').trim();
    let (names, exts) = match extract_exts(name_part) {
        Some((base, exts)) => (exts.iter().map(|e| format!("{base}.{e}")).collect(), exts.iter().map(|e| e.to_ascii_lowercase()).collect()),
        None => (Vec::new(), Vec::new()),
    };
    (Alt { display: name_part.to_string(), names, exts, crc32: clauses.crc32, sha1: clauses.sha1, ..Alt::default() }, had_clause)
}

fn find_serials(text: &str) -> Vec<String> {
    let b = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i + 10 <= b.len() {
        let w = &b[i..i + 10];
        let shape = w[..4].iter().all(u8::is_ascii_uppercase) && w[4] == b'-' && w[5..].iter().all(u8::is_ascii_digit);
        let bounded = (i == 0 || !b[i - 1].is_ascii_alphanumeric()) && b.get(i + 10).is_none_or(|c| !c.is_ascii_alphanumeric());
        if shape && bounded {
            let s = text[i..i + 10].to_string();
            if !out.contains(&s) {
                out.push(s);
            }
            i += 10;
        } else {
            i += 1;
        }
    }
    out
}

/// "SCPH1001" out of "PSX - SCPH1001.BIN" or "SCPH-5552": the name fragment a BIOS file carries.
fn scph_token(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let at = lower.find("scph")?;
    let digits: String = lower[at + 4..].chars().skip_while(|c| *c == '-' || *c == '_').take_while(char::is_ascii_digit).collect();
    (!digits.is_empty()).then(|| format!("scph{digits}"))
}

fn bios_tokens(body: &str) -> Option<Alt> {
    let at = body.find("BIOS:")?;
    let rest = &body[at + "BIOS:".len()..];
    let end = rest.find(". ").or_else(|| rest.strip_suffix('.').map(str::len)).unwrap_or(rest.len());
    let display = rest[..end].trim().to_string();
    let token = squash(&display.replace("Regional", "").replace("regional", ""));
    (!token.is_empty()).then(|| Alt { display: format!("BIOS {display}"), tokens: vec![token], ..Alt::default() })
}

/// The text after "Required files:". Usually on the same line; some entries put a bulleted list of
/// files on the following lines (accepted only when every line is a file).
fn required_files_text(instructions: &str) -> Option<String> {
    const LABEL: &str = "required files:";
    let lines: Vec<&str> = instructions.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        let Some(prefix) = t.get(..LABEL.len()) else { continue };
        if !prefix.eq_ignore_ascii_case(LABEL) {
            continue;
        }
        let rest = t[LABEL.len()..].trim();
        if !rest.is_empty() {
            return Some(rest.to_string());
        }
        let items: Vec<String> = lines[i + 1..]
            .iter()
            .skip_while(|l| l.trim().is_empty())
            .take_while(|l| !l.trim().is_empty())
            .map(|l| l.trim().trim_start_matches(|c: char| !c.is_alphanumeric()).to_string())
            .collect();
        if !items.is_empty() && items.iter().all(|s| is_file_like(s)) {
            // The list is the games the port supports, not files that are all needed.
            return Some(items.join(" or "));
        }
        return Some(String::new());
    }
    None
}

/// "Name (Disc 1/2).bin" with two checksums is two files; "Doom/2.wad" with two checksums is
/// `Doom.wad` and `Doom2.wad`. Anything else is returned as it is.
fn expand_multi_file(alt: Alt) -> Vec<Alt> {
    if let Some(open) = alt.display.find("(Disc ") {
        if let Some(len) = alt.display[open..].find(')') {
            let inside = &alt.display[open + "(Disc ".len()..open + len];
            let nums: Vec<&str> = inside.split('/').collect();
            if nums.len() >= 2 && nums.iter().all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) && alt.crc32.len() == nums.len() {
                let marker = &alt.display[open..open + len + 1];
                return nums
                    .iter()
                    .zip(&alt.crc32)
                    .map(|(n, crc)| {
                        let swap = |s: &str| s.replace(marker, &format!("(Disc {n})"));
                        Alt { display: swap(&alt.display), names: alt.names.iter().map(|s| swap(s)).collect(), crc32: vec![*crc], ..alt.clone() }
                    })
                    .collect();
            }
        }
    }
    if alt.names.len() == 1 && alt.crc32.len() == 2 {
        let stem = alt.names[0].rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or_default();
        let ext = alt.exts[0].clone();
        if let Some((left, right)) = stem.split_once('/') {
            if !left.is_empty() && !stem.contains(' ') && !right.is_empty() && right.bytes().all(|b| b.is_ascii_digit()) {
                return vec![
                    Alt { display: format!("{left}.{ext}"), names: vec![format!("{left}.{ext}")], crc32: vec![alt.crc32[0]], ..alt.clone() },
                    Alt { display: format!("{left}{right}.{ext}"), names: vec![format!("{left}{right}.{ext}")], crc32: vec![alt.crc32[1]], ..alt },
                ];
            }
        }
    }
    vec![alt]
}

/// Read a port's required files out of its `instructions` text. Returns no slots when the text names
/// nothing checkable (no "Required files:" line, or one that says "Untested", or only a link).
pub fn parse_instructions(instructions: &str) -> Requirement {
    let Some(text) = required_files_text(instructions) else { return Requirement::default() };
    let mut body = text.as_str();
    if let Some(cut) = body.find("Known issues") {
        body = body[..cut].trim();
    }
    let body = body.trim_end_matches('.').trim();
    if body.is_empty() {
        return Requirement::default();
    }
    let lower = body.to_ascii_lowercase();
    let has_clause = lower.contains("(crc32") || lower.contains("(sha-1") || lower.contains("(sha1");

    // A link to a setup guide is not a file.
    if !has_clause && (lower.contains("http://") || lower.contains("https://")) {
        return Requirement::default();
    }

    // Disc serials and a named BIOS, no checksums: "SLUS-00972. BIOS: SCPH-1001".
    let serials = find_serials(body);
    if !has_clause && (!serials.is_empty() || body.contains("BIOS:")) {
        let mut slots: Vec<Slot> = serials
            .iter()
            .map(|s| Slot {
                label: s.clone(),
                kind: SlotKind::Game,
                optional: false,
                alternatives: vec![Alt { display: s.clone(), tokens: vec![squash(s)], ..Alt::default() }],
            })
            .collect();
        if let Some(bios) = bios_tokens(body) {
            slots.push(Slot { label: bios.display.clone(), kind: SlotKind::Bios, optional: false, alternatives: vec![bios] });
        }
        return Requirement { slots, explicit: false };
    }

    // File names and checksums. First the pieces that must all be present ("+", then top-level commas
    // when each part is a file, then a slash that joins different files), each piece possibly offering
    // alternatives ("or").
    let mut pieces: Vec<Vec<(Alt, bool)>> = Vec::new();
    for plus in top_level_split(body, " + ") {
        for comma in split_if_file_like(plus, ",") {
            let parts = split_after_ext_slash(comma);
            if parts.len() > 1 {
                if let Some(each) = split_each_segment(&parts) {
                    pieces.extend(each);
                    continue;
                }
            }
            for part in parts {
                pieces.push(split_if_file_like(part, " or ").into_iter().map(parse_alt).collect());
            }
        }
    }

    // "game.gba + GBA BIOS (CRC32: A/B)": one clause after several files with none of their own
    // lists one value per file, in order. Any other count is treated as shared by all of them.
    for i in 0..pieces.len() {
        let single_with_clause = pieces[i].len() == 1 && pieces[i][0].1;
        if !single_with_clause {
            continue;
        }
        let run_start = (0..i).rev().take_while(|&j| pieces[j].len() == 1 && !pieces[j][0].1 && pieces[j][0].0.crc32.is_empty()).last().unwrap_or(i);
        let run = i - run_start;
        let crcs = pieces[i][0].0.crc32.clone();
        if run == 0 || crcs.len() <= 1 {
            continue;
        }
        if crcs.len() == run + 1 {
            for (k, j) in (run_start..i).enumerate() {
                pieces[j][0].0.crc32 = vec![crcs[k]];
            }
            pieces[i][0].0.crc32 = vec![crcs[run]];
        } else {
            for j in run_start..i {
                pieces[j][0].0.crc32 = crcs.clone();
            }
        }
    }

    let any_checkable = pieces.iter().flatten().any(|(a, had)| *had || !a.names.is_empty());
    if any_checkable {
        let mut slots = Vec::new();
        for alts in pieces {
            let alts: Vec<Alt> = alts.into_iter().map(|(a, _)| a).collect();
            let expanded: Vec<Vec<Alt>> = if alts.len() == 1 { expand_multi_file(alts.into_iter().next().unwrap()).into_iter().map(|a| vec![a]).collect() } else { vec![alts] };
            for mut alts in expanded {
                let kind = kind_of(&alts[0].display);
                if kind == SlotKind::Bios {
                    for alt in &mut alts {
                        if let Some(t) = scph_token(&alt.display) {
                            alt.tokens.push(t);
                        }
                    }
                }
                slots.push(Slot { label: alts[0].display.clone(), kind, optional: false, alternatives: alts });
            }
        }
        return Requirement { slots, explicit: false };
    }

    // Only a title: "Required files: Saru Get You: Saru Saru Daisakusen". Worth a low-confidence name
    // match, nothing more. Placeholders, prose and stray punctuation are not titles.
    const PLACEHOLDERS: [&str; 5] = ["untested", "n/a", "none", "game iso", "game disc"];
    let letters = body.chars().filter(|c| c.is_alphabetic()).count();
    if has_clause || PLACEHOLDERS.iter().any(|p| lower.contains(p)) || letters < 3 || body.chars().count() > 80 {
        return Requirement::default();
    }
    Requirement {
        slots: vec![Slot {
            label: body.to_string(),
            kind: SlotKind::Game,
            optional: false,
            alternatives: vec![Alt { display: body.to_string(), title_only: true, ..Alt::default() }],
        }],
        explicit: false,
    }
}

fn crc_list(v: &Value) -> Vec<u32> {
    match v {
        Value::String(s) => s.split(['/', ',', ' ']).filter_map(parse_crc).collect(),
        Value::Array(a) => a.iter().filter_map(Value::as_str).filter_map(parse_crc).collect(),
        _ => Vec::new(),
    }
}

fn sha1_list(v: &Value) -> Vec<[u8; 20]> {
    match v {
        Value::String(s) => s.split(['/', ',', ' ']).filter_map(parse_sha1).collect(),
        Value::Array(a) => a.iter().filter_map(Value::as_str).filter_map(parse_sha1).collect(),
        _ => Vec::new(),
    }
}

/// Explicit `rom_files` of a local entry. `None` when the field is absent or has no usable item.
pub fn explicit_requirement(entry: &Value) -> Option<Requirement> {
    let items = entry.get("rom_files")?.as_array()?;
    let slots: Vec<Slot> = items
        .iter()
        .filter_map(|item| {
            let obj = match item {
                Value::String(name) => serde_json::json!({ "name": name }),
                other => other.clone(),
            };
            let mut names: Vec<String> = obj.get("names").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
            if let Some(n) = obj.get("name").and_then(Value::as_str) {
                names.insert(0, n.to_string());
            }
            let crc32 = obj.get("crc32").map(crc_list).unwrap_or_default();
            let sha1 = obj.get("sha1").map(sha1_list).unwrap_or_default();
            let tokens = obj.get("tokens").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(squash).collect()).unwrap_or_default();
            if names.is_empty() && crc32.is_empty() && sha1.is_empty() {
                return None;
            }
            let exts = names.iter().filter_map(|n| n.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())).collect();
            let label = obj.get("label").and_then(Value::as_str).map(str::to_string).or_else(|| names.first().cloned()).unwrap_or_else(|| "game file".to_string());
            let kind = if obj.get("bios").and_then(Value::as_bool).unwrap_or(false) { SlotKind::Bios } else { SlotKind::Game };
            Some(Slot {
                label: label.clone(),
                kind,
                optional: obj.get("optional").and_then(Value::as_bool).unwrap_or(false),
                alternatives: vec![Alt { display: label, names, exts, crc32, sha1, tokens, title_only: false }],
            })
        })
        .collect();
    (!slots.is_empty()).then_some(Requirement { slots, explicit: true })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn one(text: &str) -> Requirement {
        parse_instructions(&format!("Required files: {text}"))
    }

    #[test]
    fn single_file_with_one_crc() {
        let r = one("Bomberman Hero (USA).z64 (CRC32: 2CC2E634)");
        assert_eq!(r.slots.len(), 1);
        let alt = &r.slots[0].alternatives[0];
        assert_eq!(alt.names, vec!["Bomberman Hero (USA).z64"]);
        assert_eq!(alt.exts, vec!["z64"]);
        assert_eq!(alt.crc32, vec![0x2CC2_E634]);
        assert_eq!(r.slots[0].kind, SlotKind::Game);
    }

    #[test]
    fn a_crc_list_in_one_clause_is_any_of() {
        let r = one("Perfect Dark (USA)/(Japan)/(Europe) (En,Fr,De,Es,It).z64 (CRC32: 68446AD4/639C0DA9/7718A714)");
        assert_eq!(r.slots.len(), 1);
        assert_eq!(r.slots[0].alternatives[0].crc32, vec![0x6844_6AD4, 0x639C_0DA9, 0x7718_A714]);
    }

    #[test]
    fn bin_cue_gives_two_names_and_both_crcs() {
        let r = one("Bloody Roar II (USA).bin/cue (CRC32: F5EC130A/5679034E)");
        let alt = &r.slots[0].alternatives[0];
        assert_eq!(alt.names, vec!["Bloody Roar II (USA).bin", "Bloody Roar II (USA).cue"]);
        assert_eq!(alt.crc32, vec![0xF5EC_130A, 0x5679_034E]);
        let dotted = one("Castlevania - Symphony of the Night (USA) (Track 1/2).bin/.cue (CRC32: 05BE47B2/70EFB5C6/8B07C1CC)");
        assert_eq!(dotted.slots[0].alternatives[0].exts, vec!["bin", "cue"]);
        assert_eq!(dotted.slots[0].alternatives[0].crc32.len(), 3);
        let typo = one("Syphon Filter (USA) (Rev 1)bin/cue (CRC32: 3E270096/8E1EE69A)");
        assert_eq!(typo.slots[0].alternatives[0].names[0], "Syphon Filter (USA) (Rev 1).bin");
    }

    #[test]
    fn plus_bios_with_one_crc_per_file_is_positional() {
        let r = one("Mega Man Zero (USA, Europe).gba + GBA BIOS (CRC32: 9707D2A1/81977335)");
        assert_eq!(r.slots.len(), 2);
        assert_eq!(r.slots[0].alternatives[0].crc32, vec![0x9707_D2A1]);
        assert_eq!(r.slots[1].alternatives[0].crc32, vec![0x8197_7335]);
        assert_eq!(r.slots[1].kind, SlotKind::Bios);
        assert_eq!(r.slots[0].kind, SlotKind::Game);
    }

    #[test]
    fn plus_with_the_crc_on_the_first_file_only() {
        let r = one("Mega Man X4 (USA).bin (CRC32: 84774766) + PSX - SCPH1001.BIN");
        assert_eq!(r.slots.len(), 2);
        assert_eq!(r.slots[0].alternatives[0].crc32, vec![0x8477_4766]);
        assert!(r.slots[1].alternatives[0].crc32.is_empty());
        assert_eq!(r.slots[1].alternatives[0].names, vec!["PSX - SCPH1001.BIN"]);
        assert_eq!(r.slots[1].kind, SlotKind::Bios);
        let boot = one("Legend of Zelda, The - Oracle of Ages/Seasons (USA, Australia).gbc (CRC32: 3800A387/D7E9F5D7) + cgb_boot.bin");
        assert_eq!(boot.slots.len(), 2);
        assert_eq!(boot.slots[0].alternatives[0].crc32.len(), 2);
        assert_eq!(boot.slots[1].kind, SlotKind::Bios);
    }

    #[test]
    fn plus_title_update_with_two_crcs_is_positional() {
        let r = one("DoDonPachi Saidaioujou (Japan).iso + Title Update 1.01 from microsoft_xbox360_title-updates (CRC32: B1B392D3/6D68D98D)");
        assert_eq!(r.slots.len(), 2);
        assert_eq!(r.slots[0].alternatives[0].crc32, vec![0xB1B3_92D3]);
        assert_eq!(r.slots[1].alternatives[0].crc32, vec![0x6D68_D98D]);
    }

    #[test]
    fn or_makes_alternatives_in_one_slot() {
        let r = one("Super Smash Bros. (USA).z64 (CRC32: EB97929E) or Nintendo All-Star! Dairantou Smash Brothers (Japan).z64 (CRC32: 04C9D3B1)");
        assert_eq!(r.slots.len(), 1);
        assert_eq!(r.slots[0].alternatives.len(), 2);
        assert_eq!(r.slots[0].alternatives[1].crc32, vec![0x04C9_D3B1]);
    }

    #[test]
    fn comma_separates_two_games_only_when_both_parts_are_files() {
        let r = one("Spider-Man (USA).bin/cue (CRC32: BF1C7EFF/5B840D78), Spider-Man 2 - Enter - Electro (USA).bin/cue (CRC32: CBEE4C23/260D5211)");
        assert_eq!(r.slots.len(), 2);
        // A comma inside a title or inside parentheses is not a separator.
        let title = one("Legend of Zelda, The - Majora's Mask (USA).z64 (CRC32: B428D8A7)");
        assert_eq!(title.slots.len(), 1);
        assert_eq!(title.slots[0].alternatives[0].names, vec!["Legend of Zelda, The - Majora's Mask (USA).z64"]);
        let parens = one("Kirby's Dream Land (USA, Europe).gb (CRC32: 40F25740)");
        assert_eq!(parens.slots.len(), 1);
    }

    #[test]
    fn trailing_prose_after_the_clause_is_dropped() {
        let r = one("Legend of Zelda, The - Ocarina of Time (USA) (GameCube)/Master Quest (Europe) (GameCube).z64 (CRC32: 346DE3AE/832D6449) for all language.");
        assert_eq!(r.slots.len(), 1);
        assert_eq!(r.slots[0].alternatives[0].crc32, vec![0x346D_E3AE, 0x832D_6449]);
        let xbox = one("Dragon Ball Z - Budokai HD Collection (USA).iso (CRC32: 63DEBD61) for Xbox 360");
        assert_eq!(xbox.slots[0].alternatives[0].names, vec!["Dragon Ball Z - Budokai HD Collection (USA).iso"]);
    }

    #[test]
    fn a_name_without_a_crc() {
        let r = one("Ace Combat 5 - The Unsung War (USA) (En,Ja).iso");
        let alt = &r.slots[0].alternatives[0];
        assert!(alt.crc32.is_empty());
        assert_eq!(alt.names.len(), 1);
        assert!(!alt.title_only);
    }

    #[test]
    fn serials_and_bios_become_token_slots() {
        let r = one("SLPS-02020 \u{2013} SLPS-02021. BIOS: SCPH-5552. Known issues: Wave 4 validation limits apply.");
        assert_eq!(r.slots.len(), 3);
        assert_eq!(r.slots[0].alternatives[0].tokens, vec!["slps02020"]);
        assert_eq!(r.slots[1].alternatives[0].tokens, vec!["slps02021"]);
        assert_eq!(r.slots[2].kind, SlotKind::Bios);
        assert_eq!(r.slots[2].alternatives[0].tokens, vec!["scph5552"]);
        let regional = one("SLUS-00972. BIOS: Regional SCPH");
        assert_eq!(regional.slots[1].alternatives[0].tokens, vec!["scph"]);
        let only_serial = one("SCUS-94423.");
        assert_eq!(only_serial.slots.len(), 1);
    }

    #[test]
    fn a_bare_title_is_a_vague_slot_and_placeholders_are_nothing() {
        let r = one("Saru Get You: Saru Saru Daisakusen");
        assert_eq!(r.slots.len(), 1);
        assert!(r.slots[0].is_vague());
        for text in ["USA game ISO", "Untested", ""] {
            assert!(one(text).slots.is_empty(), "{text}");
        }
    }

    #[test]
    fn instructions_without_the_line_have_no_slots() {
        assert!(parse_instructions("Languages: Multilingual.").slots.is_empty());
        assert!(parse_instructions("").slots.is_empty());
        assert!(parse_instructions("Required files:").slots.is_empty());
        assert!(parse_instructions("Required files: \n\nLanguages: English.").slots.is_empty());
    }

    #[test]
    fn only_the_required_files_line_is_read() {
        let r = parse_instructions("Required files: outrun.zip (CRC32: 87CC12C6)\n\nUnzip the outrun.zip files into the port's roms folder.");
        assert_eq!(r.slots.len(), 1);
        assert_eq!(r.slots[0].alternatives[0].names, vec!["outrun.zip"]);
        let lower = parse_instructions("Languages: English\nrequired files: a.gba (crc32: 0000ABCD)");
        assert_eq!(lower.slots[0].alternatives[0].crc32, vec![0xABCD]);
    }

    #[test]
    fn explicit_fields_replace_the_guesswork() {
        let entry = json!({
            "name": "x", "folder": "x",
            "instructions": "Required files: something else.z64 (CRC32: 11111111)",
            "rom_files": [
                {"name": "tp.iso", "crc32": ["1a2b3c4d", "BADBAD"]},
                {"name": "scph1001.bin", "bios": true, "optional": true},
                "plain.gba",
                {"label": "empty"}
            ],
            "rom_access": "copy",
            "rom_dest": "/isos/"
        });
        let port = crate::core::models::port_from_value(&entry).unwrap();
        let spec = RomSpec::for_port(&port, Some(&entry));
        assert!(spec.requirement.explicit);
        assert_eq!(spec.requirement.slots.len(), 3);
        assert_eq!(spec.requirement.slots[0].alternatives[0].crc32, vec![0x1A2B_3C4D]);
        assert_eq!(spec.requirement.slots[1].kind, SlotKind::Bios);
        assert!(spec.requirement.slots[1].optional);
        assert_eq!(spec.requirement.slots[2].alternatives[0].names, vec!["plain.gba"]);
        assert_eq!(spec.access, Access::Copy);
        assert_eq!(spec.dest_subdir, "isos");
    }

    #[test]
    fn without_explicit_fields_the_instructions_and_the_builtin_access_apply() {
        let entry = json!({"name": "Shipwright", "folder": "Shipwright", "instructions": "Required files: Oot.z64 (CRC32: 346DE3AE)"});
        let port = crate::core::models::port_from_value(&entry).unwrap();
        let spec = RomSpec::for_port(&port, None);
        assert!(!spec.requirement.explicit);
        assert_eq!(spec.requirement.slots.len(), 1);
        assert_eq!(spec.access, Access::InPlace);
        let other = crate::core::models::port_from_value(&json!({"name": "Other", "folder": "other"})).unwrap();
        assert_eq!(RomSpec::for_port(&other, None).access, Access::Link);
        assert!(RomSpec::for_port(&other, None).requirement.slots.is_empty());
    }

    #[test]
    fn unusable_explicit_fields_fall_back_to_the_instructions() {
        let entry = json!({"name": "x", "folder": "x", "instructions": "Required files: a.gba (CRC32: 0000ABCD)", "rom_files": [], "rom_access": "teleport"});
        let port = crate::core::models::port_from_value(&entry).unwrap();
        let spec = RomSpec::for_port(&port, Some(&entry));
        assert!(!spec.requirement.explicit);
        assert_eq!(spec.requirement.slots.len(), 1);
        assert_eq!(spec.access, Access::Link);
    }

    /// The real catalog: every entry parses without panicking, and the shapes the parser claims to
    /// understand are found in the quantities they occur in today.
    #[test]
    fn the_shipped_catalog_parses() {
        let text = include_str!("../../../ports.json");
        let ports = crate::core::config::parse_catalog(text).expect("catalog parses");
        let mut with_slots = 0;
        let mut with_crc = 0;
        let mut multi_slot = 0;
        let mut alternatives = 0;
        for port in &ports {
            let r = parse_instructions(&port.instructions);
            if !r.slots.is_empty() {
                with_slots += 1;
            }
            if r.slots.iter().flat_map(|s| &s.alternatives).any(|a| !a.crc32.is_empty()) {
                with_crc += 1;
            }
            if r.slots.len() > 1 {
                multi_slot += 1;
            }
            if r.slots.iter().any(|s| s.alternatives.len() > 1) {
                alternatives += 1;
            }
            for slot in &r.slots {
                assert!(!slot.alternatives.is_empty() && !slot.label.is_empty(), "{}", port.name);
                for alt in &slot.alternatives {
                    // A CRC list never exceeds the number of files that could be meant.
                    assert!(alt.crc32.len() <= 8, "{}: {:?}", port.name, alt);
                }
            }
        }
        // Counts as of 2026-10-21 with slack for catalog growth and shrinkage.
        assert!(with_slots >= 150, "slots in {with_slots} ports");
        assert!(with_crc >= 100, "CRC in {with_crc} ports");
        assert!(multi_slot >= 5, "multi-slot in {multi_slot} ports");
        assert!(alternatives >= 1, "alternatives in {alternatives} ports");
    }

    #[test]
    fn top_level_split_respects_brackets() {
        assert_eq!(top_level_split("a, b (c, d), e [f, g]", ","), vec!["a", "b (c, d)", "e [f, g]"]);
        assert_eq!(top_level_split("a + b", " + "), vec!["a", "b"]);
        assert_eq!(top_level_split("  ", ","), Vec::<&str>::new());
    }

    /// The instructions of the first catalog entry whose name contains `needle`, read straight from the
    /// JSON so entries the app itself would drop are covered too.
    fn catalog_instructions(needle: &str) -> String {
        let data: Value = serde_json::from_str(include_str!("../../../ports.json")).unwrap();
        data["ports"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| {
                let name = p.get("name").or_else(|| p.get("-name")).and_then(Value::as_str).unwrap_or("");
                name.contains(needle)
            })
            .unwrap_or_else(|| panic!("no catalog entry named like {needle:?}"))["instructions"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }

    #[test]
    fn catalog_multi_track_disc_is_one_slot_with_four_checksums() {
        let r = parse_instructions(&catalog_instructions("Tekken 3"));
        assert_eq!(r.slots.len(), 1);
        let alt = &r.slots[0].alternatives[0];
        assert_eq!(alt.crc32.len(), 4);
        assert_eq!(alt.exts, vec!["bin", "cue"]);
    }

    #[test]
    fn catalog_f_zero_x_is_three_files_with_one_checksum_each() {
        let r = parse_instructions(&catalog_instructions("F-Zero X"));
        assert_eq!(r.slots.len(), 3, "{:#?}", r.slots);
        let names: Vec<&str> = r.slots.iter().map(|s| s.alternatives[0].names[0].as_str()).collect();
        assert_eq!(names, vec!["F-Zero X (USA).z64", "[BIOS] Nintendo 64DD IPL (Japan) (v1.2).z64", "NUD-EFZE-USA_2.ndd"]);
        let crcs: Vec<Vec<u32>> = r.slots.iter().map(|s| s.alternatives[0].crc32.clone()).collect();
        assert_eq!(crcs, vec![vec![0x0B56_1FBA], vec![0x7F93_3CE2], vec![0xEBDF_BD2B]]);
        assert_eq!(r.slots[0].kind, SlotKind::Game);
        assert_eq!(r.slots[1].kind, SlotKind::Bios);
    }

    #[test]
    fn catalog_bulleted_list_is_one_slot_any_of_the_supported_games() {
        let r = parse_instructions(&catalog_instructions("mon Gen 1"));
        assert_eq!(r.slots.len(), 1, "{:#?}", r.slots.iter().map(|s| &s.label).collect::<Vec<_>>());
        assert_eq!(r.slots[0].alternatives.len(), 7);
        assert!(r.slots[0].alternatives.iter().all(|a| a.crc32.len() == 1));
        assert!(r.slots[0].alternatives[0].names[0].starts_with("Pokemon - Blue Version"));
    }

    #[test]
    fn catalog_link_only_requirement_is_nothing() {
        assert!(parse_instructions(&catalog_instructions("Namco System 22")).slots.is_empty());
    }

    #[test]
    fn catalog_sha1_requirement_is_checkable() {
        let r = parse_instructions(&catalog_instructions("BattleTanx"));
        assert_eq!(r.slots.len(), 1);
        let alt = &r.slots[0].alternatives[0];
        assert_eq!(alt.sha1.len(), 1);
        assert!(!alt.title_only);
        assert_eq!(to_hex(&alt.sha1[0]), "805248fb0a0ee694cad8d7dc927b631d860dd8cf");
        assert_eq!(alt.names, vec!["BattleTanx - Global Assault (USA).z64"]);
    }

    #[test]
    fn catalog_two_discs_in_one_name_are_two_slots() {
        let r = parse_instructions(&catalog_instructions("Parasite Eve"));
        assert_eq!(r.slots.len(), 2);
        assert_eq!(r.slots[0].alternatives[0].crc32, vec![0x4886_41E0]);
        assert_eq!(r.slots[1].alternatives[0].crc32, vec![0x226E_E2EE]);
        assert!(r.slots[1].alternatives[0].names[0].contains("(Disc 2)"));
    }

    #[test]
    fn catalog_doom_wad_pair_is_two_files() {
        let r = parse_instructions(&catalog_instructions("uzdoom"));
        assert_eq!(r.slots.len(), 2);
        assert_eq!(r.slots[0].alternatives[0].names, vec!["Doom.wad"]);
        assert_eq!(r.slots[1].alternatives[0].names, vec!["Doom2.wad"]);
        assert_eq!(r.slots[0].alternatives[0].crc32, vec![0xBF0E_AAC0]);
    }

    #[test]
    fn an_empty_checksum_clause_is_no_requirement() {
        assert!(one("(CRC32: )").slots.is_empty());
        assert!(one(":").slots.is_empty());
        let prose = one("Place your extracted game data files in the assets folder. default.xbe from the USA release must be there.");
        assert!(prose.slots.is_empty());
    }

    #[test]
    fn a_bios_file_name_carries_its_scph_token() {
        let r = one("Mega Man X4 (USA).bin (CRC32: 84774766) + PSX - SCPH1001.BIN");
        assert_eq!(r.slots[1].alternatives[0].tokens, vec!["scph1001"]);
        assert_eq!(scph_token("SCPH-5552"), Some("scph5552".to_string()));
        assert_eq!(scph_token("no bios here"), None);
    }

    #[test]
    fn slot_ids_come_from_what_is_asked_for_not_the_wording() {
        let a = one("Game (USA).z64 (CRC32: 2CC2E634)");
        let b = parse_instructions("Required files: Game (USA).z64 (CRC32: 2CC2E634) for all language.");
        let c = one("Game (USA).z64 (CRC32: 2CC2E635)");
        assert_eq!(a.slots[0].id(), b.slots[0].id());
        assert_ne!(a.slots[0].id(), c.slots[0].id());
        assert_eq!(a.slots[0].id().len(), 8);
    }

    #[test]
    fn local_entries_are_read_by_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ports.local.json");
        std::fs::write(&path, r#"{"ports":[{"name":"A","folder":"a","rom_access":"copy"},{"name":"B"}]}"#).unwrap();
        let map = load_local_entries(&path);
        assert_eq!(map.len(), 1);
        assert_eq!(map["a"]["rom_access"], "copy");
        assert!(load_local_entries(&dir.path().join("missing.json")).is_empty());
    }

    #[test]
    fn explicit_sha1_is_read() {
        let entry = json!({"name": "x", "folder": "x", "rom_files": [{"name": "a.z64", "sha1": "805248FB0A0EE694CAD8D7DC927B631D860DD8CF"}]});
        let r = explicit_requirement(&entry).unwrap();
        assert_eq!(r.slots[0].alternatives[0].sha1.len(), 1);
    }
}
