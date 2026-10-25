//! Deciding what is a program to start, from the file's own first bytes.
//!
//! An SD card on exFAT has no permissions, so every file reads as mode 0755 and the executable bit says
//! nothing. Upstream's Linux rule (bit set, or `.sh` / `.AppImage`) then listed disc images, libraries and
//! data files as programs ("Choose an executable" offered `Zelda.rvz`, and Severed Chains' bundled Java).
//! The rule here: a `.sh` / `.AppImage` name counts as before; any other file counts only when it has the
//! executable bit AND its first bytes say program: `#!`, or an ELF file that is an executable or a
//! position-independent program (PT_INTERP present), not a shared library. Only a few bytes are read.
//!
//! Known limit: a statically linked position-independent program (ET_DYN without PT_INTERP) reads as a
//! library; such programs are rare (Go and Rust build normal ones), and Info > favorite executable can
//! still name a file by hand through the list of all files.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const PT_INTERP: u32 = 3;
/// A real program has a handful of program headers; stop looking after this many.
const MAX_PROGRAM_HEADERS: u64 = 64;
/// A launcher script is small; a big extensionless text file named `start` is data.
const MAX_LAUNCHER_BYTES: u64 = 64 * 1024;

/// `libfoo.so`, `libfoo.so.1.2`: shared libraries by name, whatever their header says.
pub fn is_library_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".so") || lower.contains(".so.")
}

fn u16_at(b: &[u8], at: usize, little: bool) -> Option<u16> {
    let raw: [u8; 2] = b.get(at..at + 2)?.try_into().ok()?;
    Some(if little { u16::from_le_bytes(raw) } else { u16::from_be_bytes(raw) })
}

fn u32_at(b: &[u8], at: usize, little: bool) -> Option<u32> {
    let raw: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
    Some(if little { u32::from_le_bytes(raw) } else { u32::from_be_bytes(raw) })
}

fn u64_at(b: &[u8], at: usize, little: bool) -> Option<u64> {
    let raw: [u8; 8] = b.get(at..at + 8)?.try_into().ok()?;
    Some(if little { u64::from_le_bytes(raw) } else { u64::from_be_bytes(raw) })
}

/// Whether a file's first bytes say "program to run": a `#!` script, or an ELF executable (never a
/// shared library). Does not look at the name's extension or the executable bit.
pub fn is_program_content(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else { return false };
    let mut head = [0u8; 64];
    let mut got = 0;
    while got < head.len() {
        match file.read(&mut head[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(_) => return false,
        }
    }
    let head = &head[..got];
    if head.starts_with(b"#!") {
        return true;
    }
    if !head.starts_with(&ELF_MAGIC) {
        return is_named_text_launcher(path, head);
    }
    if path.file_name().and_then(|n| n.to_str()).is_some_and(is_library_name) {
        return false;
    }
    let little = match head.get(5) {
        Some(1) => true,
        Some(2) => false,
        _ => return false,
    };
    let Some(e_type) = u16_at(head, 16, little) else { return false };
    match e_type {
        ET_EXEC => true,
        ET_DYN => has_interpreter(&mut file, head, little),
        _ => false,
    }
}

/// The names a launcher script can have without an extension: exactly these four, in any case. `start_here`, `run_me`
/// or `play_notes` are notes (the executable bit is always set on exFAT) and are never run on their own.
fn launcher_name(name: &str) -> bool {
    ["launch", "start", "run", "play"].iter().any(|p| name.eq_ignore_ascii_case(p))
}

/// A launcher script that lost its `#!` line: an extensionless file named exactly `launch`, `start`, `run` or `play`
/// whose first bytes are plain text (no NUL, valid UTF-8 up to a cut character) and that is small. Tim's Severed
/// Chains folder has a top-level `launch` with no extension, and some zip tools drop the first line; run directly
/// such a file fails, so the launcher hands it to `sh`.
fn is_named_text_launcher(path: &Path, head: &[u8]) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return false };
    if !launcher_name(name) || head.is_empty() || head.contains(&0) {
        return false;
    }
    let text_ok = match std::str::from_utf8(head) {
        Ok(_) => true,
        // A multi-byte character cut by the 64-byte window is still text.
        Err(e) => e.error_len().is_none(),
    };
    text_ok && std::fs::metadata(path).is_ok_and(|m| m.len() <= MAX_LAUNCHER_BYTES)
}

/// A position-independent ELF is a program (not a library) when it names a dynamic loader (PT_INTERP).
fn has_interpreter(file: &mut File, head: &[u8], little: bool) -> bool {
    let wide = match head.get(4) {
        Some(2) => true,
        Some(1) => false,
        _ => return false,
    };
    let (off, size, count) = if wide {
        (u64_at(head, 32, little), u16_at(head, 54, little), u16_at(head, 56, little))
    } else {
        (u32_at(head, 28, little).map(u64::from), u16_at(head, 42, little), u16_at(head, 44, little))
    };
    let (Some(off), Some(size), Some(count)) = (off, size, count) else { return false };
    if size < 4 {
        return false;
    }
    for i in 0..u64::from(count).min(MAX_PROGRAM_HEADERS) {
        // A damaged header can hold huge numbers: an overflow is not a program.
        let Some(at) = u64::from(size).checked_mul(i).and_then(|n| n.checked_add(off)) else { return false };
        if file.seek(SeekFrom::Start(at)).is_err() {
            return false;
        }
        let mut kind = [0u8; 4];
        if file.read_exact(&mut kind).is_err() {
            return false;
        }
        let kind = if little { u32::from_le_bytes(kind) } else { u32::from_be_bytes(kind) };
        if kind == PT_INTERP {
            return true;
        }
    }
    false
}

#[cfg(unix)]
pub fn has_exec_bit(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

#[cfg(not(unix))]
pub fn has_exec_bit(_path: &Path) -> bool {
    false
}

/// Linux: whether a file is a launch candidate. `.sh` and `.AppImage` by name; anything else needs the
/// executable bit and program content (see the module text).
pub fn is_linux_program(path: &Path) -> bool {
    let named = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("sh") || e.eq_ignore_ascii_case("appimage"));
    named || (has_exec_bit(path) && is_program_content(path))
}

/// How many folders below `root` a file sits (0: directly in it). A path outside `root` counts as deep.
pub fn depth_in(root: &Path, path: &Path) -> usize {
    path.strip_prefix(root).map(|p| p.components().count().saturating_sub(1)).unwrap_or(usize::MAX)
}

/// Shallowest folders first, then by file name, so lists are stable.
pub fn sort_shallow_first(root: &Path, files: &mut [PathBuf]) {
    files.sort_by(|a, b| depth_in(root, a).cmp(&depth_in(root, b)).then_with(|| a.file_name().cmp(&b.file_name())).then_with(|| a.cmp(b)));
}

/// Only the files at the shallowest depth that has any: a top-level `launch.sh` beats `jdk/bin/java`.
pub fn shallowest(root: &Path, mut files: Vec<PathBuf>) -> Vec<PathBuf> {
    sort_shallow_first(root, &mut files);
    let Some(first) = files.first().map(|p| depth_in(root, p)) else { return files };
    files.retain(|p| depth_in(root, p) == first);
    files
}

/// The path to store as a port's favorite executable: relative to its folder, `None` when the file is
/// not inside it.
pub fn relative_pick(game_dir: &Path, exe: &Path) -> Option<String> {
    exe.strip_prefix(game_dir).ok().map(|p| p.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests;
