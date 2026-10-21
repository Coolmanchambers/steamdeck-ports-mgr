//! Walk the ROM folder and fingerprint what is in it.
//!
//! Read-only: files are opened for reading and nothing else. The walk is bounded in depth, follows
//! symlinks (SD cards and NAS mounts are often linked in) but never enters the same real folder
//! twice, and hashes in 1 MiB chunks so a 30 GB image never sits in memory.

use super::hash::{from_hex, to_hex, Digests, MultiHasher};
use super::system::{self, ByteOrder, System};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

const CHUNK: usize = 1 << 20;

#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Folders below the ROM folder to enter. 0 scans only the ROM folder itself.
    pub max_depth: usize,
    /// SHA-1 costs the most CPU of the three hashes and nothing in the catalog asks for it.
    pub sha1: bool,
    /// Hash files whose extension is not a known ROM or disc extension too.
    pub hash_all_extensions: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions { max_depth: 6, sha1: true, hash_all_extensions: false }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomFile {
    pub path: PathBuf,
    /// Path below the ROM folder (what the system was classified from).
    pub rel_path: PathBuf,
    pub system: System,
    pub size: u64,
    pub mtime_ms: u64,
    /// Lowercase, no dot.
    pub ext: String,
    /// Only for Nintendo 64 images whose header magic was recognised.
    pub byte_order: Option<ByteOrder>,
    /// Hashes of the file exactly as it is on disk.
    pub raw: Digests,
    /// Hashes of the content converted to big-endian. `None` when it equals `raw`, which is every
    /// file except a `.v64` or `.n64` image.
    pub normalized: Option<Digests>,
}

impl RomFile {
    /// The hashes checksum lists are computed over: big-endian for N64, the file itself otherwise.
    pub fn canonical(&self) -> &Digests {
        self.normalized.as_ref().unwrap_or(&self.raw)
    }

    pub fn file_name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// True when the file is an N64 image that is not in the console's byte order.
    pub fn needs_byte_order_conversion(&self) -> bool {
        self.normalized.is_some()
    }

    pub fn to_json(&self) -> Value {
        let digests = |d: &Digests| {
            json!({"crc32": d.crc32_hex(), "md5": d.md5_hex(), "sha1": d.sha1_hex()})
        };
        json!({
            "path": self.path.to_string_lossy(),
            "rel_path": self.rel_path.to_string_lossy(),
            "system": self.system.id(),
            "size": self.size,
            "mtime_ms": self.mtime_ms,
            "ext": self.ext,
            "byte_order": self.byte_order.map(ByteOrder::id),
            "raw": digests(&self.raw),
            "normalized": self.normalized.as_ref().map(digests),
        })
    }

    pub fn from_json(v: &Value) -> Option<RomFile> {
        let digests = |d: &Value| -> Option<Digests> {
            let crc32 = u32::from_str_radix(d.get("crc32")?.as_str()?, 16).ok()?;
            let md5: [u8; 16] = from_hex(d.get("md5")?.as_str()?)?.try_into().ok()?;
            let sha1 = match d.get("sha1").and_then(Value::as_str) {
                Some(s) => Some(<[u8; 20]>::try_from(from_hex(s)?).ok()?),
                None => None,
            };
            Some(Digests { crc32, md5, sha1 })
        };
        Some(RomFile {
            path: PathBuf::from(v.get("path")?.as_str()?),
            rel_path: PathBuf::from(v.get("rel_path")?.as_str()?),
            system: System::from_id(v.get("system")?.as_str()?),
            size: v.get("size")?.as_u64()?,
            mtime_ms: v.get("mtime_ms")?.as_u64()?,
            ext: v.get("ext")?.as_str()?.to_string(),
            byte_order: v.get("byte_order").and_then(Value::as_str).and_then(ByteOrder::from_id),
            raw: digests(v.get("raw")?)?,
            normalized: match v.get("normalized") {
                Some(n) if !n.is_null() => Some(digests(n)?),
                _ => None,
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanIssue {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct ScanResult {
    pub root: PathBuf,
    /// Sorted by path.
    pub files: Vec<RomFile>,
    /// Files skipped because their extension is not a ROM or disc extension.
    pub ignored_files: usize,
    /// Folders not entered because they are below `max_depth`.
    pub folders_too_deep: usize,
    /// Folders skipped because their real location was already walked (symlink loops, duplicate links).
    pub folders_already_seen: usize,
    pub issues: Vec<ScanIssue>,
    pub cancelled: bool,
}

impl ScanResult {
    /// File count per system, for the "what's in your game folders" summary.
    pub fn counts_by_system(&self) -> Vec<(System, usize)> {
        let mut counts: HashMap<System, usize> = HashMap::new();
        for f in &self.files {
            *counts.entry(f.system).or_default() += 1;
        }
        let mut out: Vec<_> = counts.into_iter().collect();
        out.sort();
        out
    }

    pub fn to_json(&self) -> Value {
        json!({
            "version": 1,
            "root": self.root.to_string_lossy(),
            "files": self.files.iter().map(RomFile::to_json).collect::<Vec<_>>(),
        })
    }

    /// Reads a saved scan so the next one can skip files that did not change. Unreadable entries are
    /// dropped; the cache is only ever an optimisation.
    pub fn from_json(v: &Value) -> ScanResult {
        ScanResult {
            root: PathBuf::from(v.get("root").and_then(Value::as_str).unwrap_or("")),
            files: v.get("files").and_then(Value::as_array).map(|a| a.iter().filter_map(RomFile::from_json).collect()).unwrap_or_default(),
            ..ScanResult::default()
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Progress<'a> {
    pub current: &'a Path,
    pub files_done: usize,
    pub bytes_hashed: u64,
}

fn mtime_ms(meta: &fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn fill(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

pub struct FileHashes {
    pub size: u64,
    pub byte_order: Option<ByteOrder>,
    pub raw: Digests,
    pub normalized: Option<Digests>,
}

/// Hash one file in a single streaming pass. With `n64` set, the header magic decides the byte
/// order and a second set of hashes is taken over the content converted to big-endian.
pub fn hash_file(
    path: &Path,
    n64: bool,
    sha1: bool,
    cancel: &AtomicBool,
    on_bytes: &mut dyn FnMut(u64),
) -> io::Result<Option<FileHashes>> {
    let mut file = File::open(path)?;
    let mut raw = MultiHasher::new(sha1);
    let mut normalized: Option<MultiHasher> = None;
    let mut byte_order: Option<ByteOrder> = None;
    let mut buf = vec![0u8; CHUNK];
    let mut size = 0u64;
    let mut first = true;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        // CHUNK is a multiple of 4, so every full chunk starts on a four-byte boundary of the file.
        let n = fill(&mut file, &mut buf)?;
        if n == 0 {
            break;
        }
        let chunk = &mut buf[..n];
        if first {
            first = false;
            if n64 {
                byte_order = system::detect_n64_byte_order(chunk);
                if matches!(byte_order, Some(ByteOrder::ByteSwapped | ByteOrder::LittleEndian)) {
                    normalized = Some(MultiHasher::new(sha1));
                }
            }
        }
        raw.update(chunk);
        if let (Some(n_hasher), Some(order)) = (normalized.as_mut(), byte_order) {
            let mut copy = chunk.to_vec();
            system::normalize_to_big_endian(order, &mut copy);
            n_hasher.update(&copy);
        }
        size += n as u64;
        on_bytes(n as u64);
        if n < CHUNK {
            break;
        }
    }
    Ok(Some(FileHashes { size, byte_order, raw: raw.finish(), normalized: normalized.map(|h| h.finish()) }))
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// Scan `root`. `prior` is an earlier result (or the saved cache): a file with the same path, size and
/// modified time keeps its old hashes instead of being read again. `progress` is called before each
/// file and after each chunk; `cancel` stops the scan at the next chunk and marks the result.
pub fn scan(
    root: &Path,
    options: &ScanOptions,
    prior: Option<&ScanResult>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress<'_>),
) -> io::Result<ScanResult> {
    let root_meta = fs::metadata(root)?;
    if !root_meta.is_dir() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{} is not a folder", root.display())));
    }
    let known: HashMap<&Path, &RomFile> = prior.map(|p| p.files.iter().map(|f| (f.path.as_path(), f)).collect()).unwrap_or_default();
    let mut result = ScanResult { root: root.to_path_buf(), ..ScanResult::default() };
    let mut seen_dirs: HashSet<PathBuf> = HashSet::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    let mut bytes_hashed = 0u64;

    'walk: while let Some((dir, depth)) = stack.pop() {
        match fs::canonicalize(&dir) {
            Ok(real) => {
                if !seen_dirs.insert(real) {
                    result.folders_already_seen += 1;
                    continue;
                }
            }
            Err(e) => {
                result.issues.push(ScanIssue { path: dir.clone(), message: format!("could not resolve folder: {e}") });
                continue;
            }
        }
        let entries = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) => {
                result.issues.push(ScanIssue { path: dir.clone(), message: format!("could not read folder: {e}") });
                continue;
            }
        };
        let mut children: Vec<(String, PathBuf)> = entries
            .filter_map(|e| e.ok())
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
            .filter(|(name, _)| !is_hidden(name))
            .collect();
        children.sort();
        let mut subdirs = Vec::new();
        for (_, path) in children {
            // `metadata` follows symlinks, so a linked file or folder is treated as what it points to.
            let meta = match fs::metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    result.issues.push(ScanIssue { path: path.clone(), message: format!("could not read: {e}") });
                    continue;
                }
            };
            if meta.is_dir() {
                if depth + 1 > options.max_depth {
                    result.folders_too_deep += 1;
                } else {
                    subdirs.push(path);
                }
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if !options.hash_all_extensions && !system::is_rom_extension(&ext) {
                result.ignored_files += 1;
                continue;
            }
            let rel_path = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            let system = system::classify(&rel_path);
            let size = meta.len();
            let modified = mtime_ms(&meta);
            progress(Progress { current: &path, files_done: result.files.len(), bytes_hashed });

            if let Some(old) = known.get(path.as_path()) {
                if old.size == size && old.mtime_ms == modified && (old.raw.sha1.is_some() || !options.sha1) {
                    let mut kept = (*old).clone();
                    kept.rel_path = rel_path;
                    kept.system = system;
                    result.files.push(kept);
                    continue;
                }
            }
            let is_n64 = system == System::N64 || matches!(ext.as_str(), "z64" | "v64" | "n64");
            let mut counted = 0u64;
            let hashed = hash_file(&path, is_n64, options.sha1, cancel, &mut |n| {
                counted += n;
                progress(Progress { current: &path, files_done: result.files.len(), bytes_hashed: bytes_hashed + counted });
            });
            match hashed {
                Ok(Some(h)) => {
                    bytes_hashed += counted;
                    result.files.push(RomFile {
                        path: path.clone(),
                        rel_path,
                        system,
                        size: h.size,
                        mtime_ms: modified,
                        ext,
                        byte_order: h.byte_order,
                        raw: h.raw,
                        normalized: h.normalized,
                    });
                }
                Ok(None) => {
                    result.cancelled = true;
                    break 'walk;
                }
                Err(e) => result.issues.push(ScanIssue { path: path.clone(), message: format!("could not read: {e}") }),
            }
        }
        // Reverse so the stack pops in name order and the result is stable.
        stack.extend(subdirs.into_iter().rev().map(|p| (p, depth + 1)));
    }
    result.files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

/// True when two files hold the same bytes. Sizes are compared first; the content is compared in
/// chunks, so large files are not read into memory.
pub fn files_identical(a: &Path, b: &Path) -> io::Result<bool> {
    let (ma, mb) = (fs::metadata(a)?, fs::metadata(b)?);
    if ma.len() != mb.len() {
        return Ok(false);
    }
    let (mut fa, mut fb) = (File::open(a)?, File::open(b)?);
    let (mut ba, mut bb) = (vec![0u8; CHUNK], vec![0u8; CHUNK]);
    loop {
        let na = fill(&mut fa, &mut ba)?;
        let nb = fill(&mut fb, &mut bb)?;
        if na != nb || ba[..na] != bb[..nb] {
            return Ok(false);
        }
        if na == 0 {
            return Ok(true);
        }
    }
}

/// Short hex form for log lines and tests.
pub fn describe_digests(d: &Digests) -> String {
    format!("crc32 {} md5 {}", d.crc32_hex(), to_hex(&d.md5))
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;
    use crate::fork::app_id::crc32;

    fn run(root: &Path, options: &ScanOptions, prior: Option<&ScanResult>) -> ScanResult {
        scan(root, options, prior, &AtomicBool::new(false), &mut |_| {}).unwrap()
    }

    #[test]
    fn hashes_a_plain_file_with_all_three_digests() {
        let dir = temp_dir();
        let data = random_bytes(1, 300_000);
        write(&dir.path().join("gba/Game (USA).gba"), &data);
        let res = run(dir.path(), &ScanOptions::default(), None);
        assert_eq!(res.files.len(), 1);
        let f = &res.files[0];
        assert_eq!(f.system, System::GameBoyAdvance);
        assert_eq!(f.size, data.len() as u64);
        assert_eq!(f.raw.crc32, crc32(&data));
        assert!(f.normalized.is_none());
        assert_eq!(f.canonical(), &f.raw);
        assert_eq!(f.raw.md5_hex().len(), 32);
        assert_eq!(f.raw.sha1_hex().unwrap().len(), 40);
    }

    #[test]
    fn sha1_can_be_turned_off() {
        let dir = temp_dir();
        write(&dir.path().join("a.gba"), &random_bytes(2, 1000));
        let res = run(dir.path(), &ScanOptions { sha1: false, ..ScanOptions::default() }, None);
        assert!(res.files[0].raw.sha1.is_none());
    }

    #[test]
    fn a_file_larger_than_one_chunk_hashes_the_same_as_the_whole_in_memory() {
        let dir = temp_dir();
        let data = random_bytes(3, CHUNK * 2 + 12_345);
        write(&dir.path().join("big.iso"), &data);
        let res = run(dir.path(), &ScanOptions::default(), None);
        assert_eq!(res.files[0].raw.crc32, crc32(&data));
        assert_eq!(res.files[0].size, data.len() as u64);
    }

    #[test]
    fn the_same_n64_rom_in_three_byte_orders_has_one_canonical_hash() {
        let dir = temp_dir();
        let z64 = n64_rom(4, 4096);
        write(&dir.path().join("n64/game.z64"), &z64);
        write(&dir.path().join("n64/game.v64"), &to_v64(&z64));
        write(&dir.path().join("n64/game.n64"), &to_n64(&z64));
        let res = run(dir.path(), &ScanOptions::default(), None);
        assert_eq!(res.files.len(), 3);
        let by_ext = |e: &str| res.files.iter().find(|f| f.ext == e).unwrap();
        let (z, v, n) = (by_ext("z64"), by_ext("v64"), by_ext("n64"));
        assert_eq!(z.byte_order, Some(ByteOrder::BigEndian));
        assert_eq!(v.byte_order, Some(ByteOrder::ByteSwapped));
        assert_eq!(n.byte_order, Some(ByteOrder::LittleEndian));
        // Raw hashes differ, the canonical (big-endian) ones agree and equal the .z64's own.
        assert_ne!(v.raw.crc32, z.raw.crc32);
        assert_ne!(n.raw.crc32, z.raw.crc32);
        assert_eq!(v.canonical(), z.canonical());
        assert_eq!(n.canonical(), z.canonical());
        assert_eq!(z.canonical().crc32, crc32(&z64));
        assert!(z.normalized.is_none() && v.normalized.is_some() && n.normalized.is_some());
        assert!(v.needs_byte_order_conversion() && !z.needs_byte_order_conversion());
    }

    #[test]
    fn byte_order_follows_the_header_not_the_extension() {
        let dir = temp_dir();
        let z64 = n64_rom(5, 2048);
        // A byte-swapped image wrongly named .z64.
        write(&dir.path().join("mislabeled.z64"), &to_v64(&z64));
        let res = run(dir.path(), &ScanOptions::default(), None);
        assert_eq!(res.files[0].byte_order, Some(ByteOrder::ByteSwapped));
        assert_eq!(res.files[0].canonical().crc32, crc32(&z64));
    }

    #[test]
    fn n64_normalization_crosses_chunk_boundaries_and_odd_sizes() {
        let dir = temp_dir();
        let z64 = n64_rom(6, CHUNK * 2 + 6); // size not a multiple of 4
        write(&dir.path().join("odd.v64"), &to_v64(&z64));
        let res = run(dir.path(), &ScanOptions::default(), None);
        let f = &res.files[0];
        // The trailing 2 bytes are not a full group, so they stay as stored; the expected value is
        // the big-endian content with the same tail.
        let mut want = to_v64(&z64);
        system::normalize_to_big_endian(ByteOrder::ByteSwapped, &mut want);
        assert_eq!(f.canonical().crc32, crc32(&want));
        assert_eq!(&want[..CHUNK * 2], &z64[..CHUNK * 2]);
    }

    #[test]
    fn an_n64_looking_file_without_the_magic_has_no_byte_order() {
        let dir = temp_dir();
        write(&dir.path().join("n64/odd.z64"), &random_bytes(7, 512));
        let res = run(dir.path(), &ScanOptions::default(), None);
        assert_eq!(res.files[0].byte_order, None);
        assert!(res.files[0].normalized.is_none());
    }

    #[test]
    fn depth_is_bounded_and_counted() {
        let dir = temp_dir();
        write(&dir.path().join("a.gba"), b"x");
        write(&dir.path().join("l1/b.gba"), b"x");
        write(&dir.path().join("l1/l2/c.gba"), b"x");
        write(&dir.path().join("l1/l2/l3/d.gba"), b"x");
        let res = run(dir.path(), &ScanOptions { max_depth: 2, ..ScanOptions::default() }, None);
        let names: Vec<_> = res.files.iter().map(|f| f.file_name()).collect();
        assert_eq!(names, vec!["a.gba", "b.gba", "c.gba"]);
        assert_eq!(res.folders_too_deep, 1);
        let shallow = run(dir.path(), &ScanOptions { max_depth: 0, ..ScanOptions::default() }, None);
        assert_eq!(shallow.files.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_loops_and_duplicate_links_are_walked_once() {
        use std::os::unix::fs::symlink;
        let dir = temp_dir();
        write(&dir.path().join("sub/game.gba"), b"game");
        symlink(dir.path(), dir.path().join("sub/back_to_root")).unwrap();
        symlink(dir.path().join("sub"), dir.path().join("sub_again")).unwrap();
        let res = run(dir.path(), &ScanOptions::default(), None);
        assert_eq!(res.files.len(), 1, "{:?}", res.files.iter().map(|f| &f.path).collect::<Vec<_>>());
        assert!(res.folders_already_seen >= 2);
    }

    #[cfg(unix)]
    #[test]
    fn a_broken_symlink_is_reported_not_fatal() {
        use std::os::unix::fs::symlink;
        let dir = temp_dir();
        write(&dir.path().join("ok.gba"), b"ok");
        symlink(dir.path().join("missing.gba"), dir.path().join("dangling.gba")).unwrap();
        let res = run(dir.path(), &ScanOptions::default(), None);
        assert_eq!(res.files.len(), 1);
        assert_eq!(res.issues.len(), 1);
    }

    #[test]
    fn hidden_files_and_non_rom_extensions_are_skipped_and_counted() {
        let dir = temp_dir();
        write(&dir.path().join("keep.gba"), b"x");
        write(&dir.path().join(".hidden.gba"), b"x");
        write(&dir.path().join(".cache/inside.gba"), b"x");
        write(&dir.path().join("cover.png"), b"x");
        write(&dir.path().join("notes.txt"), b"x");
        let res = run(dir.path(), &ScanOptions::default(), None);
        assert_eq!(res.files.len(), 1);
        assert_eq!(res.ignored_files, 2);
        let all = run(dir.path(), &ScanOptions { hash_all_extensions: true, ..ScanOptions::default() }, None);
        assert_eq!(all.files.len(), 3);
    }

    #[test]
    fn a_missing_or_non_folder_root_is_an_error() {
        let dir = temp_dir();
        assert!(scan(&dir.path().join("nope"), &ScanOptions::default(), None, &AtomicBool::new(false), &mut |_| {}).is_err());
        write(&dir.path().join("file.gba"), b"x");
        assert!(scan(&dir.path().join("file.gba"), &ScanOptions::default(), None, &AtomicBool::new(false), &mut |_| {}).is_err());
    }

    #[test]
    fn unchanged_files_keep_their_old_hashes_and_changed_ones_are_rehashed() {
        let dir = temp_dir();
        let path = dir.path().join("a.gba");
        write(&path, &random_bytes(8, 1000));
        let first = run(dir.path(), &ScanOptions::default(), None);

        // Plant a fake hash in the prior result: it only survives if the file is not read again.
        let mut planted = first.clone();
        planted.files[0].raw.crc32 = 0xDEAD_BEEF;
        let reused = run(dir.path(), &ScanOptions::default(), Some(&planted));
        assert_eq!(reused.files[0].raw.crc32, 0xDEAD_BEEF);

        write(&path, &random_bytes(9, 1001));
        let rehashed = run(dir.path(), &ScanOptions::default(), Some(&planted));
        assert_ne!(rehashed.files[0].raw.crc32, 0xDEAD_BEEF);
        assert_eq!(rehashed.files[0].size, 1001);
    }

    #[test]
    fn cancel_stops_the_scan_and_marks_it() {
        let dir = temp_dir();
        write(&dir.path().join("a.gba"), &random_bytes(10, 5000));
        let cancel = AtomicBool::new(true);
        let res = scan(dir.path(), &ScanOptions::default(), None, &cancel, &mut |_| {}).unwrap();
        assert!(res.cancelled);
        assert!(res.files.is_empty());
    }

    #[test]
    fn progress_reports_each_file() {
        let dir = temp_dir();
        write(&dir.path().join("a.gba"), b"aaa");
        write(&dir.path().join("b.gba"), b"bbb");
        let seen = std::cell::RefCell::new(Vec::new());
        scan(dir.path(), &ScanOptions::default(), None, &AtomicBool::new(false), &mut |p| {
            seen.borrow_mut().push(p.current.file_name().unwrap().to_string_lossy().into_owned());
        })
        .unwrap();
        let seen = seen.into_inner();
        assert!(seen.contains(&"a.gba".to_string()) && seen.contains(&"b.gba".to_string()), "{seen:?}");
    }

    #[test]
    fn scan_result_survives_a_json_round_trip() {
        let dir = temp_dir();
        let z64 = n64_rom(11, 4096);
        write(&dir.path().join("n64/a.v64"), &to_v64(&z64));
        write(&dir.path().join("gba/b.gba"), &random_bytes(12, 777));
        let res = run(dir.path(), &ScanOptions::default(), None);
        let back = ScanResult::from_json(&res.to_json());
        assert_eq!(back.files, res.files);
        assert_eq!(back.root, res.root);
    }

    #[test]
    fn files_identical_compares_content_not_names() {
        let dir = temp_dir();
        let data = random_bytes(13, CHUNK + 99);
        write(&dir.path().join("a"), &data);
        write(&dir.path().join("b"), &data);
        let mut other = data.clone();
        *other.last_mut().unwrap() ^= 1;
        write(&dir.path().join("c"), &other);
        write(&dir.path().join("d"), &data[..100]);
        assert!(files_identical(&dir.path().join("a"), &dir.path().join("b")).unwrap());
        assert!(!files_identical(&dir.path().join("a"), &dir.path().join("c")).unwrap());
        assert!(!files_identical(&dir.path().join("a"), &dir.path().join("d")).unwrap());
    }

    #[test]
    fn counts_by_system_groups_the_files() {
        let dir = temp_dir();
        write(&dir.path().join("a.gba"), b"1");
        write(&dir.path().join("b.gba"), b"2");
        write(&dir.path().join("gc/c.iso"), b"3");
        let res = run(dir.path(), &ScanOptions::default(), None);
        let counts = res.counts_by_system();
        assert!(counts.contains(&(System::GameBoyAdvance, 2)));
        assert!(counts.contains(&(System::GameCube, 1)));
    }
}
