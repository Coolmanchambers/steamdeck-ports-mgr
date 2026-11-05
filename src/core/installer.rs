use super::asset_select::AssetSelectionError;
use super::forge::Forge;
use super::models::{Port, SourceType};
use super::path_safety::safe_join;
use super::platform_resolve::{resolve_per_platform, resolve_preferred_asset};
use super::save_backup;
use serde_json::Value;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const MAX_UNCOMPRESSED_SIZE: u64 = 20 * 1024 * 1024 * 1024;

// Fork (install wizard): the download reports its bytes (`on_bytes(downloaded, total)`, the total when the server gives
// one) and stops when `cancel` is set; a stopped install fails with exactly `CANCELLED`, its temporary folder removed.
pub const CANCELLED: &str = "The download was stopped and removed; nothing was kept.";

#[derive(Default, Clone)]
pub struct DownloadCtl {
    pub on_bytes: Option<Arc<dyn Fn(u64, Option<u64>) + Send + Sync>>,
    pub cancel: Option<Arc<AtomicBool>>,
}

impl DownloadCtl {
    fn cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed))
    }
}

// Fork (install wizard): `io::copy` with a byte count and a cancel check after each block.
fn copy_counted(reader: &mut dyn io::Read, writer: &mut dyn io::Write, total: Option<u64>, ctl: &DownloadCtl) -> io::Result<u64> {
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    if let Some(f) = &ctl.on_bytes {
        f(0, total);
    }
    loop {
        if ctl.cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, CANCELLED));
        }
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        writer.write_all(&buf[..n])?;
        done += n as u64;
        if let Some(f) = &ctl.on_bytes {
            f(done, total);
        }
    }
    writer.flush()?;
    Ok(done)
}

#[derive(Debug)]
pub enum InstallError {
    Message(String),
    Ambiguous(Vec<Value>),
}

impl From<AssetSelectionError> for InstallError {
    fn from(e: AssetSelectionError) -> Self {
        match e {
            AssetSelectionError::Message(m) => InstallError::Message(m),
            AssetSelectionError::Ambiguous(a) => InstallError::Ambiguous(a),
        }
    }
}

trait InstallErrorExt<T> {
    fn install_err(self) -> Result<T, InstallError>;
}

impl<T, E: std::fmt::Display> InstallErrorExt<T> for Result<T, E> {
    fn install_err(self) -> Result<T, InstallError> {
        self.map_err(|e| InstallError::Message(e.to_string()))
    }
}

fn with_path<T>(result: io::Result<T>, path: &Path) -> io::Result<T> {
    // Fork (install wizard): the place the human way ("SD card > Ports > ..."), never a card's ID.
    result.map_err(|e| io::Error::new(e.kind(), format!("{e} -- {}", crate::fork::port_page::paths::human_path_here(path))))
}

fn retry_transient<T>(mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut last_err = None;
    for attempt in 1..=5u64 {
        match op() {
            Ok(v) => return Ok(v),
            Err(e) => last_err = Some(e),
        }
        std::thread::sleep(Duration::from_millis(100 * attempt));
    }
    Err(last_err.expect("at least one attempt"))
}

#[allow(dead_code)] // Fork (data safety): the merge no longer deletes in place (`fork::update_merge`); kept for merge ease
fn remove_existing(path: &Path) -> io::Result<()> {
    let Ok(meta) = fs::symlink_metadata(path) else { return Ok(()) };
    if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path).or_else(|_| fs::remove_dir(path))
    }
}

#[allow(dead_code)] // Fork (data safety): see `remove_existing`
fn is_real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

fn safe_download_name(name: &str) -> String {
    Path::new(name).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "download".to_string())
}

fn pixeldrain_file_id(url: &str) -> Option<&str> {
    ["https://pixeldrain.com/u/", "http://pixeldrain.com/u/"].iter().find_map(|prefix| {
        let rest = url.strip_prefix(prefix)?;
        let id = rest.split(['/', '?', '#']).next().unwrap_or(rest);
        (!id.is_empty()).then_some(id)
    })
}

fn pixeldrain_real_name(id: &str) -> Option<String> {
    let url = format!("https://pixeldrain.com/api/file/{id}/info");
    let mut resp = super::http::agent(Duration::from_secs(10)).get(&url).call().ok()?;
    let json: Value = resp.body_mut().read_json().ok()?;
    json.get("name").and_then(Value::as_str).map(str::to_string)
}

fn rename_dest(dest: &Path, filename: &str) -> PathBuf {
    dest.with_file_name(safe_download_name(filename))
}

fn has_known_archive_extension(name: &str) -> bool {
    let lower = name.to_lowercase();
    [".zip", ".tar.gz", ".tgz", ".tar", ".7z", ".rar", ".exe", ".tar.xz", ".txz", ".tar.zst", ".tzst"].iter().any(|ext| lower.ends_with(ext))
}

fn content_disposition_filename(header: &str) -> Option<String> {
    let after = header.split("filename=").nth(1)?.trim_start();
    match after.strip_prefix('"') {
        Some(rest) => rest.split('"').next().map(str::to_string),
        None => after.split(';').next().map(|s| s.trim().to_string()),
    }
}

// Fork (install wizard): `ctl` reports the bytes and can stop it.
fn download(url: &str, dest: &Path, on_progress: &mut dyn FnMut(&str), ctl: &DownloadCtl) -> Result<PathBuf, InstallError> {
    let mut url = url.to_string();
    let mut dest = dest.to_path_buf();
    if let Some(id) = pixeldrain_file_id(&url) {
        if let Some(real_name) = pixeldrain_real_name(id) {
            dest = rename_dest(&dest, &real_name);
        }
        url = format!("https://pixeldrain.com/api/file/{id}");
    }

    let name = super::http::decode_url(url.rsplit('/').next().unwrap_or(&url));
    if !has_known_archive_extension(&dest.to_string_lossy()) && has_known_archive_extension(&name) {
        dest = rename_dest(&dest, &name);
    }
    on_progress(&format!("Downloading {name}..."));

    // Fork (install wizard): an agent with a per-read timeout, so a stalled connection ends and a Stop is seen; any
    // failure after a Stop is the stop.
    let agent = super::http::download_agent();
    let stopped = |e: InstallError| if ctl.cancelled() { InstallError::Message(CANCELLED.to_string()) } else { e };
    let mut resp = agent.get(&super::http::encode_url(&url)).call().install_err().map_err(stopped)?;
    if !has_known_archive_extension(&dest.to_string_lossy()) {
        let disposition_name = resp
            .headers()
            .get("content-disposition")
            .and_then(|v| v.to_str().ok())
            .and_then(content_disposition_filename)
            .filter(|n| has_known_archive_extension(n));
        if let Some(disposition_name) = disposition_name {
            dest = rename_dest(&dest, &disposition_name);
        }
    }
    let final_name = ureq::ResponseExt::get_uri(&resp).path().rsplit('/').next().map(super::http::decode_url);
    if let Some(final_name) = final_name.filter(|n| has_known_archive_extension(n) && *n != name) {
        dest = rename_dest(&dest, &final_name);
    }
    // Fork (install wizard): the size the server gives, for the progress bar.
    let total = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok()).filter(|n| *n > 0);
    let mut file = with_path(fs::File::create(&dest), &dest).install_err()?;
    with_path(copy_counted(&mut resp.body_mut().as_reader(), &mut file, total, ctl), &dest).install_err().map_err(stopped)?;
    Ok(dest)
}

fn reject_if_too_large(total_bytes: u64, max_uncompressed_size: u64) -> Result<(), InstallError> {
    if total_bytes > max_uncompressed_size {
        let gb = total_bytes as f64 / 1024f64.powi(3);
        return Err(InstallError::Message(format!(
            "This archive claims to decompress to {gb:.1} GB, which looks like a zip bomb -- refusing to extract it."
        )));
    }
    Ok(())
}

fn flatten_single_wrapper_folder(dir: &Path) -> io::Result<()> {
    loop {
        // Fork (data safety, F-D2): an entry that cannot be read fails the install instead of being dropped.
        let entries: Vec<fs::DirEntry> = fs::read_dir(dir)?.collect::<io::Result<_>>()?;
        let [wrapper] = entries.as_slice() else { return Ok(()) };
        if !wrapper.file_type()?.is_dir() {
            return Ok(());
        }
        let staging = dir.join(format!(".flatten-{}", wrapper.file_name().to_string_lossy()));
        with_path(retry_transient(|| fs::rename(wrapper.path(), &staging)), &staging)?;
        for item in fs::read_dir(&staging)?.collect::<io::Result<Vec<_>>>()? {
            let target = dir.join(item.file_name());
            with_path(retry_transient(|| fs::rename(item.path(), &target)), &target)?;
        }
        with_path(retry_transient(|| fs::remove_dir(&staging)), &staging)?;
    }
}

// Fork (data safety, 2026-11-04 review W2 / D1, D7, F-D2, F-D3): the merge is `fork::update_merge::merge`: the user's
// files stay, replaced protected files are kept, a failure puts everything back. This keeps upstream's name for the
// plain case (everything replaced) its tests use.
#[cfg(test)]
fn merge_into(src: &Path, dest: &Path) -> io::Result<()> {
    let guard = crate::fork::update_merge::Guard::plain(dest.parent().unwrap_or(dest));
    crate::fork::update_merge::merge(src, dest, &guard, &mut move_entry).map(|_| ())
}

// Fork (data safety): one move of the merge, retried like upstream's renames; across drives a checked copy.
fn move_entry(from: &Path, to: &Path) -> io::Result<()> {
    with_path(retry_transient(|| crate::fork::uninstall_keep::move_path(from, to)), to)
}

// Fork (data safety, D2): the port's folder in the Library; never the Library itself (an empty or "." folder name).
pub fn port_dir(library_dir: &Path, folder: &str) -> Result<PathBuf, String> {
    let dir = safe_join(library_dir, folder)?;
    if super::path_safety::is_within(library_dir, &dir) {
        return Err(format!("\"{folder}\" is not a folder name inside the Library -- please report this port."));
    }
    Ok(dir)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)?.collect::<io::Result<Vec<_>>>()? {
        if entry.file_type()?.is_dir() {
            collect_files(&entry.path(), out)?;
        } else {
            out.push(entry.path());
        }
    }
    Ok(())
}

// Fork: the release's own top-level names; `fork::versioned` uses them to tell which old files it supersedes.
fn top_level_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    entries.filter_map(|e| e.ok()).filter_map(|e| e.file_name().into_string().ok()).collect()
}

#[cfg(target_os = "windows")]
const SEVENZIP_NAME: &str = "7z.exe";
#[cfg(target_os = "linux")]
const SEVENZIP_NAME: &str = "7zzs";

pub(crate) fn sevenzip_exe_path() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    [dir.join(SEVENZIP_NAME), dir.parent()?.join(SEVENZIP_NAME)].into_iter().find(|p| p.is_file())
}

pub(crate) fn run_7z(tool: &Path, args: &[&OsStr], action: &str, cwd: Option<&Path>) -> Result<std::process::Output, String> {
    let mut cmd = std::process::Command::new(tool);
    cmd.arg("-sccUTF-8").args(args).stdin(std::process::Stdio::null());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = cmd.output().map_err(|e| e.to_string())?;
    if output.status.code().is_none_or(|c| c >= 2) {
        return Err(format!("7-Zip failed to {action} this archive: {}", seven_zip_error(&output.stdout, &output.stderr)));
    }
    Ok(output)
}

fn seven_zip_error(stdout: &[u8], stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .chain(String::from_utf8_lossy(stdout).lines())
        .find(|l| l.contains("ERROR"))
        .map(str::trim)
        .unwrap_or("unknown error")
        .to_string()
}

fn extract_archive_via_7z(archive: &Path, staging: &Path, max_uncompressed_size: u64) -> Result<(), InstallError> {
    let tool = sevenzip_exe_path()
        .ok_or_else(|| InstallError::Message("7-Zip is required next to the application to extract this file (missing).".to_string()))?;

    let listing = run_7z(&tool, &[OsStr::new("l"), OsStr::new("-slt"), archive.as_os_str()], "read", None).install_err()?;
    let text = String::from_utf8_lossy(&listing.stdout);
    let kind = text.lines().find_map(|l| l.strip_prefix("Type = ")).unwrap_or("").trim().to_string();
    let total: u64 = text.lines().filter_map(|l| l.strip_prefix("Size = ")).filter_map(|n| n.trim().parse::<u64>().ok()).sum();
    reject_if_too_large(total, max_uncompressed_size)?;

    let out_arg = format!("-o{}", staging.display());
    let run_extract = |source: &Path, extra: Option<&str>| {
        let mut args = vec![OsStr::new("x"), source.as_os_str(), OsStr::new(&out_arg), OsStr::new("-y"), OsStr::new("-bd")];
        args.extend(extra.map(OsStr::new));
        run_7z(&tool, &args, "extract", None).install_err()
    };
    let nsis_exclusion = kind.eq_ignore_ascii_case("Nsis").then_some("-x!$PLUGINSDIR\\*");
    run_extract(archive, nsis_exclusion)?;

    // Fork: xz and zstd tarballs (Deck builds use them) unpack in two steps like gzip ones.
    if ["gzip", "xz", "zstd"].iter().any(|k| kind.eq_ignore_ascii_case(k)) {
        if let Some(inner) = fs::read_dir(staging).install_err()?.filter_map(|e| e.ok()).map(|e| e.path()).next() {
            run_extract(&inner, None)?;
            with_path(retry_transient(|| fs::remove_file(&inner)), &inner).install_err()?;
        }
    }
    Ok(())
}

fn find_exe_folder(staging: &Path, target: &str) -> Result<PathBuf, InstallError> {
    let mut matches = Vec::new();
    collect_files(staging, &mut matches).install_err()?;
    let target_lower = target.to_lowercase();
    matches.retain(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.to_lowercase().contains(&target_lower)));
    match matches.as_slice() {
        [] => Err(InstallError::Message(format!("No file matches \"{target}\" (see \"exe_is_archive\" for this port)."))),
        [single] => Ok(single.parent().unwrap_or(staging).to_path_buf()),
        many => Err(InstallError::Message(format!("{} files match \"{target}\" -- please report this port.", many.len()))),
    }
}

#[cfg(target_os = "windows")]
fn clear_readonly_recursive(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    let mut perms = metadata.permissions();
    if perms.readonly() {
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        fs::set_permissions(path, perms)?;
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path)?.filter_map(|e| e.ok()) {
            clear_readonly_recursive(&entry.path())?;
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn clear_readonly_recursive(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_symlink() {
        return Ok(());
    }
    let mut perms = metadata.permissions();
    if perms.readonly() {
        perms.set_mode(perms.mode() | 0o200);
        fs::set_permissions(path, perms)?;
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path)?.filter_map(|e| e.ok()) {
            clear_readonly_recursive(&entry.path())?;
        }
    }
    Ok(())
}

fn extract_to_staging(archive: &Path, staging: &Path, exe_is_archive: Option<&str>) -> Result<PathBuf, InstallError> {
    let name_lower = archive.to_string_lossy().to_lowercase();
    let is_archive = (name_lower.ends_with(".exe") && exe_is_archive.is_some())
        || [".zip", ".tar.gz", ".tgz", ".tar", ".7z", ".rar", ".tar.xz", ".txz", ".tar.zst", ".tzst"].iter().any(|ext| name_lower.ends_with(ext));

    if !is_archive {
        let dest = staging.join(archive.file_name().unwrap_or_default());
        with_path(retry_transient(|| fs::copy(archive, &dest)), &dest).install_err()?;
        return Ok(staging.to_path_buf());
    }
    extract_archive_via_7z(archive, staging, MAX_UNCOMPRESSED_SIZE)?;
    with_path(clear_readonly_recursive(staging), staging).install_err()?;
    match exe_is_archive {
        Some(target) => find_exe_folder(staging, target),
        None => {
            flatten_single_wrapper_folder(staging).install_err()?;
            Ok(staging.to_path_buf())
        }
    }
}

// Fork: returns the release's top-level names. Superseded versioned files are no longer deleted here; the
// app moves them to `.previous/` after the update is verified (`fork::versioned`).
#[cfg(test)]
fn extract(archive: &Path, dest_dir: &Path, library_dir: &Path, exe_is_archive: Option<&str>, on_progress: &mut dyn FnMut(&str)) -> Result<Vec<String>, InstallError> {
    let guard = crate::fork::update_merge::Guard::plain(library_dir);
    extract_guarded(archive, dest_dir, library_dir, exe_is_archive, &guard, on_progress).map(|(release, _)| release)
}

// Fork (data safety): `extract` with the merge's guard (`fork::update_merge`), returning what it set aside too.
fn extract_guarded(
    archive: &Path,
    dest_dir: &Path,
    library_dir: &Path,
    exe_is_archive: Option<&str>,
    guard: &crate::fork::update_merge::Guard,
    on_progress: &mut dyn FnMut(&str),
) -> Result<(Vec<String>, crate::fork::update_merge::MergeReport), InstallError> {
    on_progress("Extracting...");
    let staging = with_path(tempfile::Builder::new().prefix("_staging_").tempdir_in(library_dir), library_dir).install_err()?;
    let merge_root = extract_to_staging(archive, staging.path(), exe_is_archive)?;
    let release = top_level_names(&merge_root);
    let report = crate::fork::update_merge::merge(&merge_root, dest_dir, guard, &mut move_entry).install_err()?;
    Ok((release, report))
}

// Fork (data safety): the guard of a merge into `dest_dir`: the port's saves, game files (`rom_library.json` sits in the
// app's config folder, the one that holds Saves Backup), ROMs, keys and dumps stay; replaced archives go to Saves Backup.
// An unreadable record of connected game files stops the install before anything changes (review C2).
fn guard_for(port: &Port, dest_dir: &Path, library_dir: &Path, saves_backup_dir: &Path, stamp: &str) -> Result<crate::fork::update_merge::Guard, InstallError> {
    let protect = crate::fork::update_merge::Protect::for_port(port, dest_dir, saves_backup_dir.parent()).map_err(|_| {
        InstallError::Message("The game files record could not be read, so the app cannot tell your files from the release's; nothing was changed. Fix or remove rom_library.json in the app's config folder (Desktop Mode).".to_string())
    })?;
    Ok(crate::fork::update_merge::Guard::protecting(protect, saves_backup_dir, &port.folder, library_dir, stamp))
}

fn temp_dir_in(library_dir: &Path, prefix: &str) -> Result<tempfile::TempDir, InstallError> {
    with_path(tempfile::Builder::new().prefix(prefix).tempdir_in(library_dir), library_dir).install_err()
}

/// A port adopted from another folder is a symlink in the library (see `fork::adopt`). Installing or
/// updating through it would write into the user's own folder, so refuse whatever the catalog says.
fn refuse_adopted(dest_dir: &Path) -> Result<(), InstallError> {
    if fs::symlink_metadata(dest_dir).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(InstallError::Message(
            "This port was adopted from another folder, so it is not installed or updated here. Remove its link from the Library folder first if you want a managed copy.".to_string(),
        ));
    }
    Ok(())
}

// Fork (data safety, decision 4): `saves_backup_dir`, so the extras keep the user's saves, game files, ROMs, keys and
// dumps like an update does (`fork::update_merge`).
pub fn install_extra(port: &Port, library_dir: &Path, saves_backup_dir: &Path, on_progress: &mut dyn FnMut(&str)) -> Result<(), InstallError> {
    let url = port.extra.as_deref().ok_or_else(|| InstallError::Message("This port has no \"extra\" files to install.".to_string()))?;
    let dest_dir = port_dir(library_dir, &port.folder).map_err(InstallError::Message)?; // Fork (data safety, D2)
    refuse_adopted(&dest_dir)?;
    if !dest_dir.exists() {
        return Err(InstallError::Message("Install this port first, then add its extra files.".to_string()));
    }
    on_progress("Downloading extra files...");
    let tmp = temp_dir_in(library_dir, "_extra_download_")?;
    let dest = tmp.path().join(safe_download_name(url.rsplit('/').next().unwrap_or("extra")));
    let archive_path = download(url, &dest, on_progress, &DownloadCtl::default())?; // Fork (install wizard)
    let guard = guard_for(port, &dest_dir, library_dir, saves_backup_dir, &crate::fork::update_merge::stamp())?; // Fork (data safety)
    let staging = temp_dir_in(library_dir, "_extra_staging_")?;
    let merge_root = extract_to_staging(&archive_path, staging.path(), None)?;
    // Fork (port dialogs): from here on files in the port's folder are replaced, so a failure after this line may have changed
    // some of them; the extras page says "Nothing was changed" only for a failure before it.
    on_progress(EXTRA_PLACING);
    let report = crate::fork::update_merge::merge(&merge_root, &dest_dir, &guard, &mut move_entry).install_err()?;
    crate::fork::update_merge::record(&dest_dir, report);
    Ok(())
}

// Fork (port dialogs): the progress line `install_extra` sends just before files in the port's folder are replaced; the app
// matches on this constant, not on a copy of the words.
pub const EXTRA_PLACING: &str = "Putting the extra files in place...";

#[derive(Clone, Copy)]
pub struct InstallPaths<'a> {
    pub library_dir: &'a Path,
    pub saves_backup_dir: &'a Path,
}

#[derive(Default, Clone, Copy)]
pub struct InstallOverrides<'a> {
    pub asset: Option<&'a Value>,
    pub release: Option<&'a Value>,
}

fn download_release_asset(
    forge: Forge,
    repo: &str,
    token: Option<&str>,
    preferred: Option<&str>,
    overrides: InstallOverrides,
    tmp_path: &Path,
    on_progress: &mut dyn FnMut(&str),
    ctl: &DownloadCtl, // Fork (install wizard): byte progress and Stop
) -> Result<(PathBuf, Option<String>), InstallError> {
    let (release, asset) = match (overrides.release, overrides.asset) {
        (Some(r), Some(a)) => (r.clone(), a.clone()),
        (Some(r), None) => (r.clone(), forge.pick_release_asset(r, preferred)?),
        (None, Some(a)) => (forge.most_recent_release(repo, token)?, a.clone()),
        (None, None) => forge.latest_installable_release(repo, token, preferred)?,
    };
    let installed_tag = release.get("tag_name").and_then(Value::as_str).map(str::to_string);
    let name = asset.get("name").and_then(Value::as_str).unwrap_or("download");
    let url = asset
        .get(forge.url_field())
        .and_then(Value::as_str)
        .ok_or_else(|| InstallError::Message(format!("Asset without a \"{}\" field -- please report this port.", forge.url_field())))?;
    let archive_path = download(url, &tmp_path.join(safe_download_name(name)), on_progress, ctl)?;
    Ok((archive_path, installed_tag))
}

pub fn install_port(
    port: &Port,
    paths: InstallPaths,
    github_token: Option<&str>,
    gitlab_token: Option<&str>,
    overrides: InstallOverrides,
    on_progress: &mut dyn FnMut(&str),
    ctl: &DownloadCtl, // Fork (install wizard): byte progress and Stop
) -> Result<Option<String>, InstallError> {
    let library_dir = paths.library_dir;
    let dest_dir = port_dir(library_dir, &port.folder).map_err(InstallError::Message)?; // Fork (data safety, D2)
    refuse_adopted(&dest_dir)?;
    let tmp = temp_dir_in(library_dir, "_download_")?;
    let preferred_asset = port.preferred_asset.as_ref().and_then(resolve_preferred_asset);
    let repo = port.repo.as_deref().unwrap_or_default();

    let (archive_path, installed_tag) = if let Some(forge) = Forge::of(port.source_type) {
        let token = forge.token(github_token, gitlab_token);
        download_release_asset(forge, repo, token, preferred_asset.as_deref(), overrides, tmp.path(), on_progress, ctl)?
    } else if port.source_type == SourceType::DirectUrl {
        let resolved = resolve_per_platform(&port.source);
        let url = resolved
            .as_ref()
            .and_then(Value::as_str)
            .ok_or_else(|| InstallError::Message("\"source\" is not a usable link for this port".to_string()))?;
        let dest = tmp.path().join(safe_download_name(url.rsplit('/').next().unwrap_or("download")));
        (download(url, &dest, on_progress, ctl)?, None)
    } else {
        return Err(InstallError::Message("This port has nothing to download.".to_string()));
    };
    // Fork (install wizard): the download has ended: the page's phase turns to installing at once (its Stop goes), and a
    // stop pressed just before is caught here; nothing is installed (`tmp` goes with this return).
    on_progress("Installing...");
    if ctl.cancelled() {
        return Err(InstallError::Message(CANCELLED.to_string()));
    }

    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    apply_archive(port, paths, &archive_path, &dest_dir, &stamp, on_progress, ctl)?;
    on_progress("Done.");
    Ok(installed_tag)
}

pub(crate) fn apply_archive(
    port: &Port,
    paths: InstallPaths,
    archive_path: &Path,
    dest_dir: &Path,
    stamp: &str,
    on_progress: &mut dyn FnMut(&str),
    ctl: &DownloadCtl, // Fork (install wizard): a stop that raced the end of the download
) -> Result<(), InstallError> {
    let InstallPaths { library_dir, saves_backup_dir } = paths;
    let guard = guard_for(port, dest_dir, library_dir, saves_backup_dir, stamp)?; // Fork (data safety): before anything changes
    save_backup::backup_before_update(port, saves_backup_dir, dest_dir, stamp, on_progress)
        .map_err(|e| InstallError::Message(format!("Couldn't back up the saves before installing ({e}) -- nothing was changed.")))?;
    let before = crate::fork::versioned::before_update(port, dest_dir);
    // Fork (install wizard): the last moment a stop can still leave the port as it was (the save backup only adds a zip).
    if ctl.cancelled() {
        return Err(InstallError::Message(CANCELLED.to_string()));
    }
    // Fork (data safety): the user's files in the port folder stay; what was set aside is told on the Done page,
    // with the saves a restore of kept saves moved out of the way (review S3).
    let (release, mut report) = extract_guarded(archive_path, dest_dir, library_dir, port.exe_is_archive.as_deref(), &guard, on_progress)?;
    report.saves_set_aside = save_backup::restore_all_after_install(port, saves_backup_dir, dest_dir);
    crate::fork::update_merge::record(dest_dir, report);
    crate::fork::versioned::record_installed(dest_dir, before, release);
    Ok(())
}

// Fork: the app calls `uninstall_port_keeping`; this keeps upstream's tests and name.
#[cfg(test)]
pub fn uninstall_port(port: &Port, library_dir: &Path, saves_backup_dir: &Path) -> Result<(), String> {
    uninstall_port_keeping(port, library_dir, saves_backup_dir, None).map(|_| ())
}

// Fork: game files, `.o2r` archives, keys, dumps and `.previous/` are moved to Saves Backup/Kept files before the
// folder is removed (fork::uninstall_keep); `config_dir` lets it read the port's connected game files.
pub fn uninstall_port_keeping(port: &Port, library_dir: &Path, saves_backup_dir: &Path, config_dir: Option<&Path>) -> Result<Option<crate::fork::uninstall_keep::Kept>, String> {
    let dest_dir = port_dir(library_dir, &port.folder)?; // Fork (data safety, D2): never the Library itself
    let mut kept = None;
    if dest_dir.exists() {
        if !save_backup::preserve_all_before_uninstall(port, saves_backup_dir, &dest_dir) {
            return Err("Couldn't back up the local save (disk full or permission denied?) -- uninstall cancelled to avoid losing it.".to_string());
        }
        kept = crate::fork::uninstall_keep::keep_before_uninstall(port, &dest_dir, config_dir, saves_backup_dir)?;
        fs::remove_dir_all(&dest_dir).map_err(|e| e.to_string())?;
    }
    Ok(kept)
}

pub fn is_installed(port: &Port, library_dir: &Path) -> bool {
    port_dir(library_dir, &port.folder).is_ok_and(|p| p.exists()) // Fork (data safety, D2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("ports_launcher_installer_test_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn no_progress(_: &str) {}

    fn port_with_saves(folder: &str, save: Option<&str>, save2: Option<&str>) -> Port {
        let mut value = serde_json::json!({"name": "n", "folder": folder, "source": "https://example.com/x.zip"});
        if let Some(s) = save {
            value["save"] = s.into();
        }
        if let Some(s) = save2 {
            value["save2"] = s.into();
        }
        super::super::models::port_from_value(&value).unwrap()
    }

    fn port_with_save(folder: &str, save: Option<&str>) -> Port {
        port_with_saves(folder, save, None)
    }

    #[cfg(unix)]
    #[test]
    fn a_port_adopted_as_a_symlink_is_never_installed_or_updated_through_the_link() {
        let dir = temp_dir("adopted_link");
        let library_dir = dir.join("Library");
        let theirs = dir.join("theirs");
        fs::create_dir_all(&library_dir).unwrap();
        fs::create_dir_all(&theirs).unwrap();
        fs::write(theirs.join("keep.sav"), "mine").unwrap();
        std::os::unix::fs::symlink(&theirs, library_dir.join("game")).unwrap();
        let port = super::super::models::port_from_value(
            &serde_json::json!({"name": "n", "folder": "game", "source": "https://example.com/x.zip", "extra": "https://example.com/e.zip"}),
        )
        .unwrap();
        let saves = dir.join("saves");
        let paths = InstallPaths { library_dir: &library_dir, saves_backup_dir: &saves };
        let text = |e: InstallError| match e {
            InstallError::Message(m) => m,
            other => format!("{other:?}"),
        };
        let err = install_port(&port, paths, None, None, InstallOverrides::default(), &mut no_progress, &DownloadCtl::default()).unwrap_err();
        assert!(text(err).contains("adopted"));
        let err = install_extra(&port, &library_dir, &saves, &mut no_progress).unwrap_err();
        assert!(text(err).contains("adopted"));
        assert_eq!(fs::read_to_string(theirs.join("keep.sav")).unwrap(), "mine");
        assert_eq!(fs::read_dir(&theirs).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&library_dir).unwrap().count(), 1, "no download folder was created");
    }

    // Fork (install wizard).
    #[test]
    fn the_download_copy_reports_its_bytes_and_the_total() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = seen.clone();
        let ctl = DownloadCtl { on_bytes: Some(Arc::new(move |d, t| s.lock().unwrap().push((d, t)))), cancel: None };
        let data = vec![7u8; 150 * 1024];
        let mut out = Vec::new();
        assert_eq!(copy_counted(&mut data.as_slice(), &mut out, Some(data.len() as u64), &ctl).unwrap(), data.len() as u64);
        assert_eq!(out, data);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.first(), Some(&(0, Some(153_600))), "the bar starts at nothing with the size known");
        assert_eq!(seen.last(), Some(&(153_600, Some(153_600))));
        assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0), "the count only grows");
    }

    #[test]
    fn a_stopped_download_ends_with_the_cancelled_error_and_writes_no_more() {
        let flag = Arc::new(AtomicBool::new(true));
        let ctl = DownloadCtl { on_bytes: None, cancel: Some(flag.clone()) };
        let data = vec![1u8; 4096];
        let mut out = Vec::new();
        let err = copy_counted(&mut data.as_slice(), &mut out, None, &ctl).unwrap_err();
        assert_eq!((err.kind(), err.to_string()), (io::ErrorKind::Interrupted, CANCELLED.to_string()));
        assert!(out.is_empty());
        flag.store(false, Ordering::Relaxed);
        assert!(copy_counted(&mut data.as_slice(), &mut out, None, &DownloadCtl::default()).is_ok(), "no control: a plain copy");
    }

    #[test]
    fn reject_if_too_large_accepte_sous_la_limite_et_rejette_au_dessus() {
        assert!(reject_if_too_large(MAX_UNCOMPRESSED_SIZE, MAX_UNCOMPRESSED_SIZE).is_ok());
        assert!(reject_if_too_large(MAX_UNCOMPRESSED_SIZE + 1, MAX_UNCOMPRESSED_SIZE).is_err());
    }

    #[test]
    fn pixeldrain_file_id_extrait_un_lien_de_partage() {
        assert_eq!(pixeldrain_file_id("https://pixeldrain.com/u/XQjoRAjJ"), Some("XQjoRAjJ"));
        assert_eq!(pixeldrain_file_id("http://pixeldrain.com/u/XQjoRAjJ"), Some("XQjoRAjJ"));
        assert_eq!(pixeldrain_file_id("https://pixeldrain.com/u/XQjoRAjJ?x=1"), Some("XQjoRAjJ"));
        assert_eq!(pixeldrain_file_id("https://pixeldrain.com/u/XQjoRAjJ/comments"), Some("XQjoRAjJ"));
        assert_eq!(pixeldrain_file_id("https://pixeldrain.com/u/XQjoRAjJ#preview"), Some("XQjoRAjJ"));
    }

    #[test]
    fn pixeldrain_file_id_ignore_les_liens_deja_directs_ou_non_pixeldrain() {
        assert_eq!(pixeldrain_file_id("https://pixeldrain.com/api/file/XQjoRAjJ"), None);
        assert_eq!(pixeldrain_file_id("https://example.com/u/XQjoRAjJ"), None);
        assert_eq!(pixeldrain_file_id("https://github.com/foo/bar/releases/download/v1/a.zip"), None);
    }

    #[test]
    fn content_disposition_filename_extrait_le_nom_quote() {
        assert_eq!(
            content_disposition_filename("attachment; filename=\"ExtremeGRecompiled-v1.0.0-Windows-RelWithDebInfo.zip\"; filename*=UTF-8''ExtremeGRecompiled-v1.0.0-Windows-RelWithDebInfo.zip"),
            Some("ExtremeGRecompiled-v1.0.0-Windows-RelWithDebInfo.zip".to_string())
        );
    }

    #[test]
    fn content_disposition_filename_gere_le_nom_sans_guillemets() {
        assert_eq!(content_disposition_filename("attachment; filename=a.zip"), Some("a.zip".to_string()));
    }

    #[test]
    fn content_disposition_filename_none_si_absent() {
        assert_eq!(content_disposition_filename("attachment"), None);
    }

    #[test]
    fn has_known_archive_extension_ignore_les_points_dun_numero_de_version() {
        assert!(!has_known_archive_extension("ExtremeGRecompiled-v1.0.0-Windows-RelWithDebInfo"));
        assert!(has_known_archive_extension("ExtremeGRecompiled-v1.0.0-Windows-RelWithDebInfo.zip"));
        assert!(has_known_archive_extension("Foo.Bar.v2.5.TAR.GZ"));
    }

    #[test]
    fn safe_download_name_ne_garde_que_le_nom_final() {
        assert_eq!(safe_download_name("../../Startup/evil.exe"), "evil.exe");
        assert_eq!(safe_download_name(".."), "download");
    }

    fn build_archive(kind: &str, files: &[(&str, &[u8])], archive_path: &Path) {
        let src = archive_path.parent().unwrap().join("_src");
        let _ = fs::remove_dir_all(&src);
        fs::create_dir_all(&src).unwrap();
        for (rel_path, content) in files {
            let path = src.join(rel_path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, content).unwrap();
        }
        let tool = sevenzip_exe_path().expect("7-Zip introuvable, requis par les tests");
        let output =
            std::process::Command::new(&tool).current_dir(&src).arg("a").arg(format!("-t{kind}")).arg(archive_path).arg(".").output().unwrap();
        assert!(output.status.success(), "échec de création de l'archive ({kind}) : {}", String::from_utf8_lossy(&output.stdout));
        fs::remove_dir_all(&src).unwrap();
    }

    fn build_tar_gz_archive(files: &[(&str, &[u8])], archive_path: &Path) {
        build_compressed_tar("gzip", files, archive_path);
    }

    fn build_compressed_tar(kind: &str, files: &[(&str, &[u8])], archive_path: &Path) {
        let tar_path = archive_path.with_extension("");
        build_archive("tar", files, &tar_path);
        let tool = sevenzip_exe_path().unwrap();
        let output = std::process::Command::new(&tool).arg("a").arg(format!("-t{kind}")).arg(archive_path).arg(&tar_path).output().unwrap();
        assert!(output.status.success());
        fs::remove_file(&tar_path).unwrap();
    }

    #[test]
    fn extract_archive_via_7z_rejette_une_archive_trop_grosse() {
        let dir = temp_dir("zipbomb");
        let archive_path = dir.join("bomb.zip");
        build_archive("zip", &[("a.txt", b"hello world")], &archive_path);
        let staging = dir.join("staging");
        fs::create_dir_all(&staging).unwrap();
        assert!(extract_archive_via_7z(&archive_path, &staging, 0).is_err());
    }

    #[test]
    fn extract_archive_via_7z_extrait_un_zip() {
        let dir = temp_dir("zipok");
        let archive_path = dir.join("ok.zip");
        build_archive("zip", &[("sub/a.txt", b"hello world")], &archive_path);
        let staging = dir.join("staging");
        fs::create_dir_all(&staging).unwrap();
        extract_archive_via_7z(&archive_path, &staging, MAX_UNCOMPRESSED_SIZE).unwrap();
        assert_eq!(fs::read_to_string(staging.join("sub").join("a.txt")).unwrap(), "hello world");
    }

    #[test]
    fn extract_archive_via_7z_extrait_un_tar_gz() {
        let dir = temp_dir("targz");
        let archive_path = dir.join("ok.tar.gz");
        build_tar_gz_archive(&[("sub/a.txt", b"hello world")], &archive_path);
        let staging = dir.join("staging");
        fs::create_dir_all(&staging).unwrap();
        extract_archive_via_7z(&archive_path, &staging, MAX_UNCOMPRESSED_SIZE).unwrap();
        assert_eq!(fs::read_to_string(staging.join("sub").join("a.txt")).unwrap(), "hello world");
    }

    #[test]
    fn extract_to_staging_deballe_un_tar_xz() {
        let dir = temp_dir("tarxz");
        let archive_path = dir.join("Game-steamos-x86_64.tar.xz");
        build_compressed_tar("xz", &[("Game/run.sh", b"#!/bin/sh")], &archive_path);
        let staging = dir.join("staging");
        fs::create_dir_all(&staging).unwrap();
        let root = extract_to_staging(&archive_path, &staging, None).unwrap();
        assert_eq!(fs::read_to_string(root.join("run.sh")).unwrap(), "#!/bin/sh");
        assert!(!root.join("Game-steamos-x86_64.tar").exists() && !root.join("Game-steamos-x86_64.tar.xz").exists());
    }

    #[test]
    fn extract_archive_via_7z_extrait_un_7z() {
        let dir = temp_dir("sevenz");
        let archive_path = dir.join("ok.7z");
        build_archive("7z", &[("sub/a.txt", b"hello world")], &archive_path);
        let staging = dir.join("staging");
        fs::create_dir_all(&staging).unwrap();
        extract_archive_via_7z(&archive_path, &staging, MAX_UNCOMPRESSED_SIZE).unwrap();
        assert_eq!(fs::read_to_string(staging.join("sub").join("a.txt")).unwrap(), "hello world");
    }

    fn build_fake_exe_archive(path: &Path, exe_folder: &str) {
        let exe_path = format!("{exe_folder}/game.exe");
        let dll_path = format!("{exe_folder}/game.dll");
        build_archive(
            "zip",
            &[("_rels/.rels", b"<Relationships/>".as_slice()), ("[Content_Types].xml", b"<Types/>".as_slice()), (&exe_path, b"fake-exe".as_slice()), (&dll_path, b"fake-dll".as_slice())],
            path,
        );
    }

    #[test]
    fn extract_exe_is_archive_ne_garde_que_le_dossier_de_l_exe() {
        let dir = temp_dir("exe_archive_ok");
        let archive_path = dir.join("Install.exe");
        build_fake_exe_archive(&archive_path, "lib/net6.0");

        let dest_dir = dir.join("dest");
        extract(&archive_path, &dest_dir, &dir, Some("game.exe"), &mut no_progress).unwrap();

        assert_eq!(fs::read_to_string(dest_dir.join("game.exe")).unwrap(), "fake-exe");
        assert_eq!(fs::read_to_string(dest_dir.join("game.dll")).unwrap(), "fake-dll");
        assert!(!dest_dir.join("_rels").exists());
        assert!(!dest_dir.join("[Content_Types].xml").exists());
        assert!(!dest_dir.join("lib").exists());
    }

    #[test]
    fn extract_exe_is_archive_echoue_si_l_ancre_matche_plusieurs_fichiers() {
        let dir = temp_dir("exe_archive_ambiguous");
        let archive_path = dir.join("Install.exe");
        build_archive("zip", &[("win64/game.exe", b"a".as_slice()), ("win32/game.exe", b"b".as_slice())], &archive_path);
        let dest_dir = dir.join("dest");
        assert!(extract(&archive_path, &dest_dir, &dir, Some("game.exe"), &mut no_progress).is_err());
    }

    #[test]
    fn extract_exe_is_archive_ancre_leve_l_ambiguite() {
        let dir = temp_dir("exe_archive_base_exe");
        let archive_path = dir.join("Install.exe");
        build_archive("zip", &[("redist/vc_redist.x64.exe", b"redist".as_slice()), ("bin/game.exe", b"the-game".as_slice())], &archive_path);

        let dest_dir = dir.join("dest");
        extract(&archive_path, &dest_dir, &dir, Some("game.exe"), &mut no_progress).unwrap();

        assert_eq!(fs::read_to_string(dest_dir.join("game.exe")).unwrap(), "the-game");
        assert!(!dest_dir.join("vc_redist.x64.exe").exists());
        assert!(!dest_dir.join("redist").exists());
    }

    #[test]
    fn extract_exe_is_archive_echoue_proprement_si_ni_zip_ni_nsis() {
        let dir = temp_dir("exe_archive_neither_format");
        let archive_path = dir.join("Install.exe");
        fs::write(&archive_path, b"not a zip, not an nsis installer either").unwrap();
        let dest_dir = dir.join("dest");
        assert!(extract(&archive_path, &dest_dir, &dir, Some("game.exe"), &mut no_progress).is_err());
    }

    #[test]
    fn extract_route_un_rar_vers_7z_plutot_que_de_le_copier() {
        let dir = temp_dir("rar_routed_to_7z");
        let archive_path = dir.join("release.rar");
        fs::write(&archive_path, b"not a real rar file").unwrap();
        let dest_dir = dir.join("dest");
        assert!(extract(&archive_path, &dest_dir, &dir, None, &mut no_progress).is_err());
        assert!(!dest_dir.join("release.rar").exists());
    }

    #[test]
    fn extract_exe_sans_exe_is_archive_copie_l_exe() {
        let dir = temp_dir("exe_not_archive");
        let archive_path = dir.join("portable.exe");
        fs::write(&archive_path, b"MZ-fake-pe-header").unwrap();
        let dest_dir = dir.join("dest");
        extract(&archive_path, &dest_dir, &dir, None, &mut no_progress).unwrap();
        assert_eq!(fs::read(dest_dir.join("portable.exe")).unwrap(), b"MZ-fake-pe-header");
    }

    #[test]
    fn flatten_replie_un_seul_dossier_wrapper() {
        let dir = temp_dir("flatten_one");
        fs::create_dir_all(dir.join("wrapper").join("sub")).unwrap();
        fs::write(dir.join("wrapper").join("game.exe"), b"").unwrap();
        fs::write(dir.join("wrapper").join("sub").join("data.bin"), b"").unwrap();
        flatten_single_wrapper_folder(&dir).unwrap();
        assert!(dir.join("game.exe").exists());
        assert!(dir.join("sub").join("data.bin").exists());
        assert!(!dir.join("wrapper").exists());
    }

    #[test]
    fn flatten_gere_la_collision_de_nom_parent_enfant() {
        let dir = temp_dir("flatten_collision");
        fs::create_dir_all(dir.join("Wrapper").join("Wrapper")).unwrap();
        fs::write(dir.join("Wrapper").join("Wrapper").join("game.exe"), b"").unwrap();
        flatten_single_wrapper_folder(&dir).unwrap();
        assert!(dir.join("game.exe").exists());
    }

    #[test]
    fn flatten_boucle_sur_plusieurs_niveaux() {
        let dir = temp_dir("flatten_multi");
        fs::create_dir_all(dir.join("release-v1.0").join("win64")).unwrap();
        fs::write(dir.join("release-v1.0").join("win64").join("game.exe"), b"").unwrap();
        flatten_single_wrapper_folder(&dir).unwrap();
        assert!(dir.join("game.exe").exists());
    }

    #[test]
    fn flatten_ne_fait_rien_si_plusieurs_entrees() {
        let dir = temp_dir("flatten_noop");
        fs::write(dir.join("a.exe"), b"").unwrap();
        fs::write(dir.join("b.exe"), b"").unwrap();
        flatten_single_wrapper_folder(&dir).unwrap();
        assert!(dir.join("a.exe").exists());
        assert!(dir.join("b.exe").exists());
    }

    #[test]
    fn clear_readonly_recursive_leve_l_attribut_sur_tout_l_arbre() {
        let dir = temp_dir("readonly_clear");
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("sub").join("file.txt"), b"x").unwrap();
        for p in [dir.join("sub"), dir.clone()] {
            let mut perms = fs::metadata(&p).unwrap().permissions();
            perms.set_readonly(true);
            fs::set_permissions(&p, perms).unwrap();
        }

        clear_readonly_recursive(&dir).unwrap();

        assert!(!fs::metadata(&dir).unwrap().permissions().readonly());
        assert!(!fs::metadata(dir.join("sub")).unwrap().permissions().readonly());
        let renamed = dir.with_file_name("readonly_clear_renamed");
        let _ = fs::remove_dir_all(&renamed);
        fs::rename(&dir, &renamed).unwrap();
    }

    #[test]
    fn extract_leve_l_attribut_lecture_seule_herite_de_l_archive() {
        let dir = temp_dir("readonly_archive");
        let src = dir.join("_src");
        fs::create_dir_all(src.join("Wrapper")).unwrap();
        fs::write(src.join("Wrapper").join("game.exe"), b"fake-exe").unwrap();
        let mut perms = fs::metadata(src.join("Wrapper")).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(src.join("Wrapper"), perms).unwrap();

        let archive_path = dir.join("readonly.zip");
        let tool = sevenzip_exe_path().expect("7-Zip introuvable, requis par les tests");
        let output = std::process::Command::new(&tool).current_dir(&src).arg("a").arg("-tzip").arg(&archive_path).arg(".").output().unwrap();
        assert!(output.status.success());

        let dest_dir = dir.join("dest");
        extract(&archive_path, &dest_dir, &dir, None, &mut no_progress).unwrap();
        assert_eq!(fs::read_to_string(dest_dir.join("game.exe")).unwrap(), "fake-exe");
    }

    #[test]
    fn merge_remplace_par_nom_sans_toucher_aux_entrees_absentes_de_src() {
        let dir = temp_dir("merge");
        let src = dir.join("src");
        let dest = dir.join("dest");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dest).unwrap();
        fs::write(src.join("game.exe"), b"new-version").unwrap();
        fs::write(dest.join("game.exe"), b"old-version").unwrap();
        fs::write(dest.join("savegame.dat"), b"my-save").unwrap();

        merge_into(&src, &dest).unwrap();

        assert_eq!(fs::read_to_string(dest.join("game.exe")).unwrap(), "new-version");
        assert_eq!(fs::read_to_string(dest.join("savegame.dat")).unwrap(), "my-save");
    }

    #[test]
    fn merge_remplace_un_fichier_par_un_dossier_et_inversement() {
        let dir = temp_dir("merge_kind_change");
        let src = dir.join("src");
        let dest = dir.join("dest");
        fs::create_dir_all(src.join("data")).unwrap();
        fs::write(src.join("data").join("a.bin"), b"a").unwrap();
        fs::write(src.join("config"), b"file").unwrap();
        fs::create_dir_all(dest.join("config")).unwrap();
        fs::write(dest.join("data"), b"was a file").unwrap();

        merge_into(&src, &dest).unwrap();

        assert_eq!(fs::read_to_string(dest.join("data").join("a.bin")).unwrap(), "a");
        assert_eq!(fs::read_to_string(dest.join("config")).unwrap(), "file");
    }

    fn put(path: &Path, content: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn extract_garde_l_ancien_appimage_et_rend_les_noms_de_la_version() {
        // Fork: nothing is deleted here any more; `fork::versioned` moves superseded files after the update is verified.
        let dir = temp_dir("appimage_update");
        let dest_dir = dir.join("dusklight");
        put(&dest_dir.join("Dusklight-v2.0.2-linux-x86_64.AppImage"), b"old");
        put(&dest_dir.join("saves").join("slot1.dat"), b"precious");
        let archive_path = dir.join("Dusklight-v2.0.3-linux-x86_64.AppImage");
        fs::write(&archive_path, b"new").unwrap();

        let release = extract(&archive_path, &dest_dir, &dir, None, &mut no_progress).unwrap();

        assert_eq!(release, ["Dusklight-v2.0.3-linux-x86_64.AppImage"]);
        assert_eq!(fs::read(dest_dir.join("Dusklight-v2.0.2-linux-x86_64.AppImage")).unwrap(), b"old");
        assert_eq!(fs::read(dest_dir.join("Dusklight-v2.0.3-linux-x86_64.AppImage")).unwrap(), b"new");
        assert_eq!(fs::read(dest_dir.join("saves").join("slot1.dat")).unwrap(), b"precious");
    }

    #[test]
    fn extract_garde_les_appimages_quand_l_archive_n_en_contient_pas() {
        let dir = temp_dir("appimage_kept");
        let dest_dir = dir.join("game");
        put(&dest_dir.join("Game-v1.0-linux-x86_64.AppImage"), b"old");
        let archive_path = dir.join("update.zip");
        build_archive("zip", &[("data.bin", b"new data".as_slice())], &archive_path);

        extract(&archive_path, &dest_dir, &dir, None, &mut no_progress).unwrap();

        assert_eq!(fs::read(dest_dir.join("Game-v1.0-linux-x86_64.AppImage")).unwrap(), b"old");
        assert_eq!(fs::read(dest_dir.join("data.bin")).unwrap(), b"new data");
    }

    #[test]
    fn apply_archive_note_l_etat_d_avant_pour_la_suite_sans_rien_deplacer() {
        let dir = temp_dir("versioned_pending");
        let library_dir = dir.join("Library");
        let dest_dir = library_dir.join("game");
        put(&dest_dir.join("Game-v1.0-linux-x86_64.AppImage"), b"ELF old");
        let port = port_with_save("game", None);
        let archive_path = dir.join("Game-v1.1-linux-x86_64.AppImage");
        fs::write(&archive_path, b"ELF new").unwrap();
        let paths = InstallPaths { library_dir: &library_dir, saves_backup_dir: &dir.join("Saves Backup") };

        apply_archive(&port, paths, &archive_path, &dest_dir, "2026-10-24_12-00-00", &mut no_progress, &DownloadCtl::default()).unwrap();

        assert!(dest_dir.join("Game-v1.0-linux-x86_64.AppImage").is_file(), "nothing moves inside the installer");
        let pending = crate::fork::versioned::take_installed(&dest_dir).expect("the update is waiting to be finished");
        assert_eq!(pending.before.entries, ["Game-v1.0-linux-x86_64.AppImage"]);
        // An AppImage is a launch candidate on Linux only (upstream's Windows list is exe/lnk/bat/url).
        #[cfg(target_os = "linux")]
        assert_eq!(pending.before.target.as_deref(), Some("Game-v1.0-linux-x86_64.AppImage"));
        assert_eq!(pending.release, ["Game-v1.1-linux-x86_64.AppImage"]);
    }

    #[test]
    fn apply_archive_qui_echoue_ne_laisse_rien_a_terminer() {
        let dir = temp_dir("versioned_failed");
        let library_dir = dir.join("Library");
        let dest_dir = library_dir.join("game");
        put(&dest_dir.join("Game-v1.0-linux-x86_64.AppImage"), b"ELF old");
        let port = port_with_save("game", None);
        let archive_path = dir.join("Game-v1.1.zip");
        fs::write(&archive_path, b"not a real zip").unwrap();
        let paths = InstallPaths { library_dir: &library_dir, saves_backup_dir: &dir.join("Saves Backup") };

        assert!(apply_archive(&port, paths, &archive_path, &dest_dir, "2026-10-24_12-00-00", &mut no_progress, &DownloadCtl::default()).is_err());

        assert!(crate::fork::versioned::take_installed(&dest_dir).is_none());
        assert_eq!(fs::read(dest_dir.join("Game-v1.0-linux-x86_64.AppImage")).unwrap(), b"ELF old");
    }

    fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        collect_files(dir, &mut files).unwrap();
        files.sort();
        files.into_iter().map(|path| (path.strip_prefix(dir).unwrap().to_path_buf(), fs::read(&path).unwrap())).collect()
    }

    fn unzip(zip_path: &Path) -> PathBuf {
        let out_dir = zip_path.with_extension("unzipped");
        let _ = fs::remove_dir_all(&out_dir);
        let tool = sevenzip_exe_path().expect("7-Zip introuvable, requis par les tests");
        let output = std::process::Command::new(&tool).arg("x").arg(zip_path).arg(format!("-o{}", out_dir.display())).arg("-y").output().unwrap();
        assert!(output.status.success(), "échec d'extraction du zip : {}", String::from_utf8_lossy(&output.stdout));
        out_dir
    }

    fn two_saves_port(external_save: &Path) -> Port {
        let value = serde_json::json!({"name": "n", "folder": "MyGame", "source": "https://example.com/x.zip", "save": "saves", "save2": external_save.to_str().unwrap()});
        super::super::models::port_from_value(&value).unwrap()
    }

    struct UpdateFixture {
        dir: PathBuf,
        library_dir: PathBuf,
        saves_backup_dir: PathBuf,
        dest_dir: PathBuf,
        external_save: PathBuf,
        port: Port,
    }

    fn update_fixture(name: &str) -> UpdateFixture {
        let dir = temp_dir(name);
        let library_dir = dir.join("Library");
        let dest_dir = library_dir.join("MyGame");
        let external_save = dir.join("home").join(".local").join("share").join("MyGame");
        put(&dest_dir.join("game.exe"), b"old-version");
        put(&dest_dir.join("saves").join("slot1.dat"), b"slot one");
        put(&dest_dir.join("saves").join("sub").join("slot2.dat"), b"slot two");
        put(&external_save.join("memcard.raw"), b"\x00\x01\x02 memory card");
        let port = two_saves_port(&external_save);
        UpdateFixture { saves_backup_dir: dir.join("Saves Backup"), dir, library_dir, dest_dir, external_save, port }
    }

    impl UpdateFixture {
        fn paths(&self) -> InstallPaths<'_> {
            InstallPaths { library_dir: &self.library_dir, saves_backup_dir: &self.saves_backup_dir }
        }

        fn saves(&self) -> (Vec<(PathBuf, Vec<u8>)>, Vec<(PathBuf, Vec<u8>)>) {
            (snapshot(&self.dest_dir.join("saves")), snapshot(&self.external_save))
        }

        fn staging_leftovers(&self) -> Vec<String> {
            let names = fs::read_dir(&self.saves_backup_dir).map(|entries| entries.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect());
            names.unwrap_or_else(|_| Vec::new()).into_iter().filter(|name| name.starts_with("_")).collect()
        }

        fn backup_zip(&self, stamp: &str) -> PathBuf {
            self.saves_backup_dir.join("Update Backups").join("MyGame").join(format!("{stamp}.zip"))
        }
    }

    #[test]
    fn apply_archive_sauvegarde_les_saves_avant_la_maj_et_les_laisse_identiques() {
        let f = update_fixture("update_round_trip");
        let archive_path = f.dir.join("update.zip");
        build_archive("zip", &[("game.exe", b"new-version".as_slice()), ("data/new.bin", b"new data".as_slice())], &archive_path);
        let saves_before = f.saves();

        apply_archive(&f.port, f.paths(), &archive_path, &f.dest_dir, "2026-10-20_12-00-00", &mut no_progress, &DownloadCtl::default()).unwrap();

        assert_eq!(fs::read(f.dest_dir.join("game.exe")).unwrap(), b"new-version");
        assert_eq!(f.saves(), saves_before, "saves must be byte-identical after the update");
        let unzipped = unzip(&f.backup_zip("2026-10-20_12-00-00"));
        assert_eq!(snapshot(&unzipped.join("save_folder")), saves_before.0);
        assert_eq!(snapshot(&unzipped.join("save_folder2")), saves_before.1);
    }

    // Fork (install wizard): a Stop that raced the end of the download is honoured before anything is unpacked.
    #[test]
    fn a_stop_that_raced_the_end_of_the_download_installs_nothing() {
        let f = update_fixture("update_stopped_late");
        let archive_path = f.dir.join("update.zip");
        build_archive("zip", &[("game.exe", b"new-version".as_slice())], &archive_path);
        let saves_before = f.saves();
        let ctl = DownloadCtl { on_bytes: None, cancel: Some(Arc::new(AtomicBool::new(true))) };

        let err = apply_archive(&f.port, f.paths(), &archive_path, &f.dest_dir, "2026-10-20_12-00-00", &mut no_progress, &ctl).unwrap_err();

        assert!(matches!(&err, InstallError::Message(m) if m == CANCELLED), "{err:?}");
        assert_eq!(fs::read(f.dest_dir.join("game.exe")).unwrap(), b"old-version", "nothing was unpacked");
        assert_eq!(f.saves(), saves_before);
        assert!(f.staging_leftovers().is_empty(), "{:?}", f.staging_leftovers());
    }

    #[test]
    fn apply_archive_la_sauvegarde_existe_meme_si_l_installation_echoue() {
        let f = update_fixture("update_fails_after_backup");
        let archive_path = f.dir.join("broken.zip");
        fs::write(&archive_path, b"not a real zip").unwrap();
        let saves_before = f.saves();

        assert!(apply_archive(&f.port, f.paths(), &archive_path, &f.dest_dir, "2026-10-20_12-00-00", &mut no_progress, &DownloadCtl::default()).is_err());

        assert!(f.backup_zip("2026-10-20_12-00-00").is_file(), "the backup is made before the install touches anything");
        assert_eq!(fs::read(f.dest_dir.join("game.exe")).unwrap(), b"old-version");
        assert_eq!(f.saves(), saves_before);
        assert!(f.staging_leftovers().is_empty(), "{:?}", f.staging_leftovers());
    }

    #[test]
    fn apply_archive_n_installe_rien_si_la_sauvegarde_echoue() {
        let f = update_fixture("update_backup_fails");
        fs::write(&f.saves_backup_dir, b"blocks Saves Backup from being a directory").unwrap();
        let archive_path = f.dir.join("update.zip");
        build_archive("zip", &[("game.exe", b"new-version".as_slice())], &archive_path);
        let saves_before = f.saves();

        let error = apply_archive(&f.port, f.paths(), &archive_path, &f.dest_dir, "2026-10-20_12-00-00", &mut no_progress, &DownloadCtl::default()).unwrap_err();

        assert!(matches!(&error, InstallError::Message(m) if m.contains("nothing was changed")), "{error:?}");
        assert!(f.staging_leftovers().is_empty(), "{:?}", f.staging_leftovers());
        assert_eq!(fs::read(f.dest_dir.join("game.exe")).unwrap(), b"old-version");
        assert_eq!(f.saves(), saves_before);
    }

    #[test]
    fn apply_archive_ne_copie_pas_le_jeu_quand_save_designe_le_dossier_du_jeu() {
        let dir = temp_dir("update_save_is_game_dir");
        let library_dir = dir.join("Library");
        let saves_backup_dir = dir.join("Saves Backup");
        let dest_dir = library_dir.join("MyGame");
        put(&dest_dir.join("game.exe"), b"old-version");
        put(&dest_dir.join("assets").join("big.bin"), b"a whole game dump");
        for save in [".", "./"] {
            let port = port_with_save("MyGame", Some(save));
            let archive_path = dir.join("update.zip");
            build_archive("zip", &[("game.exe", b"new-version".as_slice())], &archive_path);

            apply_archive(&port, InstallPaths { library_dir: &library_dir, saves_backup_dir: &saves_backup_dir }, &archive_path, &dest_dir, "2026-10-20_12-00-00", &mut no_progress, &DownloadCtl::default()).unwrap();

            assert_eq!(fs::read(dest_dir.join("game.exe")).unwrap(), b"new-version");
            assert_eq!(fs::read(dest_dir.join("assets").join("big.bin")).unwrap(), b"a whole game dump");
            // Fork (data safety, review C4): no zip of the whole game; the replaced game.exe is kept in Replaced by
            // update instead (with a "." save folder any file may be a save).
            assert!(!saves_backup_dir.join("Update Backups").exists(), "save {save:?} would have duplicated the whole game");
            assert!(saves_backup_dir.join("Replaced by update").join("MyGame").is_dir());
        }
    }

    #[test]
    fn apply_archive_deux_maj_au_meme_instant_gardent_les_deux_sauvegardes() {
        let f = update_fixture("update_same_stamp");
        let archive_path = f.dir.join("update.zip");
        build_archive("zip", &[("game.exe", b"new-version".as_slice())], &archive_path);

        apply_archive(&f.port, f.paths(), &archive_path, &f.dest_dir, "2026-10-20_12-00-00", &mut no_progress, &DownloadCtl::default()).unwrap();
        apply_archive(&f.port, f.paths(), &archive_path, &f.dest_dir, "2026-10-20_12-00-00", &mut no_progress, &DownloadCtl::default()).unwrap();

        assert!(f.backup_zip("2026-10-20_12-00-00").is_file());
        assert!(f.backup_zip("2026-10-20_12-00-00-2").is_file());
    }

    #[test]
    fn apply_archive_sans_sauvegarde_a_faire_ne_cree_pas_de_dossier_de_backup() {
        let dir = temp_dir("update_no_saves");
        let library_dir = dir.join("Library");
        let saves_backup_dir = dir.join("Saves Backup");
        let port = port_with_save("MyGame", Some("saves"));
        let dest_dir = library_dir.join("MyGame");
        put(&dest_dir.join("game.exe"), b"old-version");
        let archive_path = dir.join("update.zip");
        build_archive("zip", &[("game.exe", b"new-version".as_slice())], &archive_path);

        apply_archive(&port, InstallPaths { library_dir: &library_dir, saves_backup_dir: &saves_backup_dir }, &archive_path, &dest_dir, "2026-10-20_12-00-00", &mut no_progress, &DownloadCtl::default()).unwrap();

        assert_eq!(fs::read(dest_dir.join("game.exe")).unwrap(), b"new-version");
        assert!(!saves_backup_dir.exists());
    }

    #[test]
    fn apply_archive_maj_d_un_appimage_sauvegarde_nettoie_et_laisse_les_saves_identiques() {
        let dir = temp_dir("update_appimage");
        let library_dir = dir.join("Library");
        let saves_backup_dir = dir.join("Saves Backup");
        let dest_dir = library_dir.join("dusklight");
        let external_save = dir.join("home").join(".local").join("share").join("TwilitRealm");
        put(&dest_dir.join("Dusklight-v2.0.2-linux-x86_64.AppImage"), b"old");
        put(&external_save.join("Dusklight").join("card1.raw"), b"memory card");
        let value = serde_json::json!({"name": "d", "folder": "dusklight", "source": "https://github.com/TwilitRealm/dusklight", "save": external_save.to_str().unwrap()});
        let port = super::super::models::port_from_value(&value).unwrap();
        let archive_path = dir.join("Dusklight-v2.0.3-linux-x86_64.AppImage");
        fs::write(&archive_path, b"new").unwrap();
        let save_before = snapshot(&external_save);

        apply_archive(&port, InstallPaths { library_dir: &library_dir, saves_backup_dir: &saves_backup_dir }, &archive_path, &dest_dir, "2026-10-20_12-00-00", &mut no_progress, &DownloadCtl::default()).unwrap();

        // Fork: the old file is moved to .previous/ by the app once the update is verified (fork::versioned).
        assert_eq!(fs::read(dest_dir.join("Dusklight-v2.0.2-linux-x86_64.AppImage")).unwrap(), b"old");
        assert_eq!(fs::read(dest_dir.join("Dusklight-v2.0.3-linux-x86_64.AppImage")).unwrap(), b"new");
        assert_eq!(snapshot(&external_save), save_before);
        let backup = saves_backup_dir.join("Update Backups").join("dusklight").join("2026-10-20_12-00-00.zip");
        assert_eq!(snapshot(&unzip(&backup).join("save_folder")), save_before);
    }

    #[test]
    fn uninstall_port_preserve_une_sauvegarde_locale() {
        let dir = temp_dir("uninstall_preserve_save");
        let saves_backup_dir = dir.join("Saves Backup");
        let port = port_with_save("MyGame", Some("Save"));
        let dest_dir = dir.join("MyGame");
        fs::create_dir_all(dest_dir.join("Save")).unwrap();
        fs::write(dest_dir.join("Save").join("slot1.dat"), b"precious").unwrap();
        fs::write(dest_dir.join("game.exe"), b"exe").unwrap();

        uninstall_port(&port, &dir, &saves_backup_dir).unwrap();

        assert!(!dest_dir.exists());
        let backed_up = save_backup::pending_restore_dir(&saves_backup_dir, "MyGame", "save_folder").join("slot1.dat");
        assert_eq!(fs::read_to_string(backed_up).unwrap(), "precious");
    }

    #[test]
    fn uninstall_port_annule_et_ne_supprime_rien_si_la_preservation_echoue() {
        let dir = temp_dir("uninstall_preserve_fails");
        let saves_backup_dir = dir.join("Saves Backup");
        fs::write(&saves_backup_dir, b"blocks Saves Backup from being a directory").unwrap();
        let port = port_with_save("MyGame", Some("Save"));
        let dest_dir = dir.join("MyGame");
        fs::create_dir_all(dest_dir.join("Save")).unwrap();
        fs::write(dest_dir.join("Save").join("slot1.dat"), b"precious").unwrap();

        assert!(uninstall_port(&port, &dir, &saves_backup_dir).is_err());
        assert_eq!(fs::read_to_string(dest_dir.join("Save").join("slot1.dat")).unwrap(), "precious");
    }

    #[test]
    fn uninstall_port_ne_touche_pas_une_sauvegarde_externe_au_dossier_du_jeu() {
        let dir = temp_dir("uninstall_external_save");
        let saves_backup_dir = dir.join("Saves Backup");
        let external_save = dir.join("external_save");
        fs::create_dir_all(&external_save).unwrap();
        fs::write(external_save.join("slot1.dat"), b"precious").unwrap();
        let port = port_with_save("MyGame", Some(external_save.to_str().unwrap()));
        fs::create_dir_all(dir.join("MyGame")).unwrap();

        uninstall_port(&port, &dir, &saves_backup_dir).unwrap();

        assert!(external_save.join("slot1.dat").exists());
        assert!(!save_backup::pending_restore_dir(&saves_backup_dir, "MyGame", "save_folder").exists());
    }

    #[test]
    fn save_et_save2_ne_s_ecrasent_jamais_meme_avec_des_noms_de_fichier_identiques() {
        let dir = temp_dir("two_saves");
        let saves_backup_dir = dir.join("Saves Backup");
        let port = port_with_saves("MyGame", Some("Save"), Some("Save2"));
        let dest_dir = dir.join("MyGame");
        fs::create_dir_all(dest_dir.join("Save")).unwrap();
        fs::write(dest_dir.join("Save").join("slot1.dat"), b"from-save-folder").unwrap();
        fs::create_dir_all(dest_dir.join("Save2")).unwrap();
        fs::write(dest_dir.join("Save2").join("slot1.dat"), b"from-save-folder2").unwrap();
        fs::write(dest_dir.join("game.exe"), b"exe").unwrap();

        uninstall_port(&port, &dir, &saves_backup_dir).unwrap();

        let backup1 = save_backup::pending_restore_dir(&saves_backup_dir, "MyGame", "save_folder").join("slot1.dat");
        let backup2 = save_backup::pending_restore_dir(&saves_backup_dir, "MyGame", "save_folder2").join("slot1.dat");
        assert_eq!(fs::read_to_string(backup1).unwrap(), "from-save-folder");
        assert_eq!(fs::read_to_string(backup2).unwrap(), "from-save-folder2");
    }

    #[test]
    fn is_installed_faux_si_folder_invalide() {
        let dir = temp_dir("is_installed");
        assert!(!is_installed(&port_with_save("../evil", None), &dir));
    }

    // Fork (data safety, D2): a port whose folder name is empty (or ".") is never the Library itself: it is not
    // installed, and an uninstall refuses instead of deleting the whole Library.
    #[test]
    fn an_empty_folder_name_never_means_the_library() {
        let dir = temp_dir("empty_folder_is_not_library");
        let library_dir = dir.join("Library");
        put(&library_dir.join("other").join("game.exe"), b"another port");
        for folder in ["", ".", "./", "sub/.."] {
            let mut port = port_with_save("x", None);
            port.folder = folder.to_string();
            assert!(port_dir(&library_dir, folder).is_err(), "{folder:?}");
            assert!(!is_installed(&port, &library_dir), "{folder:?}");
            assert!(uninstall_port_keeping(&port, &library_dir, &dir.join("Saves Backup"), None).is_err(), "{folder:?}");
            assert!(install_extra(&port, &library_dir, &dir.join("Saves Backup"), &mut no_progress).is_err(), "{folder:?}");
        }
        assert_eq!(fs::read(library_dir.join("other").join("game.exe")).unwrap(), b"another port");
        assert_eq!(port_dir(&library_dir, "game").unwrap(), library_dir.join("game"));
    }

    // Fork (data safety, W2 / D1): an update never replaces the user's files. A ROM, a key, a connected game file, a
    // generated archive and a file in the save folder stay; the release's copies go to `.previous/<stamp>/`. The
    // release's own archive (`soh.o2r`) is replaced, the old one kept in Saves Backup > Replaced by update. Program
    // files are replaced as before.
    #[test]
    fn an_update_keeps_the_users_files_and_sets_the_releases_copies_aside() {
        let dir = temp_dir("update_keeps_user_files");
        let library_dir = dir.join("Library");
        let saves_backup_dir = dir.join("App").join("Saves Backup");
        let dest_dir = library_dir.join("shipwright");
        put(&dest_dir.join("soh.appimage"), b"old program");
        put(&dest_dir.join("soh.o2r"), b"old soh archive");
        put(&dest_dir.join("oot.o2r"), b"generated from my rom");
        put(&dest_dir.join("roms").join("zelda.z64"), b"my rom");
        put(&dest_dir.join("prod.keys"), b"my keys");
        put(&dest_dir.join("mine").join("my copy.n64x"), b"connected copy");
        put(&dest_dir.join("Save").join("file1.dat"), b"my save");
        put(&dest_dir.join("same.o2r"), b"identical");
        let port = super::super::models::port_from_value(&serde_json::json!({"name": "SoH", "folder": "shipwright", "source": "https://example.com/x.zip", "save": "Save"})).unwrap();
        let store = serde_json::json!({"version": 1, "mappings": [{"port_key": port.key(), "slot": "s", "method": "copy", "source": "/elsewhere/zelda.z64", "dest": dest_dir.join("mine").join("my copy.n64x").to_string_lossy(), "size": 14}]});
        put(&dir.join("App").join("rom_library.json"), store.to_string().as_bytes());
        let archive_path = dir.join("update.zip");
        build_archive(
            "zip",
            &[
                ("soh.appimage", b"new program".as_slice()),
                ("soh.o2r", b"new soh archive".as_slice()),
                ("oot.o2r", b"release placeholder".as_slice()),
                ("roms/zelda.z64", b"release rom".as_slice()),
                ("prod.keys", b"release keys".as_slice()),
                ("mine/my copy.n64x", b"release copy".as_slice()),
                ("Save/file1.dat", b"release default save".as_slice()),
                ("same.o2r", b"identical".as_slice()),
                ("new.txt", b"new file".as_slice()),
            ],
            &archive_path,
        );
        let paths = InstallPaths { library_dir: &library_dir, saves_backup_dir: &saves_backup_dir };

        apply_archive(&port, paths, &archive_path, &dest_dir, "2026-11-04_10-00-00", &mut no_progress, &DownloadCtl::default()).unwrap();

        assert_eq!(fs::read(dest_dir.join("soh.appimage")).unwrap(), b"new program");
        assert_eq!(fs::read(dest_dir.join("new.txt")).unwrap(), b"new file");
        assert_eq!(fs::read(dest_dir.join("soh.o2r")).unwrap(), b"new soh archive", "the release's own archive is updated");
        for (rel, mine) in [("oot.o2r", "generated from my rom"), ("roms/zelda.z64", "my rom"), ("prod.keys", "my keys"), ("mine/my copy.n64x", "connected copy"), ("Save/file1.dat", "my save"), ("same.o2r", "identical")] {
            assert_eq!(fs::read_to_string(dest_dir.join(rel)).unwrap(), mine, "{rel} stays the user's");
        }
        let previous = dest_dir.join(".previous").join("update-copies-2026-11-04_10-00-00");
        assert_eq!(fs::read_to_string(previous.join("roms").join("zelda.z64")).unwrap(), "release rom");
        assert_eq!(fs::read_to_string(previous.join("oot.o2r")).unwrap(), "release placeholder");
        assert_eq!(fs::read_to_string(previous.join("Save").join("file1.dat")).unwrap(), "release default save");
        assert!(!previous.join("same.o2r").exists(), "an identical file sets nothing aside");
        let replaced = saves_backup_dir.join("Replaced by update").join("shipwright").join("2026-11-04_10-00-00");
        assert_eq!(fs::read_to_string(replaced.join("soh.o2r")).unwrap(), "old soh archive");
        let report = crate::fork::update_merge::take(&dest_dir).expect("the Done page hears of it");
        assert_eq!(report.kept_yours.len(), 5);
        assert_eq!(report.replaced, [PathBuf::from("soh.o2r")]);
        let line = report.line("update", &|p: &Path| p.file_name().unwrap().to_string_lossy().into_owned()).unwrap();
        assert!(line.starts_with("Kept your files: 5 files in the update had the same name as yours; yours are still in place."), "{line}");
        assert!(line.contains("1 older file replaced by the update is kept in shipwright."), "the place stops at the port's folder: {line}");
        let left: Vec<_> = fs::read_dir(&library_dir).unwrap().flatten().map(|e| e.file_name()).filter(|n| n.to_string_lossy().starts_with('_')).collect();
        assert!(left.is_empty(), "no staging or displaced folder is left: {left:?}");
    }

    // Fork (data safety review C2): an unreadable record of connected game files stops an update before anything changes.
    #[test]
    fn an_unreadable_game_files_record_stops_the_update_and_changes_nothing() {
        let f = update_fixture("update_unreadable_record");
        put(&f.dir.join("rom_library.json"), b"{ damaged");
        let archive_path = f.dir.join("update.zip");
        build_archive("zip", &[("game.exe", b"new-version".as_slice())], &archive_path);
        let err = apply_archive(&f.port, f.paths(), &archive_path, &f.dest_dir, "2026-11-04_10-00-00", &mut no_progress, &DownloadCtl::default()).unwrap_err();
        assert!(matches!(&err, InstallError::Message(m) if m.contains("nothing was changed")), "{err:?}");
        assert_eq!(fs::read(f.dest_dir.join("game.exe")).unwrap(), b"old-version");
        assert!(!f.saves_backup_dir.exists(), "not even the backup zip was made");
    }

    // Fork (data safety review S3): kept saves put back during an update move the newer ones aside, and the Done page
    // hears of it.
    #[test]
    fn an_update_that_puts_kept_saves_back_says_where_the_newer_ones_went() {
        let f = update_fixture("update_replays_kept_saves");
        let pending = save_backup::pending_restore_dir(&f.saves_backup_dir, "MyGame", "save_folder");
        put(&pending.join("slot1.dat"), b"kept by a cancelled uninstall");
        let archive_path = f.dir.join("update.zip");
        build_archive("zip", &[("game.exe", b"new-version".as_slice())], &archive_path);
        apply_archive(&f.port, f.paths(), &archive_path, &f.dest_dir, "2026-11-04_10-00-00", &mut no_progress, &DownloadCtl::default()).unwrap();
        let report = crate::fork::update_merge::take(&f.dest_dir).expect("the Done page hears of it");
        let aside = report.saves_set_aside.clone().expect("the newer saves went somewhere");
        assert_eq!(fs::read(aside.join("save_folder").join("slot1.dat")).unwrap(), b"slot one");
        assert_eq!(fs::read(aside.join("save_folder").join("sub").join("slot2.dat")).unwrap(), b"slot two");
        let line = report.line("update", &|p: &Path| p.file_name().unwrap().to_string_lossy().into_owned()).unwrap();
        assert!(line.contains("the saves that were there were moved to MyGame first"), "{line}");
    }

    // Fork (data safety, decision 4): the extras keep the user's files the same way.
    #[test]
    fn extras_keep_the_users_files_too() {
        let dir = temp_dir("extras_keep_user_files");
        let library_dir = dir.join("Library");
        let saves_backup_dir = dir.join("Saves Backup");
        let dest_dir = library_dir.join("game");
        put(&dest_dir.join("game.sav"), b"my save");
        put(&dest_dir.join("mods").join("tex.png"), b"old texture");
        let src = dir.join("extra");
        put(&src.join("game.sav"), b"extras save");
        put(&src.join("mods").join("tex.png"), b"new texture");
        let port = port_with_save("game", None);
        let guard = guard_for(&port, &dest_dir, &library_dir, &saves_backup_dir, "s").unwrap();
        let report = crate::fork::update_merge::merge(&src, &dest_dir, &guard, &mut move_entry).unwrap();
        assert_eq!(fs::read(dest_dir.join("game.sav")).unwrap(), b"my save");
        assert_eq!(fs::read(dest_dir.join("mods").join("tex.png")).unwrap(), b"new texture", "ordinary files are replaced");
        assert_eq!(report.kept_yours, [PathBuf::from("game.sav")]);
    }

    fn build_adversarial_zip(path: &Path, cycle: usize) {
        let wrapper = format!("Jeu-Recomp-{cycle}-\u{1F3AE}");
        let mut entries: Vec<(String, Vec<u8>)> = (0..20)
            .map(|i| (format!("{wrapper}/data/niveau_{i}_\u{00e9}.bin"), format!("contenu-{cycle}-{i}").into_bytes()))
            .collect();
        entries.push((format!("{wrapper}/game.exe"), b"fake-exe".to_vec()));
        let files: Vec<(&str, &[u8])> = entries.iter().map(|(p, c)| (p.as_str(), c.as_slice())).collect();
        build_archive("zip", &files, path);
    }

    #[test]
    fn cycles_install_desinstall_ne_laissent_aucun_dossier_de_transit() {
        let library_dir = temp_dir("cycle_stress");
        let saves_backup_dir = library_dir.join("Saves Backup");
        const CYCLES: usize = 25;

        for cycle in 0..CYCLES {
            let archive_path = library_dir.join(format!("archive_{cycle}.zip"));
            build_adversarial_zip(&archive_path, cycle);
            let port = port_with_save(&format!("Game{cycle}"), Some("Save"));
            let dest_dir = library_dir.join(&port.folder);

            extract(&archive_path, &dest_dir, &library_dir, None, &mut no_progress).unwrap();
            fs::create_dir_all(dest_dir.join("Save")).unwrap();
            fs::write(dest_dir.join("Save").join("slot.dat"), format!("save-{cycle}")).unwrap();

            uninstall_port(&port, &library_dir, &saves_backup_dir).unwrap();
            assert!(!dest_dir.exists(), "cycle {cycle} : dossier du port non supprimé");
        }

        let leftover: Vec<_> = fs::read_dir(&library_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("_staging_"))
            .collect();
        assert!(leftover.is_empty(), "dossiers de transit restants : {leftover:?}");
    }
}
