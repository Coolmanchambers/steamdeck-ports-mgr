//! Art from the Ports Launcher catalog: every catalog entry has an `image` (a 600x900 SteamGridDB
//! grid) and, for most, an `icon` (a small square SteamGridDB icon). This turns those bytes into the
//! portrait and icon slots, fitting them if their size is off.
//!
//! Nothing here touches the network. The caller supplies bytes, or the files the app already cached:
//! the portrait lives at `core::image_cache::cached_image_path(cache_dir, port.folder)`
//! ([`cached_portrait_path`] returns it when it exists). The app does not cache the icon yet; whoever
//! downloads it can pass the bytes straight in.
//!
//! Failures are per slot (see [`CatalogArt::problems`]): a bad icon does not cost the portrait.

use std::fs;
use std::path::{Path, PathBuf};

use super::fit::{decode, fit, fit_icon_keep_size, FitError, Fitted, Slot};
use super::grid::ArtSet;

/// Provenance label recorded in the manifest.
pub const SOURCE: &str = "catalog";
/// Catalog images are small; anything bigger is not one.
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct CatalogArt {
    /// Portrait and icon, where the catalog had a usable image for them.
    pub set: ArtSet,
    /// Slots the catalog supplied something for but that could not be used, and why.
    pub problems: Vec<(Slot, FitError)>,
}

/// Portrait from the catalog `image` bytes, icon from the catalog `icon` bytes. `None` means the
/// catalog has none, which is not a problem.
///
/// The portrait is fitted to exactly 600x900 (a catalog image already of that size passes through
/// unchanged apart from being re-encoded as PNG). The icon is kept at its own size when it is square
/// and between 32 and 512 px, which covers the catalog's 48, 128 and 256 px icons; see
/// [`fit_icon_keep_size`].
pub fn catalog_art(image: Option<&[u8]>, icon: Option<&[u8]>) -> CatalogArt {
    let mut out = CatalogArt::default();
    if let Some(bytes) = image {
        match decode(bytes).and_then(|img| fit(Slot::Portrait, &img)) {
            Ok(fitted) => out.set.set(fitted.with_source(SOURCE)),
            Err(e) => out.problems.push((Slot::Portrait, e)),
        }
    }
    if let Some(bytes) = icon {
        match decode(bytes).and_then(|img| fit_icon_keep_size(&img)) {
            Ok(fitted) => out.set.set(fitted.with_source(SOURCE)),
            Err(e) => out.problems.push((Slot::Icon, e)),
        }
    }
    out
}

/// Same, reading the files. An unreadable file is reported as a problem for its slot.
pub fn catalog_art_from_files(image: Option<&Path>, icon: Option<&Path>) -> CatalogArt {
    let mut out = CatalogArt::default();
    let mut bytes_of = |slot: Slot, path: Option<&Path>| -> Option<Vec<u8>> {
        let path = path?;
        match read_limited(path) {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                out.problems.push((slot, e));
                None
            }
        }
    };
    let image_bytes = bytes_of(Slot::Portrait, image);
    let icon_bytes = bytes_of(Slot::Icon, icon);
    let mut fitted = catalog_art(image_bytes.as_deref(), icon_bytes.as_deref());
    fitted.problems.splice(0..0, std::mem::take(&mut out.problems));
    fitted
}

fn read_limited(path: &Path) -> Result<Vec<u8>, FitError> {
    let read_err = |e: std::io::Error| FitError::Read(format!("{}: {e}", path.display()));
    let meta = fs::metadata(path).map_err(read_err)?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return Err(FitError::Read(format!("{} is not a normal image file", path.display())));
    }
    fs::read(path).map_err(read_err)
}

/// The catalog portrait the app already cached for a port (`port.folder`), if it is there.
pub fn cached_portrait_path(cache_dir: &Path, folder: &str) -> Option<PathBuf> {
    crate::core::image_cache::cached_image_path(cache_dir, folder).ok().filter(|p| p.is_file())
}

impl CatalogArt {
    pub fn portrait(&self) -> Option<&Fitted> {
        self.set.portrait.as_ref()
    }

    pub fn icon(&self) -> Option<&Fitted> {
        self.set.icon.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::art::grid::{write_art, SlotOutcome, WriteOptions};
    use image::{Rgba, RgbaImage};

    fn png(w: u32, h: u32, c: [u8; 4]) -> Vec<u8> {
        Fitted { slot: Slot::Portrait, image: RgbaImage::from_pixel(w, h, Rgba(c)), source: String::new(), generated: false }.to_png().unwrap()
    }

    #[test]
    fn a_600x900_catalog_image_becomes_the_portrait_unchanged() {
        let art = catalog_art(Some(&png(600, 900, [10, 20, 30, 255])), None);
        let portrait = art.portrait().unwrap();
        assert_eq!(portrait.image.dimensions(), (600, 900));
        assert_eq!(*portrait.image.get_pixel(300, 450), Rgba([10, 20, 30, 255]));
        assert_eq!(portrait.source, "catalog");
        assert!(!portrait.generated);
        assert!(art.problems.is_empty());
    }

    #[test]
    fn an_off_size_catalog_image_is_fitted() {
        let art = catalog_art(Some(&png(800, 600, [10, 20, 30, 255])), None);
        assert_eq!(art.portrait().unwrap().image.dimensions(), (600, 900));
        assert!(art.portrait().unwrap().generated);
        let art = catalog_art(Some(&png(1200, 1800, [10, 20, 30, 255])), None);
        assert_eq!(art.portrait().unwrap().image.dimensions(), (600, 900));
    }

    #[test]
    fn catalog_icons_keep_their_size() {
        for side in [48, 128, 256, 512] {
            let art = catalog_art(None, Some(&png(side, side, [1, 2, 3, 255])));
            assert_eq!(art.icon().unwrap().image.dimensions(), (side, side), "{side}");
            assert_eq!(art.icon().unwrap().source, "catalog");
        }
        let art = catalog_art(None, Some(&png(1024, 1024, [1, 2, 3, 255])));
        assert_eq!(art.icon().unwrap().image.dimensions(), (512, 512));
    }

    #[test]
    fn nothing_from_the_catalog_is_not_a_problem() {
        let art = catalog_art(None, None);
        assert!(art.set.is_empty() && art.problems.is_empty());
    }

    #[test]
    fn a_bad_icon_does_not_cost_the_portrait() {
        let art = catalog_art(Some(&png(600, 900, [1, 1, 1, 255])), Some(b"<html>404</html>"));
        assert!(art.portrait().is_some());
        assert!(art.icon().is_none());
        assert!(matches!(art.problems.as_slice(), [(Slot::Icon, FitError::Decode(_))]));
    }

    #[test]
    fn a_tiny_portrait_is_refused_not_enlarged() {
        let art = catalog_art(Some(&png(150, 225, [1, 1, 1, 255])), None);
        assert!(art.portrait().is_none());
        assert!(matches!(art.problems.as_slice(), [(Slot::Portrait, FitError::TooSmall { .. })]));
    }

    #[test]
    fn files_are_read_and_unreadable_ones_become_problems() {
        let tmp = tempfile::tempdir().unwrap();
        let cover = tmp.path().join("Game.png");
        fs::write(&cover, png(600, 900, [5, 5, 5, 255])).unwrap();
        let art = catalog_art_from_files(Some(&cover), Some(&tmp.path().join("no-such-icon.png")));
        assert!(art.portrait().is_some());
        assert!(matches!(art.problems.as_slice(), [(Slot::Icon, FitError::Read(_))]));
        let art = catalog_art_from_files(Some(tmp.path()), None);
        assert!(matches!(art.problems.as_slice(), [(Slot::Portrait, FitError::Read(_))]));
    }

    #[test]
    fn cached_portrait_path_only_returns_an_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(cached_portrait_path(tmp.path(), "Game"), None);
        assert_eq!(cached_portrait_path(tmp.path(), "../evil"), None);
        fs::write(tmp.path().join("Game.png"), png(600, 900, [0, 0, 0, 255])).unwrap();
        assert_eq!(cached_portrait_path(tmp.path(), "Game"), Some(tmp.path().join("Game.png")));
    }

    #[test]
    fn catalog_art_goes_through_the_grid_writer_end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        let grid = tmp.path().join("grid");
        let id = 2_573_349_430u32;
        let art = catalog_art(Some(&png(800, 600, [40, 40, 40, 255])), Some(&png(256, 256, [200, 0, 0, 255])));
        let report = write_art(&grid, id, &art.set, &WriteOptions::default()).unwrap();
        assert!(matches!(report.outcome(Slot::Portrait), Some(SlotOutcome::Written(_))));
        assert_eq!(report.icon_path, Some(grid.join(format!("{id}_icon.png"))));
        assert_eq!(image::image_dimensions(grid.join(format!("{id}p.png"))).unwrap(), (600, 900));
        assert_eq!(image::image_dimensions(grid.join(format!("{id}_icon.png"))).unwrap(), (256, 256));
        // No logo, so no position file; no wide or hero either.
        assert!(!grid.join(format!("{id}.json")).exists());
        assert!(!grid.join(format!("{id}_hero.png")).exists());
    }
}
