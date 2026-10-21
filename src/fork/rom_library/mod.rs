//! ROM library: find the user's game files and connect them to the ports that need them.
//!
//! Library and tests only; nothing in the app calls it yet (the UI stage wires it in).
//!
//! The flow, in the order the app would use it:
//! 1. [`RomLibraryStore::set_rom_dir`] holds the one setting, the folder the user keeps ROMs in.
//! 2. [`scan::scan`] walks it read-only and fingerprints every ROM-like file (CRC-32, MD5, SHA-1,
//!    plus the big-endian form of N64 images), classified by console from folder names and
//!    extensions.
//! 3. [`requirements::RomSpec::for_port`] reads what a port needs (catalog text or explicit fields in
//!    a local entry) and [`matching::match_port`] says found, ambiguous, wrong version, partial or
//!    missing, by checksum first and by name second (flagged low-confidence).
//! 4. [`connect::plan_connect`] plans use-in-place, symlink or copy for one file; [`connect::apply`]
//!    carries it out (a copy only with approval) and the store records the mapping.
//!
//! Hard rules, tested: originals are only ever read; nothing in a port folder is overwritten; a copy
//! is never made without explicit approval; forgetting a connection removes the record and no files.

pub mod connect;
pub mod hash;
pub mod matching;
pub mod requirements;
pub mod scan;
pub mod store;
pub mod system;

#[cfg(test)]
mod testutil;

#[allow(unused_imports)]
pub use connect::{apply, apply_set, check_mapping, plan_connect, plan_connect_set, Access, ApplyError, Approval, ConnectPlan, ConnectRequest, Health, Mapping, Method, PlanAction, PlanError};
#[allow(unused_imports)]
pub use matching::{match_all, match_port, Basis, FileMatch, PortMatch, PortStatus, SlotResult, SlotStatus};
#[allow(unused_imports)]
pub use requirements::{load_local_entries, parse_instructions, Requirement, RomSpec, Slot, SlotKind};
#[allow(unused_imports)]
pub use scan::{scan as scan_folder, RomFile, ScanOptions, ScanResult};
#[allow(unused_imports)]
pub use store::RomLibraryStore;
#[allow(unused_imports)]
pub use system::{ByteOrder, System};
