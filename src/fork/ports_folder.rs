//! Where installed ports live (the "ports folder", upstream's `Library/`), as a setting.
//!
//! Stored as `ports_folder` in `fork-settings.json`. Not set (or set to the default) means
//! `<app folder>/Library`, exactly as before, so existing installs behave the same. `main.rs` builds
//! `AppPaths` from [`at_startup`]; the Settings screen (`src/app/ports_folder_screen.rs`) changes it.
//!
//! Rules for a new ports folder ([`check`]):
//! - an absolute path, not a system folder, not the home folder itself, not a drive's top folder;
//! - not inside the app's own folder (except the default `Library`), not holding the app's folder,
//!   not inside the current ports folder;
//! - on a Linux file system: exFAT, FAT and NTFS cannot keep program bits or links, which ports need;
//! - writable, and keeps executable bits and symlinks, found by trying in a temporary folder
//!   ([`FolderProbe`], replaceable in tests);
//! - a missing folder is created only after the user says so ([`create`]), and never at the place of a
//!   drive that is not mounted.
//!
//! At startup a configured folder that is missing is never created and nothing is marked uninstalled:
//! the app says the folder is not available (SD card out?) and blocks installs until it is back or
//! another folder is chosen.

use super::folder_picker::mounts::{self, MountEntry};
use super::settings::ForkSettings;
use crate::core::path_safety::is_within;
use crate::fork::rom_screens::util::human_bytes;
use serde_json::Value;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const SETTING_KEY: &str = "ports_folder";
pub const DEFAULT_NAME: &str = "Library";

pub fn default_dir(app_dir: &Path) -> PathBuf {
    app_dir.join(DEFAULT_NAME)
}

/// The folder the user chose, if any (relative or empty values are ignored).
pub fn configured(settings: &ForkSettings) -> Option<PathBuf> {
    settings.get(SETTING_KEY).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(PathBuf::from).filter(|p| p.is_absolute())
}

/// The ports folder in use: the chosen one, else the default.
pub fn current(app_dir: &Path, settings: &ForkSettings) -> PathBuf {
    configured(settings).unwrap_or_else(|| default_dir(app_dir))
}

/// Saves the choice. The default folder is saved as "not set", so moving the app keeps working.
pub fn save_choice(settings: &mut ForkSettings, app_dir: &Path, dir: &Path) -> io::Result<()> {
    if dir == default_dir(app_dir) {
        settings.remove(SETTING_KEY)
    } else {
        settings.set(SETTING_KEY, Value::from(dir.to_string_lossy().into_owned()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Missing {
    /// The folder is on a removable drive location and nothing is mounted there.
    DriveNotMounted(PathBuf),
    /// The drive is there (or it is not a drive) but the folder is gone.
    FolderGone,
    /// Something that is not a folder has its name.
    NotAFolder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Startup {
    pub dir: PathBuf,
    /// `None` when the folder is there and usable.
    pub missing: Option<Missing>,
    pub is_default: bool,
}

impl Startup {
    pub fn available(&self) -> bool {
        self.missing.is_none()
    }
}

/// What the app should use at startup. The default folder is created when missing (as upstream
/// always did); a chosen folder never is.
pub fn at_startup(app_dir: &Path, settings: &ForkSettings, mounts: &[MountEntry], media_roots: &[PathBuf]) -> Startup {
    match configured(settings).filter(|dir| *dir != default_dir(app_dir)) {
        None => {
            let dir = default_dir(app_dir);
            let _ = fs::create_dir_all(&dir);
            Startup { dir, missing: None, is_default: true }
        }
        Some(dir) => {
            let missing = availability(&dir, mounts, media_roots);
            Startup { dir, missing, is_default: false }
        }
    }
}

/// `None` when `dir` is a usable folder now, else why not.
pub fn availability(dir: &Path, mounts: &[MountEntry], media_roots: &[PathBuf]) -> Option<Missing> {
    if dir.is_dir() {
        return None;
    }
    if fs::symlink_metadata(dir).is_ok() {
        return Some(Missing::NotAFolder);
    }
    Some(match mounts::unmounted_drive_for(dir, mounts, media_roots) {
        Some(drive) => Missing::DriveNotMounted(drive),
        None => Missing::FolderGone,
    })
}

pub fn missing_text(dir: &Path, why: &Missing) -> String {
    let path = dir.display();
    match why {
        Missing::DriveNotMounted(_) => format!("Your ports folder {path} is not available. Is the SD card in?"),
        Missing::FolderGone => format!("Your ports folder {path} is not there any more. Was it moved, renamed or deleted?"),
        Missing::NotAFolder => format!("Your ports folder {path} is not a folder."),
    }
}

// -------------------------------------------------------------------------------------------------
// Checking a new folder
// -------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProbeResult {
    /// Why a file could not be created, if it could not.
    pub write_error: Option<String>,
    pub exec_bits: bool,
    pub symlinks: bool,
}

/// Tries what a ports folder needs in a temporary sub-folder of `dir` and removes it again.
pub trait FolderProbe {
    fn probe(&self, dir: &Path) -> ProbeResult;
}

impl<F: Fn(&Path) -> ProbeResult> FolderProbe for F {
    fn probe(&self, dir: &Path) -> ProbeResult {
        self(dir)
    }
}

pub struct SystemProbe;

impl FolderProbe for SystemProbe {
    fn probe(&self, dir: &Path) -> ProbeResult {
        let tmp = dir.join(format!(".dpm-probe-{}-{}", std::process::id(), stamp_nanos()));
        if let Err(e) = fs::create_dir(&tmp) {
            return ProbeResult { write_error: Some(e.to_string()), exec_bits: false, symlinks: false };
        }
        let file = tmp.join("program");
        let mut result = ProbeResult::default();
        match fs::write(&file, b"#!/bin/sh\n") {
            Err(e) => result.write_error = Some(e.to_string()),
            Ok(()) => {
                result.exec_bits = exec_bits_stick(&file);
                result.symlinks = symlinks_work(&tmp);
            }
        }
        let _ = fs::remove_dir_all(&tmp);
        result
    }
}

fn stamp_nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

#[cfg(unix)]
fn exec_bits_stick(file: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(file, fs::Permissions::from_mode(0o755)).is_ok()
        && fs::metadata(file).is_ok_and(|m| m.permissions().mode() & 0o111 == 0o111)
        && fs::set_permissions(file, fs::Permissions::from_mode(0o644)).is_ok()
        && fs::metadata(file).is_ok_and(|m| m.permissions().mode() & 0o111 == 0)
}

#[cfg(not(unix))]
fn exec_bits_stick(_file: &Path) -> bool {
    true
}

#[cfg(unix)]
fn symlinks_work(dir: &Path) -> bool {
    let link = dir.join("link");
    std::os::unix::fs::symlink("program", &link).is_ok() && fs::read_link(&link).is_ok_and(|t| t == Path::new("program"))
}

#[cfg(not(unix))]
fn symlinks_work(_dir: &Path) -> bool {
    true
}

/// Folders a ports folder may never be, or be inside.
const SYSTEM_TREES: [&str; 14] = ["/bin", "/boot", "/dev", "/etc", "/lib", "/lib64", "/opt", "/proc", "/root", "/sbin", "/srv", "/sys", "/usr", "/var"];
/// Folders a ports folder may not be, though folders inside them may.
const SYSTEM_EXACT: [&str; 6] = ["/", "/home", "/run", "/mnt", "/media", "/tmp"];

pub struct CheckCtx<'a> {
    pub app_dir: &'a Path,
    pub current: &'a Path,
    pub home: Option<&'a Path>,
    pub mounts: &'a [MountEntry],
    pub media_roots: &'a [PathBuf],
    /// Trees refused whole (`SYSTEM_TREES` plus `/tmp` outside tests).
    pub system_trees: Vec<PathBuf>,
    pub system_exact: Vec<PathBuf>,
    pub free_space: fn(&Path) -> Option<u64>,
}

impl<'a> CheckCtx<'a> {
    /// The machine's rules. `/tmp` is refused as a tree too (it is emptied at reboot).
    pub fn system(app_dir: &'a Path, current: &'a Path, home: Option<&'a Path>, mounts: &'a [MountEntry], media_roots: &'a [PathBuf]) -> CheckCtx<'a> {
        let mut system_trees: Vec<PathBuf> = SYSTEM_TREES.iter().map(PathBuf::from).collect();
        system_trees.push(PathBuf::from("/tmp"));
        CheckCtx {
            app_dir,
            current,
            home,
            mounts,
            media_roots,
            system_trees,
            system_exact: SYSTEM_EXACT.iter().map(PathBuf::from).collect(),
            free_space: crate::fork::rom_screens::util::free_bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub dir: PathBuf,
    pub exists: bool,
    /// The folder is the one in use already.
    pub same_as_current: bool,
    /// Reasons it cannot be used; empty means it can.
    pub problems: Vec<String>,
    pub free: Option<u64>,
    pub fs_type: Option<String>,
    /// "SD card (SD512)" or "internal storage", for messages.
    pub where_: String,
}

impl Check {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }

    /// "391.0 GB free, ext4, SD card (SD512)".
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(free) = self.free {
            parts.push(format!("{} free", human_bytes(free)));
        }
        if let Some(fs_type) = &self.fs_type {
            parts.push(fs_type.clone());
        }
        if !self.where_.is_empty() {
            parts.push(self.where_.clone());
        }
        parts.join(", ")
    }
}

/// The kernel's name for a file system, as people know it.
pub fn fs_display_name(fs_type: &str) -> &str {
    match fs_type {
        "exfat" => "exFAT",
        "vfat" | "msdos" => "FAT",
        "ntfs" | "ntfs3" | "fuseblk" => "NTFS",
        other => other,
    }
}

pub fn non_linux_text(fs_type: &str) -> String {
    format!(
        "This drive is formatted as {}, which can't store Linux programs (it drops the 'may run' permission and links). \
         Ports need a card formatted by SteamOS: Steam > Settings > Storage > Format. Formatting ERASES everything on the card, \
         so copy your game files off it first.",
        fs_display_name(fs_type)
    )
}

/// When the probe finds that program permissions do not stick on a file system not known as non-Linux.
pub const NO_EXEC_TEXT: &str =
    "This folder can't keep program permissions, so ports could not start here. Pick a folder on the internal drive or on a card formatted by SteamOS.";

/// Whether `dir` can become the ports folder. Never changes anything on disk except the probe's own
/// temporary folder inside `dir` (or its nearest existing parent, when `dir` does not exist yet).
pub fn check(dir: &Path, ctx: &CheckCtx, probe: &dyn FolderProbe) -> Check {
    let exists = dir.is_dir();
    let mut c = Check {
        dir: dir.to_path_buf(),
        exists,
        same_as_current: dir == ctx.current,
        problems: Vec::new(),
        free: None,
        fs_type: None,
        where_: String::new(),
    };
    if !dir.is_absolute() {
        c.problems.push("The folder must be a full path.".into());
        return c;
    }
    if !exists && fs::symlink_metadata(dir).is_ok() {
        c.problems.push("Something that is not a folder already has this name.".into());
        return c;
    }
    if let Some(drive) = mounts::unmounted_drive_for(dir, ctx.mounts, ctx.media_roots) {
        c.problems.push(format!("Nothing is mounted at {}. Is the SD card in?", drive.display()));
        return c;
    }
    let real = mounts::existing_real_path(dir);
    let either_inside = |base: &Path| is_within(dir, base) || is_within(&real, base);
    let equals = |other: &Path| dir == other || real == mounts::existing_real_path(other);

    if ctx.system_exact.iter().any(|p| equals(p)) || ctx.system_trees.iter().any(|t| either_inside(t)) || ctx.media_roots.iter().any(|r| equals(r)) {
        c.problems.push("That is a system folder. Pick or create a folder of your own, like ~/Games/Ports or a folder on the SD card.".into());
    } else if ctx.home.is_some_and(|h| equals(h)) {
        c.problems.push("Not the home folder itself: pick or create a folder inside it, like ~/Games/Ports.".into());
    } else if mounts::removable_drives(ctx.mounts, ctx.media_roots).iter().any(|d| equals(&d.mount_point)) {
        c.problems.push("Not the top of the drive: create a folder on it for ports (New folder here...), like Ports.".into());
    }

    let default = default_dir(ctx.app_dir);
    if !equals(&default) {
        if either_inside(ctx.app_dir) {
            c.problems.push("That is inside the app's own folder. Pick a folder outside it (the default Library is the only exception).".into());
        } else if is_within(ctx.app_dir, dir) || is_within(&mounts::existing_real_path(ctx.app_dir), &real) {
            c.problems.push("The app's own folder is inside that folder. Pick a folder of its own for ports.".into());
        }
    }
    if !c.same_as_current && (is_within(dir, ctx.current) || is_within(&real, &mounts::existing_real_path(ctx.current))) {
        c.problems.push("That is inside the current ports folder (a port's own folder). Pick a folder outside it.".into());
    }

    c.fs_type = mounts::fs_type_of(dir, ctx.mounts);
    c.where_ = match mounts::drive_of(dir, ctx.mounts, ctx.media_roots) {
        Some(d) => d.title(),
        None if ctx.home.is_some_and(|h| either_inside(h)) => "internal storage".into(),
        None => String::new(),
    };
    let existing = super::folder_picker::nearest_existing_folder(dir);
    c.free = existing.as_deref().and_then(ctx.free_space);
    if let Some(fs_type) = c.fs_type.as_deref().filter(|t| mounts::is_non_linux_fs(t)) {
        c.problems.push(non_linux_text(fs_type));
        return c;
    }
    if !c.problems.is_empty() {
        return c;
    }
    let Some(probe_dir) = existing else {
        c.problems.push("None of the folders on the way exist.".into());
        return c;
    };
    let result = probe.probe(&probe_dir);
    if let Some(e) = result.write_error {
        let target = if exists { "this folder" } else { "the folder it would be created in" };
        c.problems.push(format!("You can't write to {target} ({e})."));
    } else {
        if !result.exec_bits {
            c.problems.push(NO_EXEC_TEXT.to_string());
        }
        if !result.symlinks {
            c.problems.push("Links can't be created here; ports and connected game files need them. Use an ext4 folder.".into());
        }
    }
    c
}

/// Creates a missing ports folder (and missing parents) after the user agreed. Refuses when its
/// drive is not mounted, so nothing is ever created at an empty mount point.
pub fn create(dir: &Path, mounts: &[MountEntry], media_roots: &[PathBuf]) -> Result<(), String> {
    if let Some(drive) = mounts::unmounted_drive_for(dir, mounts, media_roots) {
        return Err(format!("Nothing is mounted at {}, so the folder was not created. Is the SD card in?", drive.display()));
    }
    fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))
}

// -------------------------------------------------------------------------------------------------
// Planning a change of ports folder
// -------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortToMove {
    pub key: String,
    pub name: String,
    pub from: PathBuf,
    pub to: PathBuf,
    /// The port is a link to another folder (adopted): only the link moves.
    pub link_only: bool,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MovePlan {
    pub ports: Vec<PortToMove>,
    /// Ports that already have a folder of their name in the new place: not moved; after the switch the
    /// app uses the one in the new place.
    pub already_there: Vec<String>,
    /// Ports whose folder in the new place is an old copy the user kept after an earlier move: it is
    /// older than the one in use (`from`), so it must be replaced first, never used silently.
    pub stale: Vec<PortToMove>,
    /// Folders in the old ports folder that are no port the catalog knows: never moved.
    pub other_folders: Vec<String>,
    pub bytes: u64,
}

/// What switching from `old` to `new` would move: every catalog port whose folder is in `old`.
/// `kept_old_copies` are the old copies recorded in `port-moves.json` (their paths).
pub fn plan_move(old: &Path, new: &Path, catalog: &[crate::core::models::Port], kept_old_copies: &[PathBuf]) -> MovePlan {
    let mut plan = MovePlan::default();
    let mut known: Vec<String> = Vec::new();
    for port in catalog {
        let Ok(from) = crate::core::path_safety::safe_join(old, &port.folder) else { continue };
        let Ok(to) = crate::core::path_safety::safe_join(new, &port.folder) else { continue };
        if port.folder.is_empty() || known.contains(&port.folder) {
            continue;
        }
        let Ok(meta) = fs::symlink_metadata(&from) else { continue };
        known.push(port.folder.clone());
        let link_only = meta.file_type().is_symlink();
        let bytes = if link_only { 0 } else { super::port_mover::copy::tree_size(&from) };
        if fs::symlink_metadata(&to).is_ok() {
            if kept_old_copies.contains(&to) {
                plan.bytes += bytes;
                plan.stale.push(PortToMove { key: port.key().to_string(), name: port.display_name().to_string(), from, to, link_only, bytes });
            } else {
                plan.already_there.push(port.display_name().to_string());
            }
            continue;
        }
        plan.bytes += bytes;
        plan.ports.push(PortToMove { key: port.key().to_string(), name: port.display_name().to_string(), from, to, link_only, bytes });
    }
    if let Ok(entries) = fs::read_dir(old) {
        let mut others: Vec<String> = entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.') && !known.contains(n))
            .collect();
        others.sort();
        plan.other_folders = others;
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_move_plan_lists_installed_catalog_ports_and_leaves_the_rest() {
        use crate::core::models::port_from_value;
        let w = World::new("plan");
        let old = w.app.join("Library");
        let new = w.media.join("SD512").join("Ports");
        fs::create_dir_all(new.join("Dusk")).unwrap();
        fs::write(old.join("SoH").join("soh"), vec![0u8; 1234]).unwrap();
        fs::create_dir_all(old.join("Dusk")).unwrap();
        fs::create_dir_all(old.join("my-stuff")).unwrap();
        let catalog: Vec<_> = [
            serde_json::json!({"name": "Ship of Harkinian", "folder": "SoH", "source": "https://github.com/a/soh"}),
            serde_json::json!({"name": "Dusklight", "folder": "Dusk", "source": "https://github.com/a/dusk"}),
            serde_json::json!({"name": "Not installed", "folder": "Nope", "source": "https://github.com/a/nope"}),
        ]
        .iter()
        .map(|v| port_from_value(v).unwrap())
        .collect();
        let plan = plan_move(&old, &new, &catalog, &[]);
        assert_eq!(plan.ports.len(), 1);
        assert_eq!((plan.ports[0].name.as_str(), plan.ports[0].bytes), ("Ship of Harkinian", 1234));
        assert_eq!(plan.ports[0].to, new.join("SoH"));
        assert_eq!(plan.already_there, ["Dusklight"]);
        assert_eq!(plan.other_folders, ["my-stuff"]);
        assert_eq!(plan.bytes, 1234);

        // Moving back onto a folder that is an old copy the user kept: flagged, never used silently.
        let plan = plan_move(&old, &new, &catalog, &[new.join("Dusk")]);
        assert!(plan.already_there.is_empty());
        assert_eq!(plan.stale.len(), 1);
        assert_eq!((plan.stale[0].name.as_str(), plan.stale[0].from.clone(), plan.stale[0].to.clone()), ("Dusklight", old.join("Dusk"), new.join("Dusk")));
    }

    fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("dpm_ports_folder_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn good(_: &Path) -> ProbeResult {
        ProbeResult { write_error: None, exec_bits: true, symlinks: true }
    }

    fn exfat_like(_: &Path) -> ProbeResult {
        ProbeResult { write_error: None, exec_bits: false, symlinks: false }
    }

    fn read_only(_: &Path) -> ProbeResult {
        ProbeResult { write_error: Some("Read-only file system".into()), exec_bits: false, symlinks: false }
    }

    fn free(_: &Path) -> Option<u64> {
        Some(5_000_000_000)
    }

    struct World {
        root: PathBuf,
        app: PathBuf,
        home: PathBuf,
        media: PathBuf,
        mounts: Vec<MountEntry>,
    }

    impl World {
        fn new(name: &str) -> World {
            let root = temp(name);
            let home = root.join("home");
            let app = home.join("Applications").join("dpm");
            let media = root.join("media");
            fs::create_dir_all(app.join("Library").join("SoH")).unwrap();
            fs::create_dir_all(media.join("SD512")).unwrap();
            fs::create_dir_all(media.join("STICK")).unwrap();
            let mounts = mounts::parse_mounts(&format!(
                "/dev/mmcblk0p1 {} ext4 rw 0 0\n/dev/sda1 {} exfat rw 0 0\n",
                media.join("SD512").display(),
                media.join("STICK").display()
            ));
            World { root, app, home, media, mounts }
        }

        fn ctx(&self) -> CheckCtx<'_> {
            CheckCtx {
                app_dir: &self.app,
                current: self.app.as_path(),
                home: Some(&self.home),
                mounts: &self.mounts,
                media_roots: std::slice::from_ref(&self.media),
                system_trees: vec![PathBuf::from("/usr"), PathBuf::from("/etc")],
                system_exact: vec![PathBuf::from("/")],
                free_space: free,
            }
        }

        fn check(&self, dir: &Path, probe: &dyn FolderProbe) -> Check {
            let current = default_dir(&self.app);
            let mut ctx = self.ctx();
            ctx.current = &current;
            check(dir, &ctx, probe)
        }
    }

    impl Drop for World {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn the_setting_defaults_to_library_next_to_the_app_and_persists() {
        let w = World::new("setting");
        let path = w.app.join(super::super::settings::FILE_NAME);
        let mut s = ForkSettings::load(&path);
        assert_eq!(current(&w.app, &s), w.app.join("Library"));
        let sd = w.media.join("SD512").join("Ports");
        save_choice(&mut s, &w.app, &sd).unwrap();
        let reloaded = ForkSettings::load(&path);
        assert_eq!(current(&w.app, &reloaded), sd);
        // Other keys survive, and choosing the default again clears the key.
        let mut s = reloaded;
        s.set_steam_account(42).unwrap();
        save_choice(&mut s, &w.app, &w.app.join("Library")).unwrap();
        let reloaded = ForkSettings::load(&path);
        assert!(reloaded.get(SETTING_KEY).is_none());
        assert_eq!(reloaded.steam_account(), Some(42));
        // A relative value is ignored.
        let mut s = reloaded;
        s.set(SETTING_KEY, Value::from("Ports")).unwrap();
        assert_eq!(current(&w.app, &s), w.app.join("Library"));
    }

    #[test]
    fn at_startup_the_default_is_created_and_a_missing_chosen_folder_is_not() {
        let w = World::new("startup");
        let mut s = ForkSettings::load(&w.app.join("fork-settings.json"));
        fs::remove_dir_all(w.app.join("Library")).unwrap();
        let st = at_startup(&w.app, &s, &w.mounts, std::slice::from_ref(&w.media));
        assert!(st.available() && st.is_default && w.app.join("Library").is_dir());

        // On an SD card that is not in: reported, nothing created at the mount point.
        let card = w.media.join("OTHERCARD").join("Ports");
        s.set(SETTING_KEY, Value::from(card.to_string_lossy().into_owned())).unwrap();
        let st = at_startup(&w.app, &s, &w.mounts, std::slice::from_ref(&w.media));
        assert_eq!(st.missing, Some(Missing::DriveNotMounted(w.media.join("OTHERCARD"))));
        assert!(!w.media.join("OTHERCARD").exists());
        assert!(missing_text(&st.dir, st.missing.as_ref().unwrap()).contains("Is the SD card in?"));

        // On a mounted card, but the folder was deleted: also reported, not created.
        let gone = w.media.join("SD512").join("Ports");
        s.set(SETTING_KEY, Value::from(gone.to_string_lossy().into_owned())).unwrap();
        let st = at_startup(&w.app, &s, &w.mounts, std::slice::from_ref(&w.media));
        assert_eq!(st.missing, Some(Missing::FolderGone));
        assert!(!gone.exists());

        // Back in place: available.
        fs::create_dir_all(&gone).unwrap();
        assert!(at_startup(&w.app, &s, &w.mounts, std::slice::from_ref(&w.media)).available());
    }

    #[test]
    fn a_good_folder_passes_with_its_free_space_and_drive() {
        let w = World::new("good");
        let dir = w.media.join("SD512").join("Ports");
        fs::create_dir_all(&dir).unwrap();
        let c = w.check(&dir, &good);
        assert!(c.ok(), "{:?}", c.problems);
        assert_eq!(c.summary(), "5.0 GB free, ext4, SD card (SD512)");
        let missing = w.check(&w.home.join("Games").join("Ports"), &good);
        assert!(missing.ok() && !missing.exists, "{:?}", missing.problems);
        assert!(w.check(&default_dir(&w.app), &good).same_as_current);
    }

    #[test]
    fn exfat_and_folders_that_cannot_keep_program_bits_are_refused_with_the_reason() {
        let w = World::new("exfat");
        let stick = w.media.join("STICK").join("Ports");
        fs::create_dir_all(&stick).unwrap();
        let c = w.check(&stick, &good);
        assert!(!c.ok());
        assert!(c.problems[0].starts_with("This drive is formatted as exFAT, which can't store Linux programs") && c.problems[0].contains("ERASES"), "{:?}", c.problems);
        // Not known as exFAT from the mounts list, but the probe finds the same thing.
        let dir = w.home.join("Ports");
        fs::create_dir_all(&dir).unwrap();
        let c = w.check(&dir, &exfat_like);
        assert_eq!(c.problems.len(), 2, "{:?}", c.problems);
        assert!(c.problems[0] == NO_EXEC_TEXT && c.problems[1].contains("Links"));
        let c = w.check(&dir, &read_only);
        assert!(c.problems[0].contains("can't write"), "{:?}", c.problems);
    }

    #[test]
    fn system_home_drive_top_and_app_folders_are_refused() {
        let w = World::new("refused");
        let mut cases = vec![
            (w.media.clone(), "system folder"),
            (w.home.clone(), "home folder itself"),
            (w.media.join("SD512"), "top of the drive"),
            (w.app.clone(), "inside the app's own folder"),
            (w.app.join("cache"), "inside the app's own folder"),
            (w.home.join("Applications"), "app's own folder is inside"),
            (w.app.join("Library").join("SoH"), "inside the app's own folder"),
            (w.media.join("NOTIN").join("Ports"), "Is the SD card in?"),
        ];
        if cfg!(unix) {
            cases.push((PathBuf::from("/"), "system folder"));
            cases.push((PathBuf::from("/usr/games"), "system folder"));
        }
        for (dir, words) in cases {
            let c = w.check(&dir, &good);
            assert!(c.problems.iter().any(|p| p.contains(words)), "{} -> {:?}", dir.display(), c.problems);
        }
        assert!(w.check(Path::new("relative/Ports"), &good).problems[0].contains("full path"));
    }

    #[test]
    fn inside_a_chosen_current_folder_is_refused() {
        let w = World::new("inside_current");
        let current = w.media.join("SD512").join("Ports");
        fs::create_dir_all(current.join("SoH")).unwrap();
        let mut ctx = w.ctx();
        ctx.current = &current;
        let c = check(&current.join("SoH"), &ctx, &good);
        assert!(c.problems.iter().any(|p| p.contains("inside the current ports folder")), "{:?}", c.problems);
        assert!(check(&current, &ctx, &good).same_as_current);
    }

    #[test]
    fn create_makes_missing_folders_but_never_on_an_unmounted_drive() {
        let w = World::new("create");
        let dir = w.home.join("Games").join("Ports");
        create(&dir, &w.mounts, std::slice::from_ref(&w.media)).unwrap();
        assert!(dir.is_dir());
        let card = w.media.join("GONE").join("Ports");
        assert!(create(&card, &w.mounts, std::slice::from_ref(&w.media)).unwrap_err().contains("SD card"));
        assert!(!w.media.join("GONE").exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_real_probe_finds_a_normal_folder_fine_and_leaves_nothing_behind() {
        let w = World::new("probe");
        let r = SystemProbe.probe(&w.home);
        assert_eq!(r, ProbeResult { write_error: None, exec_bits: true, symlinks: true });
        assert_eq!(fs::read_dir(&w.home).unwrap().count(), 1, "only Applications is left");
        let r = SystemProbe.probe(&w.home.join("missing"));
        assert!(r.write_error.is_some());
    }
}
