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
//! Library only: nothing in the app calls it yet, and nothing here downloads anything.

pub mod catalog;
pub mod fit;
pub mod grid;
