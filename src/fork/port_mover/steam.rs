//! Pointing Steam's non-Steam shortcuts at a port's new folder.
//!
//! Every shortcut (in every Steam account on the device) whose program, start folder or icon is inside
//! the old folder gets the same path under the new folder. Its app ID is kept, so artwork, play time
//! and controller layouts stay attached. A shortcut with no stored app ID is left alone (Steam would
//! derive a new ID from the new path, losing its artwork); opening and quitting Steam once stores it.
//!
//! Written through the safe path (`store::ShortcutsFile::save`: refused while Steam runs, verified
//! backup, atomic write, read back). While Steam runs (always in Game Mode) the change is queued in
//! `steam-queue.json` as a "repoint" entry and applied later like any queued change.

use crate::fork::artwork;
use crate::fork::backups;
use crate::fork::settings::ForkSettings;
use crate::fork::shortcuts::{self, ShortcutUpdate, Target};
use crate::fork::steam_queue::{self, QueuedChange, QueuedKind};
use crate::fork::steam_user;
use crate::fork::store::{backup_stamp, check_steam, ShortcutsFile, SteamProcessCheck, StoreError};
use crate::fork::vdf::Document;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// `path` moved from `from` to `to`, when it is `from` or inside it (whole path components only). Tried as
/// written first, then through links: a shortcut that names the folder by another route (an SD card
/// reached through a symlink, or the real path while `from` is the alias) counts as inside it too.
pub fn moved_path(path: &str, from: &Path, to: &Path) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    let p = Path::new(path);
    let rest: PathBuf = match p.strip_prefix(from) {
        Ok(rest) => rest.to_path_buf(),
        Err(_) => {
            let real_from = std::fs::canonicalize(from).ok()?;
            match p.strip_prefix(&real_from) {
                Ok(rest) => rest.to_path_buf(),
                Err(_) => std::fs::canonicalize(p).ok()?.strip_prefix(&real_from).ok()?.to_path_buf(),
            }
        }
    };
    let new = if rest.as_os_str().is_empty() { to.to_path_buf() } else { to.join(rest) };
    Some(new.to_string_lossy().into_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repointed {
    pub name: String,
    pub app_id: u32,
    pub fields: Vec<&'static str>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepointResult {
    pub changed: Vec<Repointed>,
    /// (shortcut name, why it was left alone)
    pub skipped: Vec<(String, String)>,
}

pub const NO_APP_ID: &str = "Steam has not finished setting this shortcut up yet; open Steam once, quit it, then try again, or pick the new program in Info > Favorite executable";

/// Rewrites, in `doc`, every shortcut that points inside `from`. Pure: nothing is written.
pub fn repoint_doc(doc: &mut Document, from: &Path, to: &Path) -> RepointResult {
    let mut result = RepointResult::default();
    for s in shortcuts::list(doc) {
        let exe = moved_path(s.exe_path(), from, to);
        let start_dir = moved_path(s.start_dir_path(), from, to);
        let icon = moved_path(&s.icon, from, to);
        if exe.is_none() && start_dir.is_none() && icon.is_none() {
            continue;
        }
        if s.app_id.is_none() {
            result.skipped.push((s.app_name.clone(), NO_APP_ID.into()));
            continue;
        };
        let mut fields = Vec::new();
        if exe.is_some() {
            fields.push("program");
        }
        if start_dir.is_some() {
            fields.push("start folder");
        }
        if icon.is_some() {
            fields.push("icon");
        }
        let change = ShortcutUpdate { exe, start_dir, launch_options: None, icon, derive_missing_app_id: false };
        match shortcuts::update(doc, Target::Key(&s.key), &change) {
            Ok(updated) => result.changed.push(Repointed { name: s.app_name.clone(), app_id: updated.app_id, fields }),
            Err(e) => result.skipped.push((s.app_name.clone(), e.to_string())),
        }
    }
    result
}

/// One Steam account's shortcuts file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountFile {
    pub account_id: u32,
    pub label: String,
    pub shortcuts: PathBuf,
}

/// Every Steam account on the device (all Steam folders, each file once).
pub fn device_accounts(home: &Path) -> Vec<AccountFile> {
    let mut out: Vec<AccountFile> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    for root in artwork::existing_steam_roots(home) {
        let Ok(accounts) = steam_user::discover_accounts(&root) else { continue };
        for a in accounts {
            let path = artwork::shortcuts_path(&a.dir);
            let real = std::fs::canonicalize(&a.dir).unwrap_or_else(|_| a.dir.clone());
            if seen.contains(&real) {
                continue;
            }
            seen.push(real);
            out.push(AccountFile { account_id: a.account_id, label: a.label(), shortcuts: path });
        }
    }
    out
}

/// Names of shortcuts (any account) that still point inside `dir`. An unreadable file is an error, so
/// a caller never takes "could not tell" for "none".
pub fn shortcuts_pointing_into(home: &Path, dir: &Path) -> Result<Vec<String>, String> {
    Ok(shortcuts_pointing_into_detailed(home, dir)?.into_iter().map(|(name, _)| name).collect())
}

/// As [`shortcuts_pointing_into`], with whether each shortcut has a stored app ID (one without is never
/// repointed: Steam has not finished setting it up).
pub fn shortcuts_pointing_into_detailed(home: &Path, dir: &Path) -> Result<Vec<(String, bool)>, String> {
    let mut names = Vec::new();
    for account in device_accounts(home) {
        if !account.shortcuts.exists() {
            continue;
        }
        let file = ShortcutsFile::load(&account.shortcuts).map_err(|e| format!("Could not read Steam shortcuts of {}: {e}", account.label))?;
        for s in shortcuts::list(&file.doc) {
            let inside = |p: &str| moved_path(p, dir, dir).is_some();
            // Launch options are not rewritten by a move; a path in them still needs the old copy.
            let in_options = dir.to_str().is_some_and(|d| !d.is_empty() && s.launch_options.contains(d));
            if inside(s.exe_path()) || inside(s.start_dir_path()) || inside(&s.icon) || in_options {
                names.push((s.app_name.clone(), s.app_id.is_some()));
            }
        }
    }
    Ok(names)
}

#[derive(Debug)]
pub enum WriteError {
    /// Steam is running (or could not be checked): nothing was written; queue it instead.
    SteamUp(String),
    Failed(String),
}

/// Loads one account's file, repoints and saves it (backup first, old backups pruned). Ok with an
/// empty result when nothing pointed into `from`.
pub fn write_repoint(path: &Path, from: &Path, to: &Path, steam: &dyn SteamProcessCheck, settings: &ForkSettings, now: SystemTime) -> Result<(RepointResult, Option<PathBuf>), WriteError> {
    match check_steam(steam) {
        Ok(()) => {}
        Err(e) => return Err(WriteError::SteamUp(e.to_string())),
    }
    if !path.exists() {
        return Ok((RepointResult::default(), None));
    }
    let mut file = ShortcutsFile::load(path).map_err(|e| WriteError::Failed(e.to_string()))?;
    let result = repoint_doc(&mut file.doc, from, to);
    if result.changed.is_empty() {
        return Ok((result, None));
    }
    match file.save(steam, &backup_stamp()) {
        Ok(report) => {
            if report.wrote {
                let _ = backups::prune_backups(path, settings.backups_keep(), now);
            }
            Ok((result, report.backup))
        }
        Err(e @ (StoreError::SteamRunning(_) | StoreError::SteamCheckFailed(_))) => Err(WriteError::SteamUp(e.to_string())),
        Err(e) => Err(WriteError::Failed(e.to_string())),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SteamReport {
    /// (account, shortcuts changed)
    pub written: Vec<(String, Vec<Repointed>)>,
    /// Accounts whose change was queued until Steam is closed.
    pub queued: Vec<String>,
    pub skipped: Vec<(String, String)>,
    pub failed: Vec<String>,
}

impl SteamReport {
    pub fn changed_count(&self) -> usize {
        self.written.iter().map(|(_, c)| c.len()).sum()
    }

    /// The lines for the move summary of `port_name`.
    pub fn text_for(&self, port_name: &str) -> String {
        let mut lines = Vec::new();
        let n = self.changed_count();
        if n > 0 {
            lines.push(format!("Steam: {n} shortcut{} now point{} at the new folder (artwork and play time kept).", if n == 1 { "" } else { "s" }, if n == 1 { "s" } else { "" }));
        }
        if !self.queued.is_empty() {
            lines.push(format!(
                "Steam: the shortcut change is queued because Steam is running. Until it is applied (next time this app runs in Desktop Mode with Steam closed), starting {port_name} from Steam still runs the OLD copy, and anything you save there stays in the old copy."
            ));
        }
        for (name, why) in &self.skipped {
            lines.push(format!("Steam: {name} was left alone: {why}."));
        }
        for f in &self.failed {
            lines.push(format!("Steam: {f}"));
        }
        lines.join("\n")
    }
}

/// What repointing needs from the app.
pub struct SteamCtx<'a> {
    pub home: &'a Path,
    pub config_dir: &'a Path,
    pub settings: &'a ForkSettings,
    pub steam: &'a dyn SteamProcessCheck,
    /// Under gamescope: never write, always queue.
    pub game_mode: bool,
    pub now: SystemTime,
}

/// Repoints every account's shortcuts from `from` to `to`: written now when Steam is closed, else
/// queued per account that has a shortcut to change.
pub fn repoint_everywhere(port_key: &str, port_name: &str, from: &Path, to: &Path, ctx: &SteamCtx) -> SteamReport {
    let mut report = SteamReport::default();
    for account in device_accounts(ctx.home) {
        let can_write = !ctx.game_mode && check_steam(ctx.steam).is_ok();
        let outcome = if can_write { write_repoint(&account.shortcuts, from, to, ctx.steam, ctx.settings, ctx.now) } else { Err(WriteError::SteamUp(String::new())) };
        match outcome {
            Ok((result, _)) => {
                report.skipped.extend(result.skipped);
                if !result.changed.is_empty() {
                    report.written.push((account.label.clone(), result.changed));
                }
            }
            Err(WriteError::SteamUp(_)) => {
                // Only queue for an account that has something to change.
                let needed = match ShortcutsFile::load(&account.shortcuts) {
                    Ok(mut file) => {
                        let r = repoint_doc(&mut file.doc, from, to);
                        report.skipped.extend(r.skipped);
                        !r.changed.is_empty()
                    }
                    Err(_) => account.shortcuts.exists(),
                };
                if !needed {
                    continue;
                }
                let change = QueuedChange {
                    account_id: account.account_id,
                    account_label: account.label.clone(),
                    port_key: port_key.to_string(),
                    name: port_name.to_string(),
                    kind: QueuedKind::Repoint,
                    repoint_by_name: false,
                    art: steam_queue::ArtChoice::None,
                    from_game_mode: ctx.game_mode,
                    queued_at: backup_stamp(),
                    repoint: Some((from.to_string_lossy().into_owned(), to.to_string_lossy().into_owned())),
                };
                match steam_queue::queue_changes(&steam_queue::queue_path(ctx.config_dir), vec![change], ctx.game_mode) {
                    Ok(_) => report.queued.push(account.label.clone()),
                    Err(e) => report.failed.push(format!("could not queue the change for {}: {e}", account.label)),
                }
            }
            Err(WriteError::Failed(e)) => report.failed.push(format!("could not update the shortcuts of {}: {e}", account.label)),
        }
    }
    report
}
