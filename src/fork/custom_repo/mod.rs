//! "Use a GitHub link...": a port that is not in the catalog, added from its GitHub page.
//!
//! - `parse`: what the user typed or pasted becomes `owner/project`, or a plain-words reason it cannot be;
//! - `check`: what GitHub says about the project and its newest release, and whether that release has a file
//!   for Linux (two calls through `art::net::Fetch`; a fake answers in tests);
//! - `entry`: the user's own port list entry for the project, built and parsed before anything is written.
//!   It is handed to the import as a `NewEntry`, which writes it (backed up) just before the move and takes
//!   it off again if the port does not move.
//!
//! Nothing here downloads a port or writes a file.

pub mod check;
pub mod entry;
pub mod parse;
