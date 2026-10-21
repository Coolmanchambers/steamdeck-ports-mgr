//! Code added by this fork, kept out of upstream's modules so upstream merges stay small.
//!
//! Steam shortcuts library: read and write the binary `shortcuts.vdf`, compute app IDs, place
//! artwork. Nothing in the app calls it yet.

// No callers until the Steam stage wires it into the app; the tests exercise all of it.
#![allow(dead_code)]

pub mod app_id;
pub mod art;
pub mod artwork;
pub mod shortcuts;
pub mod store;
pub mod vdf;
