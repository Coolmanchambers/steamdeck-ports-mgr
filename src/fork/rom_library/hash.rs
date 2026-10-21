//! Streaming CRC-32, MD5 and SHA-1, written here so the fork needs no new dependency.
//!
//! ROM checksum lists (No-Intro, Redump) are published as CRC-32, MD5 and SHA-1, and a ROM can be
//! several gigabytes, so every hasher takes bytes in chunks and one pass feeds all three.

use crate::fork::app_id::crc32 as crc32_oneshot;
use std::sync::OnceLock;

fn crc_table() -> &'static [u32; 256] {
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0u32; 256];
        for (i, slot) in table.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        table
    })
}

#[derive(Clone)]
pub struct Crc32 {
    state: u32,
}

impl Crc32 {
    pub fn new() -> Self {
        Crc32 { state: 0xFFFF_FFFF }
    }

    pub fn update(&mut self, data: &[u8]) {
        let table = crc_table();
        let mut crc = self.state;
        for &b in data {
            crc = table[((crc ^ u32::from(b)) & 0xFF) as usize] ^ (crc >> 8);
        }
        self.state = crc;
    }

    pub fn finish(&self) -> u32 {
        !self.state
    }
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

const MD5_SHIFTS: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11,
    16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

fn md5_constants() -> &'static [u32; 64] {
    static K: OnceLock<[u32; 64]> = OnceLock::new();
    // RFC 1321 defines K[i] = floor(abs(sin(i + 1)) * 2^32); the test vectors below check the result.
    K.get_or_init(|| {
        let mut k = [0u32; 64];
        for (i, slot) in k.iter_mut().enumerate() {
            *slot = ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32;
        }
        k
    })
}

#[derive(Clone)]
pub struct Md5 {
    state: [u32; 4],
    buf: [u8; 64],
    buf_len: usize,
    total: u64,
}

impl Md5 {
    pub fn new() -> Self {
        Md5 { state: [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476], buf: [0; 64], buf_len: 0, total: 0 }
    }

    fn block(state: &mut [u32; 4], block: &[u8]) {
        let k = md5_constants();
        let mut m = [0u32; 16];
        for (i, word) in m.iter_mut().enumerate() {
            *word = u32::from_le_bytes([block[i * 4], block[i * 4 + 1], block[i * 4 + 2], block[i * 4 + 3]]);
        }
        let [mut a, mut b, mut c, mut d] = *state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let rotated = a.wrapping_add(f).wrapping_add(k[i]).wrapping_add(m[g]).rotate_left(MD5_SHIFTS[i]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(rotated);
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        if self.buf_len > 0 {
            let take = (64 - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len < 64 {
                return;
            }
            let block = self.buf;
            Self::block(&mut self.state, &block);
            self.buf_len = 0;
        }
        while data.len() >= 64 {
            Self::block(&mut self.state, &data[..64]);
            data = &data[64..];
        }
        self.buf[..data.len()].copy_from_slice(data);
        self.buf_len = data.len();
    }

    pub fn finish(&self) -> [u8; 16] {
        let mut copy = self.clone();
        let bit_len = self.total.wrapping_mul(8);
        copy.update(&[0x80]);
        while copy.buf_len != 56 {
            copy.update(&[0]);
        }
        copy.update(&bit_len.to_le_bytes());
        let mut out = [0u8; 16];
        for (i, word) in copy.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        out
    }
}

impl Default for Md5 {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct Sha1 {
    state: [u32; 5],
    buf: [u8; 64],
    buf_len: usize,
    total: u64,
}

impl Sha1 {
    pub fn new() -> Self {
        Sha1 { state: [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0], buf: [0; 64], buf_len: 0, total: 0 }
    }

    fn block(state: &mut [u32; 5], block: &[u8]) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([block[i * 4], block[i * 4 + 1], block[i * 4 + 2], block[i * 4 + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = *state;
        for (i, &word) in w.iter().enumerate() {
            let (f, k) = match i / 20 {
                0 => ((b & c) | (!b & d), 0x5A82_7999),
                1 => (b ^ c ^ d, 0x6ED9_EBA1),
                2 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6u32),
            };
            let temp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        for (slot, add) in state.iter_mut().zip([a, b, c, d, e]) {
            *slot = slot.wrapping_add(add);
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        if self.buf_len > 0 {
            let take = (64 - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len < 64 {
                return;
            }
            let block = self.buf;
            Self::block(&mut self.state, &block);
            self.buf_len = 0;
        }
        while data.len() >= 64 {
            Self::block(&mut self.state, &data[..64]);
            data = &data[64..];
        }
        self.buf[..data.len()].copy_from_slice(data);
        self.buf_len = data.len();
    }

    pub fn finish(&self) -> [u8; 20] {
        let mut copy = self.clone();
        let bit_len = self.total.wrapping_mul(8);
        copy.update(&[0x80]);
        while copy.buf_len != 56 {
            copy.update(&[0]);
        }
        copy.update(&bit_len.to_be_bytes());
        let mut out = [0u8; 20];
        for (i, word) in copy.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }
}

impl Default for Sha1 {
    fn default() -> Self {
        Self::new()
    }
}

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn from_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 || !text.is_ascii() {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok()).collect()
}

/// The checksums of one stream of bytes. SHA-1 is optional because it is the slowest of the three; the
/// few catalog entries that name only a SHA-1 cannot be matched without it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digests {
    pub crc32: u32,
    pub md5: [u8; 16],
    pub sha1: Option<[u8; 20]>,
}

impl Digests {
    pub fn md5_hex(&self) -> String {
        to_hex(&self.md5)
    }

    pub fn sha1_hex(&self) -> Option<String> {
        self.sha1.map(|s| to_hex(&s))
    }

    pub fn crc32_hex(&self) -> String {
        format!("{:08X}", self.crc32)
    }
}

#[derive(Clone)]
pub struct MultiHasher {
    crc: Crc32,
    md5: Md5,
    sha1: Option<Sha1>,
}

impl MultiHasher {
    pub fn new(with_sha1: bool) -> Self {
        MultiHasher { crc: Crc32::new(), md5: Md5::new(), sha1: with_sha1.then(Sha1::new) }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.crc.update(data);
        self.md5.update(data);
        if let Some(s) = &mut self.sha1 {
            s.update(data);
        }
    }

    pub fn finish(&self) -> Digests {
        Digests { crc32: self.crc.finish(), md5: self.md5.finish(), sha1: self.sha1.as_ref().map(Sha1::finish) }
    }
}

/// One-shot CRC-32 of a byte slice: the fork's `app_id::crc32`, an independent implementation the
/// tests use to cross-check the streaming one.
pub fn crc32_of(data: &[u8]) -> u32 {
    crc32_oneshot(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_standard_check_value() {
        let mut c = Crc32::new();
        c.update(b"123456789");
        assert_eq!(c.finish(), 0xCBF4_3926);
        assert_eq!(Crc32::new().finish(), 0);
    }

    #[test]
    fn md5_known_vectors() {
        // RFC 1321 appendix A.5.
        let cases: [(&[u8], &str); 4] = [
            (b"", "d41d8cd98f00b204e9800998ecf8427e"),
            (b"a", "0cc175b9c0f1b6a831c399e269772661"),
            (b"abc", "900150983cd24fb0d6963f7d28e17f72"),
            (b"12345678901234567890123456789012345678901234567890123456789012345678901234567890", "57edf4a22be3c955ac49da2e2107b67a"),
        ];
        for (input, want) in cases {
            let mut h = Md5::new();
            h.update(input);
            assert_eq!(to_hex(&h.finish()), want, "input {:?}", String::from_utf8_lossy(input));
        }
    }

    #[test]
    fn sha1_known_vectors() {
        let cases: [(&[u8], &str); 3] = [
            (b"", "da39a3ee5e6b4b0d3255bfef95601890afd80709"),
            (b"abc", "a9993e364706816aba3e25717850c26c9cd0d89d"),
            (b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq", "84983e441c3bd26ebaae4aa1f95129e5e54670f1"),
        ];
        for (input, want) in cases {
            let mut h = Sha1::new();
            h.update(input);
            assert_eq!(to_hex(&h.finish()), want);
        }
    }

    #[test]
    fn million_a_vectors_for_md5_and_sha1() {
        let chunk = [b'a'; 1000];
        let mut md5 = Md5::new();
        let mut sha1 = Sha1::new();
        for _ in 0..1000 {
            md5.update(&chunk);
            sha1.update(&chunk);
        }
        assert_eq!(to_hex(&md5.finish()), "7707d6ae4e027c70eea2a935c2296f21");
        assert_eq!(to_hex(&sha1.finish()), "34aa973cd4c4daa4f61eeb2bdbad27316534016f");
    }

    #[test]
    fn chunk_boundaries_do_not_change_the_result() {
        let data: Vec<u8> = (0..5000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
        let mut whole = MultiHasher::new(true);
        whole.update(&data);
        for chunk_size in [1usize, 3, 63, 64, 65, 127, 1000] {
            let mut parts = MultiHasher::new(true);
            for piece in data.chunks(chunk_size) {
                parts.update(piece);
            }
            assert_eq!(parts.finish(), whole.finish(), "chunk size {chunk_size}");
        }
        assert_eq!(whole.finish().crc32, crc32_of(&data));
    }

    #[test]
    fn finish_does_not_consume_the_hasher() {
        let mut h = MultiHasher::new(false);
        h.update(b"ab");
        let first = h.finish();
        h.update(b"c");
        assert_ne!(h.finish(), first);
        assert!(h.finish().sha1.is_none());
    }

    #[test]
    fn hex_round_trip_and_rejects_bad_input() {
        let bytes = [0x00, 0xAB, 0xFF, 0x10];
        assert_eq!(from_hex(&to_hex(&bytes)).unwrap(), bytes);
        assert!(from_hex("abc").is_none());
        assert!(from_hex("zz").is_none());
    }
}
