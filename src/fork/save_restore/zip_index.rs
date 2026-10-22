//! Reads the table of contents of a zip file (its central directory) without unpacking anything.
//!
//! Used to list what a save backup holds (which save folders, how many bytes) and to check every
//! entry name before 7-Zip is allowed to unpack it. The names are read exactly as stored, so a name
//! like `../x` or `/etc/x` is seen even if a tool would quietly rewrite it on the way out.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const EOCD_SIG: u32 = 0x0605_4b50;
const ZIP64_LOCATOR_SIG: u32 = 0x0706_4b50;
const ZIP64_EOCD_SIG: u32 = 0x0606_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
/// A save backup's table of contents is a few KB; anything this big is not one of ours.
const MAX_CENTRAL_DIR: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipEntry {
    /// The stored name with `\` turned into `/`.
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub is_symlink: bool,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

fn read_at(file: &mut File, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf)?;
    Ok(buf)
}

fn not_a_zip() -> String {
    "not a readable zip file".to_string()
}

/// The entry name is safe to unpack: relative, no `..`, no drive letter, no NUL.
pub fn is_safe_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if name.is_empty() || name.contains('\0') || name.starts_with(['/', '\\']) {
        return false;
    }
    if bytes.len() >= 2 && bytes[1] == b':' {
        return false;
    }
    !name.split(['/', '\\']).any(|part| part == "..")
}

/// Every entry in the zip's central directory, in stored order.
pub fn read_index(path: &Path) -> Result<Vec<ZipEntry>, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    if len < 22 {
        return Err(not_a_zip());
    }
    let tail_len = len.min(22 + 65_535);
    let tail = read_at(&mut file, len - tail_len, tail_len as usize).map_err(|e| e.to_string())?;
    let eocd = (0..=tail.len() - 22).rev().find(|&i| u32_at(&tail, i) == Some(EOCD_SIG)).ok_or_else(not_a_zip)?;
    let mut count = u64::from(u16_at(&tail, eocd + 10).ok_or_else(not_a_zip)?);
    let mut cd_size = u64::from(u32_at(&tail, eocd + 12).ok_or_else(not_a_zip)?);
    let mut cd_offset = u64::from(u32_at(&tail, eocd + 16).ok_or_else(not_a_zip)?);

    if count == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_offset == 0xFFFF_FFFF {
        let locator = eocd.checked_sub(20).ok_or_else(not_a_zip)?;
        if u32_at(&tail, locator) != Some(ZIP64_LOCATOR_SIG) {
            return Err(not_a_zip());
        }
        let z64_offset = u64_at(&tail, locator + 8).ok_or_else(not_a_zip)?;
        let z64 = read_at(&mut file, z64_offset, 56).map_err(|_| not_a_zip())?;
        if u32_at(&z64, 0) != Some(ZIP64_EOCD_SIG) {
            return Err(not_a_zip());
        }
        count = u64_at(&z64, 32).ok_or_else(not_a_zip)?;
        cd_size = u64_at(&z64, 40).ok_or_else(not_a_zip)?;
        cd_offset = u64_at(&z64, 48).ok_or_else(not_a_zip)?;
    }
    if cd_size > MAX_CENTRAL_DIR || cd_offset.checked_add(cd_size).is_none_or(|end| end > len) {
        return Err(not_a_zip());
    }
    let cd = read_at(&mut file, cd_offset, cd_size as usize).map_err(|_| not_a_zip())?;

    let mut entries = Vec::new();
    let mut at = 0usize;
    while entries.len() as u64 != count {
        if u32_at(&cd, at) != Some(CENTRAL_SIG) {
            return Err(not_a_zip());
        }
        let made_by_host = u16_at(&cd, at + 4).ok_or_else(not_a_zip)? >> 8;
        let size32 = u32_at(&cd, at + 24).ok_or_else(not_a_zip)?;
        let name_len = usize::from(u16_at(&cd, at + 28).ok_or_else(not_a_zip)?);
        let extra_len = usize::from(u16_at(&cd, at + 30).ok_or_else(not_a_zip)?);
        let comment_len = usize::from(u16_at(&cd, at + 32).ok_or_else(not_a_zip)?);
        let external = u32_at(&cd, at + 38).ok_or_else(not_a_zip)?;
        let name_start = at + 46;
        let raw_name = cd.get(name_start..name_start + name_len).ok_or_else(not_a_zip)?;
        let extra = cd.get(name_start + name_len..name_start + name_len + extra_len).ok_or_else(not_a_zip)?;

        let mut size = u64::from(size32);
        if size32 == 0xFFFF_FFFF {
            size = zip64_size(extra).ok_or_else(not_a_zip)?;
        }
        let name = String::from_utf8_lossy(raw_name).replace('\\', "/");
        let unix_mode = (external >> 16) & 0o170_000;
        let is_symlink = made_by_host == 3 && unix_mode == 0o120_000;
        let is_dir = name.ends_with('/') || external & 0x10 != 0 || (made_by_host == 3 && unix_mode == 0o040_000);
        entries.push(ZipEntry { name, size, is_dir, is_symlink });
        at = name_start + name_len + extra_len + comment_len;
    }
    Ok(entries)
}

/// The uncompressed size from the zip64 extra field (header ID 1); it is the first value stored.
fn zip64_size(extra: &[u8]) -> Option<u64> {
    let mut at = 0;
    while at + 4 <= extra.len() {
        let id = u16_at(extra, at)?;
        let len = usize::from(u16_at(extra, at + 2)?);
        if id == 1 {
            return u64_at(extra, at + 4);
        }
        at += 4 + len;
    }
    None
}

/// Test helper: writes a zip with stored (uncompressed) entries, names exactly as given.
#[cfg(test)]
pub(crate) fn write_stored_zip(path: &Path, entries: &[(&str, &[u8])]) {
    use crate::fork::rom_library::hash::Crc32;
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let mut crc = Crc32::new();
        crc.update(data);
        let crc = crc.finish();
        let offset = out.len() as u32;
        let is_dir = name.ends_with('/');
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);

        central.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
        central.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&(if is_dir { 0x10u32 } else { 0x20 }).to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&EOCD_SIG.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    std::fs::write(path, out).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_that_escape_are_unsafe() {
        for bad in ["../x", "a/../../x", "/etc/passwd", "\\x", "C:/x", "c:x", "a\\..\\x", "", "a\0b"] {
            assert!(!is_safe_name(bad), "{bad:?} should be unsafe");
        }
        for good in ["save_folder/slot1.dat", "MyGame/save_folder2/a b/c.sav", "save_folder/", "x..y/z", "...", "a/.hidden"] {
            assert!(is_safe_name(good), "{good:?} should be safe");
        }
    }

    #[test]
    fn reads_names_sizes_and_folders_as_stored() {
        let dir = tempfile::tempdir().unwrap();
        let zip = dir.path().join("t.zip");
        write_stored_zip(&zip, &[("save_folder/", b""), ("save_folder/slot1.dat", b"hello"), ("../evil", b"x"), ("win\\path.txt", b"abc")]);
        let entries = read_index(&zip).unwrap();
        let summary: Vec<(&str, u64, bool)> = entries.iter().map(|e| (e.name.as_str(), e.size, e.is_dir)).collect();
        assert_eq!(summary, vec![("save_folder/", 0, true), ("save_folder/slot1.dat", 5, false), ("../evil", 1, false), ("win/path.txt", 3, false)]);
    }

    #[test]
    fn a_file_that_is_not_a_zip_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.zip");
        std::fs::write(&path, vec![7u8; 300]).unwrap();
        assert!(read_index(&path).is_err());
        std::fs::write(&path, b"short").unwrap();
        assert!(read_index(&path).is_err());
    }

    #[test]
    fn a_truncated_zip_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let zip = dir.path().join("t.zip");
        write_stored_zip(&zip, &[("save_folder/a.dat", b"hello world")]);
        let bytes = std::fs::read(&zip).unwrap();
        let eocd = bytes.len() - 22;
        // The end record points past the end of the file.
        let mut past_end = bytes.clone();
        past_end[eocd + 16..eocd + 20].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
        std::fs::write(&zip, past_end).unwrap();
        assert!(read_index(&zip).is_err());
        // The end record points at something that is not a central directory entry.
        let mut wrong_sig = bytes.clone();
        let cd = bytes.windows(4).position(|w| w == CENTRAL_SIG.to_le_bytes()).unwrap();
        wrong_sig[cd] = 0;
        std::fs::write(&zip, wrong_sig).unwrap();
        assert!(read_index(&zip).is_err());
        // Cut off in the middle.
        std::fs::write(&zip, &bytes[..bytes.len() / 2]).unwrap();
        assert!(read_index(&zip).is_err());
    }

    #[test]
    fn unix_symlink_entries_are_flagged() {
        let dir = tempfile::tempdir().unwrap();
        let zip = dir.path().join("t.zip");
        write_stored_zip(&zip, &[("save_folder/link", b"/etc/passwd")]);
        let mut bytes = std::fs::read(&zip).unwrap();
        let cd = bytes.windows(4).position(|w| w == CENTRAL_SIG.to_le_bytes()).unwrap();
        bytes[cd + 5] = 3; // made by: Unix
        bytes[cd + 38..cd + 42].copy_from_slice(&((0o120_777u32) << 16).to_le_bytes());
        std::fs::write(&zip, bytes).unwrap();
        let entries = read_index(&zip).unwrap();
        assert!(entries[0].is_symlink);
    }
}
