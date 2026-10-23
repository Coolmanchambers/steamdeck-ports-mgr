//! Steam artwork pipeline, the part that needs no API key and no network.
//!
//! Three steps, each usable on its own:
//!
//! 1. [`catalog`]: catalog `image` / `icon` bytes (or cached files) become a portrait and an icon
//!    [`fit::Fitted`].
//! 2. [`fit`]: any source image is fitted to the exact size of a Steam slot ([`fit::Slot`]) or refused
//!    with a typed [`fit::FitError`]. Later sources (SteamGridDB, ScreenScraper, TheGamesDB) call
//!    [`fit::fit_bytes`] and hand the result to step 3.
//! 3. [`grid`]: [`grid::write_art`] puts an [`grid::ArtSet`] into a user's `config/grid/` folder under
//!    Steam's file names, never overwrites art the user chose, keeps a provenance manifest, writes the
//!    logo position file once, and returns the path for the shortcut's `icon` field.
//!
//! Typical call from the Add to Steam screen:
//!
//! ```ignore
//! let art = art::catalog::catalog_art_from_files(portrait_cache_path.as_deref(), None);
//! let grid = artwork::grid_dir(&user_dir);
//! let report = art::grid::write_art(&grid, app_id, &art.set, &art::grid::WriteOptions::default())?;
//! // report.icon_path -> ShortcutUpdate::icon / NewShortcut::icon (None: leave the field alone)
//! // art.problems and report.slots say what to show the user per slot.
//! ```
//!
//! Things the caller should know:
//!
//! * `report.icon_path` is `grid_dir.join(..)`: pass an absolute `grid_dir` and it is absolute.
//! * `write_art` calls are serialized inside the process; a second process writing the same folder at
//!   once is not guarded against.
//! * `report.manifest_error` is set when the provenance record could not be saved. The files are on
//!   disk but later runs will treat them as the user's. Show it.
//! * A slot image must have its slot's size (`Fitted::fits_slot`), or the writer fails that slot.
//!   `Slot::size()` is a bounding box for the logo, not an exact size.
//! * Limits worth surfacing to the user: sources over 4096 px a side are refused (as `Decode`),
//!   EXIF rotation is ignored, and a logo under 64 px or a portrait source wider than about 3:1 is
//!   refused.
//! * The app does not cache catalog icons yet; pass the bytes straight to `catalog::catalog_art`.
//!
//! Online sources (added after the no-key half): [`net`] (the one HTTP door), [`sgdb`], [`tgdb`] and
//! [`screenscraper`] (the clients), [`cache`] (ids and images under `cache/art/`), [`source`] and [`title`]
//! (errors, saved per-source state, same-title matching), [`resolve`] (per-slot priority, `art_for_port`) and
//! [`screen`] (the Artwork sources screen's logic). Keys live in `crate::fork::secrets`. See `FORK.md`,
//! "Online art sources and credentials". Only `resolve` with a key set up, and the screen's Test and Fetch
//! rows, ever touch the network.

pub mod cache;
pub mod catalog;
pub mod fit;
pub mod grid;
pub mod net;
pub mod resolve;
#[cfg(test)]
mod resolve_tests;
pub mod screen;
pub mod screenscraper;
pub mod sgdb;
pub mod source;
pub mod thumbs;
pub mod tgdb;
pub mod title;
