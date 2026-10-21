//! Steam's binary KeyValues format, as used by `shortcuts.vdf`.
//!
//! The codec is lossless: everything it reads (key and string bytes, value types, duplicate keys,
//! key casing, order, bytes after the root's end marker) is kept, so encoding a parsed file gives
//! the same bytes back. Strings stay raw bytes because Steam does not promise UTF-8.
//!
//! Layout: each entry is `tag, key\0, value`. Tags: 0 = nested object (entries until a `0x08`),
//! 1 = string, 2 = int32, 3 = float, 4 = pointer, 5 = wide string, 6 = color, 7 = uint64, 10 = int64.
//! A `0x08` where a tag is expected ends the current object. The file is one root object that ends
//! with its own `0x08` (so a real file ends `... 08 08`: end of `shortcuts`, end of root).

use std::fmt;

const TAG_OBJECT: u8 = 0;
const TAG_STRING: u8 = 1;
const TAG_INT: u8 = 2;
const TAG_FLOAT: u8 = 3;
const TAG_PTR: u8 = 4;
const TAG_WIDE_STRING: u8 = 5;
const TAG_COLOR: u8 = 6;
const TAG_UINT64: u8 = 7;
const TAG_END: u8 = 8;
const TAG_INT64: u8 = 10;

/// Real files nest 3 or 4 deep; the cap only stops a hostile file from overflowing the stack.
const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Object(Vec<Entry>),
    String(Vec<u8>),
    /// The int32 tag. Steam stores app IDs here as a signed value; the bits are kept as `u32`.
    Int(u32),
    /// Float, pointer or color: four bytes kept as read, not interpreted.
    Raw4 { tag: u8, bytes: [u8; 4] },
    /// uint64 or int64: eight bytes kept as read, not interpreted.
    Raw8 { tag: u8, bytes: [u8; 8] },
    WideString(Vec<u16>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: Vec<u8>,
    pub value: Value,
}

impl Entry {
    pub fn new(key: &str, value: Value) -> Self {
        Entry { key: key.as_bytes().to_vec(), value }
    }

    pub fn string(key: &str, value: &str) -> Self {
        Entry::new(key, Value::String(value.as_bytes().to_vec()))
    }

    pub fn int(key: &str, value: u32) -> Self {
        Entry::new(key, Value::Int(value))
    }

    pub fn key_matches(&self, name: &str) -> bool {
        self.key.eq_ignore_ascii_case(name.as_bytes())
    }
}

/// The whole file: a root object plus whatever followed its end marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub root: Vec<Entry>,
    /// False only for input that ended without the root's `0x08` (an empty file, or a cut-off one).
    /// Kept so such a file still round-trips; edits turn it on.
    pub terminated: bool,
    /// Bytes after the root's end marker, kept untouched.
    pub trailing: Vec<u8>,
}

impl Document {
    pub fn empty() -> Self {
        Document { root: Vec::new(), terminated: true, trailing: Vec::new() }
    }

    pub fn parse(data: &[u8]) -> Result<Document, ParseError> {
        let mut reader = Reader { data, pos: 0 };
        let (root, terminated) = parse_entries(&mut reader, 0)?;
        Ok(Document { root, terminated, trailing: data[reader.pos..].to_vec() })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write_entries(&mut out, &self.root);
        if self.terminated {
            out.push(TAG_END);
        }
        out.extend_from_slice(&self.trailing);
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Truncated { offset: usize },
    UnknownType { tag: u8, offset: usize },
    TooDeep { offset: usize },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Truncated { offset } => write!(f, "file ends early at byte {offset}"),
            ParseError::UnknownType { tag, offset } => write!(f, "unknown value type {tag} at byte {offset}"),
            ParseError::TooDeep { offset } => write!(f, "nesting too deep at byte {offset}"),
        }
    }
}

impl std::error::Error for ParseError {}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, len: usize) -> Result<&[u8], ParseError> {
        let end = self.pos.checked_add(len).filter(|&end| end <= self.data.len());
        let Some(end) = end else {
            return Err(ParseError::Truncated { offset: self.data.len() });
        };
        let bytes = &self.data[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    fn cstring(&mut self) -> Result<Vec<u8>, ParseError> {
        let rest = &self.data[self.pos..];
        let Some(len) = rest.iter().position(|&b| b == 0) else {
            return Err(ParseError::Truncated { offset: self.data.len() });
        };
        let bytes = rest[..len].to_vec();
        self.pos += len + 1;
        Ok(bytes)
    }

    fn wide_cstring(&mut self) -> Result<Vec<u16>, ParseError> {
        let mut units = Vec::new();
        loop {
            let pair = self.take(2)?;
            let unit = u16::from_le_bytes([pair[0], pair[1]]);
            if unit == 0 {
                return Ok(units);
            }
            units.push(unit);
        }
    }
}

fn parse_entries(r: &mut Reader, depth: usize) -> Result<(Vec<Entry>, bool), ParseError> {
    let mut entries = Vec::new();
    loop {
        if r.pos == r.data.len() {
            // Only the root may run out of input between entries.
            return if depth == 0 { Ok((entries, false)) } else { Err(ParseError::Truncated { offset: r.pos }) };
        }
        let tag_offset = r.pos;
        let tag = r.take(1)?[0];
        if tag == TAG_END {
            return Ok((entries, true));
        }
        let key = r.cstring()?;
        let value = match tag {
            TAG_OBJECT => {
                if depth + 1 >= MAX_DEPTH {
                    return Err(ParseError::TooDeep { offset: tag_offset });
                }
                let (children, terminated) = parse_entries(r, depth + 1)?;
                if !terminated {
                    return Err(ParseError::Truncated { offset: r.pos });
                }
                Value::Object(children)
            }
            TAG_STRING => Value::String(r.cstring()?),
            TAG_INT => Value::Int(u32::from_le_bytes(r.take(4)?.try_into().expect("4 bytes"))),
            TAG_FLOAT | TAG_PTR | TAG_COLOR => Value::Raw4 { tag, bytes: r.take(4)?.try_into().expect("4 bytes") },
            TAG_UINT64 | TAG_INT64 => Value::Raw8 { tag, bytes: r.take(8)?.try_into().expect("8 bytes") },
            TAG_WIDE_STRING => Value::WideString(r.wide_cstring()?),
            other => return Err(ParseError::UnknownType { tag: other, offset: tag_offset }),
        };
        entries.push(Entry { key, value });
    }
}

fn write_entries(out: &mut Vec<u8>, entries: &[Entry]) {
    for entry in entries {
        match &entry.value {
            Value::Object(children) => {
                out.push(TAG_OBJECT);
                write_key(out, &entry.key);
                write_entries(out, children);
                out.push(TAG_END);
            }
            Value::String(s) => {
                out.push(TAG_STRING);
                write_key(out, &entry.key);
                out.extend_from_slice(s);
                out.push(0);
            }
            Value::Int(v) => {
                out.push(TAG_INT);
                write_key(out, &entry.key);
                out.extend_from_slice(&v.to_le_bytes());
            }
            Value::Raw4 { tag, bytes } => {
                out.push(*tag);
                write_key(out, &entry.key);
                out.extend_from_slice(bytes);
            }
            Value::Raw8 { tag, bytes } => {
                out.push(*tag);
                write_key(out, &entry.key);
                out.extend_from_slice(bytes);
            }
            Value::WideString(units) => {
                out.push(TAG_WIDE_STRING);
                write_key(out, &entry.key);
                for unit in units {
                    out.extend_from_slice(&unit.to_le_bytes());
                }
                out.extend_from_slice(&[0, 0]);
            }
        }
    }
}

fn write_key(out: &mut Vec<u8>, key: &[u8]) {
    out.extend_from_slice(key);
    out.push(0);
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Hand-assembled fixture files. These builders do not use the codec, so a round-trip test is not
    //! checking the encoder against itself.

    pub fn str_field(key: &str, value: &[u8]) -> Vec<u8> {
        let mut v = vec![1];
        v.extend_from_slice(key.as_bytes());
        v.push(0);
        v.extend_from_slice(value);
        v.push(0);
        v
    }

    pub fn int_field(key: &str, value: u32) -> Vec<u8> {
        let mut v = vec![2];
        v.extend_from_slice(key.as_bytes());
        v.push(0);
        v.extend_from_slice(&value.to_le_bytes());
        v
    }

    pub fn object(key: &str, children: &[Vec<u8>]) -> Vec<u8> {
        let mut v = vec![0];
        v.extend_from_slice(key.as_bytes());
        v.push(0);
        for child in children {
            v.extend_from_slice(child);
        }
        v.push(8);
        v
    }

    /// A shortcut entry laid out the way Steam writes it.
    pub fn steam_entry(index: usize, app_id: u32, name: &str, exe: &str, start_dir: &str, tags: &[&str]) -> Vec<u8> {
        let tag_fields: Vec<Vec<u8>> =
            tags.iter().enumerate().map(|(i, t)| str_field(&i.to_string(), t.as_bytes())).collect();
        object(
            &index.to_string(),
            &[
                int_field("appid", app_id),
                str_field("AppName", name.as_bytes()),
                str_field("Exe", exe.as_bytes()),
                str_field("StartDir", start_dir.as_bytes()),
                str_field("icon", b""),
                str_field("ShortcutPath", b""),
                str_field("LaunchOptions", b""),
                int_field("IsHidden", 0),
                int_field("AllowDesktopConfig", 1),
                int_field("AllowOverlay", 1),
                int_field("OpenVR", 0),
                int_field("Devkit", 0),
                str_field("DevkitGameID", b""),
                int_field("DevkitOverrideAppID", 0),
                int_field("LastPlayTime", 1_700_000_000),
                str_field("FlatpakAppID", b""),
                object("tags", &tag_fields),
            ],
        )
    }

    /// `shortcuts` object inside the root, plus the root's end marker.
    pub fn file_of(entries: &[Vec<u8>]) -> Vec<u8> {
        let mut v = object("shortcuts", entries);
        v.push(8);
        v
    }

    pub const EMPTY_FILE: &[u8] = b"\0shortcuts\0\x08\x08";

    /// Two ordinary shortcuts, one with tags.
    pub fn plain_file() -> Vec<u8> {
        file_of(&[
            steam_entry(0, 0xF154_8865, "Zelda Ship", "\"/home/deck/Ports/soh/soh.AppImage\"", "\"/home/deck/Ports/soh/\"", &["Ports", "Favorites"]),
            steam_entry(1, 0x9962_3236, "Chiaki", "\"/usr/bin/chiaki\"", "\"/usr/bin/\"", &[]),
        ])
    }

    /// Fields this library does not model: mixed key casing, duplicate keys, every scalar type, wide
    /// strings, non-UTF-8 bytes, a nested object of unknown shape, a second root entry and trailing bytes.
    pub fn odd_file() -> Vec<u8> {
        let mut entry = Vec::new();
        entry.extend(int_field("appid", 0xDEAD_BEEF));
        entry.extend(str_field("appname", b"Odd \xFF\xFE name"));
        entry.extend(str_field("exe", b"\"/opt/odd/run\""));
        entry.extend(str_field("StartDir", b"\"/opt/odd/\""));
        entry.extend(str_field("StartDir", b"\"/duplicate/\""));
        entry.extend([3, b'S', b'c', b'o', b'r', b'e', 0, 0x00, 0x00, 0x80, 0x3F]);
        entry.extend([4, b'P', b't', b'r', 0, 1, 2, 3, 4]);
        entry.extend([6, b'C', b'o', b'l', 0, 0xFF, 0x00, 0x80, 0xFF]);
        entry.extend([7, b'B', b'i', b'g', 0, 1, 2, 3, 4, 5, 6, 7, 8]);
        entry.extend([10, b'S', b'i', b'g', 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        entry.extend([5, b'W', b'i', b'd', b'e', 0, b'h', 0, 0xE9, 0, 0, 0]);
        entry.extend(object("vendor_blob", &[object("deeper", &[int_field("n", 7), str_field("s", b"x")]), str_field("k", b"")]));
        entry.extend(object("tags", &[str_field("0", b"Odd")]));
        let mut file = object("shortcuts", &[object("0", &[entry]), object("7", &[int_field("appid", 5)])]);
        file.extend(object("extra_root_object", &[int_field("v", 1)]));
        file.push(8);
        file.extend_from_slice(b"\xAA\xBBtrailing");
        file
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn empty_file_matches_what_steam_writes() {
        let doc = Document::parse(EMPTY_FILE).unwrap();
        assert_eq!(doc.encode(), EMPTY_FILE);
        assert_eq!(doc, Document { root: vec![Entry::new("shortcuts", Value::Object(vec![]))], terminated: true, trailing: vec![] });
        assert_eq!(Document::empty().encode(), vec![8]);
    }

    #[test]
    fn round_trip_is_byte_identical_for_a_plain_file() {
        let bytes = plain_file();
        assert_eq!(Document::parse(&bytes).unwrap().encode(), bytes);
    }

    #[test]
    fn round_trip_is_byte_identical_with_fields_we_do_not_model() {
        let bytes = odd_file();
        let doc = Document::parse(&bytes).unwrap();
        assert_eq!(doc.trailing, b"\xAA\xBBtrailing");
        assert_eq!(doc.encode(), bytes);
    }

    #[test]
    fn zero_byte_file_round_trips() {
        let doc = Document::parse(&[]).unwrap();
        assert!(!doc.terminated);
        assert_eq!(doc.encode(), Vec::<u8>::new());
    }

    #[test]
    fn parsed_values_have_the_right_shape() {
        let doc = Document::parse(&odd_file()).unwrap();
        let Value::Object(shortcuts) = &doc.root[0].value else { panic!("shortcuts is an object") };
        let Value::Object(first) = &shortcuts[0].value else { panic!("entry is an object") };
        assert_eq!(first[0], Entry::int("appid", 0xDEAD_BEEF));
        assert_eq!(first[1].value, Value::String(b"Odd \xFF\xFE name".to_vec()));
        assert_eq!(first[5].value, Value::Raw4 { tag: 3, bytes: [0, 0, 0x80, 0x3F] });
        assert_eq!(first[9].value, Value::Raw8 { tag: 10, bytes: [0xFF; 8] });
        assert_eq!(first[10].value, Value::WideString(vec![0x68, 0xE9]));
    }

    #[test]
    fn every_truncation_is_an_error_or_a_clean_root_cut_never_a_panic() {
        for bytes in [plain_file(), odd_file()] {
            for cut in 0..bytes.len() {
                let _ = Document::parse(&bytes[..cut]);
            }
        }
    }

    #[test]
    fn truncation_inside_an_object_is_an_error() {
        let bytes = plain_file();
        assert!(matches!(Document::parse(&bytes[..bytes.len() - 2]), Err(ParseError::Truncated { .. })));
    }

    #[test]
    fn unknown_value_type_is_refused_not_guessed() {
        let mut bytes = vec![0];
        bytes.extend_from_slice(b"shortcuts\0");
        bytes.extend_from_slice(&[9, b'x', 0, 1, 2, 3, 4]);
        bytes.extend_from_slice(&[8, 8]);
        assert_eq!(Document::parse(&bytes), Err(ParseError::UnknownType { tag: 9, offset: 11 }));
    }

    #[test]
    fn runaway_nesting_is_refused() {
        let mut bytes = Vec::new();
        for _ in 0..200 {
            bytes.extend_from_slice(&[0, b'a', 0]);
        }
        assert!(matches!(Document::parse(&bytes), Err(ParseError::TooDeep { .. })));
    }

    #[test]
    fn known_layout_of_one_int_and_one_string() {
        let doc = Document {
            root: vec![Entry::new("shortcuts", Value::Object(vec![Entry::int("appid", 0x8000_0001), Entry::string("AppName", "A")]))],
            terminated: true,
            trailing: vec![],
        };
        let expected: &[u8] = b"\x00shortcuts\x00\x02appid\x00\x01\x00\x00\x80\x01AppName\x00A\x00\x08\x08";
        assert_eq!(doc.encode(), expected);
    }
}
