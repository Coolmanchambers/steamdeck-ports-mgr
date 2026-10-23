//! Import ports: finds ports installed outside the ports folder (it replaces "Adopt installed ports")
//! and moves them in.
//!
//! - `folders`: the scan folders (well-known ones switchable, the user's own added with the folder
//!   picker) and the "look deeper" switch, saved in `fork-settings.json`;
//! - `scan`: matching folders to catalog ports, how sure each match is, size, and the Steam shortcuts
//!   that point into each find (the shortcuts file is only read);
//! - `plan`: the step-by-step plan for a port (check game files, copy, verify, switch over, repoint or
//!   queue Steam shortcuts, keep the old folder), with the disk checks that can refuse it;
//! - `engine`: the one adapter to whatever moves a port (`move_selected`), the summary, `Prompter`, and a
//!   dry-run engine for tests;
//! - `real_engine`: the engine the screen uses, over the port mover (copy, verify, game files, switch over,
//!   Steam repoint or queue, old copy kept);
//! - `fallback`: link a port in place when the move was refused for lack of space (the old adopt);
//! - `screen`: what the lists say, without UI code. The screens are in `src/app/import_screen.rs`.
//!
//! The old adopt code stays in `fork::adopt` (scan matching and the link fallback use it).

pub mod engine;
pub mod fallback;
pub mod folders;
pub mod plan;
pub mod real_engine;
pub mod scan;
pub mod screen;

use std::path::{Path, PathBuf};

/// Where imported ports go: the ports folder in use (`AppPaths::library_dir()`).
pub fn ports_folder(library_dir: &Path) -> PathBuf {
    library_dir.to_path_buf()
}
