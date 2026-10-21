//! What game files a port needs, read from the catalog's free text or from explicit fields.
//!
//! The catalog states it in an `instructions` string, by hand, so the shapes vary. The ones that
//! occur in `ports.json` (checked by a test that parses every entry):
//!
//! ```text
//! Required files: Bomberman Hero (USA).z64 (CRC32: 2CC2E634)
//! Required files: Perfect Dark (USA)/(Japan)/(Europe) (En,Fr,De,Es,It).z64 (CRC32: 68446AD4/639C0DA9/7718A714)
//! Required files: Bloody Roar II (USA).bin/cue (CRC32: F5EC130A/5679034E)
//! Required files: Mega Man Zero (USA, Europe).gba + GBA BIOS (CRC32: 9707D2A1/81977335)
//! Required files: Super Smash Bros. (USA).z64 (CRC32: EB97929E) or Nintendo All-Star! ... (Japan).z64 (CRC32: 04C9D3B1)
//! Required files: Spider-Man (USA).bin/cue (CRC32: BF1C7EFF/5B840D78), Spider-Man 2 ... .bin/cue (CRC32: ...)
//! Required files: SLUS-00972. BIOS: Regional SCPH
//! Required files: USA game ISO            (nothing checkable)
//! ```
//!
//! The model is a list of slots (files that must all be present). A slot has one or more
//! alternatives (any one will do), and an alternative has file names, CRC-32 values that identify
//! it, and extra name tokens (disc serials, BIOS names). A CRC list inside one clause is "any of
//! these": it lists regions or revisions, or the parts of one disc (`bin/cue`).
//!
//! Local entries (`ports.local.json`) can state this explicitly and skip the guessing:
//!
//! ```json
//! "rom_files": [ {"name": "tp.iso", "crc32": ["1A2B3C4D"]}, {"name": "scph1001.bin", "bios": true, "optional": true} ],
//! "rom_access": "link",
//! "rom_dest": "isos"
//! ```

use super::connect::Access;
use super::system::is_rom_extension;
use crate::core::models::Port;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    Game,
    /// A console BIOS or boot ROM: needed, but not the game itself.
    Bios,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Alt {
    /// The text as written, minus the CRC clause.
    pub display: String,
    /// Full file names when the text gave an extension (`bin/cue` gives two).
    pub names: Vec<String>,
    /// Lowercase extensions the text gave, no dot.
    pub exts: Vec<String>,
    pub crc32: Vec<u32>,
    /// Lowercase alphanumeric fragments a file name should contain (disc serials, BIOS names).
    pub tokens: Vec<String>,
    /// The text named no file, CRC or serial: a game title and nothing more to check against.
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

/// Ports known to read the game file once and build their own assets from it. From the findings
/// document, not yet confirmed on a Deck; a local entry's `rom_access` overrides it.
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

/// Cut `(CRC32: AAAAAAAA/BBBBBBBB)` out of a piece. Returns the text before it and the values.
/// Anything after the clause (" for all language.") is dropped.
fn split_crc_clause(piece: &str) -> (&str, Option<Vec<u32>>) {
    let lower = piece.to_ascii_lowercase();
    let Some(open) = lower.find("(crc32") else { return (piece.trim(), None) };
    let after = &piece[open..];
    let close = after.find(')').unwrap_or(after.len());
    let inner = after["(crc32".len()..close].trim_start_matches([':', ' ']);
    let crcs = inner.split(['/', ',', ' ']).filter_map(parse_crc).collect();
    (piece[..open].trim(), Some(crcs))
}

/// `Name (USA).z64` gives (`Name (USA)`, [z64]); `Name.bin/cue` and `Name.bin/.cue` give two
/// extensions; `Name (Rev 1)bin/cue` (a typo in the catalog) is accepted too.
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
    let (name, crcs) = split_crc_clause(piece);
    crcs.is_some() || extract_exts(name).is_some()
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

fn kind_of(display: &str) -> SlotKind {
    let l = display.to_ascii_lowercase();
    if l.contains("bios") || l.contains("scph") || l.contains("boot.bin") || l.contains("boot.rom") {
        SlotKind::Bios
    } else {
        SlotKind::Game
    }
}

/// Parse one alternative. The bool says whether it carried a CRC clause of its own.
fn parse_alt(piece: &str) -> (Alt, bool) {
    let (name_part, crcs) = split_crc_clause(piece);
    let had_clause = crcs.is_some();
    let name_part = name_part.trim().trim_end_matches('.').trim();
    let (names, exts) = match extract_exts(name_part) {
        Some((base, exts)) => (exts.iter().map(|e| format!("{base}.{e}")).collect(), exts.iter().map(|e| e.to_ascii_lowercase()).collect()),
        None => (Vec::new(), Vec::new()),
    };
    (Alt { display: name_part.to_string(), names, exts, crc32: crcs.unwrap_or_default(), tokens: Vec::new(), title_only: false }, had_clause)
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

fn bios_tokens(body: &str) -> Option<Alt> {
    let at = body.find("BIOS:")?;
    let rest = &body[at + "BIOS:".len()..];
    let end = rest.find(". ").or_else(|| rest.strip_suffix('.').map(str::len)).unwrap_or(rest.len());
    let display = rest[..end].trim().to_string();
    let token = squash(&display.replace("Regional", "").replace("regional", ""));
    (!token.is_empty()).then(|| Alt { display: format!("BIOS {display}"), tokens: vec![token], ..Alt::default() })
}

/// The text after "Required files:" on its line, if the instructions have such a line.
fn required_files_text(instructions: &str) -> Option<&str> {
    instructions.lines().find_map(|line| {
        let t = line.trim_start();
        let prefix = t.get(.."required files:".len())?;
        prefix.eq_ignore_ascii_case("required files:").then(|| t["required files:".len()..].trim())
    })
}

/// Read a port's required files out of its `instructions` text. Returns no slots when the text names
/// nothing checkable (no "Required files:" line, or one that says "Untested").
pub fn parse_instructions(instructions: &str) -> Requirement {
    let Some(mut body) = required_files_text(instructions) else { return Requirement::default() };
    if let Some(cut) = body.find("Known issues") {
        body = body[..cut].trim();
    }
    let body = body.trim_end_matches('.').trim();
    if body.is_empty() {
        return Requirement::default();
    }
    let lower = body.to_ascii_lowercase();

    // Disc serials and a named BIOS, no checksums: "SLUS-00972. BIOS: SCPH-1001".
    let serials = find_serials(body);
    if !lower.contains("(crc32") && (!serials.is_empty() || body.contains("BIOS:")) {
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

    // File names and CRCs. First the pieces that must all be present ("+", then top-level commas
    // when each part is a file), each piece possibly offering alternatives ("or").
    let mut pieces: Vec<Vec<(Alt, bool)>> = Vec::new();
    for plus in top_level_split(body, " + ") {
        for comma in split_if_file_like(plus, ",") {
            pieces.push(split_if_file_like(comma, " or ").into_iter().map(parse_alt).collect());
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
        let slots = pieces
            .into_iter()
            .map(|alts| {
                let alts: Vec<Alt> = alts.into_iter().map(|(a, _)| a).collect();
                Slot { label: alts[0].display.clone(), kind: kind_of(&alts[0].display), optional: false, alternatives: alts }
            })
            .collect();
        return Requirement { slots, explicit: false };
    }

    // Only a title: "Required files: Saru Get You: Saru Saru Daisakusen". Worth a low-confidence name
    // match, nothing more. Placeholders are not titles.
    const PLACEHOLDERS: [&str; 5] = ["untested", "n/a", "none", "game iso", "game disc"];
    if PLACEHOLDERS.iter().any(|p| lower.contains(p)) {
        return Requirement::default();
    }
    let title = body.chars().take(100).collect::<String>();
    Requirement {
        slots: vec![Slot {
            label: title.clone(),
            kind: SlotKind::Game,
            optional: false,
            alternatives: vec![Alt { display: title, title_only: true, ..Alt::default() }],
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
            let tokens = obj.get("tokens").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(squash).collect()).unwrap_or_default();
            if names.is_empty() && crc32.is_empty() {
                return None;
            }
            let exts = names.iter().filter_map(|n| n.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())).collect();
            let label = obj.get("label").and_then(Value::as_str).map(str::to_string).or_else(|| names.first().cloned()).unwrap_or_else(|| "game file".to_string());
            let kind = if obj.get("bios").and_then(Value::as_bool).unwrap_or(false) { SlotKind::Bios } else { SlotKind::Game };
            Some(Slot {
                label: label.clone(),
                kind,
                optional: obj.get("optional").and_then(Value::as_bool).unwrap_or(false),
                alternatives: vec![Alt { display: label, names, exts, crc32, tokens, title_only: false }],
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
}
