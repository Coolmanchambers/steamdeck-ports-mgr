//! Code added by this fork, kept out of upstream's modules so upstream merges stay small.
//!
//! Steam shortcuts library: read and write the binary `shortcuts.vdf`, compute app IDs, place
//! artwork, pick the Steam account, adopt installs, plan and apply shortcuts. The Settings screens in
//! `src/app/steam_screens.rs` call it.

// Parts of the library (a few helpers, the 64-bit grid IDs) have no caller yet; the tests exercise them.
#![allow(dead_code)]

pub mod adopt;
pub mod app_id;
pub mod art;
pub mod artwork;
pub mod backups;
pub mod rom_library;
pub mod screens;
pub mod settings;
pub mod shortcuts;
pub mod steam_sync;
pub mod steam_user;
pub mod store;
pub mod vdf;
