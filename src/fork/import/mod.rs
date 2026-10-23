//! Import ports: finds ports installed outside the ports folder (it replaces "Adopt installed ports")
//! and moves them in.
//!
//! - `folders`: the scan folders (well-known ones switchable, the user's own added with the folder
//!   picker) and the "look deeper" switch, saved in `fork-settings.json`;
//! - `scan`: matching folders to catalog ports, how sure each match is, size, and the Steam shortcuts
//!   that point into each find (the shortcuts file is only read);
//! - `plan`: the step-by-step plan for a port (check game files, copy, verify, switch over, repoint or
//!   queue Steam shortcuts, keep the old folder), with the disk checks that can refuse it;
//! - `engine`: the one adapter to whatever moves a port (`move_selected`), the summary, and the STUB
//!   dry-run engine used until the real move engine is on `main`;
//! - `picker`: the folder picker (an adapter over the game-files browser for now);
//! - `fallback`: link a port in place when the move was refused for lack of space (the old adopt);
//! - `screen`: what the lists say, without UI code. The screens are in `src/app/import_screen.rs`.
//!
//! The old adopt code stays in `fork::adopt` (scan matching and the link fallback use it).

pub mod engine;
pub mod fallback;
pub mod folders;
pub mod picker;
pub mod plan;
pub mod scan;
pub mod screen;

use std::path::{Path, PathBuf};

/// Where imported ports go. STUB: the app's library folder until the ports folder setting of the setup
/// work is on `main`; the screen asks only this function, so that change is one line here.
pub fn ports_folder(library_dir: &Path) -> PathBuf {
    library_dir.to_path_buf()
}
