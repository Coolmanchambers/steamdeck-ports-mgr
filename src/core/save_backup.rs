use super::installer::{run_7z, sevenzip_exe_path};
use super::models::Port;
use super::path_safety::{is_within, safe_join};
use super::platform_resolve::{resolve_save_folder, save_location, SaveLocation};
use serde_json::Value;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const PENDING_RESTORE_DIR: &str = "Pending Restore";
const GLOBAL_BACKUPS_DIR: &str = "Global Backups";
const UPDATE_BACKUPS_DIR: &str = "Update Backups";

fn save_fields(port: &Port) -> [(Option<&Value>, &'static str); 2] {
    [(port.save.as_ref(), "save_folder"), (port.save2.as_ref(), "save_folder2")]
}

fn copy_non_empty(src: &Path, dst: &Path) -> io::Result<bool> {
    let mut copied_any = false;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copied_any |= copy_non_empty(&entry.path(), &dst.join(entry.file_name()))?;
        } else if file_type.is_file() {
            fs::create_dir_all(dst)?;
            fs::copy(entry.path(), dst.join(entry.file_name()))?;
            copied_any = true;
        }
    }
    Ok(copied_any)
}

fn copy_save(src: &Path, dst: &Path) -> io::Result<bool> {
    if !src.is_file() {
        return copy_non_empty(src, dst);
    }
    let name = src.file_name().ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    fs::create_dir_all(dst)?;
    fs::copy(src, dst.join(name))?;
    Ok(true)
}

pub(crate) fn pending_restore_dir(saves_backup_dir: &Path, folder_name: &str, field: &str) -> PathBuf {
    saves_backup_dir.join(PENDING_RESTORE_DIR).join(folder_name).join(field)
}

// Fork (data safety, F-D1): a save folder inside the game folder that leads, through a link, to a folder far too broad to
// be a save folder (the home folder, all of Steam: `save_restore::folder_problem`) is not copied: the uninstall removes
// only the link, and its target stays where it is. Copying it would fill the drive and then cancel the uninstall.
fn leads_somewhere_too_broad(src: &Path, game_dir: &Path, saves_backup_dir: &Path) -> Option<String> {
    let (Ok(real_src), Ok(real_game)) = (fs::canonicalize(src), fs::canonicalize(game_dir)) else { return None };
    if is_within(&real_src, &real_game) {
        return None;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).map(|h| fs::canonicalize(&h).unwrap_or(h));
    crate::fork::save_restore::folder_problem(&real_src, &real_game, real_game.parent(), saves_backup_dir, home.as_deref()).map(|why| format!("{} leads to {}, which {why}", src.display(), real_src.display()))
}

#[must_use]
fn preserve_before_uninstall(save_folder: Option<&Value>, saves_backup_dir: &Path, dest_dir: &Path, folder_name: &str, field: &str) -> bool {
    let Some(src) = save_folder.and_then(|v| resolve_save_folder(v, dest_dir)) else { return true };
    if !is_within(&src, dest_dir) || !src.exists() {
        return true;
    }
    if let Some(why) = leads_somewhere_too_broad(&src, dest_dir, saves_backup_dir) {
        eprintln!("uninstall: not copying the save folder of {folder_name}: {why}; only the link inside the game folder goes");
        return true;
    }
    let dst = pending_restore_dir(saves_backup_dir, folder_name, field);
    if !is_within(&dst, saves_backup_dir) {
        return false;
    }
    // Fork (data safety, F-W1): a copy kept from an earlier uninstall (the fork keeps it when a restore fails) may be the
    // only copy of those saves: it is renamed `<field>.<time>`, never deleted.
    if fs::symlink_metadata(&dst).is_ok() {
        let aside = crate::fork::uninstall_keep::unused_stamp_dir(dst.parent().unwrap_or(saves_backup_dir), &format!("{field}.{}", crate::fork::update_merge::stamp()));
        if let Err(e) = fs::rename(&dst, &aside) {
            eprintln!("uninstall: could not set aside the earlier kept saves {} ({e})", dst.display());
            return false;
        }
    }
    copy_save(&src, &dst).is_ok()
}

/// True when `dir` holds anything but empty folders.
fn holds_anything(dir: &Path) -> bool {
    fs::read_dir(dir).map_or(true, |entries| {
        entries.into_iter().any(|e| match e {
            Ok(e) => !e.file_type().is_ok_and(|t| t.is_dir()) || holds_anything(&e.path()),
            Err(_) => true,
        })
    })
}

// Fork (data safety, D6): before a kept copy goes back, what is at the save's place now (the live saves, or files the
// release put there) is moved to `<Saves Backup>/Replaced by restore/<folder>/<time>/<field>`, never deleted. A folder
// with nothing but empty folders in it is just removed. `Err` when it could not be moved: then nothing is restored.
// `Ok(Some(folder))`: the time-named folder that now holds what was there (review S3: the Done page says so).
fn move_live_aside(dst: &Path, saves_backup_dir: &Path, folder_name: &str, field: &str) -> Result<Option<PathBuf>, ()> {
    let Ok(meta) = fs::symlink_metadata(dst) else { return Ok(None) };
    if meta.is_dir() && !holds_anything(dst) {
        return fs::remove_dir_all(dst).map(|()| None).map_err(|_| ());
    }
    let base = crate::fork::uninstall_keep::replaced_dir_for(saves_backup_dir, folder_name);
    if !is_within(&base, saves_backup_dir) {
        return Err(());
    }
    let stamp_dir = crate::fork::uninstall_keep::unused_stamp_dir(&base, &crate::fork::update_merge::stamp());
    let to = if meta.is_dir() { stamp_dir.join(field) } else { stamp_dir.join(field).join(dst.file_name().unwrap_or_default()) };
    match crate::fork::uninstall_keep::move_path(dst, &to) {
        Ok(()) => {
            eprintln!("save restore: moved what was at {} to {} before putting the kept saves back", dst.display(), to.display());
            Ok(Some(stamp_dir))
        }
        Err(e) => {
            eprintln!("save restore: could not move {} aside ({e}); the kept saves stay kept", dst.display());
            Err(())
        }
    }
}

// Fork (data safety): `Some((folder, restored))` when saves in the way were moved to `folder`; `restored` says whether
// the kept copy then went back.
fn restore_after_install(save_folder: Option<&Value>, saves_backup_dir: &Path, dest_dir: &Path, folder_name: &str, field: &str) -> Option<(PathBuf, bool)> {
    let src = pending_restore_dir(saves_backup_dir, folder_name, field);
    if !is_within(&src, saves_backup_dir) || !src.exists() {
        return None;
    }
    // Fork: with no folder to restore into (no save field, or no known Linux location) the kept copy stays;
    // upstream deleted it here, losing the only copy of the saves.
    let Some(dst) = save_folder.and_then(|v| resolve_save_folder(v, dest_dir)) else {
        eprintln!("save restore: no save folder for {field} of {folder_name} here; kept {}", src.display());
        return None;
    };
    if !is_within(&dst, dest_dir) {
        return None;
    }
    // Fork (data safety, D6): a "save folder" that is the game folder itself ("save": ".") is never replaced: that would
    // replace the program just installed. The kept copy stays in Pending Restore.
    if is_within(dest_dir, &dst) {
        eprintln!("save restore: the save folder of {folder_name} is the game folder itself; kept {}", src.display());
        return None;
    }
    // Upstream (2026-10-30): a save that is a single file (`game.sav`) was kept as `<pending>/<name>` and goes
    // back to its place without touching the rest of the game folder. Fork: only when that file is all the kept
    // copy holds; a kept folder that happens to contain a file of the folder's name goes back whole (never a part).
    let only_entry = fs::read_dir(&src).map(|d| d.count() == 1).unwrap_or(false);
    let saved_file = dst.file_name().map(|name| src.join(name)).filter(|f| only_entry && f.is_file() && !dst.is_dir());
    // Fork (data safety, D6): what is at the place now is moved aside first, never deleted (upstream removed it).
    let Ok(aside) = move_live_aside(&dst, saves_backup_dir, folder_name, field) else { return None };
    let restored = match saved_file {
        Some(saved_file) => dst.parent().is_some_and(|parent| fs::create_dir_all(parent).is_ok()) && fs::copy(&saved_file, &dst).is_ok(),
        None => copy_non_empty(&src, &dst).is_ok(),
    };
    if !restored {
        // Recheck 1: the saves that were in the way moved, but the kept copy did not go back (it stays in Pending
        // Restore); the caller must not say it was put back.
        return aside.map(|dir| (dir, false));
    }
    let _ = fs::remove_dir_all(&src);
    aside.map(|dir| (dir, true))
}

#[must_use]
pub fn preserve_all_before_uninstall(port: &Port, saves_backup_dir: &Path, dest_dir: &Path) -> bool {
    let results = save_fields(port).map(|(save_folder, field)| preserve_before_uninstall(save_folder, saves_backup_dir, dest_dir, &port.folder, field));
    results.iter().all(|&ok| ok)
}

// Fork (data safety, review S3): returns the folder the saves that were in the way went to, when any did, and whether
// every kept copy then went back (recheck 1: false when one stayed in Pending Restore).
pub fn restore_all_after_install(port: &Port, saves_backup_dir: &Path, dest_dir: &Path) -> Option<(PathBuf, bool)> {
    let mut aside: Option<(PathBuf, bool)> = None;
    for (save_folder, field) in save_fields(port) {
        if let Some((dir, restored)) = restore_after_install(save_folder, saves_backup_dir, dest_dir, &port.folder, field) {
            aside = Some((dir, restored && aside.as_ref().is_none_or(|(_, all)| *all)));
        }
    }
    aside
}

fn unused_zip_path(dir: &Path, stamp: &str) -> Result<PathBuf, String> {
    let first = dir.join(format!("{stamp}.zip"));
    if !first.exists() {
        return Ok(first);
    }
    (2..100).map(|n| dir.join(format!("{stamp}-{n}.zip"))).find(|path| !path.exists()).ok_or_else(|| format!("too many backups named {stamp}"))
}

// Zips the port's save folders into "Update Backups/<folder>/<stamp>.zip" before an install writes into the game folder.
// Ok(None) means there was nothing to back up. An error means no backup exists, and the caller must not install.
pub fn backup_before_update(port: &Port, saves_backup_dir: &Path, game_dir: &Path, stamp: &str, on_progress: &mut dyn FnMut(&str)) -> Result<Option<PathBuf>, String> {
    let sources: Vec<(PathBuf, &str)> = save_fields(port)
        .into_iter()
        .filter_map(|(save_folder, field)| {
            // A save that is one file (upstream 2026-10-30, `copy_save`) is backed up too, but not through a link
            // (a link to a big file elsewhere would be copied with no room check; folders have `link_problem`).
            let src = save_folder.and_then(|v| resolve_save_folder(v, game_dir)).filter(|src| src.is_dir() || fs::symlink_metadata(src).is_ok_and(|m| m.is_file()))?;
            // "save": "." (or a folder holding the game) is not a save folder: copying it would duplicate the whole game on every update.
            (!is_within(game_dir, &src) && !is_within(saves_backup_dir, &src)).then_some((src, field))
        })
        .collect();
    if save_folder_unknown_on_linux(port, game_dir) {
        // Logged too: the progress dialog shows only the last message of each 100 ms batch.
        eprintln!("update backup: no Linux save folder is known for {}, so those saves are not backed up", port.name);
        on_progress("No Linux save folder is known for this port, so those saves are not backed up.");
    }
    if sources.is_empty() {
        return Ok(None);
    }
    // Fork (save-guards): a save folder (a hand-edited one, or a link swapped in later) that holds many apps' files,
    // or a link that leads to one, is never copied: it would recurse into its own staging and copy all of Steam.
    let home = std::env::var_os("HOME").map(PathBuf::from);
    for (src, _) in sources.iter().filter(|(src, _)| src.is_dir()) {
        let problem = crate::fork::save_restore::folder_problem(src, game_dir, game_dir.parent(), saves_backup_dir, home.as_deref())
            .map(|why| format!("{} {why}; nothing was backed up", crate::fork::port_page::paths::human_path_here(src))) // Fork (install wizard): the place the human way
            .or_else(|| crate::fork::save_restore::link_problem(src, game_dir, saves_backup_dir, home.as_deref()).map(|why| why.replace("nothing was changed", "nothing was backed up")));
        if let Some(problem) = problem {
            return Err(format!("The save folder of {} cannot be backed up: {problem}. Open Info > Find saves... and choose the game's own save folder, then update again.", port.name));
        }
    }
    // Fork (save finder): a save folder outside the game folder (one the user chose) can be big; check the room first.
    let outside: u64 = sources
        .iter()
        .filter(|(src, _)| !is_within(src, game_dir))
        .map(|(src, _)| if src.is_file() { fs::metadata(src).map_or(0, |m| m.len()) } else { crate::fork::save_restore::tree_size_capped(src, u64::MAX / 2, 2_000_000).0 })
        .sum();
    if outside > 0 {
        let place = saves_backup_dir.ancestors().find(|a| a.exists()).unwrap_or(saves_backup_dir);
        if let Some(problem) = crate::fork::save_restore::space_problem(outside.saturating_mul(2), crate::fork::save_restore::free_bytes(place), place) {
            return Err(problem);
        }
    }
    let backups_dir = safe_join(&saves_backup_dir.join(UPDATE_BACKUPS_DIR), &port.folder)?;
    on_progress("Backing up saves...");
    fs::create_dir_all(saves_backup_dir).map_err(|e| e.to_string())?;
    let staging = tempfile::Builder::new().prefix("_update_backup_").tempdir_in(saves_backup_dir).map_err(|e| e.to_string())?;
    let mut copied_any = false;
    for (src, field) in &sources {
        copied_any |= copy_save(src, &staging.path().join(field)).map_err(|e| format!("{e} -- {}", crate::fork::port_page::paths::human_path_here(src)))?; // Fork (install wizard): the place the human way
    }
    if !copied_any {
        return Ok(None);
    }
    let zip_path = unused_zip_path(&backups_dir, stamp)?;
    // Fork (install wizard): the missing zip is named the human way (`human_path_here`). Fork (data safety, W1 / D5):
    // `zip_staging_dir` checks the file and removes its own part file on failure; nothing at `zip_path` is removed here.
    zip_staging_dir(staging.path(), &zip_path)?;
    Ok(Some(zip_path))
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct GlobalBackupSummary {
    pub copied: usize,
    pub skipped: usize,
    pub failed: usize,
    // Fork: installed ports whose save folder is not known on Linux (also counted in `skipped`).
    pub no_linux_folder: Vec<String>,
    // Fork (back up saves page): what happened to each port, by name (installed ports, and any port whose saves were
    // found), and why nothing at all could be done (the Saves Backup folder could not be made).
    pub ports: Vec<(String, PortBackup)>,
    pub problem: Option<String>,
}

// Fork (back up saves page): one port's outcome, for a done page that counts ports, not save folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortBackup {
    BackedUp,
    /// Its save folders are empty or not there yet.
    NothingYet,
    /// The catalog does not say where it keeps its saves on Linux.
    NoPlace,
    /// Why, in plain words.
    Failed(String),
}

// Fork (back up saves page): an error in plain words.
fn plain_io_reason(e: &io::Error) -> String {
    match (e.kind(), e.raw_os_error()) {
        (_, Some(28)) => "the drive is full".to_string(),
        (io::ErrorKind::PermissionDenied, _) => "a file could not be read (no permission)".to_string(),
        (io::ErrorKind::NotFound, _) => "a file went away while it was copied".to_string(),
        _ => format!("a file could not be copied ({e})"),
    }
}

// Fork: for `save` and `save2`, whether it has no known Linux location (see `SaveLocation`).
pub fn unknown_on_linux_fields(port: &Port, game_dir: &Path) -> (bool, bool) {
    let unknown = |v: Option<&Value>| v.is_some_and(|v| save_location(v, game_dir) == SaveLocation::UnknownOnLinux);
    (unknown(port.save.as_ref()), unknown(port.save2.as_ref()))
}

pub fn save_folder_unknown_on_linux(port: &Port, game_dir: &Path) -> bool {
    let (save, save2) = unknown_on_linux_fields(port, game_dir);
    save || save2
}

fn zip_staging_dir(staging: &Path, zip_path: &Path) -> Result<(), String> {
    let tool = sevenzip_exe_path().ok_or("7-Zip is required next to the application to create this backup (missing).")?;
    if let Some(parent) = zip_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Fork (data safety, W1 / D5): 7-Zip writes `<name>.zip.part` (no reader takes it for a backup: they all look for
    // `.zip`), and only a complete, non-empty file is renamed to its name; a failure removes the part file. A backup
    // already at `zip_path` is never removed: callers pick a free name (`unused_zip_path`).
    let part = part_path(zip_path);
    let _ = fs::remove_file(&part);
    let made = run_7z(&tool, &[OsStr::new("a"), OsStr::new("-tzip"), part.as_os_str(), OsStr::new(".")], "create", Some(staging)).and_then(|_| {
        if fs::metadata(&part).map_or(true, |m| m.len() == 0) {
            return Err(format!("the backup file is missing -- {}", crate::fork::port_page::paths::human_path_here(zip_path)));
        }
        if fs::symlink_metadata(zip_path).is_ok() {
            return Err(format!("a backup of that name appeared meanwhile -- {}", crate::fork::port_page::paths::human_path_here(zip_path)));
        }
        fs::rename(&part, zip_path).map_err(|e| e.to_string())
    });
    if made.is_err() {
        let _ = fs::remove_file(&part);
    }
    made
}

// Fork (data safety, W1 / D5): the name a backup is written under until it is complete.
fn part_path(zip_path: &Path) -> PathBuf {
    let mut name = zip_path.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    zip_path.with_file_name(name)
}

pub fn run_global_backup(
    catalog: &[Port],
    library_dir: &Path,
    saves_backup_dir: &Path,
    date: &str,
    on_progress: &mut dyn FnMut(&str),
) -> GlobalBackupSummary {
    let mut summary = GlobalBackupSummary::default();
    if let Err(e) = fs::create_dir_all(saves_backup_dir) {
        summary.problem = Some(format!("the backup folder could not be made: {}", plain_io_reason(&e))); // Fork
        return summary;
    }
    let staging = match tempfile::Builder::new().prefix("_backup_").tempdir_in(saves_backup_dir) {
        Ok(staging) => staging,
        Err(e) => {
            summary.problem = Some(format!("nothing could be written in the backup folder: {}", plain_io_reason(&e))); // Fork
            return summary;
        }
    };

    for port in catalog {
        on_progress(&port.name);
        let Ok(game_dir) = safe_join(library_dir, &port.folder) else { continue };
        let Ok(port_staging) = safe_join(staging.path(), &port.folder) else { continue };
        let no_linux = game_dir.is_dir() && save_folder_unknown_on_linux(port, &game_dir);
        if no_linux {
            summary.no_linux_folder.push(port.name.clone());
        }
        let (mut copied, mut failed) = (false, None); // Fork: this port's outcome
        for (save_folder, field) in save_fields(port) {
            match save_folder.and_then(|v| resolve_save_folder(v, &game_dir)).filter(|src| src.exists()) {
                Some(src) => match copy_save(&src, &port_staging.join(field)) {
                    Ok(true) => {
                        summary.copied += 1;
                        copied = true;
                    }
                    Ok(false) => summary.skipped += 1,
                    Err(e) => {
                        summary.failed += 1;
                        eprintln!("save backup: {} ({field}): {e}", port.name); // Fork: the raw error, for a report
                        failed.get_or_insert_with(|| plain_io_reason(&e));
                    }
                },
                None => summary.skipped += 1,
            }
        }
        // Fork (back up saves page): one outcome per port that is installed or had saves.
        let outcome = match failed {
            Some(why) => Some(PortBackup::Failed(why)),
            None if copied => Some(PortBackup::BackedUp),
            None if !game_dir.is_dir() => None,
            None if no_linux || (port.save.is_none() && port.save2.is_none()) => Some(PortBackup::NoPlace),
            None => Some(PortBackup::NothingYet),
        };
        summary.ports.extend(outcome.map(|o| (port.name.clone(), o)));
    }

    if summary.copied > 0 {
        // Fork (data safety, W1 / D5): a second backup the same day is `<date>-2.zip`; the morning's stays.
        let zipped = unused_zip_path(&saves_backup_dir.join(GLOBAL_BACKUPS_DIR), date).and_then(|zip_path| zip_staging_dir(staging.path(), &zip_path));
        if let Err(e) = zipped {
            summary.failed += summary.copied;
            summary.copied = 0;
            // Fork (back up saves page): the copies were made, but the one backup file that holds them was not.
            eprintln!("save backup: the backup file could not be made: {e}");
            for (_, o) in summary.ports.iter_mut().filter(|(_, o)| *o == PortBackup::BackedUp) {
                *o = PortBackup::Failed("the backup file could not be made".to_string());
            }
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("ports_launcher_save_backup_test_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn port_with_save_folders(folder_name: &str, save: Option<&str>, save2: Option<&str>) -> Port {
        let mut value = serde_json::json!({"name": folder_name, "folder": folder_name, "source": "https://example.com/x.zip"});
        if let Some(s) = save {
            value["save"] = s.into();
        }
        if let Some(s) = save2 {
            value["save2"] = s.into();
        }
        super::super::models::port_from_value(&value).unwrap()
    }

    #[test]
    fn copy_non_empty_copie_les_fichiers_et_larborescence() {
        let dir = temp_dir("copy_basic");
        let src = dir.join("src");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.dat"), b"a").unwrap();
        fs::write(src.join("sub").join("b.dat"), b"b").unwrap();

        let dst = dir.join("dst");
        assert!(copy_non_empty(&src, &dst).unwrap());
        assert_eq!(fs::read_to_string(dst.join("a.dat")).unwrap(), "a");
        assert_eq!(fs::read_to_string(dst.join("sub").join("b.dat")).unwrap(), "b");
    }

    #[test]
    fn copy_non_empty_ignore_les_sous_dossiers_vides_et_ne_cree_rien_si_tout_est_vide() {
        let dir = temp_dir("copy_empty");
        let src = dir.join("src");
        fs::create_dir_all(src.join("empty_sub")).unwrap();
        let dst = dir.join("dst");
        assert!(!copy_non_empty(&src, &dst).unwrap());
        assert!(!dst.exists());
    }

    #[test]
    fn preserve_before_uninstall_echoue_sans_perdre_l_original_si_la_copie_echoue() {
        let dir = temp_dir("preserve_copy_fails");
        let saves_backup_dir = dir.join("Saves Backup");
        fs::write(&saves_backup_dir, b"blocks Saves Backup from being a directory").unwrap();
        let port = port_with_save_folders("MyGame", Some("Save"), None);
        let dest_dir = dir.join("MyGame");
        fs::create_dir_all(dest_dir.join("Save")).unwrap();
        fs::write(dest_dir.join("Save").join("slot1.dat"), b"precious").unwrap();

        assert!(!preserve_before_uninstall(port.save.as_ref(), &saves_backup_dir, &dest_dir, &port.folder, "save_folder"));
        assert_eq!(fs::read_to_string(dest_dir.join("Save").join("slot1.dat")).unwrap(), "precious");
    }

    #[test]
    fn restore_after_install_garde_la_copie_si_la_restauration_echoue() {
        let dir = temp_dir("restore_copy_fails");
        let saves_backup_dir = dir.join("Saves Backup");
        let port = port_with_save_folders("MyGame", Some("Save"), None);
        let backup = pending_restore_dir(&saves_backup_dir, "MyGame", "save_folder");
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("slot1.dat"), b"precious").unwrap();

        let dest_dir = dir.join("MyGame");
        fs::create_dir_all(&dest_dir).unwrap();
        fs::write(dest_dir.join("Save"), b"blocks Save from being a directory").unwrap();
        // Fork (data safety, D6): the file in the way is moved aside before a restore; here that cannot happen either
        // (a file sits where Replaced by restore would go), so nothing is restored and both stay.
        fs::write(saves_backup_dir.join(crate::fork::uninstall_keep::REPLACED_DIR), b"blocks the aside folder").unwrap();

        restore_after_install(port.save.as_ref(), &saves_backup_dir, &dest_dir, &port.folder, "save_folder");
        assert_eq!(fs::read_to_string(backup.join("slot1.dat")).unwrap(), "precious");
        assert_eq!(fs::read_to_string(dest_dir.join("Save")).unwrap(), "blocks Save from being a directory");
    }

    #[test]
    fn preserve_puis_restore_aller_retour() {
        let dir = temp_dir("round_trip");
        let saves_backup_dir = dir.join("Saves Backup");
        let port = port_with_save_folders("MyGame", Some("Save"), None);
        let dest_dir = dir.join("MyGame");
        fs::create_dir_all(dest_dir.join("Save")).unwrap();
        fs::write(dest_dir.join("Save").join("slot1.dat"), b"precious").unwrap();

        assert!(preserve_before_uninstall(port.save.as_ref(), &saves_backup_dir, &dest_dir, &port.folder, "save_folder"));
        assert_eq!(fs::read_to_string(pending_restore_dir(&saves_backup_dir, "MyGame", "save_folder").join("slot1.dat")).unwrap(), "precious");

        fs::remove_dir_all(&dest_dir).unwrap();
        fs::create_dir_all(&dest_dir).unwrap();
        restore_after_install(port.save.as_ref(), &saves_backup_dir, &dest_dir, &port.folder, "save_folder");

        assert_eq!(fs::read_to_string(dest_dir.join("Save").join("slot1.dat")).unwrap(), "precious");
        assert!(!pending_restore_dir(&saves_backup_dir, "MyGame", "save_folder").exists());
    }

    #[test]
    fn sauvegarde_fichier_aller_retour_sans_toucher_au_reste_du_jeu() {
        let dir = temp_dir("file_round_trip");
        let saves_backup_dir = dir.join("Saves Backup");
        let port = port_with_save_folders("MyGame", Some("game.sav"), None);
        let dest_dir = dir.join("MyGame");
        fs::create_dir_all(&dest_dir).unwrap();
        fs::write(dest_dir.join("game.sav"), b"precious").unwrap();
        fs::write(dest_dir.join("game.exe"), b"old").unwrap();

        assert!(preserve_before_uninstall(port.save.as_ref(), &saves_backup_dir, &dest_dir, &port.folder, "save_folder"));
        assert_eq!(fs::read_to_string(pending_restore_dir(&saves_backup_dir, "MyGame", "save_folder").join("game.sav")).unwrap(), "precious");

        fs::remove_dir_all(&dest_dir).unwrap();
        fs::create_dir_all(&dest_dir).unwrap();
        fs::write(dest_dir.join("game.exe"), b"new").unwrap();
        restore_after_install(port.save.as_ref(), &saves_backup_dir, &dest_dir, &port.folder, "save_folder");

        assert_eq!(fs::read_to_string(dest_dir.join("game.sav")).unwrap(), "precious");
        assert_eq!(fs::read_to_string(dest_dir.join("game.exe")).unwrap(), "new");
        assert!(!pending_restore_dir(&saves_backup_dir, "MyGame", "save_folder").exists());
    }

    #[test]
    fn sauvegarde_fichier_dans_un_sous_dossier_recree_le_dossier() {
        let dir = temp_dir("file_in_subdir");
        let saves_backup_dir = dir.join("Saves Backup");
        let port = port_with_save_folders("MyGame", Some("data/slot.sav"), None);
        let dest_dir = dir.join("MyGame");
        fs::create_dir_all(dest_dir.join("data")).unwrap();
        fs::write(dest_dir.join("data").join("slot.sav"), b"precious").unwrap();

        assert!(preserve_before_uninstall(port.save.as_ref(), &saves_backup_dir, &dest_dir, &port.folder, "save_folder"));
        fs::remove_dir_all(&dest_dir).unwrap();
        fs::create_dir_all(&dest_dir).unwrap();
        restore_after_install(port.save.as_ref(), &saves_backup_dir, &dest_dir, &port.folder, "save_folder");

        assert_eq!(fs::read_to_string(dest_dir.join("data").join("slot.sav")).unwrap(), "precious");
    }

    #[test]
    fn run_global_backup_copie_une_sauvegarde_fichier() {
        let dir = temp_dir("global_file");
        let library_dir = dir.join("Library");
        let saves_backup_dir = dir.join("Saves Backup");
        let game_dir = library_dir.join("MyGame");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("game.sav"), b"precious").unwrap();
        fs::write(game_dir.join("game.exe"), b"exe").unwrap();
        let port = port_with_save_folders("MyGame", Some("game.sav"), None);

        let summary = run_global_backup(&[port], &library_dir, &saves_backup_dir, "2026-08-23", &mut |_| {});

        assert_eq!(summary, GlobalBackupSummary { copied: 1, skipped: 1, failed: 0, no_linux_folder: vec![], ports: vec![("MyGame".into(), PortBackup::BackedUp)], problem: None });
        let zip_path = saves_backup_dir.join(GLOBAL_BACKUPS_DIR).join("2026-08-23.zip");
        assert_eq!(read_zip_entry(&zip_path, "MyGame/save_folder/game.sav"), "precious");
        assert!(!zip_path.with_extension("extracted").join("MyGame").join("save_folder").join("game.exe").exists());
    }

    #[test]
    fn restore_after_install_refuse_un_save_folder_qui_sort_de_dest_dir() {
        let dir = temp_dir("restore_escape");
        let saves_backup_dir = dir.join("Saves Backup");
        let backup = pending_restore_dir(&saves_backup_dir, "MyGame", "save_folder");
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("slot1.dat"), b"precious").unwrap();

        let dest_dir = dir.join("MyGame");
        fs::create_dir_all(&dest_dir).unwrap();
        let outside = dir.join("Escaped");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("do_not_delete.dat"), b"unrelated").unwrap();

        let escaping_port = port_with_save_folders("MyGame", Some("../Escaped"), None);
        restore_after_install(escaping_port.save.as_ref(), &saves_backup_dir, &dest_dir, "MyGame", "save_folder");

        assert_eq!(fs::read_to_string(outside.join("do_not_delete.dat")).unwrap(), "unrelated");
        assert_eq!(fs::read_to_string(backup.join("slot1.dat")).unwrap(), "precious");
    }

    #[test]
    fn preserve_before_uninstall_refuse_un_folder_name_qui_sort_de_saves_backup_dir() {
        let dir = temp_dir("preserve_folder_name_escape");
        let saves_backup_dir = dir.join("Saves Backup");
        let dest_dir = dir.join("MyGame");
        fs::create_dir_all(dest_dir.join("Save")).unwrap();
        fs::write(dest_dir.join("Save").join("slot1.dat"), b"precious").unwrap();
        let port = port_with_save_folders("MyGame", Some("Save"), None);

        assert!(!preserve_before_uninstall(port.save.as_ref(), &saves_backup_dir, &dest_dir, "../../Escaped", "save_folder"));
        assert!(!dir.join("Escaped").exists());
    }

    #[test]
    fn restore_after_install_refuse_un_folder_name_qui_sort_de_saves_backup_dir() {
        let dir = temp_dir("restore_folder_name_escape");
        let saves_backup_dir = dir.join("Saves Backup");
        let outside = dir.join("Escaped");
        fs::create_dir_all(outside.join("save_folder")).unwrap();
        fs::write(outside.join("save_folder").join("slot1.dat"), b"unrelated").unwrap();

        let dest_dir = dir.join("MyGame");
        fs::create_dir_all(&dest_dir).unwrap();
        let port = port_with_save_folders("MyGame", Some("Save"), None);

        restore_after_install(port.save.as_ref(), &saves_backup_dir, &dest_dir, "../../Escaped", "save_folder");

        assert!(!dest_dir.join("Save").exists());
        assert_eq!(fs::read_to_string(outside.join("save_folder").join("slot1.dat")).unwrap(), "unrelated");
    }

    #[test]
    fn preserve_ignore_un_save_folder_interne_jamais_cree() {
        let dir = temp_dir("preserve_never_created");
        let saves_backup_dir = dir.join("Saves Backup");
        let port = port_with_save_folders("MyGame", Some("saves"), None);
        let dest_dir = dir.join("MyGame");
        fs::create_dir_all(&dest_dir).unwrap();

        assert!(preserve_before_uninstall(port.save.as_ref(), &saves_backup_dir, &dest_dir, &port.folder, "save_folder"));
        assert!(!pending_restore_dir(&saves_backup_dir, "MyGame", "save_folder").exists());
    }

    #[test]
    fn preserve_ignore_une_sauvegarde_externe_au_dossier_du_jeu() {
        let dir = temp_dir("preserve_external");
        let saves_backup_dir = dir.join("Saves Backup");
        let external_save = dir.join("external_save");
        fs::create_dir_all(&external_save).unwrap();
        fs::write(external_save.join("slot1.dat"), b"precious").unwrap();
        let port = port_with_save_folders("MyGame", Some(external_save.to_str().unwrap()), None);
        let dest_dir = dir.join("MyGame");
        fs::create_dir_all(&dest_dir).unwrap();

        assert!(preserve_before_uninstall(port.save.as_ref(), &saves_backup_dir, &dest_dir, &port.folder, "save_folder"));
        assert!(external_save.join("slot1.dat").exists());
        assert!(!pending_restore_dir(&saves_backup_dir, "MyGame", "save_folder").exists());
    }

    fn read_zip_entry(zip_path: &Path, entry_path: &str) -> String {
        let tool = sevenzip_exe_path().unwrap();
        let out_dir = zip_path.with_extension("extracted");
        let _ = fs::remove_dir_all(&out_dir);
        let out_arg = format!("-o{}", out_dir.display());
        let output = std::process::Command::new(&tool).arg("x").arg(zip_path).arg(&out_arg).arg("-y").output().unwrap();
        assert!(output.status.success(), "échec d'extraction du zip : {}", String::from_utf8_lossy(&output.stdout));
        fs::read_to_string(out_dir.join(entry_path)).unwrap()
    }

    #[test]
    fn run_global_backup_sauvegarde_un_port_non_installe_via_un_save_folder_externe() {
        let dir = temp_dir("global_not_installed");
        let library_dir = dir.join("Library");
        let saves_backup_dir = dir.join("Saves Backup");
        fs::create_dir_all(&library_dir).unwrap();
        let external_save = dir.join("external_save");
        fs::create_dir_all(&external_save).unwrap();
        fs::write(external_save.join("slot1.dat"), b"precious").unwrap();
        let port = port_with_save_folders("MyGame", Some(external_save.to_str().unwrap()), None);

        let mut seen = Vec::new();
        let summary = run_global_backup(&[port], &library_dir, &saves_backup_dir, "2026-08-23", &mut |name| seen.push(name.to_string()));

        assert_eq!(seen, vec!["MyGame".to_string()]);
        assert_eq!(summary, GlobalBackupSummary { copied: 1, skipped: 1, failed: 0, no_linux_folder: vec![], ports: vec![("MyGame".into(), PortBackup::BackedUp)], problem: None });
        let zip_path = saves_backup_dir.join(GLOBAL_BACKUPS_DIR).join("2026-08-23.zip");
        assert_eq!(read_zip_entry(&zip_path, "MyGame/save_folder/slot1.dat"), "precious");
    }

    #[test]
    fn run_global_backup_resout_un_save_folder_relatif_contre_library() {
        let dir = temp_dir("global_relative");
        let library_dir = dir.join("Library");
        let saves_backup_dir = dir.join("Saves Backup");
        let game_dir = library_dir.join("MyGame");
        fs::create_dir_all(game_dir.join("saves")).unwrap();
        fs::write(game_dir.join("saves").join("slot1.dat"), b"precious").unwrap();
        let port = port_with_save_folders("MyGame", None, Some("saves"));

        let summary = run_global_backup(&[port], &library_dir, &saves_backup_dir, "2026-08-23", &mut |_| {});

        assert_eq!(summary, GlobalBackupSummary { copied: 1, skipped: 1, failed: 0, no_linux_folder: vec![], ports: vec![("MyGame".into(), PortBackup::BackedUp)], problem: None });
        let zip_path = saves_backup_dir.join(GLOBAL_BACKUPS_DIR).join("2026-08-23.zip");
        assert_eq!(read_zip_entry(&zip_path, "MyGame/save_folder2/slot1.dat"), "precious");
    }

    // Fork (back up saves page): each installed port gets one outcome; a port that is not installed and has no saves
    // is not counted at all.
    #[test]
    fn run_global_backup_says_what_happened_to_each_port() {
        let dir = temp_dir("global_outcomes");
        let library_dir = dir.join("Library");
        let saves_backup_dir = dir.join("Saves Backup");
        fs::create_dir_all(library_dir.join("Empty").join("saves")).unwrap();
        fs::create_dir_all(library_dir.join("NoSaves")).unwrap();
        fs::create_dir_all(library_dir.join("Full").join("saves")).unwrap();
        fs::write(library_dir.join("Full").join("saves").join("a.sav"), b"a").unwrap();
        let ports = [
            port_with_save_folders("Empty", Some("saves"), None),
            port_with_save_folders("NoSaves", None, None),
            port_with_save_folders("Full", Some("saves"), None),
            port_with_save_folders("NotHere", Some("saves"), None),
        ];
        let summary = run_global_backup(&ports, &library_dir, &saves_backup_dir, "2026-08-24", &mut |_| {});
        assert_eq!(summary.ports, vec![("Empty".into(), PortBackup::NothingYet), ("NoSaves".into(), PortBackup::NoPlace), ("Full".into(), PortBackup::BackedUp)]);
        assert_eq!(summary.problem, None);
        assert_eq!(plain_io_reason(&io::Error::from_raw_os_error(28)), "the drive is full");
        assert_eq!(plain_io_reason(&io::Error::from(io::ErrorKind::PermissionDenied)), "a file could not be read (no permission)");
    }

    // Fork (data safety, W1 / D5): a second backup the same day gets its own name; the first one is never replaced.
    #[test]
    fn a_second_backup_the_same_day_keeps_the_first() {
        let dir = temp_dir("global_same_day_kept");
        let library_dir = dir.join("Library");
        let saves_backup_dir = dir.join("Saves Backup");
        fs::create_dir_all(&library_dir).unwrap();
        let external_a = dir.join("external_a");
        fs::create_dir_all(&external_a).unwrap();
        fs::write(external_a.join("slot1.dat"), b"a").unwrap();
        run_global_backup(&[port_with_save_folders("GameA", Some(external_a.to_str().unwrap()), None)], &library_dir, &saves_backup_dir, "2026-08-23", &mut |_| {});
        let first = saves_backup_dir.join(GLOBAL_BACKUPS_DIR).join("2026-08-23.zip");
        let morning = fs::read(&first).unwrap();

        // An evening run whose zip cannot be made (a folder blocks the part file): the morning's backup is untouched.
        let blocked = saves_backup_dir.join(GLOBAL_BACKUPS_DIR).join("2026-08-23-2.zip.part");
        fs::create_dir_all(blocked.join("x")).unwrap();
        let failed = run_global_backup(&[port_with_save_folders("GameA", Some(external_a.to_str().unwrap()), None)], &library_dir, &saves_backup_dir, "2026-08-23", &mut |_| {});
        assert_eq!(failed.ports, vec![("GameA".into(), PortBackup::Failed("the backup file could not be made".into()))]);
        assert_eq!(fs::read(&first).unwrap(), morning, "the morning's backup is never removed");
        assert!(!saves_backup_dir.join(GLOBAL_BACKUPS_DIR).join("2026-08-23-2.zip").exists());
        fs::remove_dir_all(&blocked).unwrap();

        let external_b = dir.join("external_b");
        fs::create_dir_all(&external_b).unwrap();
        fs::write(external_b.join("slot1.dat"), b"b").unwrap();
        run_global_backup(&[port_with_save_folders("GameB", Some(external_b.to_str().unwrap()), None)], &library_dir, &saves_backup_dir, "2026-08-23", &mut |_| {});
        assert_eq!(fs::read(&first).unwrap(), morning);
        assert_eq!(read_zip_entry(&first, "GameA/save_folder/slot1.dat"), "a");
        let second = saves_backup_dir.join(GLOBAL_BACKUPS_DIR).join("2026-08-23-2.zip");
        assert_eq!(read_zip_entry(&second, "GameB/save_folder/slot1.dat"), "b");
        let left: Vec<String> = fs::read_dir(saves_backup_dir.join(GLOBAL_BACKUPS_DIR)).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".part")).collect();
        assert!(left.is_empty(), "no part file is left: {left:?}");
    }

    // Fork (data safety, D6): putting kept saves back moves what is there now aside; nothing is deleted.
    #[test]
    fn restoring_kept_saves_moves_the_live_ones_aside() {
        let dir = temp_dir("restore_moves_live_aside");
        let saves_backup_dir = dir.join("Saves Backup");
        let port = port_with_save_folders("MyGame", Some("Save"), Some("game.sav"));
        let kept = pending_restore_dir(&saves_backup_dir, "MyGame", "save_folder");
        fs::create_dir_all(&kept).unwrap();
        fs::write(kept.join("slot1.dat"), b"kept from the uninstall").unwrap();
        let kept_file = pending_restore_dir(&saves_backup_dir, "MyGame", "save_folder2");
        fs::create_dir_all(&kept_file).unwrap();
        fs::write(kept_file.join("game.sav"), b"kept file save").unwrap();
        let dest_dir = dir.join("MyGame");
        fs::create_dir_all(dest_dir.join("Save")).unwrap();
        fs::write(dest_dir.join("Save").join("slot2.dat"), b"played since").unwrap();
        fs::write(dest_dir.join("game.sav"), b"live file save").unwrap();

        restore_all_after_install(&port, &saves_backup_dir, &dest_dir);

        assert_eq!(fs::read(dest_dir.join("Save").join("slot1.dat")).unwrap(), b"kept from the uninstall");
        assert_eq!(fs::read(dest_dir.join("game.sav")).unwrap(), b"kept file save");
        let replaced = crate::fork::uninstall_keep::replaced_dir_for(&saves_backup_dir, "MyGame");
        let mut found = Vec::new();
        for stamp in fs::read_dir(&replaced).unwrap().flatten() {
            for field in fs::read_dir(stamp.path()).unwrap().flatten() {
                for f in fs::read_dir(field.path()).unwrap().flatten() {
                    found.push((field.file_name().to_string_lossy().into_owned(), fs::read(f.path()).unwrap()));
                }
            }
        }
        found.sort();
        assert_eq!(found, vec![("save_folder".to_string(), b"played since".to_vec()), ("save_folder2".to_string(), b"live file save".to_vec())]);
    }

    // Fork (data safety, recheck 1): a kept copy that cannot go back is reported as not put back, and stays kept.
    #[test]
    #[cfg(unix)]
    fn a_kept_copy_that_cannot_go_back_is_reported_as_not_put_back() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("restore_copy_back_fails");
        let saves_backup_dir = dir.join("Saves Backup");
        let kept = pending_restore_dir(&saves_backup_dir, "MyGame", "save_folder");
        fs::create_dir_all(&kept).unwrap();
        fs::write(kept.join("slot1.dat"), b"kept").unwrap();
        fs::set_permissions(kept.join("slot1.dat"), fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(kept.join("slot1.dat")).is_ok() {
            return; // running as root: permissions do not stop a read
        }
        let dest_dir = dir.join("MyGame");
        fs::create_dir_all(dest_dir.join("Save")).unwrap();
        fs::write(dest_dir.join("Save").join("slot2.dat"), b"newer").unwrap();
        let result = restore_all_after_install(&port_with_save_folders("MyGame", Some("Save"), None), &saves_backup_dir, &dest_dir);
        fs::set_permissions(kept.join("slot1.dat"), fs::Permissions::from_mode(0o644)).unwrap();
        let (aside, put_back) = result.expect("the newer saves were moved aside");
        assert!(!put_back);
        assert_eq!(fs::read(aside.join("save_folder").join("slot2.dat")).unwrap(), b"newer");
        assert_eq!(fs::read(kept.join("slot1.dat")).unwrap(), b"kept", "the kept copy stays in Pending Restore");
    }

    // Fork (data safety, D6): a "save folder" that is the game folder itself is never replaced by a kept copy.
    #[test]
    fn a_kept_copy_never_replaces_the_whole_game_folder() {
        let dir = temp_dir("restore_whole_game");
        let saves_backup_dir = dir.join("Saves Backup");
        let kept = pending_restore_dir(&saves_backup_dir, "MyGame", "save_folder");
        fs::create_dir_all(&kept).unwrap();
        fs::write(kept.join("game.exe"), b"old program").unwrap();
        let dest_dir = dir.join("MyGame");
        fs::create_dir_all(&dest_dir).unwrap();
        fs::write(dest_dir.join("game.exe"), b"new program").unwrap();
        restore_all_after_install(&port_with_save_folders("MyGame", Some("."), None), &saves_backup_dir, &dest_dir);
        assert_eq!(fs::read(dest_dir.join("game.exe")).unwrap(), b"new program");
        assert_eq!(fs::read(kept.join("game.exe")).unwrap(), b"old program", "the kept copy stays kept");
    }

    // Fork (data safety, F-W1): a second uninstall never deletes a kept copy an earlier one left (a failed restore).
    #[test]
    fn a_second_uninstall_keeps_the_earlier_kept_copy() {
        let dir = temp_dir("preserve_keeps_earlier");
        let saves_backup_dir = dir.join("Saves Backup");
        let port = port_with_save_folders("MyGame", Some("Save"), None);
        let pending = pending_restore_dir(&saves_backup_dir, "MyGame", "save_folder");
        fs::create_dir_all(&pending).unwrap();
        fs::write(pending.join("slot1.dat"), b"the only copy").unwrap();
        let dest_dir = dir.join("MyGame");
        fs::create_dir_all(dest_dir.join("Save")).unwrap();
        fs::write(dest_dir.join("Save").join("slot1.dat"), b"newer").unwrap();

        assert!(preserve_before_uninstall(port.save.as_ref(), &saves_backup_dir, &dest_dir, &port.folder, "save_folder"));

        assert_eq!(fs::read(pending.join("slot1.dat")).unwrap(), b"newer");
        let earlier: Vec<_> = fs::read_dir(pending.parent().unwrap()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("save_folder.")).collect();
        assert_eq!(earlier.len(), 1);
        assert_eq!(fs::read(earlier[0].path().join("slot1.dat")).unwrap(), b"the only copy");
    }

    // Fork (data safety, F-D1): a save folder that is a link to a folder far too broad (here the drive's top) is not
    // copied; the uninstall goes on and removes only the link.
    #[test]
    #[cfg(unix)]
    fn a_save_folder_linked_to_somewhere_too_broad_is_not_copied() {
        let dir = temp_dir("preserve_broad_link");
        let saves_backup_dir = dir.join("Saves Backup");
        let dest_dir = dir.join("MyGame");
        fs::create_dir_all(&dest_dir).unwrap();
        std::os::unix::fs::symlink("/", dest_dir.join("Save")).unwrap();
        let port = port_with_save_folders("MyGame", Some("Save"), None);
        assert!(preserve_before_uninstall(port.save.as_ref(), &saves_backup_dir, &dest_dir, &port.folder, "save_folder"));
        assert!(!pending_restore_dir(&saves_backup_dir, "MyGame", "save_folder").exists(), "nothing was copied");
    }

    #[test]
    fn run_global_backup_second_export_le_meme_jour_remplace_le_premier() {
        let dir = temp_dir("global_same_day_overwrite");
        let library_dir = dir.join("Library");
        let saves_backup_dir = dir.join("Saves Backup");
        fs::create_dir_all(&library_dir).unwrap();

        let external_a = dir.join("external_a");
        fs::create_dir_all(&external_a).unwrap();
        fs::write(external_a.join("slot1.dat"), b"a").unwrap();
        let port_a = port_with_save_folders("GameA", Some(external_a.to_str().unwrap()), None);
        run_global_backup(&[port_a], &library_dir, &saves_backup_dir, "2026-08-23", &mut |_| {});

        let external_b = dir.join("external_b");
        fs::create_dir_all(&external_b).unwrap();
        fs::write(external_b.join("slot1.dat"), b"b").unwrap();
        let port_b = port_with_save_folders("GameB", Some(external_b.to_str().unwrap()), None);
        run_global_backup(&[port_b], &library_dir, &saves_backup_dir, "2026-08-23", &mut |_| {});

        // Fork (data safety, W1 / D5): the second export of the day is `<date>-2.zip`; the first is no longer replaced.
        let zip_path = saves_backup_dir.join(GLOBAL_BACKUPS_DIR).join("2026-08-23-2.zip");
        assert_eq!(read_zip_entry(&zip_path, "GameB/save_folder/slot1.dat"), "b");
        assert!(!zip_path.with_extension("extracted").join("GameA").exists());
        assert!(saves_backup_dir.join(GLOBAL_BACKUPS_DIR).join("2026-08-23.zip").is_file());
    }

    #[cfg(target_os = "linux")]
    fn port_from_json(value: serde_json::Value) -> Port {
        super::super::models::port_from_value(&value).unwrap()
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn run_global_backup_names_installed_ports_with_no_linux_save_folder() {
        let dir = temp_dir("global_no_linux_folder");
        let library_dir = dir.join("Library");
        let saves_backup_dir = dir.join("Saves Backup");
        fs::create_dir_all(library_dir.join("Osu")).unwrap();
        // A folder literally named like the Windows path must not be picked up as the save folder any more.
        fs::create_dir_all(library_dir.join("Osu").join("%APPDATA%\\osu")).unwrap();
        fs::write(library_dir.join("Osu").join("%APPDATA%\\osu").join("x.dat"), b"x").unwrap();
        let installed = port_from_json(serde_json::json!({"name": "Osu", "folder": "Osu", "source": "https://example.com/x.zip", "save": {"windows": "%APPDATA%\\osu", "android": "/sdcard/osu"}}));
        let not_installed = port_from_json(serde_json::json!({"name": "Other", "folder": "Other", "source": "https://example.com/x.zip", "save": "%APPDATA%\\other"}));

        let summary = run_global_backup(&[installed, not_installed], &library_dir, &saves_backup_dir, "2026-10-21", &mut |_| {});

        assert_eq!(summary, GlobalBackupSummary { copied: 0, skipped: 4, failed: 0, no_linux_folder: vec!["Osu".to_string()], ports: vec![("Osu".into(), PortBackup::NoPlace)], problem: None });
        assert!(!saves_backup_dir.join(GLOBAL_BACKUPS_DIR).join("2026-10-21.zip").exists());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn backup_before_update_says_when_no_linux_save_folder_is_known() {
        let dir = temp_dir("update_no_linux_folder");
        let saves_backup_dir = dir.join("Saves Backup");
        let game_dir = dir.join("Library").join("Osu");
        fs::create_dir_all(&game_dir).unwrap();
        let port = port_from_json(serde_json::json!({"name": "Osu", "folder": "Osu", "source": "https://example.com/x.zip", "save": {"windows": "%APPDATA%\\osu"}}));

        let mut messages = Vec::new();
        let result = backup_before_update(&port, &saves_backup_dir, &game_dir, "stamp", &mut |m| messages.push(m.to_string()));

        assert_eq!(result, Ok(None));
        assert_eq!(messages, vec!["No Linux save folder is known for this port, so those saves are not backed up.".to_string()]);
        assert!(save_folder_unknown_on_linux(&port, &game_dir));
        let known = port_with_save_folders("Known", Some("saves"), None);
        assert!(!save_folder_unknown_on_linux(&known, &game_dir));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_kept_copy_survives_a_reinstall_when_there_is_no_folder_to_restore_into() {
        let dir = temp_dir("restore_unknown_keeps_copy");
        let saves_backup_dir = dir.join("Saves Backup");
        let dest_dir = dir.join("Osu");
        fs::create_dir_all(&dest_dir).unwrap();
        for field in ["save_folder", "save_folder2"] {
            let pending = pending_restore_dir(&saves_backup_dir, "Osu", field);
            fs::create_dir_all(&pending).unwrap();
            fs::write(pending.join("slot1.dat"), b"precious").unwrap();
        }
        let port = port_from_json(serde_json::json!({"name": "Osu", "folder": "Osu", "source": "https://example.com/x.zip", "save": {"windows": "%APPDATA%\\osu"}}));

        restore_all_after_install(&port, &saves_backup_dir, &dest_dir);

        assert_eq!(fs::read_to_string(pending_restore_dir(&saves_backup_dir, "Osu", "save_folder").join("slot1.dat")).unwrap(), "precious");
        assert_eq!(fs::read_to_string(pending_restore_dir(&saves_backup_dir, "Osu", "save_folder2").join("slot1.dat")).unwrap(), "precious", "no save2 field either");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_windows_only_folder_inside_the_game_is_preserved_and_restored_on_linux() {
        let dir = temp_dir("relative_windows_only");
        let saves_backup_dir = dir.join("Saves Backup");
        let dest_dir = dir.join("zelda3");
        fs::create_dir_all(dest_dir.join("saves")).unwrap();
        fs::write(dest_dir.join("saves").join("sram.dat"), b"precious").unwrap();
        let port = port_from_json(serde_json::json!({"name": "zelda3", "folder": "zelda3", "source": "https://example.com/x.zip", "save": {"windows": "saves", "android": "Android/data/com.dishii.zelda3/files"}}));

        assert!(!save_folder_unknown_on_linux(&port, &dest_dir));
        assert!(preserve_all_before_uninstall(&port, &saves_backup_dir, &dest_dir));
        assert_eq!(fs::read_to_string(pending_restore_dir(&saves_backup_dir, "zelda3", "save_folder").join("sram.dat")).unwrap(), "precious");
        fs::remove_dir_all(&dest_dir).unwrap();
        fs::create_dir_all(&dest_dir).unwrap();
        restore_all_after_install(&port, &saves_backup_dir, &dest_dir);
        assert_eq!(fs::read_to_string(dest_dir.join("saves").join("sram.dat")).unwrap(), "precious");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_windows_folder_with_backslashes_is_the_same_nested_folder_for_preserve_and_restore() {
        let dir = temp_dir("backslash_nested");
        let saves_backup_dir = dir.join("Saves Backup");
        let dest_dir = dir.join("Game");
        fs::create_dir_all(dest_dir.join("saves").join("slots")).unwrap();
        fs::write(dest_dir.join("saves").join("slots").join("1.sav"), b"precious").unwrap();
        let port = port_from_json(serde_json::json!({"name": "Game", "folder": "Game", "source": "https://example.com/x.zip", "save": {"windows": "saves\\slots"}}));

        assert!(preserve_all_before_uninstall(&port, &saves_backup_dir, &dest_dir));
        assert_eq!(fs::read_to_string(pending_restore_dir(&saves_backup_dir, "Game", "save_folder").join("1.sav")).unwrap(), "precious");
        fs::remove_dir_all(&dest_dir).unwrap();
        fs::create_dir_all(&dest_dir).unwrap();
        restore_all_after_install(&port, &saves_backup_dir, &dest_dir);
        assert_eq!(fs::read_to_string(dest_dir.join("saves").join("slots").join("1.sav")).unwrap(), "precious");
        assert!(!dest_dir.join("saves\\slots").exists());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_known_save_with_an_unknown_save2_is_still_named() {
        let dir = temp_dir("save2_unknown");
        let library_dir = dir.join("Library");
        let saves_backup_dir = dir.join("Saves Backup");
        fs::create_dir_all(library_dir.join("Game")).unwrap();
        let port = port_from_json(serde_json::json!({"name": "Game", "folder": "Game", "source": "https://example.com/x.zip", "save": "saves", "save2": {"windows": "%LOCALAPPDATA%\\Game"}}));
        assert_eq!(unknown_on_linux_fields(&port, &library_dir.join("Game")), (false, true));
        let summary = run_global_backup(&[port], &library_dir, &saves_backup_dir, "2026-10-21", &mut |_| {});
        assert_eq!(summary.no_linux_folder, vec!["Game".to_string()]);
    }
}
