//! Copying a port folder exactly, and proving the copy matches.
//!
//! The copy keeps permission bits (program bits matter: ports do not start without them), symlinks
//! (as links, never followed), and file times. Every file is hashed (SHA-1) while it is read for the
//! copy, so the source is read once; the verify step walks the new tree on its own, compares the file
//! list, kinds, sizes, permission bits and link targets, then reads files back from the new copy and
//! compares hashes: always for programs and game files, and for every file when the port is smaller
//! than [`Policy::full_hash_limit`]. Each written file is synced to disk before it counts.
//!
//! One deliberate change: a relative link that points outside the port folder (`../../roms/x.z64`)
//! would break at the new place, so the copy gets the absolute path of what it pointed at. Links
//! inside the folder stay as written.

use crate::core::path_safety::is_within;
use crate::fork::rom_library::hash::Sha1;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

const BUFFER: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Dir { mode: u32 },
    File { size: u64, mode: u32, modified: Option<SystemTime>, sha1: [u8; 20] },
    /// `target` is what the new link holds; `original` what the old one held.
    Link { target: PathBuf, original: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Relative to the port folder; empty for the folder itself.
    pub rel: PathBuf,
    pub kind: Kind,
}

#[derive(Debug, Clone, Default)]
pub struct Manifest {
    pub entries: Vec<Entry>,
}

impl Manifest {
    pub fn bytes(&self) -> u64 {
        self.entries.iter().map(|e| if let Kind::File { size, .. } = e.kind { size } else { 0 }).sum()
    }

    pub fn files(&self) -> usize {
        self.entries.iter().filter(|e| matches!(e.kind, Kind::File { .. })).count()
    }

    /// Links that were rewritten from relative to absolute.
    pub fn rewritten_links(&self) -> Vec<&Entry> {
        self.entries.iter().filter(|e| matches!(&e.kind, Kind::Link { target, original } if target != original)).collect()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Policy {
    /// Up to this many bytes in all, every file is read back and hashed; above it only programs and
    /// game files are (sizes and the file list are always compared).
    pub full_hash_limit: u64,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { full_hash_limit: 4 * 1024 * 1024 * 1024 }
    }
}

#[derive(Debug)]
pub enum CopyError {
    Cancelled,
    /// Something that is not a file, folder or link (a device, a pipe).
    Special(PathBuf),
    /// A source file changed while it was being copied.
    Changed(PathBuf),
    Io { action: &'static str, path: PathBuf, source: io::Error },
}

impl std::fmt::Display for CopyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CopyError::Cancelled => write!(f, "cancelled"),
            CopyError::Special(p) => write!(f, "{} is not a normal file, folder or link", p.display()),
            CopyError::Changed(p) => write!(f, "{} changed while it was being copied (is the game or another program using it?)", p.display()),
            CopyError::Io { action, path, source } => write!(f, "could not {action} {}: {source}", path.display()),
        }
    }
}

fn io_err<'a>(action: &'static str, path: &'a Path) -> impl FnOnce(io::Error) -> CopyError + 'a {
    move |source| CopyError::Io { action, path: path.to_path_buf(), source }
}

#[cfg(unix)]
pub fn mode_of(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
pub fn mode_of(meta: &fs::Metadata) -> u32 {
    if meta.permissions().readonly() {
        0o444
    } else {
        0o644
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_readonly(mode & 0o222 == 0);
    fs::set_permissions(path, perms)
}

#[cfg(unix)]
pub fn make_symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
pub fn make_symlink(_target: &Path, _link: &Path) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "links are not supported here"))
}

/// Removes `.` and `..` without touching the disk.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// What a link's target means as an absolute path, given where the link sits.
pub fn absolute_target(link: &Path, target: &Path) -> PathBuf {
    if target.is_absolute() {
        normalize(target)
    } else {
        normalize(&link.parent().unwrap_or(Path::new("")).join(target))
    }
}

/// A relative link whose path leaves `root` on the way, even if it ends inside again
/// (`../../SoH/saves` from `SoH/sub/`): it only works while the folder keeps its name and place.
pub fn steps_outside(root: &Path, link: &Path, target: &Path) -> bool {
    let base = normalize(link.parent().unwrap_or(root));
    let Ok(rel) = base.strip_prefix(normalize(root)) else { return true };
    let mut depth = rel.components().count() as i64;
    for c in target.components() {
        match c {
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return true;
                }
            }
            Component::Normal(_) => depth += 1,
            _ => {}
        }
    }
    false
}

/// `target` as seen from the folder `from` (both absolute and normalized): `../x/y`.
pub fn relative_path(from: &Path, target: &Path) -> PathBuf {
    let from: Vec<Component> = from.components().collect();
    let to: Vec<Component> = target.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for _ in common..from.len() {
        out.push("..");
    }
    for c in &to[common..] {
        out.push(c.as_os_str());
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// The target the copy of a link gets. A relative link that leaves `root` would break at the new
/// place, so it becomes absolute; an absolute link into `root` itself (or a relative one that leaves
/// `root` on its way back in) would keep depending on the old place
/// folder, so it becomes relative (and moves with the copy). Everything else stays as written.
pub fn copied_link_target(root: &Path, link: &Path, target: &Path) -> PathBuf {
    let abs = absolute_target(link, target);
    let inside = is_within(&abs, root);
    if target.is_relative() && !inside {
        abs
    } else if (target.is_absolute() && inside) || (target.is_relative() && steps_outside(root, link, target)) {
        relative_path(&normalize(link.parent().unwrap_or(root)), &abs)
    } else {
        target.to_path_buf()
    }
}

/// Total bytes of regular files under `root`, never following links (`root` itself may be a link,
/// which counts as nothing).
pub fn tree_size(root: &Path) -> u64 {
    fn walk(dir: &Path) -> u64 {
        let Ok(entries) = fs::read_dir(dir) else { return 0 };
        entries
            .flatten()
            .map(|e| match e.file_type() {
                Ok(t) if t.is_dir() => walk(&e.path()),
                Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
                _ => 0,
            })
            .sum()
    }
    match fs::symlink_metadata(root) {
        Ok(m) if m.is_dir() => walk(root),
        Ok(m) if m.is_file() => m.len(),
        _ => 0,
    }
}

fn sorted_children(dir: &Path) -> Result<Vec<fs::DirEntry>, CopyError> {
    let mut children: Vec<fs::DirEntry> = fs::read_dir(dir).map_err(io_err("read", dir))?.collect::<Result<_, _>>().map_err(io_err("read", dir))?;
    children.sort_by_key(|e| e.file_name());
    Ok(children)
}

fn same_time(a: Option<SystemTime>, b: Option<SystemTime>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.duration_since(b).or_else(|_| b.duration_since(a)).is_ok_and(|d| d <= Duration::from_secs(2)),
        (None, None) => true,
        _ => false,
    }
}

pub struct Copier<'a> {
    pub cancel: &'a AtomicBool,
    pub progress: &'a mut dyn FnMut(u64, &Path),
    done: u64,
}

impl<'a> Copier<'a> {
    pub fn new(cancel: &'a AtomicBool, progress: &'a mut dyn FnMut(u64, &Path)) -> Copier<'a> {
        Copier { cancel, progress, done: 0 }
    }

    /// Copies the folder `src` (not a link) into `dst`, which must not exist. Returns the manifest.
    pub fn copy_tree(&mut self, src: &Path, dst: &Path) -> Result<Manifest, CopyError> {
        let mut manifest = Manifest::default();
        let meta = fs::symlink_metadata(src).map_err(io_err("read", src))?;
        let mode = mode_of(&meta);
        // Owner-writable while filling it; the real bits go on at the end.
        fs::create_dir(dst).map_err(io_err("create", dst))?;
        manifest.entries.push(Entry { rel: PathBuf::new(), kind: Kind::Dir { mode } });
        self.copy_children(src, src, dst, Path::new(""), &mut manifest)?;
        finish_dir(dst, mode, &meta);
        Ok(manifest)
    }

    fn copy_children(&mut self, root: &Path, src: &Path, dst: &Path, rel: &Path, manifest: &mut Manifest) -> Result<(), CopyError> {
        for child in sorted_children(src)? {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(CopyError::Cancelled);
            }
            let name = child.file_name();
            let (from, to, child_rel) = (src.join(&name), dst.join(&name), rel.join(&name));
            let meta = fs::symlink_metadata(&from).map_err(io_err("read", &from))?;
            let kind = meta.file_type();
            if kind.is_symlink() {
                let original = fs::read_link(&from).map_err(io_err("read the link", &from))?;
                let target = copied_link_target(root, &from, &original);
                make_symlink(&target, &to).map_err(io_err("create the link", &to))?;
                manifest.entries.push(Entry { rel: child_rel, kind: Kind::Link { target, original } });
            } else if kind.is_dir() {
                fs::create_dir(&to).map_err(io_err("create", &to))?;
                let mode = mode_of(&meta);
                manifest.entries.push(Entry { rel: child_rel.clone(), kind: Kind::Dir { mode } });
                self.copy_children(root, &from, &to, &child_rel, manifest)?;
                finish_dir(&to, mode, &meta);
            } else if kind.is_file() {
                let sha1 = self.copy_file(&from, &to, &meta)?;
                let after = fs::symlink_metadata(&from).map_err(io_err("read", &from))?;
                if after.len() != meta.len() || !same_time(after.modified().ok(), meta.modified().ok()) {
                    return Err(CopyError::Changed(from));
                }
                manifest.entries.push(Entry { rel: child_rel, kind: Kind::File { size: meta.len(), mode: mode_of(&meta), modified: meta.modified().ok(), sha1 } });
            } else {
                return Err(CopyError::Special(from));
            }
        }
        Ok(())
    }

    fn copy_file(&mut self, from: &Path, to: &Path, meta: &fs::Metadata) -> Result<[u8; 20], CopyError> {
        let mut input = File::open(from).map_err(io_err("open", from))?;
        let mut output = OpenOptions::new().write(true).create_new(true).open(to).map_err(io_err("create", to))?;
        let mut buf = vec![0u8; BUFFER];
        let mut hash = Sha1::new();
        loop {
            let n = input.read(&mut buf).map_err(io_err("read", from))?;
            if n == 0 {
                break;
            }
            hash.update(&buf[..n]);
            output.write_all(&buf[..n]).map_err(io_err("write", to))?;
            self.done += n as u64;
            (self.progress)(self.done, from);
            if self.cancel.load(Ordering::Relaxed) {
                return Err(CopyError::Cancelled);
            }
        }
        output.sync_all().map_err(io_err("write", to))?;
        let mut times = fs::FileTimes::new();
        if let Ok(t) = meta.modified() {
            times = times.set_modified(t);
        }
        if let Ok(t) = meta.accessed() {
            times = times.set_accessed(t);
        }
        output.set_times(times).map_err(io_err("set the time of", to))?;
        drop(output);
        set_mode(to, mode_of(meta)).map_err(io_err("set the permissions of", to))?;
        Ok(hash.finish())
    }
}

/// Folder times and bits, best effort (a folder's time is not part of the check).
fn finish_dir(dir: &Path, mode: u32, meta: &fs::Metadata) {
    if let (Ok(file), Ok(modified)) = (File::open(dir), meta.modified()) {
        let _ = file.set_times(fs::FileTimes::new().set_modified(modified));
    }
    let _ = set_mode(dir, mode);
}

pub fn sha1_of(path: &Path) -> io::Result<[u8; 20]> {
    let mut file = File::open(path)?;
    let mut buf = vec![0u8; BUFFER];
    let mut hash = Sha1::new();
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(hash.finish());
        }
        hash.update(&buf[..n]);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerifyStats {
    pub files: usize,
    pub bytes: u64,
    /// Files read back and hashed.
    pub hashed: usize,
}

fn walk_new(root: &Path, rel: &Path, out: &mut BTreeMap<PathBuf, (fs::Metadata, Option<PathBuf>)>) -> Result<(), String> {
    let dir = root.join(rel);
    let entries = fs::read_dir(&dir).map_err(|e| format!("could not read {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("could not read {}: {e}", dir.display()))?;
        let child_rel = rel.join(entry.file_name());
        let path = root.join(&child_rel);
        let meta = fs::symlink_metadata(&path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
        let link = if meta.file_type().is_symlink() { Some(fs::read_link(&path).map_err(|e| format!("could not read {}: {e}", path.display()))?) } else { None };
        let is_dir = meta.is_dir();
        out.insert(child_rel.clone(), (meta, link));
        if is_dir {
            walk_new(root, &child_rel, out)?;
        }
    }
    Ok(())
}

/// Compares the new tree at `root` with the manifest. `must_hash` names files (relative) that are
/// always read back; programs (any execute bit) always are too.
pub fn verify(root: &Path, manifest: &Manifest, policy: Policy, must_hash: &BTreeSet<PathBuf>, cancel: &AtomicBool) -> Result<VerifyStats, String> {
    let mut found = BTreeMap::new();
    walk_new(root, Path::new(""), &mut found)?;
    let expected: BTreeSet<&PathBuf> = manifest.entries.iter().filter(|e| !e.rel.as_os_str().is_empty()).map(|e| &e.rel).collect();
    let present: BTreeSet<&PathBuf> = found.keys().collect();
    if let Some(extra) = present.difference(&expected).next() {
        return Err(format!("the copy has {} that the original does not", extra.display()));
    }
    if let Some(missing) = expected.difference(&present).next() {
        return Err(format!("{} is missing from the copy", missing.display()));
    }
    let hash_all = manifest.bytes() <= policy.full_hash_limit;
    let mut stats = VerifyStats::default();
    for entry in &manifest.entries {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        if entry.rel.as_os_str().is_empty() {
            continue;
        }
        let (meta, link) = &found[&entry.rel];
        let shown = entry.rel.display();
        match &entry.kind {
            Kind::Dir { mode } => {
                if !meta.is_dir() {
                    return Err(format!("{shown} should be a folder in the copy"));
                }
                if mode_of(meta) != *mode {
                    return Err(format!("{shown} has other permissions in the copy"));
                }
            }
            Kind::Link { target, .. } => {
                if link.as_ref() != Some(target) {
                    return Err(format!("the link {shown} points somewhere else in the copy"));
                }
            }
            Kind::File { size, mode, modified, sha1 } => {
                if !meta.is_file() || meta.len() != *size {
                    return Err(format!("{shown} has a different size in the copy"));
                }
                if mode_of(meta) != *mode {
                    return Err(format!("{shown} has other permissions in the copy"));
                }
                if !same_time(meta.modified().ok(), *modified) {
                    return Err(format!("{shown} has a different date in the copy"));
                }
                stats.files += 1;
                stats.bytes += size;
                if hash_all || mode & 0o111 != 0 || must_hash.contains(&entry.rel) {
                    let path = root.join(&entry.rel);
                    let got = sha1_of(&path).map_err(|e| format!("could not read back {}: {e}", path.display()))?;
                    if got != *sha1 {
                        return Err(format!("{shown} does not match the original (its contents differ)"));
                    }
                    stats.hashed += 1;
                }
            }
        }
    }
    Ok(stats)
}

/// Removes a copy this engine made (a folder, or a single link for an adopted port).
pub fn remove_copy(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() || m.is_file() => fs::remove_file(path),
        Ok(_) => {
            make_writable(path);
            fs::remove_dir_all(path)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Read-only folders (copied bits) would stop `remove_dir_all`; give the owner write back first.
fn make_writable(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(m) = fs::symlink_metadata(dir) {
            if m.is_dir() {
                let _ = fs::set_permissions(dir, fs::Permissions::from_mode(mode_of(&m) | 0o700));
                if let Ok(entries) = fs::read_dir(dir) {
                    for e in entries.flatten() {
                        make_writable(&e.path());
                    }
                }
            }
        }
    }
    #[cfg(not(unix))]
    let _ = dir;
}
