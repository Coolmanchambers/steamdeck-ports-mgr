//! Code added by this fork, kept out of upstream's modules so upstream merges stay small.
//!
//! Steam shortcuts library: read and write the binary `shortcuts.vdf`, compute app IDs, place
//! artwork, pick the Steam account, adopt installs, plan and apply shortcuts. The Settings screens in
//! `src/app/steam_screens.rs` call it.
//!
//! Deck-ready list (`deck_ready`): read, fetch and show `index/deck-ready.json`; the screen is
//! `src/app/deck_ready_screen.rs`.

// Parts of the library (a few helpers, the 64-bit grid IDs) have no caller yet; the tests exercise them.
#![allow(dead_code)]

pub mod add_port;
pub mod adopt;
pub mod app_id;
pub mod art;
pub mod artwork;
pub mod backups;
pub mod catalog_extras;
pub mod checkup;
pub mod cloud;
pub mod deck_ready;
pub mod folder_picker;
pub mod custom_repo;
pub mod import;
pub mod install_wizard;
pub mod list_back; // B on a picker list with a Back row goes up one level
pub mod page;
pub mod play_pages; // Play: "Could not start" and "Which program starts"
pub mod port_page;
pub mod rom_library;
pub mod rom_screens;
pub mod same_game;
pub mod port_mover;
pub mod ports_folder;
pub mod program_detect;
pub mod home;
pub mod ports_folder_pages; // Settings > Ports > Ports folder as pages
pub mod ports_folder_view;
pub mod backup_pages; // Back up saves: the waiting page and the done page
pub mod press_echo;
pub mod save_finder;
pub mod save_notes;
pub mod save_restore;
pub mod screens;
pub mod self_update;
pub mod secrets;
pub mod settings;
pub mod settings_choices;
pub mod settings_menu;
pub mod setup_wizard;
pub mod shortcuts;
pub mod steam_add;
pub mod steam_close;
pub mod steam_icons;
pub mod steam_pages;
pub mod steam_queue;
pub mod steam_sync;
pub mod steam_user;
pub mod store;
pub mod uninstall_keep;
pub mod update_merge; // an update or the extras never replace the user's files (data safety review, 2026-11-04)
pub mod vdf;
pub mod versioned;
