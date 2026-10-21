//! Writing fitted slot images into a Steam user's `config/grid/` folder, and remembering which files
//! this app wrote.
//!
//! Placement goes through [`crate::fork::artwork::place_artwork`] (temp file, rename, extension
//! check). On top of that this module decides, per slot, whether the file may be written at all:
//!
//! * No artwork for that ID and slot: write it.
//! * Artwork exists and **every** such file is recorded in the manifest with an unchanged
//!   fingerprint: it is ours and untouched, so it may be refreshed (or left alone if the new image is
//!   byte-identical, or if [`WriteOptions::refresh_own`] is off).
//! * Anything else (a file the manifest does not know, or one that changed since we wrote it, in any
//!   extension): it is the user's choice. It is never overwritten or deleted, and the slot reports
//!   [`SlotOutcome::SkippedUserArt`].
//!
//! The manifest is `.dpm-art-manifest.json` in the same folder (a dotfile Steam ignores). If it is
//! unreadable it is renamed to `.dpm-art-manifest.json.corrupt` and treated as empty, which means every
//! existing file counts as the user's: the safe direction.
//!
//! The logo only shows in Steam with a position file `<id>.json`. It is written when the logo slot
//! ends up holding our logo and the file does not exist yet; an existing one is never touched.
//!
//! Steam does not need to be closed to write here, but it usually shows new art only after a restart.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::fit::{Fitted, Slot};
use crate::fork::app_id::crc32;
use crate::fork::artwork::{existing_artwork, place_artwork};

pub const MANIFEST_FILE: &str = ".dpm-art-manifest.json";
/// Content of `<id>.json` next to a custom logo (a community-reported schema, see the research report).
pub const LOGO_POSITION_JSON: &str = r#"{"nVersion":1,"logoPosition":{"pinnedPosition":"BottomLeft","nWidthPct":50,"nHeightPct":50}}"#;
const MANIFEST_VERSION: u64 = 1;

/// The slot images to write for one shortcut. Leave a slot `None` to leave it alone.
#[derive(Debug, Clone, Default)]
pub struct ArtSet {
    pub portrait: Option<Fitted>,
    pub wide: Option<Fitted>,
    pub hero: Option<Fitted>,
    pub logo: Option<Fitted>,
    pub icon: Option<Fitted>,
}

impl ArtSet {
    pub fn get(&self, slot: Slot) -> Option<&Fitted> {
        match slot {
            Slot::Portrait => self.portrait.as_ref(),
            Slot::Wide => self.wide.as_ref(),
            Slot::Hero => self.hero.as_ref(),
            Slot::Logo => self.logo.as_ref(),
            Slot::Icon => self.icon.as_ref(),
        }
    }

    /// Puts `image` in the slot it was fitted for, replacing what was there.
    pub fn set(&mut self, image: Fitted) {
        let slot = image.slot;
        let target = match slot {
            Slot::Portrait => &mut self.portrait,
            Slot::Wide => &mut self.wide,
            Slot::Hero => &mut self.hero,
            Slot::Logo => &mut self.logo,
            Slot::Icon => &mut self.icon,
        };
        *target = Some(image);
    }

    pub fn is_empty(&self) -> bool {
        Slot::ALL.iter().all(|s| self.get(*s).is_none())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WriteOptions {
    /// Replace art this app wrote earlier (and that has not been touched since) with the new image.
    /// When off, any existing artwork for a slot leaves it alone.
    pub refresh_own: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        WriteOptions { refresh_own: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotOutcome {
    /// The file was written (or replaced our earlier one) at this path.
    Written(PathBuf),
    /// Our own file from an earlier run is already byte-identical.
    Unchanged(PathBuf),
    /// Artwork the user chose (or that we cannot prove is ours) is in place; left alone.
    SkippedUserArt(PathBuf),
    /// Our own earlier file exists and `refresh_own` is off.
    SkippedExisting(PathBuf),
    /// This one slot failed; the other slots were still processed.
    Failed(String),
}

impl SlotOutcome {
    /// The path of our file now in the slot, if there is one.
    pub fn our_path(&self) -> Option<&Path> {
        match self {
            SlotOutcome::Written(p) | SlotOutcome::Unchanged(p) => Some(p),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogoPositionOutcome {
    /// No logo of ours is in place, so no position file was considered.
    NotNeeded,
    Written(PathBuf),
    /// A file already existed and was left exactly as it was.
    AlreadyPresent(PathBuf),
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct WriteReport {
    /// One entry per slot that was in the set, in portrait, wide, hero, logo, icon order.
    pub slots: Vec<(Slot, SlotOutcome)>,
    /// Value for the shortcut's `icon` field (`ShortcutUpdate::icon` / `NewShortcut::icon`): the icon
    /// file of ours now in place. `None` when no icon was given or the user's own icon is there, which
    /// means: leave the `icon` field alone.
    pub icon_path: Option<PathBuf>,
    pub logo_position: LogoPositionOutcome,
}

impl WriteReport {
    pub fn outcome(&self, slot: Slot) -> Option<&SlotOutcome> {
        self.slots.iter().find(|(s, _)| *s == slot).map(|(_, o)| o)
    }
}

#[derive(Debug)]
pub enum GridError {
    CreateFolder { path: PathBuf, source: io::Error },
    /// The manifest could not be saved. Files already written stay, but are not recorded, so a later
    /// run will treat them as the user's.
    ManifestWrite { path: PathBuf, source: io::Error },
}

impl fmt::Display for GridError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GridError::CreateFolder { path, source } => write!(f, "could not create {}: {source}", path.display()),
            GridError::ManifestWrite { path, source } => write!(f, "could not save the artwork record {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for GridError {}

// ---- manifest ---------------------------------------------------------------------------------

/// Content fingerprint: length, CRC-32 and FNV-1a 64. Detects "someone replaced or edited this file"
/// (not an adversary).
fn fingerprint(bytes: &[u8]) -> String {
    let mut fnv: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        fnv ^= u64::from(b);
        fnv = fnv.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{}:{:08x}:{:016x}", bytes.len(), crc32(bytes), fnv)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    pub app_id: u32,
    /// `portrait`, `wide`, `hero`, `logo`, `icon` or `logo-position`.
    pub slot: String,
    pub source: String,
    pub generated: bool,
    pub fingerprint: String,
    pub written_at: String,
}

/// Which files in a grid folder this app wrote.
#[derive(Debug, Clone, Default)]
pub struct Manifest {
    entries: BTreeMap<String, ManifestEntry>,
}

impl Manifest {
    /// Loads the folder's manifest. Missing: empty. Unreadable or from an unknown version: moved
    /// aside as `.corrupt` (best effort) and empty.
    pub fn load(grid_dir: &Path) -> Manifest {
        let path = grid_dir.join(MANIFEST_FILE);
        let Ok(bytes) = fs::read(&path) else { return Manifest::default() };
        match Manifest::parse(&bytes) {
            Some(m) => m,
            None => {
                let _ = fs::rename(&path, grid_dir.join(format!("{MANIFEST_FILE}.corrupt")));
                Manifest::default()
            }
        }
    }

    fn parse(bytes: &[u8]) -> Option<Manifest> {
        let v: Value = serde_json::from_slice(bytes).ok()?;
        if v.get("version")?.as_u64()? != MANIFEST_VERSION {
            return None;
        }
        let mut entries = BTreeMap::new();
        for (name, e) in v.get("files")?.as_object()? {
            entries.insert(
                name.clone(),
                ManifestEntry {
                    app_id: u32::try_from(e.get("app_id")?.as_u64()?).ok()?,
                    slot: e.get("slot")?.as_str()?.to_string(),
                    source: e.get("source")?.as_str()?.to_string(),
                    generated: e.get("generated")?.as_bool()?,
                    fingerprint: e.get("fingerprint")?.as_str()?.to_string(),
                    written_at: e.get("written_at")?.as_str()?.to_string(),
                },
            );
        }
        Some(Manifest { entries })
    }

    fn save(&self, grid_dir: &Path) -> io::Result<()> {
        let mut files = serde_json::Map::new();
        for (name, e) in &self.entries {
            files.insert(
                name.clone(),
                json!({
                    "app_id": e.app_id,
                    "slot": e.slot,
                    "source": e.source,
                    "generated": e.generated,
                    "fingerprint": e.fingerprint,
                    "written_at": e.written_at,
                }),
            );
        }
        let doc = json!({ "version": MANIFEST_VERSION, "files": files });
        let text = serde_json::to_string_pretty(&doc).map_err(io::Error::other)?;
        crate::core::files::write_atomic(&grid_dir.join(MANIFEST_FILE), text.as_bytes())
    }

    /// Every file this app recorded, by file name.
    pub fn entries(&self) -> &BTreeMap<String, ManifestEntry> {
        &self.entries
    }

    /// True when the file is recorded and its bytes still match what we wrote.
    pub fn is_ours(&self, path: &Path) -> bool {
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { return false };
        let Some(entry) = self.entries.get(&name) else { return false };
        fs::read(path).is_ok_and(|bytes| fingerprint(&bytes) == entry.fingerprint)
    }

    fn record(&mut self, path: &Path, app_id: u32, slot: &str, source: &str, generated: bool, bytes: &[u8]) {
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { return };
        let entry = ManifestEntry {
            app_id,
            slot: slot.to_string(),
            source: source.to_string(),
            generated,
            fingerprint: fingerprint(bytes),
            written_at: chrono::Utc::now().to_rfc3339(),
        };
        self.entries.insert(name, entry);
    }

    fn forget(&mut self, path: &Path) {
        if let Some(name) = path.file_name() {
            self.entries.remove(name.to_string_lossy().as_ref());
        }
    }
}

// ---- writing ----------------------------------------------------------------------------------

/// Writes the slots in `art` for `app_id` into `grid_dir` (a user's `config/grid/` folder; created if
/// missing). See the module docs for what is and is not overwritten. A failure in one slot is
/// reported in its outcome and does not stop the others.
pub fn write_art(grid_dir: &Path, app_id: u32, art: &ArtSet, options: &WriteOptions) -> Result<WriteReport, GridError> {
    fs::create_dir_all(grid_dir).map_err(|source| GridError::CreateFolder { path: grid_dir.to_path_buf(), source })?;
    let mut manifest = Manifest::load(grid_dir);
    let mut slots = Vec::new();
    for slot in Slot::ALL {
        let Some(image) = art.get(slot) else { continue };
        let outcome = write_slot(grid_dir, app_id, image, options, &mut manifest)?;
        slots.push((slot, outcome));
    }
    let outcome_of = |slot: Slot| slots.iter().find(|(s, _)| *s == slot).map(|(_, o)| o);
    let icon_path = outcome_of(Slot::Icon).and_then(SlotOutcome::our_path).map(Path::to_path_buf);
    let logo_position = if outcome_of(Slot::Logo).and_then(SlotOutcome::our_path).is_some() {
        write_logo_position(grid_dir, app_id, art.logo.as_ref().map_or("", |l| l.source.as_str()), &mut manifest)?
    } else {
        LogoPositionOutcome::NotNeeded
    };
    Ok(WriteReport { slots, icon_path, logo_position })
}

fn save_manifest(manifest: &Manifest, grid_dir: &Path) -> Result<(), GridError> {
    manifest.save(grid_dir).map_err(|source| GridError::ManifestWrite { path: grid_dir.join(MANIFEST_FILE), source })
}

fn write_slot(grid_dir: &Path, app_id: u32, image: &Fitted, options: &WriteOptions, manifest: &mut Manifest) -> Result<SlotOutcome, GridError> {
    let kind = image.slot.kind();
    let png = match image.to_png() {
        Ok(png) => png,
        Err(e) => return Ok(SlotOutcome::Failed(e.to_string())),
    };
    let existing = match existing_artwork(grid_dir, app_id, kind) {
        Ok(existing) => existing,
        Err(e) => return Ok(SlotOutcome::Failed(format!("could not list {}: {e}", grid_dir.display()))),
    };
    if let Some(theirs) = existing.iter().find(|p| !manifest.is_ours(p)) {
        return Ok(SlotOutcome::SkippedUserArt(theirs.clone()));
    }
    if let Some(first) = existing.first() {
        if !options.refresh_own {
            return Ok(SlotOutcome::SkippedExisting(first.clone()));
        }
        if existing.len() == 1 && fs::read(first).is_ok_and(|bytes| bytes == png) {
            return Ok(SlotOutcome::Unchanged(first.clone()));
        }
    }

    let staging = match tempfile::Builder::new().prefix("dpm-art-").tempdir() {
        Ok(dir) => dir,
        Err(e) => return Ok(SlotOutcome::Failed(format!("could not create a temporary folder: {e}"))),
    };
    let staged = staging.path().join("slot.png");
    if let Err(e) = fs::write(&staged, &png) {
        return Ok(SlotOutcome::Failed(format!("could not stage the image: {e}")));
    }
    let dest = match place_artwork(grid_dir, app_id, kind, &staged, !existing.is_empty()) {
        Ok(dest) => dest,
        Err(e) => return Ok(SlotOutcome::Failed(e.to_string())),
    };
    for old in &existing {
        if !old.exists() {
            manifest.forget(old);
        }
    }
    manifest.record(&dest, app_id, image.slot.name(), &image.source, image.generated, &png);
    save_manifest(manifest, grid_dir)?;
    Ok(SlotOutcome::Written(dest))
}

fn write_logo_position(grid_dir: &Path, app_id: u32, source: &str, manifest: &mut Manifest) -> Result<LogoPositionOutcome, GridError> {
    let path = grid_dir.join(format!("{app_id}.json"));
    let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(LogoPositionOutcome::AlreadyPresent(path)),
        Err(e) => return Ok(LogoPositionOutcome::Failed(format!("could not create {}: {e}", path.display()))),
    };
    let written = file.write_all(LOGO_POSITION_JSON.as_bytes()).and_then(|()| file.sync_all());
    drop(file);
    if let Err(e) = written {
        let _ = fs::remove_file(&path);
        return Ok(LogoPositionOutcome::Failed(format!("could not write {}: {e}", path.display())));
    }
    manifest.record(&path, app_id, "logo-position", source, false, LOGO_POSITION_JSON.as_bytes());
    save_manifest(manifest, grid_dir)?;
    Ok(LogoPositionOutcome::Written(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    const ID: u32 = 4_048_848_997;

    fn fitted(slot: Slot, w: u32, h: u32, shade: u8) -> Fitted {
        Fitted { slot, image: RgbaImage::from_pixel(w, h, Rgba([shade, shade, shade, 255])), source: "test".into(), generated: false }
    }

    fn full_set(shade: u8) -> ArtSet {
        let mut set = ArtSet::default();
        set.set(fitted(Slot::Portrait, 600, 900, shade));
        set.set(fitted(Slot::Wide, 920, 430, shade));
        set.set(fitted(Slot::Hero, 1920, 620, shade));
        set.set(fitted(Slot::Logo, 400, 200, shade));
        set.set(fitted(Slot::Icon, 512, 512, shade));
        set
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn writes_all_five_slots_under_steams_names_and_returns_the_icon_path() {
        let tmp = tempfile::tempdir().unwrap();
        let grid = tmp.path().join("config").join("grid");
        let report = write_art(&grid, ID, &full_set(50), &WriteOptions::default()).unwrap();

        let expected = [
            format!("{ID}.json"),
            format!("{ID}.png"),
            format!("{ID}_hero.png"),
            format!("{ID}_icon.png"),
            format!("{ID}_logo.png"),
            format!("{ID}p.png"),
            MANIFEST_FILE.to_string(),
        ];
        let mut expected = expected.to_vec();
        expected.sort();
        assert_eq!(names(&grid), expected);
        assert!(report.slots.iter().all(|(_, o)| matches!(o, SlotOutcome::Written(_))), "{report:?}");
        assert_eq!(report.icon_path, Some(grid.join(format!("{ID}_icon.png"))));
        for (name, size) in [(format!("{ID}p.png"), (600, 900)), (format!("{ID}.png"), (920, 430)), (format!("{ID}_hero.png"), (1920, 620)), (format!("{ID}_icon.png"), (512, 512))] {
            assert_eq!(image::image_dimensions(grid.join(&name)).unwrap(), size, "{name}");
        }
        assert_eq!(image::image_dimensions(grid.join(format!("{ID}_logo.png"))).unwrap(), (400, 200));
    }

    #[test]
    fn logo_position_file_has_the_exact_content_and_is_written_once() {
        let tmp = tempfile::tempdir().unwrap();
        let report = write_art(tmp.path(), ID, &full_set(50), &WriteOptions::default()).unwrap();
        let json_path = tmp.path().join(format!("{ID}.json"));
        assert_eq!(report.logo_position, LogoPositionOutcome::Written(json_path.clone()));
        let text = fs::read_to_string(&json_path).unwrap();
        assert_eq!(text, r#"{"nVersion":1,"logoPosition":{"pinnedPosition":"BottomLeft","nWidthPct":50,"nHeightPct":50}}"#);
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["logoPosition"]["pinnedPosition"], "BottomLeft");
    }

    #[test]
    fn an_existing_logo_position_file_is_never_touched() {
        let tmp = tempfile::tempdir().unwrap();
        let json_path = tmp.path().join(format!("{ID}.json"));
        fs::write(&json_path, r#"{"nVersion":1,"logoPosition":{"pinnedPosition":"TopRight","nWidthPct":20,"nHeightPct":20}}"#).unwrap();
        let before = fs::read(&json_path).unwrap();
        let report = write_art(tmp.path(), ID, &full_set(50), &WriteOptions::default()).unwrap();
        assert_eq!(report.logo_position, LogoPositionOutcome::AlreadyPresent(json_path.clone()));
        assert_eq!(fs::read(&json_path).unwrap(), before);
        assert!(!Manifest::load(tmp.path()).entries().contains_key(&format!("{ID}.json")));
    }

    #[test]
    fn no_logo_means_no_position_file() {
        let tmp = tempfile::tempdir().unwrap();
        let mut set = ArtSet::default();
        set.set(fitted(Slot::Portrait, 600, 900, 1));
        let report = write_art(tmp.path(), ID, &set, &WriteOptions::default()).unwrap();
        assert_eq!(report.logo_position, LogoPositionOutcome::NotNeeded);
        assert!(!tmp.path().join(format!("{ID}.json")).exists());
        assert_eq!(report.icon_path, None);
        assert_eq!(report.slots.len(), 1);
    }

    #[test]
    fn user_art_in_any_extension_is_never_overwritten_or_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        let theirs_jpg = tmp.path().join(format!("{ID}p.jpg"));
        let theirs_hero = tmp.path().join(format!("{ID}_hero.png"));
        let theirs_logo = tmp.path().join(format!("{ID}_logo.webp"));
        fs::write(&theirs_jpg, b"their cover").unwrap();
        fs::write(&theirs_hero, b"their hero").unwrap();
        fs::write(&theirs_logo, b"their logo").unwrap();
        let report = write_art(tmp.path(), ID, &full_set(50), &WriteOptions::default()).unwrap();

        assert_eq!(report.outcome(Slot::Portrait), Some(&SlotOutcome::SkippedUserArt(theirs_jpg.clone())));
        assert_eq!(report.outcome(Slot::Hero), Some(&SlotOutcome::SkippedUserArt(theirs_hero.clone())));
        assert_eq!(report.outcome(Slot::Logo), Some(&SlotOutcome::SkippedUserArt(theirs_logo.clone())));
        assert_eq!(fs::read(&theirs_jpg).unwrap(), b"their cover");
        assert_eq!(fs::read(&theirs_hero).unwrap(), b"their hero");
        assert_eq!(fs::read(&theirs_logo).unwrap(), b"their logo");
        assert!(!tmp.path().join(format!("{ID}p.png")).exists(), "must not add a second cover next to theirs");
        // Other slots still land, and nothing is recorded for the skipped ones.
        assert!(matches!(report.outcome(Slot::Wide), Some(SlotOutcome::Written(_))));
        assert!(matches!(report.outcome(Slot::Icon), Some(SlotOutcome::Written(_))));
        let manifest = Manifest::load(tmp.path());
        assert!(!manifest.entries().contains_key(&format!("{ID}_hero.png")));
        // Their logo means no position file from us either.
        assert_eq!(report.logo_position, LogoPositionOutcome::NotNeeded);
        assert!(!tmp.path().join(format!("{ID}.json")).exists());
    }

    #[test]
    fn skipped_user_icon_gives_no_icon_path() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(format!("{ID}_icon.ico")), b"\0\0\x01\0 their icon").unwrap();
        let report = write_art(tmp.path(), ID, &full_set(50), &WriteOptions::default()).unwrap();
        assert!(matches!(report.outcome(Slot::Icon), Some(SlotOutcome::SkippedUserArt(_))));
        assert_eq!(report.icon_path, None);
    }

    #[test]
    fn a_second_run_with_the_same_images_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        write_art(tmp.path(), ID, &full_set(50), &WriteOptions::default()).unwrap();
        let before = fs::read(tmp.path().join(MANIFEST_FILE)).unwrap();
        let report = write_art(tmp.path(), ID, &full_set(50), &WriteOptions::default()).unwrap();
        assert!(report.slots.iter().all(|(_, o)| matches!(o, SlotOutcome::Unchanged(_))), "{report:?}");
        assert_eq!(report.icon_path, Some(tmp.path().join(format!("{ID}_icon.png"))));
        assert_eq!(report.logo_position, LogoPositionOutcome::AlreadyPresent(tmp.path().join(format!("{ID}.json"))));
        assert_eq!(fs::read(tmp.path().join(MANIFEST_FILE)).unwrap(), before);
    }

    #[test]
    fn our_own_earlier_art_is_refreshed_with_new_images() {
        let tmp = tempfile::tempdir().unwrap();
        write_art(tmp.path(), ID, &full_set(50), &WriteOptions::default()).unwrap();
        let cover = tmp.path().join(format!("{ID}p.png"));
        let old = fs::read(&cover).unwrap();
        let report = write_art(tmp.path(), ID, &full_set(200), &WriteOptions::default()).unwrap();
        assert_eq!(report.outcome(Slot::Portrait), Some(&SlotOutcome::Written(cover.clone())));
        assert_ne!(fs::read(&cover).unwrap(), old);
        // The record follows the new bytes, so a third run sees the file as ours and unchanged.
        let again = write_art(tmp.path(), ID, &full_set(200), &WriteOptions::default()).unwrap();
        assert_eq!(again.outcome(Slot::Portrait), Some(&SlotOutcome::Unchanged(cover)));
    }

    #[test]
    fn refresh_own_off_leaves_our_earlier_art_alone() {
        let tmp = tempfile::tempdir().unwrap();
        write_art(tmp.path(), ID, &full_set(50), &WriteOptions::default()).unwrap();
        let cover = tmp.path().join(format!("{ID}p.png"));
        let old = fs::read(&cover).unwrap();
        let report = write_art(tmp.path(), ID, &full_set(200), &WriteOptions { refresh_own: false }).unwrap();
        assert_eq!(report.outcome(Slot::Portrait), Some(&SlotOutcome::SkippedExisting(cover.clone())));
        assert_eq!(fs::read(&cover).unwrap(), old);
    }

    #[test]
    fn a_file_the_user_changed_after_we_wrote_it_becomes_theirs() {
        let tmp = tempfile::tempdir().unwrap();
        write_art(tmp.path(), ID, &full_set(50), &WriteOptions::default()).unwrap();
        let hero = tmp.path().join(format!("{ID}_hero.png"));
        fs::write(&hero, b"edited by the user").unwrap();
        let report = write_art(tmp.path(), ID, &full_set(200), &WriteOptions::default()).unwrap();
        assert_eq!(report.outcome(Slot::Hero), Some(&SlotOutcome::SkippedUserArt(hero.clone())));
        assert_eq!(fs::read(&hero).unwrap(), b"edited by the user");
    }

    #[test]
    fn user_art_of_another_extension_next_to_ours_blocks_the_refresh() {
        let tmp = tempfile::tempdir().unwrap();
        write_art(tmp.path(), ID, &full_set(50), &WriteOptions::default()).unwrap();
        let theirs = tmp.path().join(format!("{ID}p.jpg"));
        fs::write(&theirs, b"user picked this in Steam").unwrap();
        let report = write_art(tmp.path(), ID, &full_set(200), &WriteOptions::default()).unwrap();
        assert_eq!(report.outcome(Slot::Portrait), Some(&SlotOutcome::SkippedUserArt(theirs.clone())));
        assert_eq!(fs::read(&theirs).unwrap(), b"user picked this in Steam");
        assert!(tmp.path().join(format!("{ID}p.png")).exists(), "our older file stays too");
    }

    #[test]
    fn other_shortcuts_art_is_not_touched() {
        let tmp = tempfile::tempdir().unwrap();
        let other = tmp.path().join("3000000000p.png");
        fs::write(&other, b"someone else's").unwrap();
        write_art(tmp.path(), ID, &full_set(50), &WriteOptions::default()).unwrap();
        assert_eq!(fs::read(&other).unwrap(), b"someone else's");
    }

    #[test]
    fn manifest_records_where_each_file_came_from() {
        let tmp = tempfile::tempdir().unwrap();
        let mut set = ArtSet::default();
        let mut cover = fitted(Slot::Portrait, 600, 900, 9);
        cover.source = "catalog".into();
        cover.generated = true;
        set.set(cover);
        write_art(tmp.path(), ID, &set, &WriteOptions::default()).unwrap();
        let manifest = Manifest::load(tmp.path());
        let entry = &manifest.entries()[&format!("{ID}p.png")];
        assert_eq!((entry.app_id, entry.slot.as_str(), entry.source.as_str(), entry.generated), (ID, "portrait", "catalog", true));
        assert!(manifest.is_ours(&tmp.path().join(format!("{ID}p.png"))));
        assert!(!manifest.is_ours(&tmp.path().join("missing.png")));
    }

    #[test]
    fn a_corrupt_manifest_is_set_aside_and_nothing_counts_as_ours() {
        let tmp = tempfile::tempdir().unwrap();
        write_art(tmp.path(), ID, &full_set(50), &WriteOptions::default()).unwrap();
        fs::write(tmp.path().join(MANIFEST_FILE), b"{ not json").unwrap();
        let cover = tmp.path().join(format!("{ID}p.png"));
        let before = fs::read(&cover).unwrap();
        let report = write_art(tmp.path(), ID, &full_set(200), &WriteOptions::default()).unwrap();
        assert!(matches!(report.outcome(Slot::Portrait), Some(SlotOutcome::SkippedUserArt(_))));
        assert_eq!(fs::read(&cover).unwrap(), before);
        assert!(tmp.path().join(format!("{MANIFEST_FILE}.corrupt")).exists());
    }

    #[test]
    fn a_manifest_from_an_unknown_version_is_not_trusted() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(MANIFEST_FILE), br#"{"version":99,"files":{}}"#).unwrap();
        assert!(Manifest::load(tmp.path()).entries().is_empty());
        assert!(tmp.path().join(format!("{MANIFEST_FILE}.corrupt")).exists());
    }

    #[test]
    fn something_unreadable_in_a_slots_place_is_left_alone_and_the_others_still_land() {
        let tmp = tempfile::tempdir().unwrap();
        // A folder named like the wide capsule cannot be proven ours, so it counts as the user's.
        let blocker = tmp.path().join(format!("{ID}.png"));
        fs::create_dir(&blocker).unwrap();
        let report = write_art(tmp.path(), ID, &full_set(50), &WriteOptions::default()).unwrap();
        assert_eq!(report.outcome(Slot::Wide), Some(&SlotOutcome::SkippedUserArt(blocker.clone())));
        assert!(blocker.is_dir());
        assert!(matches!(report.outcome(Slot::Portrait), Some(SlotOutcome::Written(_))));
        assert!(matches!(report.outcome(Slot::Icon), Some(SlotOutcome::Written(_))));
    }

    #[test]
    fn no_staging_files_are_left_behind() {
        let tmp = tempfile::tempdir().unwrap();
        write_art(tmp.path(), ID, &full_set(50), &WriteOptions::default()).unwrap();
        assert!(names(tmp.path()).iter().all(|n| !n.contains("tmp") && !n.contains("stage")));
    }

    #[test]
    fn fingerprint_differs_for_different_bytes_and_matches_for_equal() {
        assert_eq!(fingerprint(b"abc"), fingerprint(b"abc"));
        assert_ne!(fingerprint(b"abc"), fingerprint(b"abd"));
        assert_ne!(fingerprint(b"abc"), fingerprint(b"abc "));
    }
}
