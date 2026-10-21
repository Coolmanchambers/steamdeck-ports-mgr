//! Helpers for the tests. Every file the tests use is generated here from a seed: random bytes, with a
//! crafted header for N64 images. No real game data is ever read or written.

use super::hash::{Digests, MultiHasher};
use std::fs;
use std::path::{Path, PathBuf};

pub fn temp_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp dir")
}

/// Deterministic pseudo-random bytes (xorshift), so a failing test reproduces.
pub fn random_bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.truncate(len);
    out
}

/// A big-endian (`.z64`) N64 image: random content behind the real PI configuration word.
pub fn n64_rom(seed: u64, len: usize) -> Vec<u8> {
    let mut rom = random_bytes(seed, len);
    rom[..4].copy_from_slice(&[0x80, 0x37, 0x12, 0x40]);
    rom
}

/// The same image as a `.v64` (every pair of bytes swapped).
pub fn to_v64(z64: &[u8]) -> Vec<u8> {
    let mut out = z64.to_vec();
    for pair in out.chunks_exact_mut(2) {
        pair.swap(0, 1);
    }
    out
}

/// The same image as an `.n64` (every group of four bytes reversed).
pub fn to_n64(z64: &[u8]) -> Vec<u8> {
    let mut out = z64.to_vec();
    for group in out.chunks_exact_mut(4) {
        group.reverse();
    }
    out
}

pub fn write(path: &Path, data: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, data).unwrap();
}

pub fn digests_of(data: &[u8]) -> Digests {
    let mut h = MultiHasher::new(true);
    h.update(data);
    h.finish()
}

/// Every file under `dir` with its content hash, sorted by path: compare before and after an
/// operation to prove nothing under it changed.
pub fn snapshot(dir: &Path) -> Vec<(PathBuf, Digests)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, Digests)>) {
        let mut entries: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
        entries.sort();
        for path in entries {
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                walk(&path, out);
            } else if meta.file_type().is_symlink() {
                out.push((path.clone(), digests_of(fs::read_link(&path).unwrap().to_string_lossy().as_bytes())));
            } else {
                out.push((path.clone(), digests_of(&fs::read(&path).unwrap())));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out
}
