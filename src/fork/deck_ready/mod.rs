//! The Deck-ready list inside the app: our own `index/deck-ready.json` (refreshed weekly by a GitHub
//! Action on this fork), fetched with an ETag and saved in `cache/`, shown as a second list beside All
//! ports. `index` reads the file, `sync` fetches and saves it, `view` decides what each row shows and
//! does, `entry` adds an entry the app doesn't have to `ports.local.json`. The screen itself is
//! `src/app/deck_ready_screen.rs`.

pub mod entry;
pub mod index;
pub mod sync;
pub mod view;
