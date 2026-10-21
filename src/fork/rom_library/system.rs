//! Which console a file belongs to, and the byte order of Nintendo 64 dumps.
//!
//! Folder names win over extensions: `.iso` and `.bin` mean different things on every console, while
//! a folder called `gc` or `psx` says it plainly. Extensions classify what folders cannot.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum System {
    N64,
    Nes,
    Snes,
    GameBoy,
    GameBoyColor,
    GameBoyAdvance,
    Nds,
    N3ds,
    GameCube,
    Wii,
    WiiU,
    Switch,
    Ps1,
    Ps2,
    Psp,
    Genesis,
    MasterSystem,
    Saturn,
    Dreamcast,
    Xbox,
    Xbox360,
    Arcade,
    Unknown,
}

impl System {
    pub fn label(self) -> &'static str {
        match self {
            System::N64 => "N64",
            System::Nes => "NES",
            System::Snes => "SNES",
            System::GameBoy => "Game Boy",
            System::GameBoyColor => "Game Boy Color",
            System::GameBoyAdvance => "GBA",
            System::Nds => "DS",
            System::N3ds => "3DS",
            System::GameCube => "GameCube",
            System::Wii => "Wii",
            System::WiiU => "Wii U",
            System::Switch => "Switch",
            System::Ps1 => "PS1",
            System::Ps2 => "PS2",
            System::Psp => "PSP",
            System::Genesis => "Genesis",
            System::MasterSystem => "Master System",
            System::Saturn => "Saturn",
            System::Dreamcast => "Dreamcast",
            System::Xbox => "Xbox",
            System::Xbox360 => "Xbox 360",
            System::Arcade => "Arcade",
            System::Unknown => "Other",
        }
    }

    /// Stable lowercase id used in the saved scan cache.
    pub fn id(self) -> &'static str {
        match self {
            System::N64 => "n64",
            System::Nes => "nes",
            System::Snes => "snes",
            System::GameBoy => "gb",
            System::GameBoyColor => "gbc",
            System::GameBoyAdvance => "gba",
            System::Nds => "nds",
            System::N3ds => "3ds",
            System::GameCube => "gc",
            System::Wii => "wii",
            System::WiiU => "wiiu",
            System::Switch => "switch",
            System::Ps1 => "ps1",
            System::Ps2 => "ps2",
            System::Psp => "psp",
            System::Genesis => "genesis",
            System::MasterSystem => "sms",
            System::Saturn => "saturn",
            System::Dreamcast => "dreamcast",
            System::Xbox => "xbox",
            System::Xbox360 => "xbox360",
            System::Arcade => "arcade",
            System::Unknown => "unknown",
        }
    }

    pub fn from_id(id: &str) -> System {
        const ALL: [System; 23] = [
            System::N64,
            System::Nes,
            System::Snes,
            System::GameBoy,
            System::GameBoyColor,
            System::GameBoyAdvance,
            System::Nds,
            System::N3ds,
            System::GameCube,
            System::Wii,
            System::WiiU,
            System::Switch,
            System::Ps1,
            System::Ps2,
            System::Psp,
            System::Genesis,
            System::MasterSystem,
            System::Saturn,
            System::Dreamcast,
            System::Xbox,
            System::Xbox360,
            System::Arcade,
            System::Unknown,
        ];
        ALL.into_iter().find(|s| s.id() == id).unwrap_or(System::Unknown)
    }
}

fn squash(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

/// A folder name that names a console, in the spellings EmuDeck, RetroPie, Batocera and people use.
pub fn system_from_folder_name(name: &str) -> Option<System> {
    Some(match squash(name).as_str() {
        "n64" | "nintendo64" => System::N64,
        "nes" | "famicom" | "fc" | "nintendoentertainmentsystem" => System::Nes,
        "snes" | "sfc" | "superfamicom" | "supernintendo" | "supernes" => System::Snes,
        "gb" | "gameboy" => System::GameBoy,
        "gbc" | "gameboycolor" => System::GameBoyColor,
        "gba" | "gameboyadvance" => System::GameBoyAdvance,
        "nds" | "ds" | "nintendods" => System::Nds,
        "3ds" | "n3ds" | "nintendo3ds" => System::N3ds,
        "gc" | "ngc" | "gamecube" | "nintendogamecube" | "dolphin" => System::GameCube,
        "wii" | "nintendowii" => System::Wii,
        "wiiu" | "nintendowiiu" => System::WiiU,
        "switch" | "nsw" | "nintendoswitch" => System::Switch,
        "ps1" | "psx" | "psone" | "playstation" | "playstation1" | "sonyplaystation" => System::Ps1,
        "ps2" | "playstation2" => System::Ps2,
        "psp" | "playstationportable" => System::Psp,
        "genesis" | "megadrive" | "md" | "segagenesis" | "segamegadrive" => System::Genesis,
        "mastersystem" | "sms" | "segamastersystem" => System::MasterSystem,
        "saturn" | "segasaturn" => System::Saturn,
        "dreamcast" | "dc" | "segadreamcast" => System::Dreamcast,
        "xbox" | "microsoftxbox" | "originalxbox" => System::Xbox,
        "xbox360" | "x360" | "microsoftxbox360" => System::Xbox360,
        "arcade" | "mame" | "fbneo" | "fba" | "neogeo" | "cps1" | "cps2" | "cps3" => System::Arcade,
        _ => return None,
    })
}

/// An extension that names one console. Generic container extensions (`iso`, `bin`, `cue`, `chd`,
/// `zip`) return `None`: only a folder can say what is inside them.
pub fn system_from_extension(ext: &str) -> Option<System> {
    Some(match ext.to_ascii_lowercase().as_str() {
        "z64" | "v64" | "n64" | "ndd" => System::N64,
        "nes" | "unf" | "unif" => System::Nes,
        "sfc" | "smc" | "swc" | "fig" => System::Snes,
        "gb" => System::GameBoy,
        "gbc" => System::GameBoyColor,
        "gba" => System::GameBoyAdvance,
        "nds" | "dsi" => System::Nds,
        "3ds" | "cia" | "cci" | "cxi" => System::N3ds,
        "gcm" | "gcz" | "dol" => System::GameCube,
        "wbfs" | "wad" => System::Wii,
        "wux" | "wua" | "rpx" => System::WiiU,
        "nsp" | "xci" | "nca" | "nro" | "nsz" | "xcz" => System::Switch,
        "pbp" | "cso" => System::Psp,
        "md" | "gen" | "smd" | "32x" => System::Genesis,
        "sms" | "gg" => System::MasterSystem,
        "gdi" | "cdi" => System::Dreamcast,
        "xex" => System::Xbox360,
        _ => return None,
    })
}

/// Extensions the scan hashes. Anything else (images, saves, text) is counted and skipped.
pub fn is_rom_extension(ext: &str) -> bool {
    let e = ext.to_ascii_lowercase();
    system_from_extension(&e).is_some() || matches!(e.as_str(), "iso" | "bin" | "cue" | "chd" | "img" | "xbe" | "xiso" | "swf" | "mdf" | "ciso" | "rvz" | "zip" | "7z" | "rom" | "a26" | "pce" | "wsc" | "ws" | "ngp" | "ngc" | "dat" | "cdt" | "tap")
}

/// Classify a file by its path relative to the ROM folder. The folder closest to the file that names
/// a console wins; then the extension; otherwise `Unknown`.
pub fn classify(relative_path: &Path) -> System {
    let ext = relative_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let dirs: Vec<&str> = relative_path.parent().map(|p| p.iter().filter_map(|c| c.to_str()).collect()).unwrap_or_default();
    for dir in dirs.iter().rev() {
        if let Some(system) = system_from_folder_name(dir) {
            return system;
        }
    }
    system_from_extension(ext).unwrap_or(System::Unknown)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrder {
    /// `.z64`: the order the console uses and the one checksum lists are computed over.
    BigEndian,
    /// `.v64`: every pair of bytes swapped (Doctor V64 backup unit).
    ByteSwapped,
    /// `.n64`: every group of four bytes reversed.
    LittleEndian,
}

impl ByteOrder {
    pub fn label(self) -> &'static str {
        match self {
            ByteOrder::BigEndian => "big-endian (.z64)",
            ByteOrder::ByteSwapped => "byte-swapped (.v64)",
            ByteOrder::LittleEndian => "little-endian (.n64)",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            ByteOrder::BigEndian => "z64",
            ByteOrder::ByteSwapped => "v64",
            ByteOrder::LittleEndian => "n64",
        }
    }

    pub fn from_id(id: &str) -> Option<ByteOrder> {
        match id {
            "z64" => Some(ByteOrder::BigEndian),
            "v64" => Some(ByteOrder::ByteSwapped),
            "n64" => Some(ByteOrder::LittleEndian),
            _ => None,
        }
    }
}

/// The first four bytes of every N64 cartridge image are the PI bus configuration, `80 37 12 40`
/// in the console's own byte order. The three dump formats show that word differently.
pub fn detect_n64_byte_order(header: &[u8]) -> Option<ByteOrder> {
    match header.get(..4)? {
        [0x80, 0x37, 0x12, 0x40] => Some(ByteOrder::BigEndian),
        [0x37, 0x80, 0x40, 0x12] => Some(ByteOrder::ByteSwapped),
        [0x40, 0x12, 0x37, 0x80] => Some(ByteOrder::LittleEndian),
        _ => None,
    }
}

/// Convert whole four-byte groups of `buf` to big-endian in place. A trailing partial group (a file
/// whose size is not a multiple of four) is left alone. Applying it to a chunk that starts on a
/// four-byte boundary of the file gives the same bytes as converting the whole file at once.
pub fn normalize_to_big_endian(order: ByteOrder, buf: &mut [u8]) {
    match order {
        ByteOrder::BigEndian => {}
        ByteOrder::ByteSwapped => {
            for group in buf.chunks_exact_mut(4) {
                group.swap(0, 1);
                group.swap(2, 3);
            }
        }
        ByteOrder::LittleEndian => {
            for group in buf.chunks_exact_mut(4) {
                group.reverse();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn folder_name_wins_over_extension() {
        assert_eq!(classify(&PathBuf::from("gc/Twilight Princess.iso")), System::GameCube);
        assert_eq!(classify(&PathBuf::from("ps2/Game.iso")), System::Ps2);
        assert_eq!(classify(&PathBuf::from("psx/Game.bin")), System::Ps1);
        assert_eq!(classify(&PathBuf::from("snes/odd.z64")), System::Snes);
    }

    #[test]
    fn closest_folder_that_names_a_console_wins() {
        assert_eq!(classify(&PathBuf::from("roms/psx/N64 ports/game.bin")), System::Ps1);
        assert_eq!(classify(&PathBuf::from("n64/psx/game.bin")), System::Ps1);
        assert_eq!(classify(&PathBuf::from("Games/N64/Hacks/rom.z64")), System::N64);
    }

    #[test]
    fn folder_spellings_are_forgiving() {
        for (name, want) in [
            ("Nintendo 64", System::N64),
            ("GameCube", System::GameCube),
            ("Game Cube", System::GameCube),
            ("PlayStation", System::Ps1),
            ("Mega-Drive", System::Genesis),
            ("X360", System::Xbox360),
            ("SD512", System::Unknown),
        ] {
            assert_eq!(system_from_folder_name(name).unwrap_or(System::Unknown), want, "{name}");
        }
    }

    #[test]
    fn extension_classifies_when_no_folder_does() {
        assert_eq!(classify(&PathBuf::from("Mario Kart 64 (USA).z64")), System::N64);
        assert_eq!(classify(&PathBuf::from("dumps/Metroid.gba")), System::GameBoyAdvance);
        assert_eq!(classify(&PathBuf::from("whatever/Game.xci")), System::Switch);
        assert_eq!(classify(&PathBuf::from("Game.iso")), System::Unknown);
        assert_eq!(classify(&PathBuf::from("Game.zip")), System::Unknown);
    }

    #[test]
    fn rom_extension_filter() {
        for ext in ["z64", "ISO", "cue", "bin", "gba", "xci", "zip", "chd"] {
            assert!(is_rom_extension(ext), "{ext}");
        }
        for ext in ["txt", "jpg", "srm", "sav", "png", "json", ""] {
            assert!(!is_rom_extension(ext), "{ext}");
        }
    }

    #[test]
    fn system_ids_round_trip() {
        for s in [System::N64, System::Ps1, System::Xbox360, System::Unknown, System::WiiU] {
            assert_eq!(System::from_id(s.id()), s);
        }
        assert_eq!(System::from_id("nonsense"), System::Unknown);
    }

    #[test]
    fn detects_all_three_n64_byte_orders_from_the_header_magic() {
        assert_eq!(detect_n64_byte_order(&[0x80, 0x37, 0x12, 0x40, 9, 9]), Some(ByteOrder::BigEndian));
        assert_eq!(detect_n64_byte_order(&[0x37, 0x80, 0x40, 0x12, 9, 9]), Some(ByteOrder::ByteSwapped));
        assert_eq!(detect_n64_byte_order(&[0x40, 0x12, 0x37, 0x80, 9, 9]), Some(ByteOrder::LittleEndian));
        assert_eq!(detect_n64_byte_order(&[0, 1, 2, 3]), None);
        assert_eq!(detect_n64_byte_order(&[0x80, 0x37]), None);
    }

    #[test]
    fn normalizing_either_swapped_order_gives_the_big_endian_bytes() {
        let big: Vec<u8> = (0u8..32).collect();
        let mut v64 = big.clone();
        for pair in v64.chunks_exact_mut(2) {
            pair.swap(0, 1);
        }
        let mut n64 = big.clone();
        for group in n64.chunks_exact_mut(4) {
            group.reverse();
        }
        normalize_to_big_endian(ByteOrder::ByteSwapped, &mut v64);
        normalize_to_big_endian(ByteOrder::LittleEndian, &mut n64);
        assert_eq!(v64, big);
        assert_eq!(n64, big);
    }

    #[test]
    fn a_trailing_partial_group_is_left_alone() {
        let mut data = vec![1u8, 2, 3, 4, 5, 6];
        normalize_to_big_endian(ByteOrder::LittleEndian, &mut data);
        assert_eq!(data, vec![4, 3, 2, 1, 5, 6]);
    }
}
