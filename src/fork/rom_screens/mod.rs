//! Logic for the ROM library screens (stage R2): choose the game files folder, scan it in the
//! background, show what was found per system and port, and connect a port's game files.
//!
//! Everything here is decisions and text with no UI, tested with generated files. The app draws it in
//! `src/app/rom_screens.rs` using the picker dialogs the other fork screens already use, so it works
//! with the controller, keyboard and mouse, in a window or fullscreen. The text is English only.
//!
//! [`find`] is the "find the game files for this port" call other screens use (setup wizard, import);
//! its module docs describe the API.
//!
//! Hard rules, inherited from `rom_library` and kept: the game folder is only read; nothing is
//! overwritten; a copy is never made without the user's explicit yes; forgetting a connection removes
//! the record and no files.

pub mod browse;
pub mod connect_screen;
pub mod connections;
pub mod file_pick;
pub mod find;
pub mod job;
pub mod offer;
pub mod pages;
pub mod report;
pub mod scan_work;
pub mod util;
