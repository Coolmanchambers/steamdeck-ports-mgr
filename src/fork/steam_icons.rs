//! Steam icons of the ports' shortcuts: find the ones that show nothing, say why, and repair them.
//!
//! Engine only; `src/app/steam_icons_screen.rs` shows it. A later "Fix problems" checkup can call the same three
//! steps:
//!
//! 1. [`diagnose`]: for each installed port, every Steam shortcut (all accounts) whose program is inside the port's
//!    folder (the longest folder wins), and the state of its `icon` field ([`IconState`]).
//! 2. [`plan`]: for each shortcut that needs a new icon, where one can come from ([`Replacement`]), first that
//!    works: (1) the same app ID's icon in a `shortcuts.vdf.bak-*` backup next to the live file (newest first),
//!    when it differs from the current one and that file exists, or exists after moving it into the port's current
//!    folder (from any old folder recorded in `port-moves.json`); (2) an `<app id>_icon.*` already in the account's
//!    `config/grid/`; (3) the port's catalog icon from the app's cache, written into `config/grid/` through
//!    `art::grid::write_art` (which never overwrites art the user chose); (4) nothing.
//! 3. [`resolve`] then [`write_fixes`]: placing the catalog icon needs no Steam shutdown; changing the shortcut does
//!    (`store::ShortcutsFile::save` refuses while Steam runs), so with Steam running the fixes are queued
//!    (`steam_queue::QueuedKind::Icon`) and applied by the usual "Close Steam and update shortcuts" flow.
//!
//! Only the `icon` field of a shortcut is ever changed (never the app ID, program, start folder, launch options or
//! name), and only when it still holds the text it had when the problem was found. An icon is written as a plain
//! path, the way Steam stores one.

use crate::core::models::Port;
use crate::fork::art::catalog::catalog_art_from_files;
use crate::fork::art::fit::Slot;
use crate::fork::art::grid::{write_art, SlotOutcome, WriteOptions};
use crate::fork::backups;
use crate::fork::port_mover::old_copies;
use crate::fork::port_mover::steam::{device_accounts, moved_path, AccountFile, WriteError};
use crate::fork::settings::ForkSettings;
use crate::fork::steam_queue::{ArtChoice, IconChange, QueuedChange, QueuedKind};
use crate::fork::shortcuts::{self, ShortcutUpdate, Target};
use crate::fork::store::{backup_stamp, check_steam, ShortcutsFile, SteamProcessCheck, StoreError};
use crate::fork::vdf::Document;
use crate::fork::artwork::{self, ArtworkKind};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Picture types Steam can show as a shortcut's icon.
pub const ICON_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "ico", "tga"];
/// How many backups of `shortcuts.vdf` are looked through, newest first.
const MAX_BACKUPS: usize = 30;

/// A readable picture of a type Steam can show: the extension is one of [`ICON_EXTENSIONS`], the file is not empty and
/// (for the types that have one) starts with that type's signature.
pub fn is_showable_image(path: &Path) -> bool {
    let Some(ext) = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()) else { return false };
    if !ICON_EXTENSIONS.contains(&ext.as_str()) {
        return false;
    }
    let Ok(meta) = fs::metadata(path) else { return false };
    if !meta.is_file() || meta.len() == 0 {
        return false;
    }
    let mut head = [0u8; 8];
    let Ok(mut file) = fs::File::open(path) else { return false };
    let got = file.read(&mut head).unwrap_or(0);
    let head = &head[..got];
    match ext.as_str() {
        "png" => head.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
        "jpg" | "jpeg" => head.starts_with(&[0xFF, 0xD8, 0xFF]),
        "ico" => head.starts_with(&[0, 0, 1, 0]),
        _ => true,
    }
}

/// What a shortcut's `icon` field holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconState {
    /// A picture Steam can show, inside the port's folder or Steam's grid folder.
    Ok,
    /// Nothing is set.
    Empty,
    /// A path, but no file there.
    Missing(String),
    /// A file is there but it is not a picture Steam can show (wrong type, empty, damaged).
    NotAnImage(String),
    /// Stored inside quote marks. `path` is the text without them; `exists` says whether a picture is at that path.
    Quoted { path: String, exists: bool },
    /// A picture outside the port's folder and outside Steam's grid folder. Informational: left as it is.
    Outside(String),
}

impl IconState {
    /// The icon needs replacing.
    pub fn needs_fix(&self) -> bool {
        matches!(self, IconState::Empty | IconState::Missing(_) | IconState::NotAnImage(_) | IconState::Quoted { .. })
    }

    /// One short sentence about the icon, for lists and the log.
    pub fn wording(&self) -> String {
        match self {
            IconState::Ok => "icon is fine".to_string(),
            IconState::Empty => "no icon is set".to_string(),
            IconState::Missing(p) => format!("icon file missing ({p})"),
            IconState::NotAnImage(p) => format!("icon file is not a picture Steam can show ({p})"),
            IconState::Quoted { path, exists: true } => format!("icon path is stored in quote marks ({path}); the file is there"),
            IconState::Quoted { path, exists: false } => format!("icon path is stored in quote marks and the file is missing ({path})"),
            IconState::Outside(p) => format!("icon is outside the port folder ({p}); left as it is"),
        }
    }
}

/// One Steam shortcut that belongs to a port, with what its icon is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconFinding {
    pub port_key: String,
    pub port_name: String,
    pub port_folder: String,
    pub account_id: u32,
    pub account_label: String,
    pub shortcuts_file: PathBuf,
    pub shortcut_name: String,
    pub app_id: Option<u32>,
    /// The `icon` field as stored (quote marks and all).
    pub stored: String,
    pub state: IconState,
}

impl IconFinding {
    /// "Twilight Princess: icon file missing (/run/media/x/tp.png)".
    pub fn describe(&self) -> String {
        format!("{}: {}", self.port_name, self.state.wording())
    }
}

/// The port whose folder holds `exe` (as written, or as the file system names it); the longest folder wins.
fn owner_of<'a>(exe: &Path, dirs: &'a [(usize, PathBuf)]) -> Option<usize> {
    let canonical = fs::canonicalize(exe).ok();
    dirs.iter()
        .filter(|(_, d)| exe.starts_with(d) || canonical.as_ref().is_some_and(|c| c.starts_with(d)))
        .max_by_key(|(_, d)| d.as_os_str().len())
        .map(|(i, _)| *i)
}

fn inside(path: &Path, dir: &Path) -> bool {
    moved_path(&path.to_string_lossy(), dir, dir).is_some()
}

/// The state of one stored icon value, for a shortcut of the port whose folder is `port_dir`, in an account whose grid
/// folder is `grid_dir`.
pub fn icon_state(stored: &str, port_dir: &Path, grid_dir: &Path) -> IconState {
    let plain = shortcuts::unquote(stored);
    if plain.is_empty() {
        return IconState::Empty;
    }
    let path = Path::new(plain);
    if plain.len() != stored.len() {
        return IconState::Quoted { path: plain.to_string(), exists: is_showable_image(path) };
    }
    if !path.exists() {
        return IconState::Missing(plain.to_string());
    }
    if !is_showable_image(path) {
        return IconState::NotAnImage(plain.to_string());
    }
    if inside(path, port_dir) || inside(path, grid_dir) {
        IconState::Ok
    } else {
        IconState::Outside(plain.to_string())
    }
}

/// Every Steam shortcut (all accounts) that belongs to one of `ports`, with its icon state. `ports` are the installed
/// ones. An unreadable shortcuts file is an error, so a caller never takes "could not tell" for "nothing wrong".
pub fn diagnose(home: &Path, library_dir: &Path, ports: &[Port]) -> Result<Vec<IconFinding>, String> {
    let dirs: Vec<(usize, PathBuf)> = ports.iter().enumerate().map(|(i, p)| (i, library_dir.join(&p.folder))).collect();
    let mut out = Vec::new();
    for account in device_accounts(home) {
        if !account.shortcuts.exists() {
            continue;
        }
        let file = ShortcutsFile::load(&account.shortcuts).map_err(|e| format!("Could not read the Steam shortcuts of {}: {e}", account.label))?;
        let grid = account.shortcuts.parent().map(|c| c.join("grid")).unwrap_or_default();
        for s in shortcuts::list(&file.doc) {
            let Some(i) = owner_of(Path::new(s.exe_path()), &dirs) else { continue };
            let state = icon_state(&s.icon, &dirs[i].1, &grid);
            let finding = IconFinding {
                port_key: ports[i].key().to_string(),
                port_name: ports[i].display_name().to_string(),
                port_folder: ports[i].folder.clone(),
                account_id: account.account_id,
                account_label: account.label.clone(),
                shortcuts_file: account.shortcuts.clone(),
                shortcut_name: s.app_name.clone(),
                app_id: s.app_id,
                stored: s.icon.clone(),
                state,
            };
            eprintln!(
                "steam icons: {} / shortcut \"{}\" (app id {}, account {}): {}; stored {:?}",
                finding.port_name,
                finding.shortcut_name,
                finding.app_id.map_or("none".to_string(), |id| id.to_string()),
                finding.account_id,
                finding.state.wording(),
                finding.stored
            );
            out.push(finding);
        }
    }
    Ok(out)
}

/// Where a replacement icon comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The stored path without its quote marks (the file is there).
    Unquoted,
    /// The icon this app ID had in the backup of that name.
    Backup(String),
    /// The icon the backup of that name had, moved from an old folder of the port into its current one.
    BackupMoved(String),
    /// A file already in the account's `config/grid/`.
    Grid,
    /// The port's catalog icon, to be written into `config/grid/`.
    Catalog,
}

impl Source {
    fn wording(&self) -> String {
        match self {
            Source::Unquoted => "use the same file without the quote marks".to_string(),
            Source::Backup(_) => "use the icon from before the problem (from a Steam shortcuts backup)".to_string(),
            Source::BackupMoved(_) => "use the icon from before the move (found in the port's new folder)".to_string(),
            Source::Grid => "use the icon already in Steam's grid folder".to_string(),
            Source::Catalog => "use the catalog icon (copied into Steam's grid folder)".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Replacement {
    /// Point the shortcut at this existing file.
    Use { path: PathBuf, source: Source },
    /// Write this cached catalog icon into the grid folder, then point the shortcut at it.
    WriteCatalog { cached: PathBuf },
    /// Nothing to use, and why.
    Nothing(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanItem {
    pub finding: IconFinding,
    pub replacement: Replacement,
}

impl PlanItem {
    pub fn can_fix(&self) -> bool {
        !matches!(self.replacement, Replacement::Nothing(_))
    }

    /// "Twilight Princess: icon file missing (/run/media/x/tp.png); will use the icon from before the move".
    pub fn line(&self) -> String {
        let action = match &self.replacement {
            Replacement::Use { source, .. } => format!("will {}", source.wording()),
            Replacement::WriteCatalog { .. } => format!("will {}", Source::Catalog.wording()),
            Replacement::Nothing(why) => format!("cannot be fixed: {why}"),
        };
        format!("{}; {action}", self.finding.describe())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub items: Vec<PlanItem>,
}

impl Plan {
    pub fn fixable(&self) -> usize {
        self.items.iter().filter(|i| i.can_fix()).count()
    }
}

/// What [`plan`] reads besides the findings.
pub struct PlanInputs<'a> {
    pub config_dir: &'a Path,
    /// The app's image cache (`<folder>_icon.png` is the cached catalog icon of a port).
    pub cache_dir: &'a Path,
    pub library_dir: &'a Path,
}

/// The `icon` of the shortcut with `app_id` in `doc`, as a plain path (empty when none).
fn icon_of(doc: &Document, app_id: u32) -> Option<String> {
    shortcuts::list(doc).into_iter().find(|s| s.app_id == Some(app_id)).map(|s| s.icon_path().to_string())
}

fn from_backups(f: &IconFinding, current: &str, old_folders: &[PathBuf], port_dir: &Path) -> Option<Replacement> {
    let backups = backups::list_backups(&f.shortcuts_file).ok()?;
    for backup in backups.into_iter().take(MAX_BACKUPS) {
        let Ok(bytes) = fs::read(&backup.path) else { continue };
        let Ok(doc) = Document::parse(&bytes) else { continue };
        let Some(candidate) = f.app_id.and_then(|id| icon_of(&doc, id)).filter(|c| !c.is_empty() && c != current) else { continue };
        let name = backup.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        // Moved into the port's current folder first: the old folder's file is not where the port lives now.
        for old in old_folders {
            if let Some(moved) = moved_path(&candidate, old, port_dir).map(PathBuf::from).filter(|m| is_showable_image(m)) {
                return Some(Replacement::Use { path: moved, source: Source::BackupMoved(name) });
            }
        }
        let path = PathBuf::from(&candidate);
        // A file inside an old copy of the port is not a good home for an icon: that copy is to be removed.
        let in_old_copy = old_folders.iter().any(|old| inside(&path, old));
        if !in_old_copy && is_showable_image(&path) {
            return Some(Replacement::Use { path, source: Source::Backup(name) });
        }
    }
    None
}

fn replacement_for(f: &IconFinding, inputs: &PlanInputs, port: Option<&Port>, old_folders: &[PathBuf]) -> Replacement {
    let Some(app_id) = f.app_id else {
        return Replacement::Nothing("Steam has not stored an app ID for this shortcut yet (open Steam once, quit it, then try again)".into());
    };
    if let IconState::Quoted { path, exists: true } = &f.state {
        return Replacement::Use { path: PathBuf::from(path), source: Source::Unquoted };
    }
    let port_dir = inputs.library_dir.join(&f.port_folder);
    let current = shortcuts::unquote(&f.stored).to_string();
    if let Some(found) = from_backups(f, &current, old_folders, &port_dir) {
        return found;
    }
    let grid = f.shortcuts_file.parent().map(|c| c.join("grid")).unwrap_or_default();
    let in_grid = artwork::existing_artwork(&grid, app_id, ArtworkKind::Icon).unwrap_or_default();
    if let Some(path) = in_grid.iter().find(|p| is_showable_image(p) && p.to_string_lossy() != current) {
        return Replacement::Use { path: path.clone(), source: Source::Grid };
    }
    if !in_grid.is_empty() {
        return Replacement::Nothing("a file you put in Steam's grid folder for this shortcut cannot be shown as an icon; it was left alone".into());
    }
    let cached = crate::core::image_cache::cached_image_path(inputs.cache_dir, &format!("{}_icon", f.port_folder)).ok().filter(|p| is_showable_image(p));
    match (cached, port.and_then(|p| p.icon.as_ref())) {
        (Some(cached), _) => Replacement::WriteCatalog { cached },
        (None, Some(_)) => Replacement::Nothing("the catalog icon has not been downloaded yet".into()),
        (None, None) => Replacement::Nothing("no earlier icon was found and the catalog has none for this port".into()),
    }
}

/// What to do about each finding that needs a new icon. Writes nothing.
pub fn plan(inputs: &PlanInputs, findings: &[IconFinding], ports: &[Port]) -> Plan {
    let records = old_copies::load(inputs.config_dir).unwrap_or_default();
    let mut items = Vec::new();
    for f in findings.iter().filter(|f| f.state.needs_fix()) {
        let old: Vec<PathBuf> = records.iter().filter(|r| r.port_key == f.port_key).map(|r| r.old_path.clone()).collect();
        let port = ports.iter().find(|p| p.key() == f.port_key);
        let replacement = replacement_for(f, inputs, port, &old);
        let item = PlanItem { finding: f.clone(), replacement };
        eprintln!("steam icons: plan for {} / app id {:?}: {}", f.port_name, f.app_id, item.line());
        items.push(item);
    }
    Plan { items }
}

/// One change to a shortcut's icon, for a given account's file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconFix {
    pub account_id: u32,
    pub account_label: String,
    pub port_key: String,
    pub name: String,
    pub app_id: u32,
    /// The `icon` text the shortcut had when the problem was found; the change is made only while it still does.
    pub from: String,
    /// The plain path to store.
    pub to: String,
}

/// What [`resolve`] did with the plan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolved {
    pub fixes: Vec<IconFix>,
    /// (port name, why it was left out): a catalog icon that could not be placed, an item with nothing to use.
    pub left: Vec<(String, String)>,
}

/// Turns the plan into changes to make: places catalog icons in the grid folder (never over art the user chose) and
/// names the file each shortcut will point at. Steam does not need to be closed for this.
pub fn resolve(plan: &Plan) -> Resolved {
    let mut out = Resolved::default();
    for item in &plan.items {
        let f = &item.finding;
        let Some(app_id) = f.app_id else {
            out.left.push((f.port_name.clone(), "no stored app ID".into()));
            continue;
        };
        let path = match &item.replacement {
            Replacement::Use { path, .. } => path.clone(),
            Replacement::Nothing(why) => {
                out.left.push((f.port_name.clone(), why.clone()));
                continue;
            }
            Replacement::WriteCatalog { cached } => {
                let grid = f.shortcuts_file.parent().map(|c| c.join("grid")).unwrap_or_default();
                let art = catalog_art_from_files(None, Some(cached));
                if art.set.icon.is_none() {
                    out.left.push((f.port_name.clone(), "the catalog icon could not be read".into()));
                    continue;
                }
                match write_art(&grid, app_id, &art.set, &WriteOptions { refresh_own: false }) {
                    Ok(report) => match report.outcome(Slot::Icon) {
                        Some(o) if o.our_path().is_some() => o.our_path().unwrap().to_path_buf(),
                        Some(SlotOutcome::SkippedUserArt(p)) => {
                            out.left.push((f.port_name.clone(), format!("{} is already in Steam's grid folder and was left alone", p.display())));
                            continue;
                        }
                        other => {
                            out.left.push((f.port_name.clone(), format!("the icon could not be placed: {other:?}")));
                            continue;
                        }
                    },
                    Err(e) => {
                        out.left.push((f.port_name.clone(), format!("the icon could not be placed: {e}")));
                        continue;
                    }
                }
            }
        };
        let Some(to) = path.to_str().map(str::to_string) else {
            out.left.push((f.port_name.clone(), "its path is not plain text".into()));
            continue;
        };
        out.fixes.push(IconFix { account_id: f.account_id, account_label: f.account_label.clone(), port_key: f.port_key.clone(), name: f.shortcut_name.clone(), app_id, from: f.stored.clone(), to });
    }
    out
}

/// What [`write_fixes`] did to one account's file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FixResult {
    /// (shortcut name, app ID, icon before, icon now)
    pub changed: Vec<(String, u32, String, String)>,
    /// (shortcut name, why it was left alone)
    pub skipped: Vec<(String, String)>,
}

/// Changes the `icon` of the listed shortcuts in the document. Pure: nothing is written.
pub fn fix_doc(doc: &mut Document, fixes: &[&IconFix]) -> FixResult {
    let mut result = FixResult::default();
    for fix in fixes {
        let Some(now) = shortcuts::list(doc).into_iter().find(|s| s.app_id == Some(fix.app_id)) else {
            result.skipped.push((fix.name.clone(), "no longer in Steam".into()));
            continue;
        };
        if now.icon == fix.to {
            continue;
        }
        if now.icon != fix.from {
            result.skipped.push((fix.name.clone(), "its icon changed since the problem was found".into()));
            continue;
        }
        let change = ShortcutUpdate { icon: Some(fix.to.clone()), ..Default::default() };
        match shortcuts::update(doc, Target::AppId(fix.app_id), &change) {
            Ok(_) => result.changed.push((now.app_name.clone(), fix.app_id, now.icon.clone(), fix.to.clone())),
            Err(e) => result.skipped.push((fix.name.clone(), e.to_string())),
        }
    }
    result
}

/// Loads one account's file, fixes the icons and saves it (refused while Steam runs; backup first; old backups pruned).
pub fn write_fixes(path: &Path, fixes: &[&IconFix], steam: &dyn SteamProcessCheck, settings: &ForkSettings, now: SystemTime) -> Result<(FixResult, Option<PathBuf>), WriteError> {
    check_steam(steam).map_err(|e| WriteError::SteamUp(e.to_string()))?;
    let mut file = ShortcutsFile::load(path).map_err(|e| WriteError::Failed(e.to_string()))?;
    let result = fix_doc(&mut file.doc, fixes);
    if result.changed.is_empty() {
        return Ok((result, None));
    }
    match file.save(steam, &backup_stamp()) {
        Ok(report) => {
            if report.wrote {
                let _ = backups::prune_backups(path, settings.backups_keep(), now);
                for (name, app_id, before, after) in &result.changed {
                    eprintln!("steam icons: fixed \"{name}\" (app id {app_id}) in {}: icon {before:?} -> {after:?}", path.display());
                }
            }
            Ok((result, report.backup))
        }
        Err(e @ (StoreError::SteamRunning(_) | StoreError::SteamCheckFailed(_))) => Err(WriteError::SteamUp(e.to_string())),
        Err(e) => Err(WriteError::Failed(e.to_string())),
    }
}

/// The accounts the fixes belong to, each with its file, in the order first seen.
pub fn by_account<'a>(fixes: &'a [IconFix], accounts: &[AccountFile]) -> Vec<(AccountFile, Vec<&'a IconFix>)> {
    let mut out: Vec<(AccountFile, Vec<&IconFix>)> = Vec::new();
    for fix in fixes {
        let Some(account) = accounts.iter().find(|a| a.account_id == fix.account_id) else { continue };
        match out.iter_mut().find(|(a, _)| a.account_id == fix.account_id) {
            Some((_, list)) => list.push(fix),
            None => out.push((account.clone(), vec![fix])),
        }
    }
    out
}

/// The fixes as Steam queue entries (applied later by "Close Steam and update shortcuts").
pub fn queue_entries(fixes: &[IconFix], game_mode: bool) -> Vec<QueuedChange> {
    fixes
        .iter()
        .map(|f| QueuedChange {
            account_id: f.account_id,
            account_label: f.account_label.clone(),
            port_key: f.port_key.clone(),
            name: f.name.clone(),
            kind: QueuedKind::Icon,
            repoint_by_name: false,
            art: ArtChoice::None,
            from_game_mode: game_mode,
            queued_at: backup_stamp(),
            repoint: None,
            icon: Some(IconChange { app_id: f.app_id, from: f.from.clone(), to: f.to.clone() }),
        })
        .collect()
}

/// What [`apply`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyReport {
    /// Written now.
    pub written: Vec<(String, u32, String, String)>,
    /// Queued until Steam is closed (account labels).
    pub queued: Vec<IconFix>,
    pub skipped: Vec<(String, String)>,
    pub failed: Vec<String>,
}

/// Writes the fixes now when Steam is closed (outside Game Mode); otherwise returns them in `queued` for the caller
/// to put in the Steam queue. A write that is refused because Steam started meanwhile also ends up in `queued`.
pub fn apply(fixes: &[IconFix], home: &Path, steam: &dyn SteamProcessCheck, settings: &ForkSettings, game_mode: bool, now: SystemTime) -> ApplyReport {
    let mut report = ApplyReport::default();
    let accounts = device_accounts(home);
    for (account, list) in by_account(fixes, &accounts) {
        if game_mode || check_steam(steam).is_err() {
            report.queued.extend(list.into_iter().cloned());
            continue;
        }
        match write_fixes(&account.shortcuts, &list, steam, settings, now) {
            Ok((result, _)) => {
                report.written.extend(result.changed);
                report.skipped.extend(result.skipped);
            }
            Err(WriteError::SteamUp(_)) => report.queued.extend(list.into_iter().cloned()),
            Err(WriteError::Failed(e)) => report.failed.push(format!("could not update the shortcuts of {}: {e}", account.label)),
        }
    }
    report
}

pub mod pages;

#[cfg(test)]
mod tests;
