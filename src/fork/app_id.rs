//! App IDs of non-Steam shortcuts.
//!
//! Two sizes show up. The 32-bit ID is what `shortcuts.vdf` stores in `appid` (as a signed int32;
//! `i32::from_ne_bytes` of the same bits) and what current Steam uses in artwork file names. The
//! 64-bit grid ID is that value shifted into the high word with `0x02000000` below it; Big Picture and
//! `steam://rungameid/` use it, and old clients named artwork after it.
//!
//! The formula tools use for a new shortcut is `crc32(exe + app name) | 0x80000000`, where `exe` is the
//! string exactly as stored in the `Exe` field, quotes included. Steam ROM Manager and other tools
//! write the result into `appid`, and Steam keeps whatever is stored. That is the only reason we use
//! it: **an entry that already has an `appid` always keeps it.** Newer Steam builds hand out other
//! values when you add a shortcut in the Steam UI (community reports, e.g. ValveSoftware/steam-for-linux
//! issue 9463), so the formula must never be used to guess what Steam calls an existing shortcut.

pub const SHORTCUT_BIT: u32 = 0x8000_0000;
const GRID_ID_LOW_WORD: u64 = 0x0200_0000;

const fn make_crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut bit = 0;
        while bit < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            bit += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = make_crc_table();

/// CRC-32/ISO-HDLC, the one zlib and Python's `binascii.crc32` compute.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc = CRC_TABLE[((crc ^ u32::from(byte)) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

/// App ID for a new shortcut. `exe` is the stored `Exe` string, quotes included.
pub fn shortcut_app_id(exe: &str, app_name: &str) -> u32 {
    shortcut_app_id_bytes(exe.as_bytes(), app_name.as_bytes())
}

/// Same, for strings read from a file that may not be valid UTF-8.
pub fn shortcut_app_id_bytes(exe: &[u8], app_name: &[u8]) -> u32 {
    let mut key = Vec::with_capacity(exe.len() + app_name.len());
    key.extend_from_slice(exe);
    key.extend_from_slice(app_name);
    crc32(&key) | SHORTCUT_BIT
}

/// The 64-bit ID for a 32-bit app ID.
pub fn grid_id_64(app_id: u32) -> u64 {
    (u64::from(app_id) << 32) | GRID_ID_LOW_WORD
}

/// The 32-bit app ID inside a 64-bit grid ID.
pub fn app_id_from_grid_id(grid_id: u64) -> u32 {
    (grid_id >> 32) as u32
}

/// The value as `shortcuts.vdf` shows it in tools that print the int32 as signed.
pub fn to_signed(app_id: u32) -> i32 {
    app_id as i32
}

pub fn from_signed(stored: i32) -> u32 {
    stored as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    // Where each vector comes from:
    // - CRC-32 check value for "123456789" (0xCBF43926): the standard catalogue value for
    //   CRC-32/ISO-HDLC, also asserted in the chiaki-rust test below.
    // - "Chiaki Remaster" vectors: inputs and expected values from the golden tests in
    //   muentzee/chiaki-rust (crates/chiaki-steam/src/lib.rs, test_shortcut_id_golden). Their values were
    //   recomputed here with Python zlib.crc32 (an independent implementation) and matched. Both follow
    //   the algorithm in SteamGridDB/steam-rom-manager (src/lib/helpers/steam/generate-app-id.ts) and
    //   PhilipK/steam_shortcuts_util (src/app_id_generator.rs).
    // - Signed/unsigned: a real shortcut from a real shortcuts.vdf, documented in dahui/griddle
    //   (crates/griddle-core/src/appid.rs): stored -246118299, artwork named for 4048848997.
    // None of these were captured from a Steam install of ours; the formula is checked against other
    // tools' published values, not against Steam itself.

    #[test]
    fn crc32_standard_check_values() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn app_id_matches_published_vectors() {
        assert_eq!(crc32(b"\"C:\\Games\\chiaki.exe\"Chiaki Remaster"), 0x1962_3236);
        assert_eq!(shortcut_app_id("\"C:\\Games\\chiaki.exe\"", "Chiaki Remaster"), 0x9962_3236);
        assert_eq!(shortcut_app_id("\"C:\\Games\\chiaki.exe\"", "Chiaki Remaster"), 2_573_349_430);
        assert_eq!(shortcut_app_id("\"C:\\Program Files\\chiaki\\chiaki.exe\"", "Chiaki Remaster"), 0xA677_6126);
    }

    #[test]
    fn grid_id_64_matches_published_vectors() {
        let id = shortcut_app_id("\"C:\\Games\\chiaki.exe\"", "Chiaki Remaster");
        assert_eq!(grid_id_64(id), 0x9962_3236_0200_0000);
        assert_eq!(grid_id_64(id).to_string(), "11052451643063795712");
        assert_eq!(app_id_from_grid_id(grid_id_64(id)), id);
    }

    #[test]
    fn signed_form_matches_a_real_stored_value() {
        assert_eq!(from_signed(-246_118_299), 4_048_848_997);
        assert_eq!(to_signed(4_048_848_997), -246_118_299);
        assert_eq!(grid_id_64(4_048_848_997) & 0xFFFF_FFFF, 0x0200_0000);
    }

    #[test]
    fn signed_round_trip_covers_the_range() {
        for v in [0u32, 1, 0x7FFF_FFFF, 0x8000_0000, 0xF154_8865, u32::MAX] {
            assert_eq!(from_signed(to_signed(v)), v);
        }
    }

    #[test]
    fn the_name_is_part_of_the_id() {
        assert_ne!(shortcut_app_id("\"/a\"", "One"), shortcut_app_id("\"/a\"", "Two"));
        assert_ne!(shortcut_app_id("\"/a\"", "One"), shortcut_app_id("\"/b\"", "One"));
        assert_ne!(shortcut_app_id("\"/a\"", "One") & SHORTCUT_BIT, 0);
    }
}
