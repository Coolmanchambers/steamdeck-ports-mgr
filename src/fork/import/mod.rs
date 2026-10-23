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
//! - `targets`: what folders are matched against: the catalog plus the Deck-ready list (download names,
//!   and projects the catalog lacks, which importing adds to the user's port list first);
//! - `screen`: what the lists say, without UI code. The screens are in `src/app/import_screen.rs`.
//!
//! The old adopt code stays in `fork::adopt` (scan matching and the link fallback use it).

pub mod cards;
pub mod engine;
pub mod fallback;
pub mod folders;
pub mod old_batch;
pub mod plan;
pub mod real_engine;
pub mod scan;
pub mod screen;
pub mod targets;

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Runs something at most once, however many exits of a flow call it (the wizard's `finish`). Clones share
/// the one flag.
#[derive(Clone, Default)]
pub struct Once(Rc<Cell<bool>>);

impl Once {
    /// Runs `f` the first time; later calls do nothing. True when it ran.
    pub fn run(&self, f: impl FnOnce()) -> bool {
        if self.0.replace(true) {
            return false;
        }
        f();
        true
    }
}

/// Where imported ports go: the ports folder in use (`AppPaths::library_dir()`).
pub fn ports_folder(library_dir: &Path) -> PathBuf {
    library_dir.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_once_runs_the_first_call_only_and_clones_share_it() {
        let once = Once::default();
        let other = once.clone();
        let count = Cell::new(0);
        assert!(once.run(|| count.set(count.get() + 1)));
        assert!(!other.run(|| count.set(count.get() + 1)));
        assert!(!once.run(|| count.set(count.get() + 1)));
        assert_eq!(count.get(), 1, "however many exits call it");
        assert!(Once::default().run(|| {}), "another flow has its own");
    }
}
