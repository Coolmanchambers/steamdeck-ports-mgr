//! Artwork files in a Steam user's `config/grid/` folder, and the Steam folder layout around them.
//!
//! Steam finds artwork by file name: the 32-bit app ID plus a suffix, with any image extension.
//! Cover is `<id>p`, wide capsule `<id>`, hero `<id>_hero`, logo `<id>_logo`, icon `<id>_icon`
//! (the names Steam ROM Manager and SteamGridDB tools use). Old Big Picture clients named files after
//! the 64-bit grid ID; that is not written here.
//!
//! Placing artwork does not need Steam closed (Steam does not rewrite this folder on exit), so unlike
//! `shortcuts.vdf` there is no process check. An existing file is only replaced when asked to be.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const IMAGE_EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];
const ICON_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "ico"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtworkKind {
    /// Portrait capsule (600x900).
    Cover,
    /// Wide capsule (920x430), the default grid image.
    Wide,
    Hero,
    Logo,
    Icon,
}

impl ArtworkKind {
    fn suffix(self) -> &'static str {
        match self {
            ArtworkKind::Cover => "p",
            ArtworkKind::Wide => "",
            ArtworkKind::Hero => "_hero",
            ArtworkKind::Logo => "_logo",
            ArtworkKind::Icon => "_icon",
        }
    }

    fn extensions(self) -> &'static [&'static str] {
        if self == ArtworkKind::Icon {
            &ICON_EXTENSIONS
        } else {
            &IMAGE_EXTENSIONS
        }
    }
}

/// File name without extension, e.g. `2573349430p`.
pub fn grid_file_stem(app_id: u32, kind: ArtworkKind) -> String {
    format!("{app_id}{}", kind.suffix())
}

#[derive(Debug)]
pub enum ArtworkError {
    BadSource { path: PathBuf, reason: &'static str },
    AlreadyExists(PathBuf),
    Io { action: &'static str, path: PathBuf, source: io::Error },
}

impl fmt::Display for ArtworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArtworkError::BadSource { path, reason } => write!(f, "{} cannot be used as artwork: {reason}", path.display()),
            ArtworkError::AlreadyExists(path) => write!(f, "{} already exists", path.display()),
            ArtworkError::Io { action, path, source } => write!(f, "could not {action} {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for ArtworkError {}

fn io_err<'a>(action: &'static str, path: &'a Path) -> impl FnOnce(io::Error) -> ArtworkError + 'a {
    move |source| ArtworkError::Io { action, path: path.to_path_buf(), source }
}

/// Existing files for this ID and kind, whatever their extension.
pub fn existing_artwork(grid_dir: &Path, app_id: u32, kind: ArtworkKind) -> io::Result<Vec<PathBuf>> {
    let stem = grid_file_stem(app_id, kind);
    let mut found = Vec::new();
    let entries = match fs::read_dir(grid_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(found),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let path = entry?.path();
        let same_stem = path.file_stem().is_some_and(|s| s.to_string_lossy() == stem);
        let known_ext = path
            .extension()
            .is_some_and(|e| kind.extensions().contains(&e.to_string_lossy().to_ascii_lowercase().as_str()));
        if same_stem && known_ext {
            found.push(path);
        }
    }
    found.sort();
    Ok(found)
}

/// Copies `source` to `<grid_dir>/<app id><suffix>.<extension of source>` through a temp file and a
/// rename. Without `replace`, any existing artwork for this ID and kind (any extension) stops it.
/// With `replace`, artwork of the same ID and kind under other extensions is removed afterwards so
/// Steam does not pick the stale one. Returns the new file's path.
pub fn place_artwork(
    grid_dir: &Path,
    app_id: u32,
    kind: ArtworkKind,
    source: &Path,
    replace: bool,
) -> Result<PathBuf, ArtworkError> {
    let bad = |reason| ArtworkError::BadSource { path: source.to_path_buf(), reason };
    let ext = source
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .filter(|e| kind.extensions().contains(&e.as_str()))
        .ok_or_else(|| bad("not a supported image type for this artwork kind"))?;
    let meta = fs::metadata(source).map_err(io_err("read", source))?;
    if !meta.is_file() {
        return Err(bad("not a file"));
    }
    if meta.len() == 0 {
        return Err(bad("empty file"));
    }

    let existing = existing_artwork(grid_dir, app_id, kind).map_err(io_err("list", grid_dir))?;
    if !replace {
        if let Some(path) = existing.first() {
            return Err(ArtworkError::AlreadyExists(path.clone()));
        }
    }

    fs::create_dir_all(grid_dir).map_err(io_err("create folder", grid_dir))?;
    let dest = grid_dir.join(format!("{}.{ext}", grid_file_stem(app_id, kind)));
    let temp = grid_dir.join(format!("{}.{ext}.dpm-tmp", grid_file_stem(app_id, kind)));
    let result = (|| {
        let bytes = fs::read(source).map_err(io_err("read", source))?;
        let mut file = File::create(&temp).map_err(io_err("create", &temp))?;
        file.write_all(&bytes).map_err(io_err("write", &temp))?;
        file.sync_all().map_err(io_err("sync", &temp))?;
        drop(file);
        fs::rename(&temp, &dest).map_err(io_err("replace", &dest))
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }

    for stale in existing {
        if stale != dest {
            let _ = fs::remove_file(stale);
        }
    }
    Ok(dest)
}

/// `<steam root>/userdata/<account id>` folders (numeric names; `0` is Steam's placeholder).
pub fn user_dirs(steam_root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(steam_root.join("userdata"))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name != "0" && !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()) && entry.path().is_dir() {
            dirs.push(entry.path());
        }
    }
    dirs.sort();
    Ok(dirs)
}

pub fn shortcuts_path(user_dir: &Path) -> PathBuf {
    user_dir.join("config").join("shortcuts.vdf")
}

pub fn grid_dir(user_dir: &Path) -> PathBuf {
    user_dir.join("config").join("grid")
}

/// Where Steam usually lives under a home folder: the native install, the legacy symlink, Flatpak.
pub fn candidate_steam_roots(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("dpm_steam_art_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn image(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn file_names_follow_steams_convention() {
        let id = 2_573_349_430;
        assert_eq!(grid_file_stem(id, ArtworkKind::Cover), "2573349430p");
        assert_eq!(grid_file_stem(id, ArtworkKind::Wide), "2573349430");
        assert_eq!(grid_file_stem(id, ArtworkKind::Hero), "2573349430_hero");
        assert_eq!(grid_file_stem(id, ArtworkKind::Logo), "2573349430_logo");
        assert_eq!(grid_file_stem(id, ArtworkKind::Icon), "2573349430_icon");
    }

    #[test]
    fn places_each_kind_under_config_grid_with_the_source_extension() {
        let dir = temp_dir("place");
        let grid = dir.join("config").join("grid");
        for (kind, src, want) in [
            (ArtworkKind::Cover, "c.PNG", "77p.png"),
            (ArtworkKind::Wide, "w.jpg", "77.jpg"),
            (ArtworkKind::Hero, "h.webp", "77_hero.webp"),
            (ArtworkKind::Logo, "l.png", "77_logo.png"),
            (ArtworkKind::Icon, "i.ico", "77_icon.ico"),
        ] {
            let source = image(&dir, src, b"image bytes");
            let placed = place_artwork(&grid, 77, kind, &source, false).unwrap();
            assert_eq!(placed, grid.join(want));
            assert_eq!(fs::read(&placed).unwrap(), b"image bytes");
        }
        let mut names: Vec<String> = fs::read_dir(&grid).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["77.jpg", "77_hero.webp", "77_icon.ico", "77_logo.png", "77p.png"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn refuses_to_overwrite_existing_artwork_in_any_extension_unless_told_to() {
        let dir = temp_dir("exists");
        let grid = dir.join("grid");
        fs::create_dir_all(&grid).unwrap();
        image(&grid, "77p.jpg", b"hand made");
        let source = image(&dir, "new.png", b"new");
        let err = place_artwork(&grid, 77, ArtworkKind::Cover, &source, false).unwrap_err();
        assert!(matches!(err, ArtworkError::AlreadyExists(p) if p == grid.join("77p.jpg")));
        assert_eq!(fs::read(grid.join("77p.jpg")).unwrap(), b"hand made");
        assert!(!grid.join("77p.png").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn replace_swaps_the_file_and_removes_other_extensions_of_the_same_artwork_only() {
        let dir = temp_dir("replace");
        let grid = dir.join("grid");
        fs::create_dir_all(&grid).unwrap();
        image(&grid, "77p.jpg", b"old cover");
        image(&grid, "77_hero.jpg", b"hero stays");
        image(&grid, "770p.png", b"other app stays");
        let source = image(&dir, "new.png", b"new cover");
        let placed = place_artwork(&grid, 77, ArtworkKind::Cover, &source, true).unwrap();
        assert_eq!(fs::read(&placed).unwrap(), b"new cover");
        assert!(!grid.join("77p.jpg").exists());
        assert_eq!(fs::read(grid.join("77_hero.jpg")).unwrap(), b"hero stays");
        assert_eq!(fs::read(grid.join("770p.png")).unwrap(), b"other app stays");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn replacing_the_same_extension_overwrites_in_place() {
        let dir = temp_dir("replace_same");
        let grid = dir.join("grid");
        fs::create_dir_all(&grid).unwrap();
        image(&grid, "77.png", b"old");
        let source = image(&dir, "new.png", b"new");
        let placed = place_artwork(&grid, 77, ArtworkKind::Wide, &source, true).unwrap();
        assert_eq!(fs::read(placed).unwrap(), b"new");
        assert_eq!(fs::read_dir(&grid).unwrap().count(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_unusable_sources_without_creating_the_folder() {
        let dir = temp_dir("bad");
        let grid = dir.join("grid");
        let txt = image(&dir, "a.txt", b"x");
        let ico = image(&dir, "a.ico", b"x");
        let empty = image(&dir, "empty.png", b"");
        for (kind, src) in [(ArtworkKind::Cover, &txt), (ArtworkKind::Cover, &ico), (ArtworkKind::Hero, &empty)] {
            assert!(matches!(place_artwork(&grid, 1, kind, src, false), Err(ArtworkError::BadSource { .. })));
        }
        assert!(matches!(place_artwork(&grid, 1, ArtworkKind::Hero, &dir.join("missing.png"), false), Err(ArtworkError::Io { .. })));
        assert!(matches!(place_artwork(&grid, 1, ArtworkKind::Hero, &dir.join("grid.png"), false), Err(ArtworkError::Io { .. })));
        let folder = dir.join("folder.png");
        fs::create_dir(&folder).unwrap();
        assert!(matches!(place_artwork(&grid, 1, ArtworkKind::Hero, &folder, false), Err(ArtworkError::BadSource { .. })));
        assert!(!grid.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn existing_artwork_on_a_missing_folder_is_empty() {
        let dir = temp_dir("nofolder");
        assert!(existing_artwork(&dir.join("nope"), 1, ArtworkKind::Wide).unwrap().is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn user_dirs_lists_numeric_accounts_and_skips_the_placeholder() {
        let dir = temp_dir("users");
        for name in ["0", "1234", "56789", "anonymous"] {
            fs::create_dir_all(dir.join("userdata").join(name)).unwrap();
        }
        fs::write(dir.join("userdata").join("999"), "file").unwrap();
        let dirs = user_dirs(&dir).unwrap();
        assert_eq!(dirs, [dir.join("userdata").join("1234"), dir.join("userdata").join("56789")]);
        assert_eq!(shortcuts_path(&dirs[0]), dir.join("userdata/1234/config/shortcuts.vdf"));
        assert_eq!(grid_dir(&dirs[0]), dir.join("userdata/1234/config/grid"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn candidate_roots_cover_native_legacy_and_flatpak() {
        let roots = candidate_steam_roots(Path::new("/home/x"));
        assert_eq!(roots.len(), 3);
        assert!(roots[2].ends_with("com.valvesoftware.Steam/.local/share/Steam"));
    }
}
