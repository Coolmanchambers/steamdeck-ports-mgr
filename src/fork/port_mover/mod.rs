//! Moving a port folder: copy, verify, switch over, keep the old folder.
//!
//! Used when the ports folder changes (Settings > Ports folder) and by the import screen (stage 2).
//! "Move" never deletes: the old folder stays until the user removes it with the separate, explicit
//! "Remove old copy" action ([`old_copies::remove`]), which the app offers right after a move and
//! lists later.
//!
//! Two steps, so a caller moving several ports can copy all of them before switching any:
//!
//! 1. [`prepare`] checks (the port is not running, the target is free, there is room, the game files
//!    are all there: [`game_files::check_before`]), copies into a hidden staging folder next to the
//!    target, verifies the copy ([`copy::verify`]: file list, sizes, permission bits, link targets,
//!    hashes) and checks the game files again at the new place ([`game_files::check_after`]), then
//!    renames the staging folder to the target. Any failure removes only the staging folder: the
//!    original and everything else stay as they were. Nothing outside the target is written.
//! 2. [`switch_over`] makes the new copy the one in use for everything except the app's own ports
//!    folder setting (the caller owns that): Steam shortcuts are repointed (written now, or queued
//!    while Steam runs; app IDs kept), ROM library connections follow, and the old folder is recorded
//!    as an old copy. The app's other records need no change: `state.json` (install records, favorite
//!    program, play time) and `ports.local.json` are keyed by port and hold paths relative to the port
//!    folder.
//!
//! [`discard`] removes a prepared copy that was never switched to (a cancelled ports folder change).
//!
//! API summary for other screens:
//! - `prepare(&PortRef, from, to, &MoveEnv, &MoveOptions, progress, cancel) -> Result<Prepared, MoveError>`
//! - `switch_over(&Prepared, &MoveEnv) -> SwitchReport` (`.text()` for the result dialog)
//! - `discard(&Prepared)`
//! - `old_copies::{load, removal_check, remove, forget}` for the cleanup prompt and list
//! - `game_files::GameFiles::for_port(&Port, config_dir)` builds what the game file check needs.

pub mod copy;
pub mod game_files;
pub mod old_copies;
pub mod steam;

#[cfg(test)]
mod tests;

use crate::core::path_safety::is_within;
use crate::fork::rom_library::RomLibraryStore;
use crate::fork::rom_screens::util::human_bytes;
use crate::fork::settings::ForkSettings;
use crate::fork::store::SteamProcessCheck;
use copy::{Copier, CopyError, Manifest, Policy};
use game_files::{GameFiles, Issue};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

/// Room left free on the target drive besides the copy itself.
pub const SPACE_MARGIN: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortRef {
    pub key: String,
    pub name: String,
}

/// What the engine needs from the app (all replaceable in tests).
pub struct MoveEnv<'a> {
    pub home: &'a Path,
    /// The app folder: `steam-queue.json`, `port-moves.json`, `rom_library.json` live here.
    pub config_dir: &'a Path,
    pub settings: &'a ForkSettings,
    pub steam: &'a dyn SteamProcessCheck,
    pub game_mode: bool,
    /// Programs running from a folder (names); the app passes save restore's /proc check.
    pub running: &'a dyn Fn(&Path) -> Vec<String>,
    pub free_space: &'a dyn Fn(&Path) -> Option<u64>,
}

#[derive(Default)]
pub struct MoveOptions<'a> {
    pub game_files: GameFiles,
    /// The user saw the game file problems found before the copy and chose "Move anyway". Problems
    /// found after the copy always stop the move.
    pub accept_issues_before: bool,
    pub policy: Option<Policy>,
    /// Tests only: runs on the staging folder after the copy, before it is checked.
    pub after_copy: Option<&'a dyn Fn(&Path)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    BeforeCopy,
    AfterCopy,
}

#[derive(Debug)]
pub enum MoveError {
    /// A program (the game) runs from the folder.
    Running(Vec<String>),
    SourceMissing(PathBuf),
    TargetExists(PathBuf),
    TargetParentMissing(PathBuf),
    /// One folder is inside the other.
    Nested,
    NotEnoughSpace { needed: u64, free: u64 },
    GameFiles { stage: Stage, issues: Vec<Issue> },
    Cancelled,
    Copy(String),
    Verify(String),
}

impl std::fmt::Display for MoveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let unchanged = "Nothing was changed.";
        match self {
            MoveError::Running(names) => write!(f, "{} is running from this port's folder. Close it, then try again. {unchanged}", names.join(", ")),
            MoveError::SourceMissing(p) => write!(f, "{} is not there. {unchanged}", p.display()),
            MoveError::TargetExists(p) => write!(f, "There is already something at {}. {unchanged}", p.display()),
            MoveError::TargetParentMissing(p) => write!(f, "The folder {} does not exist. {unchanged}", p.display()),
            MoveError::Nested => write!(f, "The old and new places are inside each other. {unchanged}"),
            MoveError::NotEnoughSpace { needed, free } => write!(f, "Not enough space: the copy needs {} and {} is free. {unchanged}", human_bytes(*needed), human_bytes(*free)),
            MoveError::GameFiles { stage: Stage::BeforeCopy, issues } => {
                write!(f, "Some game files of this port are missing or broken:\n{}\n{unchanged}", issues.iter().map(|i| format!("  {}", i.line())).collect::<Vec<_>>().join("\n"))
            }
            MoveError::GameFiles { stage: Stage::AfterCopy, issues } => write!(
                f,
                "The game files did not come through the copy intact:\n{}\nThe new copy was removed; the original is untouched.",
                issues.iter().map(|i| format!("  {}", i.line())).collect::<Vec<_>>().join("\n")
            ),
            MoveError::Cancelled => write!(f, "Cancelled. The partial copy was removed. {unchanged}"),
            MoveError::Copy(e) => write!(f, "The copy failed: {e}. The partial copy was removed; the original is untouched."),
            MoveError::Verify(e) => write!(f, "The copy did not match the original: {e}. The new copy was removed; the original is untouched."),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Prepared {
    pub port: PortRef,
    pub from: PathBuf,
    pub to: PathBuf,
    pub bytes: u64,
    pub files: usize,
    /// Files read back and compared by hash.
    pub hashed: usize,
    /// The port was a link to another folder (adopted): only the link was copied.
    pub link_only: bool,
    /// Links that pointed outside the folder with a relative path and now hold an absolute one.
    pub rewritten_links: Vec<PathBuf>,
    /// Game file problems the user accepted before the copy ("Move anyway").
    pub accepted_issues: Vec<Issue>,
}

fn staging_path(to: &Path) -> PathBuf {
    let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "port".into());
    to.with_file_name(format!(".{name}.dpm-moving-{}", std::process::id()))
}

fn prepare_checks(from: &Path, to: &Path, env: &MoveEnv) -> Result<fs::Metadata, MoveError> {
    let meta = fs::symlink_metadata(from).map_err(|_| MoveError::SourceMissing(from.to_path_buf()))?;
    if fs::symlink_metadata(to).is_ok() {
        return Err(MoveError::TargetExists(to.to_path_buf()));
    }
    let parent = to.parent().filter(|p| p.is_dir()).ok_or_else(|| MoveError::TargetParentMissing(to.parent().unwrap_or(to).to_path_buf()))?;
    let real_from = fs::canonicalize(from).unwrap_or_else(|_| from.to_path_buf());
    let real_parent = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
    if is_within(to, from) || is_within(from, to) || (!meta.file_type().is_symlink() && is_within(&real_parent, &real_from)) {
        return Err(MoveError::Nested);
    }
    let running = (env.running)(from);
    if !running.is_empty() {
        return Err(MoveError::Running(running));
    }
    Ok(meta)
}

/// Step 1: check, copy, verify. See the module docs. `progress(done_bytes, total_bytes, current file)`.
pub fn prepare(
    port: &PortRef,
    from: &Path,
    to: &Path,
    env: &MoveEnv,
    options: &MoveOptions,
    progress: &mut dyn FnMut(u64, u64, &Path),
    cancel: &AtomicBool,
) -> Result<Prepared, MoveError> {
    let meta = prepare_checks(from, to, env)?;
    let issues = game_files::check_before(from, &options.game_files);
    if !issues.is_empty() && !options.accept_issues_before {
        return Err(MoveError::GameFiles { stage: Stage::BeforeCopy, issues });
    }
    let staging = staging_path(to);
    if fs::symlink_metadata(&staging).is_ok() {
        // Left by a run of this same process id that was killed; it is ours by name.
        copy::remove_copy(&staging).map_err(|e| MoveError::Copy(format!("could not clear the old staging folder {}: {e}", staging.display())))?;
    }

    let link_only = meta.file_type().is_symlink();
    let result = if link_only {
        copy_link(from, &staging, options)
    } else {
        let total = copy::tree_size(from);
        if let Some(free) = (env.free_space)(to.parent().unwrap_or(to)) {
            if free < total.saturating_add(SPACE_MARGIN) {
                return Err(MoveError::NotEnoughSpace { needed: total.saturating_add(SPACE_MARGIN), free });
            }
        }
        copy_folder(from, &staging, total, options, progress, cancel)
    };
    let (manifest, stats) = match result {
        Ok(done) => done,
        Err(e) => {
            let _ = copy::remove_copy(&staging);
            return Err(e);
        }
    };
    if !link_only {
        let after = game_files::check_after(from, &staging, &manifest, &options.game_files);
        if !after.is_empty() {
            let _ = copy::remove_copy(&staging);
            return Err(MoveError::GameFiles { stage: Stage::AfterCopy, issues: after });
        }
    }
    // The target may have appeared while copying.
    if fs::symlink_metadata(to).is_ok() {
        let _ = copy::remove_copy(&staging);
        return Err(MoveError::TargetExists(to.to_path_buf()));
    }
    if let Err(e) = fs::rename(&staging, to) {
        let _ = copy::remove_copy(&staging);
        return Err(MoveError::Copy(format!("could not put the copy in place at {}: {e}", to.display())));
    }
    sync_dir(to.parent());
    Ok(Prepared {
        port: port.clone(),
        from: from.to_path_buf(),
        to: to.to_path_buf(),
        bytes: stats.bytes,
        files: stats.files,
        hashed: stats.hashed,
        link_only,
        rewritten_links: manifest.rewritten_links().iter().map(|e| to.join(&e.rel)).collect(),
        accepted_issues: if options.accept_issues_before { issues } else { Vec::new() },
    })
}

fn copy_link(from: &Path, staging: &Path, options: &MoveOptions) -> Result<(Manifest, copy::VerifyStats), MoveError> {
    let original = fs::read_link(from).map_err(|e| MoveError::Copy(format!("could not read the link {}: {e}", from.display())))?;
    let target = copy::absolute_target(from, &original);
    copy::make_symlink(&target, staging).map_err(|e| MoveError::Copy(format!("could not create the link {}: {e}", staging.display())))?;
    if let Some(hook) = options.after_copy {
        hook(staging);
    }
    let same = matches!((fs::canonicalize(from), fs::canonicalize(staging)), (Ok(a), Ok(b)) if a == b);
    if fs::read_link(staging).ok().as_deref() != Some(target.as_path()) || !same {
        return Err(MoveError::Verify("the new link does not reach the same folder".into()));
    }
    Ok((Manifest::default(), copy::VerifyStats::default()))
}

fn copy_folder(
    from: &Path,
    staging: &Path,
    total: u64,
    options: &MoveOptions,
    progress: &mut dyn FnMut(u64, u64, &Path),
    cancel: &AtomicBool,
) -> Result<(Manifest, copy::VerifyStats), MoveError> {
    let mut report = |done: u64, file: &Path| progress(done, total, file);
    let mut copier = Copier::new(cancel, &mut report);
    let manifest = copier.copy_tree(from, staging).map_err(|e| match e {
        CopyError::Cancelled => MoveError::Cancelled,
        other => MoveError::Copy(other.to_string()),
    })?;
    if let Some(hook) = options.after_copy {
        hook(staging);
    }
    let must = game_files::must_hash(from, &options.game_files);
    let stats = copy::verify(staging, &manifest, options.policy.unwrap_or_default(), &must, cancel).map_err(|e| if e == "cancelled" { MoveError::Cancelled } else { MoveError::Verify(e) })?;
    Ok((manifest, stats))
}

fn sync_dir(dir: Option<&Path>) {
    #[cfg(unix)]
    if let Some(dir) = dir {
        if let Ok(f) = fs::File::open(dir) {
            let _ = f.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Removes a prepared copy that was never switched to. Only the new copy is touched.
pub fn discard(prepared: &Prepared) -> std::io::Result<()> {
    copy::remove_copy(&prepared.to)
}

#[derive(Debug, Clone, Default)]
pub struct SwitchReport {
    pub steam: steam::SteamReport,
    pub rom_records: usize,
    pub notes: Vec<String>,
}

impl SwitchReport {
    pub fn text(&self) -> String {
        let mut lines = Vec::new();
        let steam = self.steam.text();
        if !steam.is_empty() {
            lines.push(steam);
        }
        if self.rom_records > 0 {
            lines.push(format!("Game files: {} connection record(s) follow the new folder.", self.rom_records));
        }
        lines.extend(self.notes.iter().cloned());
        lines.join("\n")
    }
}

/// Step 2: point Steam, the ROM library records and the old-copy list at the new copy. Never deletes.
pub fn switch_over(prepared: &Prepared, env: &MoveEnv) -> SwitchReport {
    let mut report = SwitchReport::default();
    let ctx = steam::SteamCtx { home: env.home, config_dir: env.config_dir, settings: env.settings, steam: env.steam, game_mode: env.game_mode, now: SystemTime::now() };
    report.steam = steam::repoint_everywhere(&prepared.port.key, &prepared.port.name, &prepared.from, &prepared.to, &ctx);
    match RomLibraryStore::load(&env.config_dir.join("rom_library.json")) {
        Ok(mut store) => match store.repoint_dests(&prepared.from, &prepared.to) {
            Ok(n) => report.rom_records = n,
            Err(e) => report.notes.push(format!("Game files: could not update the connection records ({e}); check them in Settings > Game files.")),
        },
        Err(e) => report.notes.push(format!("Game files: {e}")),
    }
    let entry = old_copies::OldCopy {
        port_key: prepared.port.key.clone(),
        name: prepared.port.name.clone(),
        old_path: prepared.from.clone(),
        new_path: prepared.to.clone(),
        bytes: prepared.bytes,
        moved_at: crate::fork::store::backup_stamp(),
        link_only: prepared.link_only,
    };
    if let Err(e) = old_copies::record(env.config_dir, entry) {
        report.notes.push(format!("The old copy at {} could not be added to the list of old copies ({e}); it is still there.", prepared.from.display()));
    }
    report
}
