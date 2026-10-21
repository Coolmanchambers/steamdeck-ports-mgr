//! Connect a port to a game file: plan first, change nothing until the plan is applied.
//!
//! Three methods, cheapest first:
//! - **Use original**: the port is given the path to the file where it is. Nothing is written.
//! - **Link**: a symlink in the port's folder pointing at the original. No extra space.
//! - **Copy**: a second copy in the port's folder. Costs the file's size, so a plan for it says so and
//!   `apply` refuses it without explicit approval.
//!
//! Rules that hold for every method:
//! - The original is opened for reading only. It is never moved, renamed, deleted or written.
//! - Nothing already in the port folder is overwritten. A different file at the destination refuses
//!   the plan; an identical one (or a link already pointing at the original) is accepted as already
//!   connected and left exactly as it is.
//! - Destinations are confined to the port's folder.
//! - A copy is written under a temporary name, checked, and only then given its final name, without
//!   replacing anything; a failed or cancelled copy removes only the temporary file it made.

use super::hash::Crc32;
use super::scan::files_identical;
use super::system::ByteOrder;
use crate::core::path_safety::safe_join;
use chrono::Utc;
use serde_json::{json, Value};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// The cheapest way a port can reach a game file. Each level also allows the more expensive ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Can be given a path to the file (reads it once to build its own assets, or has a setting).
    InPlace,
    /// Wants the file inside its own folder but follows a symlink.
    Link,
    /// Wants a real file inside its own folder.
    Copy,
}

impl Access {
    pub fn id(self) -> &'static str {
        match self {
            Access::InPlace => "in_place",
            Access::Link => "link",
            Access::Copy => "copy",
        }
    }

    pub fn from_id(id: &str) -> Option<Access> {
        match id {
            "in_place" => Some(Access::InPlace),
            "link" => Some(Access::Link),
            "copy" => Some(Access::Copy),
            _ => None,
        }
    }

    pub fn allows(self, method: Method) -> bool {
        match (self, method) {
            (Access::InPlace, _) => true,
            (Access::Link, Method::Symlink | Method::Copy) => true,
            (Access::Copy, Method::Copy) => true,
            _ => false,
        }
    }

    /// The method to suggest first.
    pub fn recommended(self) -> Method {
        match self {
            Access::InPlace => Method::UseInPlace,
            Access::Link => Method::Symlink,
            Access::Copy => Method::Copy,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    UseInPlace,
    Symlink,
    Copy,
}

impl Method {
    pub fn id(self) -> &'static str {
        match self {
            Method::UseInPlace => "in_place",
            Method::Symlink => "link",
            Method::Copy => "copy",
        }
    }

    pub fn from_id(id: &str) -> Option<Method> {
        match id {
            "in_place" => Some(Method::UseInPlace),
            "link" => Some(Method::Symlink),
            "copy" => Some(Method::Copy),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Method::UseInPlace => "Use original",
            Method::Symlink => "Link",
            Method::Copy => "Copy",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConnectRequest {
    pub port_key: String,
    /// Which required file this is (`Slot::label`), so a port with several files keeps one mapping each.
    pub slot: String,
    /// The original, wherever the scan found it.
    pub source: PathBuf,
    /// The port's install folder. Links and copies go inside it.
    pub port_dir: PathBuf,
    /// Folder inside `port_dir` ("" = `port_dir` itself).
    pub dest_subdir: String,
    /// Name for the link or copy; the original's own name when `None`.
    pub dest_name: Option<String>,
    pub method: Method,
    pub access: Access,
    pub byte_order: Option<ByteOrder>,
    pub known_crc32: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanAction {
    UseInPlace,
    CreateLink,
    CreateCopy,
    /// The destination already holds what this request would create. Nothing will be written.
    AlreadyConnected(Method),
}

#[derive(Debug, Clone)]
pub struct ConnectPlan {
    pub request: ConnectRequest,
    /// The original, resolved.
    pub source: PathBuf,
    pub dest: Option<PathBuf>,
    pub size: u64,
    pub mtime_ms: u64,
    pub action: PlanAction,
    /// Disk space this will use: the file's size for a copy, nothing otherwise.
    pub extra_bytes: u64,
    /// True for a copy: it is not applied without approval.
    pub needs_confirmation: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub enum PlanError {
    SourceMissing(PathBuf),
    SourceNotAFile(PathBuf),
    MethodNotSupported { method: Method, access: Access },
    InvalidDestName(String),
    DestOutsidePortFolder(String),
    PortFolderMissing(PathBuf),
    /// Something else is already at the destination. It is never replaced.
    DestExistsDifferent(PathBuf),
    /// The destination is the original itself.
    DestIsSource(PathBuf),
    Io { path: PathBuf, source: io::Error },
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::SourceMissing(p) => write!(f, "{} is not there any more", p.display()),
            PlanError::SourceNotAFile(p) => write!(f, "{} is not a file", p.display()),
            PlanError::MethodNotSupported { method, access } => {
                write!(f, "this port cannot use \"{}\" (the cheapest it supports is \"{}\")", method.label(), access.recommended().label())
            }
            PlanError::InvalidDestName(n) => write!(f, "\"{n}\" is not a usable file name"),
            PlanError::DestOutsidePortFolder(p) => write!(f, "\"{p}\" points outside the port's folder"),
            PlanError::PortFolderMissing(p) => write!(f, "the port's folder {} does not exist; install the port first", p.display()),
            PlanError::DestExistsDifferent(p) => write!(f, "{} already exists and is not the same file; it was left alone", p.display()),
            PlanError::DestIsSource(p) => write!(f, "{} is the original file itself", p.display()),
            PlanError::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for PlanError {}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> PlanError + '_ {
    move |source| PlanError::Io { path: path.to_path_buf(), source }
}

fn mtime_ms(meta: &fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn valid_file_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0']) && !name.contains(':')
}

#[cfg(unix)]
fn make_symlink(original: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(original, link)
}

#[cfg(windows)]
fn make_symlink(original: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(original, link)
}

/// Decide what connecting would do. Reads the file system, writes nothing.
pub fn plan_connect(req: &ConnectRequest) -> Result<ConnectPlan, PlanError> {
    let meta = match fs::metadata(&req.source) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(PlanError::SourceMissing(req.source.clone())),
        Err(e) => return Err(io_err(&req.source)(e)),
    };
    if !meta.is_file() {
        return Err(PlanError::SourceNotAFile(req.source.clone()));
    }
    if !req.access.allows(req.method) {
        return Err(PlanError::MethodNotSupported { method: req.method, access: req.access });
    }
    let source = fs::canonicalize(&req.source).map_err(io_err(&req.source))?;
    let (size, mtime) = (meta.len(), mtime_ms(&meta));
    let mut warnings = Vec::new();
    if matches!(req.byte_order, Some(ByteOrder::ByteSwapped | ByteOrder::LittleEndian)) {
        warnings.push(format!(
            "This Nintendo 64 file is {}. Most ports want the big-endian .z64 order and will not read it as it is.",
            req.byte_order.map(ByteOrder::label).unwrap_or("")
        ));
    }
    let base = ConnectPlan {
        request: req.clone(),
        source: source.clone(),
        dest: None,
        size,
        mtime_ms: mtime,
        action: PlanAction::UseInPlace,
        extra_bytes: 0,
        needs_confirmation: false,
        warnings,
    };
    if req.method == Method::UseInPlace {
        return Ok(base);
    }

    if !req.port_dir.is_dir() {
        return Err(PlanError::PortFolderMissing(req.port_dir.clone()));
    }
    let name = match &req.dest_name {
        Some(n) => n.clone(),
        None => source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
    };
    if !valid_file_name(&name) {
        return Err(PlanError::InvalidDestName(name));
    }
    let dest_dir = safe_join(&req.port_dir, &req.dest_subdir).map_err(|_| PlanError::DestOutsidePortFolder(req.dest_subdir.clone()))?;
    let dest = dest_dir.join(&name);

    match fs::symlink_metadata(&dest) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let copy = req.method == Method::Copy;
            Ok(ConnectPlan {
                dest: Some(dest),
                action: if copy { PlanAction::CreateCopy } else { PlanAction::CreateLink },
                extra_bytes: if copy { size } else { 0 },
                needs_confirmation: copy,
                ..base
            })
        }
        Err(e) => Err(io_err(&dest)(e)),
        Ok(existing) => {
            if fs::canonicalize(&dest).map(|c| c == source).unwrap_or(false) {
                if !existing.file_type().is_symlink() {
                    return Err(PlanError::DestIsSource(dest));
                }
                return Ok(ConnectPlan { dest: Some(dest), action: PlanAction::AlreadyConnected(Method::Symlink), ..base });
            }
            // Something else is there. Identical content is fine to keep; anything else is not ours to touch.
            let same = fs::metadata(&dest).map(|m| m.is_file()).unwrap_or(false) && files_identical(&source, &dest).unwrap_or(false);
            if !same {
                return Err(PlanError::DestExistsDifferent(dest));
            }
            let found = if existing.file_type().is_symlink() { Method::Symlink } else { Method::Copy };
            Ok(ConnectPlan { dest: Some(dest), action: PlanAction::AlreadyConnected(found), ..base })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    None,
    /// The user saw the size and said yes to a copy.
    CopyAllowed,
}

#[derive(Debug)]
pub enum ApplyError {
    /// A copy needs `Approval::CopyAllowed`. Nothing was written.
    NeedsConfirmation,
    /// The original changed after the plan was made. Plan again.
    SourceChanged,
    /// Something appeared at the destination after the plan was made. It was left alone.
    DestAppeared(PathBuf),
    /// The file system refused a symlink (exFAT, FAT32, missing privilege on Windows). Plan a copy
    /// instead; that asks first.
    SymlinkUnsupported(io::Error),
    /// The copy did not match the original. The bad copy was removed.
    VerifyFailed(PathBuf),
    Io { path: PathBuf, source: io::Error },
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApplyError::NeedsConfirmation => write!(f, "a copy needs your approval first; nothing was written"),
            ApplyError::SourceChanged => write!(f, "the original changed since the plan was made; plan again"),
            ApplyError::DestAppeared(p) => write!(f, "{} appeared in the meantime; it was left alone", p.display()),
            ApplyError::SymlinkUnsupported(e) => write!(f, "this drive does not support links ({e}); a copy is the alternative"),
            ApplyError::VerifyFailed(p) => write!(f, "the copy at {} did not match the original and was removed", p.display()),
            ApplyError::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for ApplyError {}

fn apply_io(path: &Path) -> impl FnOnce(io::Error) -> ApplyError + '_ {
    move |source| ApplyError::Io { path: path.to_path_buf(), source }
}

/// What is recorded about a connection so it survives a restart and can be checked later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    pub port_key: String,
    pub slot: String,
    pub method: Method,
    /// The original.
    pub source: PathBuf,
    /// The link or copy; `None` for "use original".
    pub dest: Option<PathBuf>,
    pub size: u64,
    pub mtime_ms: u64,
    pub crc32: Option<u32>,
    /// True when this app created `dest`. A copy it created is a managed copy; one it found already
    /// there is not its to remove.
    pub created_by_us: bool,
    pub connected_at: String,
}

impl Mapping {
    pub fn to_json(&self) -> Value {
        json!({
            "port_key": self.port_key,
            "slot": self.slot,
            "method": self.method.id(),
            "source": self.source.to_string_lossy(),
            "dest": self.dest.as_ref().map(|d| d.to_string_lossy().into_owned()),
            "size": self.size,
            "mtime_ms": self.mtime_ms,
            "crc32": self.crc32.map(|c| format!("{c:08X}")),
            "created_by_us": self.created_by_us,
            "connected_at": self.connected_at,
        })
    }

    pub fn from_json(v: &Value) -> Option<Mapping> {
        Some(Mapping {
            port_key: v.get("port_key")?.as_str()?.to_string(),
            slot: v.get("slot").and_then(Value::as_str).unwrap_or("").to_string(),
            method: Method::from_id(v.get("method")?.as_str()?)?,
            source: PathBuf::from(v.get("source")?.as_str()?),
            dest: v.get("dest").and_then(Value::as_str).map(PathBuf::from),
            size: v.get("size")?.as_u64()?,
            mtime_ms: v.get("mtime_ms").and_then(Value::as_u64).unwrap_or(0),
            crc32: v.get("crc32").and_then(Value::as_str).and_then(|s| u32::from_str_radix(s, 16).ok()),
            created_by_us: v.get("created_by_us").and_then(Value::as_bool).unwrap_or(false),
            connected_at: v.get("connected_at").and_then(Value::as_str).unwrap_or("").to_string(),
        })
    }
}

fn mapping_for(plan: &ConnectPlan, method: Method, created_by_us: bool, crc32: Option<u32>) -> Mapping {
    Mapping {
        port_key: plan.request.port_key.clone(),
        slot: plan.request.slot.clone(),
        method,
        source: plan.source.clone(),
        dest: if method == Method::UseInPlace { None } else { plan.dest.clone() },
        size: plan.size,
        mtime_ms: plan.mtime_ms,
        crc32: crc32.or(plan.request.known_crc32),
        created_by_us,
        connected_at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    }
}

fn source_unchanged(plan: &ConnectPlan) -> Result<(), ApplyError> {
    let meta = fs::metadata(&plan.source).map_err(|_| ApplyError::SourceChanged)?;
    if meta.len() != plan.size || mtime_ms(&meta) != plan.mtime_ms {
        return Err(ApplyError::SourceChanged);
    }
    Ok(())
}

fn absent(path: &Path) -> bool {
    matches!(fs::symlink_metadata(path), Err(e) if e.kind() == io::ErrorKind::NotFound)
}

/// Carry out a plan. Returns the mapping to record.
pub fn apply(plan: &ConnectPlan, approval: Approval) -> Result<Mapping, ApplyError> {
    match &plan.action {
        PlanAction::UseInPlace => {
            source_unchanged(plan)?;
            Ok(mapping_for(plan, Method::UseInPlace, false, None))
        }
        PlanAction::AlreadyConnected(found) => {
            source_unchanged(plan)?;
            Ok(mapping_for(plan, *found, false, None))
        }
        PlanAction::CreateLink => {
            let dest = plan.dest.as_ref().expect("a link plan has a destination");
            source_unchanged(plan)?;
            fs::create_dir_all(dest.parent().unwrap_or(Path::new("."))).map_err(apply_io(dest))?;
            if !absent(dest) {
                return Err(ApplyError::DestAppeared(dest.clone()));
            }
            make_symlink(&plan.source, dest).map_err(|e| {
                if e.kind() == io::ErrorKind::AlreadyExists {
                    ApplyError::DestAppeared(dest.clone())
                } else {
                    ApplyError::SymlinkUnsupported(e)
                }
            })?;
            // Read back through the link: it must resolve to the original.
            if fs::canonicalize(dest).map(|c| c != plan.source).unwrap_or(true) {
                let _ = fs::remove_file(dest);
                return Err(ApplyError::VerifyFailed(dest.clone()));
            }
            Ok(mapping_for(plan, Method::Symlink, true, None))
        }
        PlanAction::CreateCopy => {
            if approval != Approval::CopyAllowed {
                return Err(ApplyError::NeedsConfirmation);
            }
            let dest = plan.dest.as_ref().expect("a copy plan has a destination");
            source_unchanged(plan)?;
            let crc = copy_without_overwriting(plan, dest)?;
            Ok(mapping_for(plan, Method::Copy, true, Some(crc)))
        }
    }
}

fn copy_without_overwriting(plan: &ConnectPlan, dest: &Path) -> Result<u32, ApplyError> {
    let dest_dir = dest.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dest_dir).map_err(apply_io(dest_dir))?;
    if !absent(dest) {
        return Err(ApplyError::DestAppeared(dest.to_path_buf()));
    }
    let tmp_name = format!(".{}.dpm-copy-{}", dest.file_name().unwrap_or_default().to_string_lossy(), std::process::id());
    let tmp = dest_dir.join(tmp_name);
    let result = write_temp_and_publish(plan, &tmp, dest);
    // Only the temporary file this call made is ever removed here.
    if tmp.exists() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn write_temp_and_publish(plan: &ConnectPlan, tmp: &Path, dest: &Path) -> Result<u32, ApplyError> {
    let mut src = File::open(&plan.source).map_err(apply_io(&plan.source))?;
    let mut out = OpenOptions::new().write(true).create_new(true).open(tmp).map_err(apply_io(tmp))?;
    let mut crc = Crc32::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        let n = match src.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(apply_io(&plan.source)(e)),
        };
        crc.update(&buf[..n]);
        out.write_all(&buf[..n]).map_err(apply_io(tmp))?;
        total += n as u64;
    }
    out.sync_all().map_err(apply_io(tmp))?;
    drop(out);
    if total != plan.size {
        return Err(ApplyError::SourceChanged);
    }
    // `hard_link` fails if the name is taken, so unlike `rename` it cannot replace anything. Where
    // hard links are unavailable (exFAT) fall back to a checked rename.
    match fs::hard_link(tmp, dest) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(ApplyError::DestAppeared(dest.to_path_buf())),
        Err(_) => {
            if !absent(dest) {
                return Err(ApplyError::DestAppeared(dest.to_path_buf()));
            }
            fs::rename(tmp, dest).map_err(apply_io(dest))?;
        }
    }
    match files_identical(&plan.source, dest) {
        Ok(true) => Ok(crc.finish()),
        _ => {
            // The destination did not exist before this call, so this removes only our own bad copy.
            let _ = fs::remove_file(dest);
            Err(ApplyError::VerifyFailed(dest.to_path_buf()))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Ok,
    /// The original is gone (drive unplugged, file moved).
    SourceMissing,
    /// The original has a different size or date than when it was connected.
    SourceChanged,
    /// The link or copy is gone.
    DestMissing,
    /// The link points somewhere else, or the copy's size differs.
    DestChanged,
}

/// Check a recorded mapping against the disk. Read-only.
pub fn check_mapping(m: &Mapping) -> Health {
    let Ok(src) = fs::metadata(&m.source) else { return Health::SourceMissing };
    if src.len() != m.size || mtime_ms(&src) != m.mtime_ms {
        return Health::SourceChanged;
    }
    let Some(dest) = &m.dest else { return Health::Ok };
    let Ok(lmeta) = fs::symlink_metadata(dest) else { return Health::DestMissing };
    match m.method {
        Method::Symlink => {
            if !lmeta.file_type().is_symlink() {
                return Health::DestChanged;
            }
            match (fs::canonicalize(dest), fs::canonicalize(&m.source)) {
                (Ok(a), Ok(b)) if a == b => Health::Ok,
                (Err(_), _) => Health::DestMissing,
                _ => Health::DestChanged,
            }
        }
        _ => {
            if lmeta.len() == m.size {
                Health::Ok
            } else {
                Health::DestChanged
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;

    struct Fixture {
        _tmp: tempfile::TempDir,
        roms: PathBuf,
        port: PathBuf,
        rom: PathBuf,
        data: Vec<u8>,
    }

    fn fixture() -> Fixture {
        let tmp = temp_dir();
        let roms = tmp.path().join("roms");
        let port = tmp.path().join("library").join("MyPort");
        let data = random_bytes(42, 300_000);
        let rom = roms.join("n64").join("Game (USA).z64");
        write(&rom, &data);
        fs::create_dir_all(&port).unwrap();
        Fixture { _tmp: tmp, roms, port, rom, data }
    }

    fn request(f: &Fixture, method: Method, access: Access) -> ConnectRequest {
        ConnectRequest {
            port_key: "owner/repo".into(),
            slot: "Game (USA).z64".into(),
            source: f.rom.clone(),
            port_dir: f.port.clone(),
            dest_subdir: String::new(),
            dest_name: None,
            method,
            access,
            byte_order: None,
            known_crc32: None,
        }
    }

    fn originals_untouched(f: &Fixture, before: &[(PathBuf, super::super::hash::Digests)]) {
        assert_eq!(&snapshot(&f.roms), before, "the ROM folder changed");
        assert_eq!(fs::read(&f.rom).unwrap(), f.data);
    }

    #[test]
    fn access_levels_allow_the_methods_at_or_above_them() {
        assert!(Access::InPlace.allows(Method::UseInPlace) && Access::InPlace.allows(Method::Symlink) && Access::InPlace.allows(Method::Copy));
        assert!(!Access::Link.allows(Method::UseInPlace) && Access::Link.allows(Method::Symlink) && Access::Link.allows(Method::Copy));
        assert!(!Access::Copy.allows(Method::UseInPlace) && !Access::Copy.allows(Method::Symlink) && Access::Copy.allows(Method::Copy));
        for a in [Access::InPlace, Access::Link, Access::Copy] {
            assert_eq!(Access::from_id(a.id()), Some(a));
            assert!(a.allows(a.recommended()));
        }
    }

    #[test]
    fn use_in_place_is_refused_for_a_port_that_cannot_take_a_path() {
        let f = fixture();
        let err = plan_connect(&request(&f, Method::UseInPlace, Access::Link)).unwrap_err();
        assert!(matches!(err, PlanError::MethodNotSupported { method: Method::UseInPlace, access: Access::Link }));
        assert!(matches!(plan_connect(&request(&f, Method::Symlink, Access::Copy)).unwrap_err(), PlanError::MethodNotSupported { .. }));
    }

    #[test]
    fn use_in_place_writes_nothing_and_records_the_original() {
        let f = fixture();
        let before = snapshot(&f.roms);
        let port_before = snapshot(&f.port);
        let plan = plan_connect(&request(&f, Method::UseInPlace, Access::InPlace)).unwrap();
        assert_eq!(plan.action, PlanAction::UseInPlace);
        assert!(plan.dest.is_none() && !plan.needs_confirmation && plan.extra_bytes == 0);
        let mapping = apply(&plan, Approval::None).unwrap();
        assert_eq!(mapping.method, Method::UseInPlace);
        assert_eq!(mapping.source, fs::canonicalize(&f.rom).unwrap());
        assert!(mapping.dest.is_none() && !mapping.created_by_us);
        originals_untouched(&f, &before);
        assert_eq!(snapshot(&f.port), port_before);
    }

    #[cfg(unix)]
    #[test]
    fn link_plan_is_free_and_applying_it_creates_a_symlink_to_the_original() {
        let f = fixture();
        let before = snapshot(&f.roms);
        let plan = plan_connect(&request(&f, Method::Symlink, Access::Link)).unwrap();
        assert_eq!(plan.action, PlanAction::CreateLink);
        assert_eq!(plan.extra_bytes, 0);
        assert!(!plan.needs_confirmation);
        let dest = plan.dest.clone().unwrap();
        assert_eq!(dest, f.port.join("Game (USA).z64"));
        assert!(!dest.exists(), "planning must not write");

        let mapping = apply(&plan, Approval::None).unwrap();
        assert!(fs::symlink_metadata(&dest).unwrap().file_type().is_symlink());
        assert_eq!(fs::canonicalize(&dest).unwrap(), fs::canonicalize(&f.rom).unwrap());
        assert_eq!(fs::read(&dest).unwrap(), f.data);
        assert!(mapping.created_by_us);
        assert_eq!(check_mapping(&mapping), Health::Ok);
        originals_untouched(&f, &before);
    }

    #[test]
    fn copy_plan_says_what_it_costs_and_apply_refuses_without_approval() {
        let f = fixture();
        let before = snapshot(&f.roms);
        let plan = plan_connect(&request(&f, Method::Copy, Access::Copy)).unwrap();
        assert_eq!(plan.action, PlanAction::CreateCopy);
        assert_eq!(plan.extra_bytes, f.data.len() as u64);
        assert!(plan.needs_confirmation);

        assert!(matches!(apply(&plan, Approval::None), Err(ApplyError::NeedsConfirmation)));
        assert!(snapshot(&f.port).is_empty(), "nothing is written without approval");
        originals_untouched(&f, &before);
    }

    #[test]
    fn approved_copy_is_byte_identical_leaves_no_temp_file_and_never_touches_the_original() {
        let f = fixture();
        let before = snapshot(&f.roms);
        let plan = plan_connect(&request(&f, Method::Copy, Access::Link)).unwrap();
        let mapping = apply(&plan, Approval::CopyAllowed).unwrap();
        let dest = f.port.join("Game (USA).z64");
        assert_eq!(fs::read(&dest).unwrap(), f.data);
        assert!(!fs::symlink_metadata(&dest).unwrap().file_type().is_symlink());
        assert_eq!(mapping.method, Method::Copy);
        assert!(mapping.created_by_us, "a copy this app made is recorded as a managed copy");
        assert_eq!(mapping.crc32, Some(digests_of(&f.data).crc32));
        let names: Vec<_> = fs::read_dir(&f.port).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["Game (USA).z64"], "no temporary file is left behind");
        assert_eq!(check_mapping(&mapping), Health::Ok);
        originals_untouched(&f, &before);
    }

    #[test]
    fn a_different_file_at_the_destination_is_never_overwritten() {
        let f = fixture();
        let theirs = b"somebody else's file".to_vec();
        write(&f.port.join("Game (USA).z64"), &theirs);
        for method in [Method::Symlink, Method::Copy] {
            let err = plan_connect(&request(&f, method, Access::Link)).unwrap_err();
            assert!(matches!(err, PlanError::DestExistsDifferent(_)), "{method:?}");
        }
        assert_eq!(fs::read(f.port.join("Game (USA).z64")).unwrap(), theirs);
    }

    #[test]
    fn a_file_that_appears_between_plan_and_apply_is_not_overwritten() {
        let f = fixture();
        for method in [Method::Symlink, Method::Copy] {
            let _ = fs::remove_file(f.port.join("Game (USA).z64"));
            let plan = plan_connect(&request(&f, method, Access::Link)).unwrap();
            let theirs = b"appeared later".to_vec();
            write(&f.port.join("Game (USA).z64"), &theirs);
            let err = apply(&plan, Approval::CopyAllowed).unwrap_err();
            assert!(matches!(err, ApplyError::DestAppeared(_)), "{method:?}: {err}");
            assert_eq!(fs::read(f.port.join("Game (USA).z64")).unwrap(), theirs);
            let leftovers: Vec<_> = fs::read_dir(&f.port).unwrap().map(|e| e.unwrap().file_name()).collect();
            assert_eq!(leftovers.len(), 1, "{leftovers:?}");
        }
    }

    #[test]
    fn an_identical_file_already_there_counts_as_connected_and_is_left_alone() {
        let f = fixture();
        write(&f.port.join("Game (USA).z64"), &f.data);
        let before_port = snapshot(&f.port);
        let plan = plan_connect(&request(&f, Method::Copy, Access::Link)).unwrap();
        assert_eq!(plan.action, PlanAction::AlreadyConnected(Method::Copy));
        assert!(!plan.needs_confirmation);
        let mapping = apply(&plan, Approval::None).unwrap();
        assert_eq!(mapping.method, Method::Copy);
        assert!(!mapping.created_by_us, "a file that was already there is not ours");
        assert_eq!(snapshot(&f.port), before_port);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_already_pointing_at_the_original_counts_as_connected() {
        let f = fixture();
        std::os::unix::fs::symlink(fs::canonicalize(&f.rom).unwrap(), f.port.join("Game (USA).z64")).unwrap();
        let plan = plan_connect(&request(&f, Method::Symlink, Access::Link)).unwrap();
        assert_eq!(plan.action, PlanAction::AlreadyConnected(Method::Symlink));
        let mapping = apply(&plan, Approval::None).unwrap();
        assert!(!mapping.created_by_us);
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_or_foreign_symlink_at_the_destination_is_not_replaced() {
        let f = fixture();
        let dest = f.port.join("Game (USA).z64");
        std::os::unix::fs::symlink(f.port.join("nowhere"), &dest).unwrap();
        assert!(matches!(plan_connect(&request(&f, Method::Symlink, Access::Link)).unwrap_err(), PlanError::DestExistsDifferent(_)));
        assert_eq!(fs::read_link(&dest).unwrap(), f.port.join("nowhere"));
    }

    #[test]
    fn the_destination_stays_inside_the_port_folder() {
        let f = fixture();
        for name in ["../evil.z64", "sub/evil.z64", "..", "", "a\\b.z64"] {
            let mut req = request(&f, Method::Copy, Access::Copy);
            req.dest_name = Some(name.to_string());
            assert!(matches!(plan_connect(&req).unwrap_err(), PlanError::InvalidDestName(_)), "{name:?}");
        }
        for sub in ["..", "../other", "a/../../b"] {
            let mut req = request(&f, Method::Copy, Access::Copy);
            req.dest_subdir = sub.to_string();
            assert!(matches!(plan_connect(&req).unwrap_err(), PlanError::DestOutsidePortFolder(_)), "{sub:?}");
        }
        assert!(!f.port.parent().unwrap().join("evil.z64").exists());
    }

    #[test]
    fn a_subfolder_and_a_new_name_are_honoured_and_created_inside_the_port() {
        let f = fixture();
        let mut req = request(&f, Method::Copy, Access::Copy);
        req.dest_subdir = "isos/disc1".into();
        req.dest_name = Some("tp.iso".into());
        let plan = plan_connect(&req).unwrap();
        apply(&plan, Approval::CopyAllowed).unwrap();
        assert_eq!(fs::read(f.port.join("isos/disc1/tp.iso")).unwrap(), f.data);
    }

    #[test]
    fn a_missing_port_folder_or_original_is_a_plan_error() {
        let f = fixture();
        let mut req = request(&f, Method::Copy, Access::Copy);
        req.port_dir = f.port.join("not installed");
        assert!(matches!(plan_connect(&req).unwrap_err(), PlanError::PortFolderMissing(_)));
        let mut req = request(&f, Method::UseInPlace, Access::InPlace);
        req.source = f.roms.join("gone.z64");
        assert!(matches!(plan_connect(&req).unwrap_err(), PlanError::SourceMissing(_)));
        req.source = f.roms.clone();
        assert!(matches!(plan_connect(&req).unwrap_err(), PlanError::SourceNotAFile(_)));
    }

    #[test]
    fn a_destination_that_is_the_original_is_refused() {
        let f = fixture();
        let mut req = request(&f, Method::Copy, Access::Copy);
        req.port_dir = f.roms.clone();
        req.dest_subdir = "n64".into();
        assert!(matches!(plan_connect(&req).unwrap_err(), PlanError::DestIsSource(_)));
        assert_eq!(fs::read(&f.rom).unwrap(), f.data);
    }

    #[test]
    fn an_original_that_changes_after_planning_stops_the_apply() {
        let f = fixture();
        let plan = plan_connect(&request(&f, Method::Copy, Access::Copy)).unwrap();
        let mut changed = f.data.clone();
        changed.push(1);
        fs::write(&f.rom, &changed).unwrap();
        assert!(matches!(apply(&plan, Approval::CopyAllowed), Err(ApplyError::SourceChanged)));
        assert!(snapshot(&f.port).is_empty());
    }

    #[test]
    fn a_swapped_n64_file_gets_a_warning_on_the_plan() {
        let f = fixture();
        let mut req = request(&f, Method::UseInPlace, Access::InPlace);
        req.byte_order = Some(ByteOrder::ByteSwapped);
        let plan = plan_connect(&req).unwrap();
        assert_eq!(plan.warnings.len(), 1);
        assert!(plan.warnings[0].contains("byte-swapped"));
        req.byte_order = Some(ByteOrder::BigEndian);
        assert!(plan_connect(&req).unwrap().warnings.is_empty());
    }

    #[test]
    fn mapping_survives_a_json_round_trip() {
        let f = fixture();
        let mut req = request(&f, Method::Copy, Access::Copy);
        req.known_crc32 = Some(0xDEAD_BEEF);
        let m = apply(&plan_connect(&req).unwrap(), Approval::CopyAllowed).unwrap();
        assert_eq!(Mapping::from_json(&m.to_json()), Some(m));
        assert!(Mapping::from_json(&json!({"port_key": "x"})).is_none());
    }

    #[test]
    fn health_notices_a_moved_original_a_removed_copy_and_a_changed_copy() {
        let f = fixture();
        let m = apply(&plan_connect(&request(&f, Method::Copy, Access::Copy)).unwrap(), Approval::CopyAllowed).unwrap();
        assert_eq!(check_mapping(&m), Health::Ok);

        let dest = m.dest.clone().unwrap();
        fs::write(&dest, b"short").unwrap();
        assert_eq!(check_mapping(&m), Health::DestChanged);
        fs::remove_file(&dest).unwrap();
        assert_eq!(check_mapping(&m), Health::DestMissing);

        fs::write(&f.rom, b"edited").unwrap();
        assert_eq!(check_mapping(&m), Health::SourceChanged);
        fs::remove_file(&f.rom).unwrap();
        assert_eq!(check_mapping(&m), Health::SourceMissing);
    }

    #[cfg(unix)]
    #[test]
    fn health_notices_a_link_that_points_elsewhere() {
        let f = fixture();
        let m = apply(&plan_connect(&request(&f, Method::Symlink, Access::Link)).unwrap(), Approval::None).unwrap();
        let dest = m.dest.clone().unwrap();
        fs::remove_file(&dest).unwrap();
        let other = f.roms.join("other.z64");
        write(&other, b"other");
        std::os::unix::fs::symlink(&other, &dest).unwrap();
        assert_eq!(check_mapping(&m), Health::DestChanged);
    }
}
