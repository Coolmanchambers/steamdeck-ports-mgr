//! What is mounted where: removable drives (SD card, USB) and the file system type of any folder.
//!
//! Read from `/proc/self/mounts` (the kernel's own list) rather than guessed from folder names, so an
//! empty leftover folder under `/run/media` is not offered as a drive, and the file system type of a
//! folder is the one of the deepest mount that holds it.

use crate::core::path_safety::is_within;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    pub device: String,
    pub mount_point: PathBuf,
    pub fs_type: String,
}

/// File systems that cannot hold a Linux port: no program (executable) bits and no links.
pub const NON_LINUX_FS: [&str; 6] = ["vfat", "msdos", "exfat", "ntfs", "ntfs3", "fuseblk"];

pub fn is_non_linux_fs(fs_type: &str) -> bool {
    NON_LINUX_FS.contains(&fs_type)
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

/// How screens name a file system: "Linux format (ext4)", "exFAT", or the kernel name for anything else.
pub fn fs_label(fs_type: &str) -> String {
    match fs_type {
        "ext2" | "ext3" | "ext4" | "btrfs" | "xfs" | "f2fs" => format!("Linux format ({fs_type})"),
        other => fs_display_name(other).to_string(),
    }
}

/// Undoes the octal escapes the kernel uses in the mounts list (`\040` space, `\011` tab, `\012`
/// newline, `\134` backslash).
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 4 <= bytes.len() {
            let digits = &bytes[i + 1..i + 4];
            if digits.iter().all(|d| (b'0'..=b'7').contains(d)) {
                let value = (digits[0] - b'0') as u32 * 64 + (digits[1] - b'0') as u32 * 8 + (digits[2] - b'0') as u32;
                if let Ok(byte) = u8::try_from(value) {
                    out.push(byte);
                    i += 4;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parses the text of `/proc/self/mounts`. Lines that do not have the expected fields are skipped.
pub fn parse_mounts(text: &str) -> Vec<MountEntry> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let device = unescape(fields.next()?);
            let mount_point = PathBuf::from(unescape(fields.next()?));
            let fs_type = unescape(fields.next()?);
            Some(MountEntry { device, mount_point, fs_type })
        })
        .collect()
}

/// A folder named by `DPM_TEST_MOUNTS` that holds the marker file `DPM_REPLAY_OK` stands in for the machine's
/// mounts: its `mounts` file (the `/proc/self/mounts` format) and its `media-root` file (one folder where removable
/// drives appear). It lets a walk of the screens under a virtual display plug in an SD card without mounting
/// anything; without the marker the variable does nothing.
fn test_mounts_dir() -> Option<PathBuf> {
    marked(std::env::var_os("DPM_TEST_MOUNTS"))
}

fn marked(dir: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let dir = PathBuf::from(dir?);
    dir.join("DPM_REPLAY_OK").is_file().then_some(dir)
}

/// The machine's mounts; empty where there is no `/proc` (Windows).
pub fn system_mounts() -> Vec<MountEntry> {
    if let Some(dir) = test_mounts_dir() {
        return std::fs::read_to_string(dir.join("mounts")).map(|t| parse_mounts(&t)).unwrap_or_default();
    }
    std::fs::read_to_string("/proc/self/mounts").map(|t| parse_mounts(&t)).unwrap_or_default()
}

/// The deepest mount holding `path` (compared as written and, when it exists, by its real location).
pub fn mount_of<'a>(path: &Path, mounts: &'a [MountEntry]) -> Option<&'a MountEntry> {
    let real = existing_real_path(path);
    // A later line for the same mount point wins (it was mounted on top).
    mounts
        .iter()
        .enumerate()
        .filter(|(_, m)| is_within(&real, &m.mount_point) || is_within(path, &m.mount_point))
        .max_by_key(|(i, m)| (m.mount_point.components().count(), *i))
        .map(|(_, m)| m)
}

/// The real location of the deepest part of `path` that exists, with the missing rest put back on.
pub fn existing_real_path(path: &Path) -> PathBuf {
    let mut missing: Vec<std::ffi::OsString> = Vec::new();
    let mut probe = path.to_path_buf();
    loop {
        if let Ok(real) = std::fs::canonicalize(&probe) {
            let mut out = real;
            for part in missing.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (probe.file_name().map(|n| n.to_os_string()), probe.parent().map(Path::to_path_buf)) {
            (Some(name), Some(parent)) => {
                missing.push(name);
                probe = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

pub fn fs_type_of(path: &Path, mounts: &[MountEntry]) -> Option<String> {
    mount_of(path, mounts).map(|m| m.fs_type.clone())
}

/// Where removable drives appear: `/run/media/<user>` (SteamOS 3.5 and later, most desktops) and
/// `/run/media` (older SteamOS mounted the SD card as `/run/media/mmcblk0p1`). The user folder is
/// named from `USER` and from the home folder's name, so a launch without `USER` (Steam can start apps
/// with a trimmed environment) still finds it; [`removable_drives`] also takes any mount one folder
/// deeper than a root.
pub fn default_media_roots() -> Vec<PathBuf> {
    if let Some(dir) = test_mounts_dir() {
        return std::fs::read_to_string(dir.join("media-root")).map(|t| vec![PathBuf::from(t.trim())]).unwrap_or_default();
    }
    media_roots_for(std::env::var("USER").ok().as_deref(), std::env::var_os("HOME").map(PathBuf::from).as_deref())
}

pub fn media_roots_for(user: Option<&str>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let home_name = home.and_then(|h| h.file_name()).map(|n| n.to_string_lossy().into_owned());
    for name in [user.map(str::to_string), home_name].into_iter().flatten().filter(|u| !u.is_empty() && !u.contains('/')) {
        let root = PathBuf::from("/run/media").join(name);
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    roots.push(PathBuf::from("/run/media"));
    roots.push(PathBuf::from("/media"));
    roots
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drive {
    pub mount_point: PathBuf,
    /// "SD card", "USB drive" or "Drive".
    pub kind: &'static str,
    /// The folder name the system gave it (usually the drive's label).
    pub label: String,
    pub fs_type: String,
}

impl Drive {
    /// "SD card (SD512)"; a card whose folder is only its ID ("c14f9023-6854-...") is just "SD card".
    pub fn title(&self) -> String {
        if looks_like_an_id(&self.label) {
            self.kind.to_string()
        } else {
            format!("{} ({})", self.kind, self.label)
        }
    }
}

/// A file system ID rather than a name: hex digits and dashes, 8 or more of them (UUIDs, FAT serials like 1A2B-3C4D).
fn looks_like_an_id(label: &str) -> bool {
    label.len() >= 8 && label.contains('-') && label.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// A folder as a place, never a card's ID: "SD card > Emulation > roms", "Home > ROMs". A folder outside the
/// drives and the home folder is its path.
pub fn folder_label(path: &Path, drives: &[Drive], home: Option<&Path>) -> String {
    let crumbs = |base: &str, rest: &Path| {
        let mut parts = vec![base.to_string()];
        parts.extend(rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()));
        parts.join(" > ")
    };
    if let Some(d) = drives.iter().filter(|d| path.starts_with(&d.mount_point)).max_by_key(|d| d.mount_point.as_os_str().len()) {
        // The drive's title ("USB drive (STICK)"; a card named only by its ID is just "SD card"), so two drives differ.
        return crumbs(&d.title(), path.strip_prefix(&d.mount_point).unwrap_or(Path::new("")));
    }
    if let Some(rest) = home.and_then(|h| path.strip_prefix(h).ok()) {
        return crumbs("Home", rest);
    }
    path.display().to_string()
}

fn drive_kind(device: &str) -> &'static str {
    let name = device.rsplit('/').next().unwrap_or(device);
    if name.starts_with("mmcblk") {
        "SD card"
    } else if name.starts_with("sd") {
        "USB drive"
    } else {
        "Drive"
    }
}

/// Mounted removable drives: mounts directly inside one of `media_roots`, or one folder deeper
/// (`/run/media/<user>/<label>` when the user folder is not a root, say `USER` was unset) unless that
/// middle folder is a mount itself (then the deeper mount sits inside a drive). A media root that is
/// itself a mount (a user folder) is not a drive. Sorted SD cards first, then by label.
pub fn removable_drives(mounts: &[MountEntry], media_roots: &[PathBuf]) -> Vec<Drive> {
    let mut drives: Vec<Drive> = Vec::new();
    let is_mount = |p: &Path| mounts.iter().any(|m| m.mount_point == p);
    let is_root = |p: &Path| media_roots.iter().any(|r| r == p);
    for m in mounts {
        let Some(parent) = m.mount_point.parent() else { continue };
        if is_root(&m.mount_point) {
            continue;
        }
        let direct = is_root(parent);
        let deeper = !direct && parent.parent().is_some_and(is_root) && !is_mount(parent);
        if !direct && !deeper {
            continue;
        }
        if drives.iter().any(|d| d.mount_point == m.mount_point) {
            continue;
        }
        let label = m.mount_point.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        drives.push(Drive { mount_point: m.mount_point.clone(), kind: drive_kind(&m.device), label, fs_type: m.fs_type.clone() });
    }
    drives.sort_by_key(|d| (d.kind != "SD card", d.label.to_lowercase()));
    drives
}

/// The removable drive holding `path`, if any.
pub fn drive_of(path: &Path, mounts: &[MountEntry], media_roots: &[PathBuf]) -> Option<Drive> {
    let real = existing_real_path(path);
    removable_drives(mounts, media_roots).into_iter().find(|d| is_within(path, &d.mount_point) || is_within(&real, &d.mount_point))
}

/// For a path under a media root whose drive is not mounted: the drive folder it expects
/// (`/run/media/deck/SD512` for `/run/media/deck/SD512/Ports`). `None` when the path is not on a
/// removable drive location, or its drive is mounted.
pub fn unmounted_drive_for(path: &Path, mounts: &[MountEntry], media_roots: &[PathBuf]) -> Option<PathBuf> {
    // The most specific root first: `/run/media/deck` before `/run/media`.
    let mut roots: Vec<&PathBuf> = media_roots.iter().collect();
    roots.sort_by_key(|r| std::cmp::Reverse(r.components().count()));
    for root in roots {
        let Ok(rest) = path.strip_prefix(root) else { continue };
        let mut parts = rest.components();
        let first = parts.next()?;
        let drive = root.join(first.as_os_str());
        if media_roots.contains(&drive) {
            continue;
        }
        let is_mount = |p: &Path| mounts.iter().any(|m| m.mount_point == p);
        if is_mount(&drive) {
            return None;
        }
        // `/run/media/<user>/<label>` when the user folder is not a root: the drive is one level deeper.
        if let Some(second) = parts.next() {
            let deeper = drive.join(second.as_os_str());
            if is_mount(&deeper) {
                return None;
            }
            if drive.is_dir() || mounts.iter().any(|m| m.mount_point.parent() == Some(drive.as_path())) {
                return Some(deeper);
            }
        }
        return Some(drive);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUNTS: &str = "\
/dev/nvme0n1p8 / btrfs ro,relatime 0 0
/dev/nvme0n1p8 /home ext4 rw,relatime 0 0
tmpfs /run tmpfs rw 0 0
/dev/mmcblk0p1 /run/media/deck/SD512 ext4 rw,nosuid,nodev 0 0
/dev/sda1 /run/media/deck/My\\040Stick exfat rw 0 0
/dev/sdb1 /run/media/OLDCARD vfat rw 0 0
";

    fn roots() -> Vec<PathBuf> {
        vec![PathBuf::from("/run/media/deck"), PathBuf::from("/run/media")]
    }

    #[test]
    fn the_mounts_list_is_parsed_with_escaped_spaces() {
        let m = parse_mounts(MOUNTS);
        assert_eq!(m.len(), 6);
        assert_eq!(m[4].mount_point, PathBuf::from("/run/media/deck/My Stick"));
        assert_eq!(m[4].fs_type, "exfat");
        assert_eq!(unescape("a\\134b\\011c"), "a\\b\tc");
        assert_eq!(unescape("trailing\\04"), "trailing\\04");
        assert!(parse_mounts("garbage\n\n").is_empty());
    }

    #[test]
    fn removable_drives_come_from_mounts_under_the_media_folders_sd_first() {
        let drives = removable_drives(&parse_mounts(MOUNTS), &roots());
        let titles: Vec<String> = drives.iter().map(Drive::title).collect();
        assert_eq!(titles, ["SD card (SD512)", "USB drive (My Stick)", "USB drive (OLDCARD)"]);
        assert_eq!(drives[1].fs_type, "exfat");
        // A media root that is mounted itself is not a drive; /home is not under a media root.
        let with_user_mount = format!("{MOUNTS}tmpfs /run/media/deck tmpfs rw 0 0\n");
        assert_eq!(removable_drives(&parse_mounts(&with_user_mount), &roots()).len(), 3);
    }

    #[test]
    fn the_file_system_of_a_folder_is_the_deepest_mount_holding_it() {
        let m = parse_mounts(MOUNTS);
        assert_eq!(fs_type_of(Path::new("/run/media/deck/SD512/Ports/x"), &m).as_deref(), Some("ext4"));
        assert_eq!(fs_type_of(Path::new("/run/media/deck/My Stick/Ports"), &m).as_deref(), Some("exfat"));
        assert_eq!(fs_type_of(Path::new("/home/deck/Games"), &m).as_deref(), Some("ext4"));
        assert_eq!(fs_type_of(Path::new("/usr/bin"), &m).as_deref(), Some("btrfs"));
        assert!(is_non_linux_fs("exfat") && is_non_linux_fs("vfat") && !is_non_linux_fs("ext4") && !is_non_linux_fs("btrfs"));
    }

    #[test]
    fn a_path_on_a_drive_that_is_not_mounted_names_the_drive_folder() {
        let m = parse_mounts(MOUNTS);
        assert_eq!(unmounted_drive_for(Path::new("/run/media/deck/OTHER/Ports"), &m, &roots()), Some(PathBuf::from("/run/media/deck/OTHER")));
        assert_eq!(unmounted_drive_for(Path::new("/run/media/mmcblk0p1/Ports"), &m, &roots()), Some(PathBuf::from("/run/media/mmcblk0p1")));
        assert_eq!(unmounted_drive_for(Path::new("/run/media/deck/SD512/Ports"), &m, &roots()), None);
        assert_eq!(unmounted_drive_for(Path::new("/home/deck/Ports"), &m, &roots()), None);
        assert_eq!(drive_of(Path::new("/run/media/deck/SD512/Ports"), &m, &roots()).map(|d| d.label), Some("SD512".into()));
        assert_eq!(drive_of(Path::new("/home/deck/Ports"), &m, &roots()), None);
    }

    #[test]
    fn without_user_the_home_name_and_the_deeper_rule_still_find_the_drives() {
        // Steam can start the app without USER: the home folder's name gives the same root.
        assert_eq!(media_roots_for(None, Some(Path::new("/home/deck")))[0], PathBuf::from("/run/media/deck"));
        assert_eq!(media_roots_for(Some("deck"), Some(Path::new("/home/deck"))).len(), 3, "no duplicate root");
        // With neither, only /run/media and /media: drives one level deeper still count.
        let bare = media_roots_for(None, None);
        assert_eq!(bare, [PathBuf::from("/run/media"), PathBuf::from("/media")]);
        let m = parse_mounts(MOUNTS);
        let titles: Vec<String> = removable_drives(&m, &bare).iter().map(Drive::title).collect();
        assert_eq!(titles, ["SD card (SD512)", "USB drive (My Stick)", "USB drive (OLDCARD)"]);
        // And a missing card is still told apart from a mounted one.
        assert_eq!(unmounted_drive_for(Path::new("/run/media/deck/SD512/Ports"), &m, &bare), None);
        assert_eq!(unmounted_drive_for(Path::new("/run/media/deck/OTHER/Ports"), &m, &bare), Some(PathBuf::from("/run/media/deck/OTHER")));
        // A mount inside a drive is not a drive of its own.
        let nested = format!("{MOUNTS}/dev/loop0 /run/media/deck/SD512/iso iso9660 ro 0 0\n");
        assert_eq!(removable_drives(&parse_mounts(&nested), &bare).len(), 3);
    }

    #[test]
    fn labels_with_accents_and_two_sd_cards_are_listed() {
        let text = "/dev/mmcblk0p1 /run/media/deck/Carte\\303\\251 ext4 rw 0 0\n/dev/mmcblk1p1 /run/media/deck/Second ext4 rw 0 0\n";
        let drives = removable_drives(&parse_mounts(text), &roots());
        let titles: Vec<String> = drives.iter().map(Drive::title).collect();
        assert_eq!(titles, ["SD card (Carte\u{e9})", "SD card (Second)"]);
    }
}

#[cfg(test)]
mod test_override_tests {
    use super::*;

    #[test]
    fn the_test_mounts_folder_counts_only_with_its_marker() {
        let dir = std::env::temp_dir().join(format!("dpm_test_mounts_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mounts"), "/dev/mmcblk0p1 /x/run/media/deck/SD512 ext4 rw 0 0\n").unwrap();
        std::fs::write(dir.join("media-root"), "/x/run/media/deck\n").unwrap();
        assert!(marked(Some(dir.clone().into_os_string())).is_none(), "no marker, no effect");
        assert!(marked(None).is_none());
        std::fs::write(dir.join("DPM_REPLAY_OK"), "").unwrap();
        let used = marked(Some(dir.clone().into_os_string())).expect("the marker makes it count");
        let mounts = parse_mounts(&std::fs::read_to_string(used.join("mounts")).unwrap());
        let roots = vec![PathBuf::from(std::fs::read_to_string(used.join("media-root")).unwrap().trim())];
        assert_eq!(roots, [PathBuf::from("/x/run/media/deck")]);
        assert_eq!(removable_drives(&mounts, &roots)[0].title(), "SD card (SD512)");
        let _ = std::fs::remove_dir_all(dir);
    }
}
