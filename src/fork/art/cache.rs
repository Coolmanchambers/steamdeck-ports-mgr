//! What the art sources have already told us, kept on disk under `cache/art/` (the app's cache folder,
//! never the repo, never `state.json`):
//!
//! * `ids.json`: per port, which game id each source uses for it (found by a same-title search or
//!   confirmed by the user), the look-alike candidates that were *not* trusted, when a search was last
//!   made so a miss is not repeated every run, and the ScreenScraper game id behind each ROM hash.
//! * `images/<source>/<game id>-<slot>.img` with a `.json` beside it: the best image found for a slot,
//!   as downloaded, so the next run needs no network. `.none` marks "this source has nothing for that
//!   slot" for a few days.
//! * `state.json`: see [`super::source::SourceStates`].
//!
//! No key, login or media address is ever written here: only ids, names, hashes of ROMs the user
//! already has, and the downloaded images themselves.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::fit::Slot;
use super::source::SourceId;
use super::title::Confidence;

/// A downloaded image is reused for this long.
pub const IMAGE_TTL_SECS: u64 = 90 * 86_400;
/// "Nothing for this slot" is believed for this long.
pub const MISSING_TTL_SECS: u64 = 3 * 86_400;
/// A title search that found nothing usable is not repeated for this long.
pub const SEARCH_TTL_SECS: u64 = 3 * 86_400;
/// The image cache is trimmed (oldest first) when it grows past this.
pub const MAX_CACHE_BYTES: u64 = 300 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageMeta {
    /// The source's own id for the image, for the manifest and for debugging.
    pub image_id: u64,
    pub fetched_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheLookup {
    Hit { bytes: Vec<u8>, meta: ImageMeta },
    /// The source was asked recently and had nothing for this slot.
    KnownMissing,
    Miss,
}

pub struct ArtCache {
    root: PathBuf,
}

impl ArtCache {
    /// `cache_dir` is the app's `cache/` folder.
    pub fn new(cache_dir: &Path) -> ArtCache {
        ArtCache { root: cache_dir.join("art") }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn states_path(&self) -> PathBuf {
        self.root.join("state.json")
    }

    pub fn ids_path(&self) -> PathBuf {
        self.root.join("ids.json")
    }

    fn image_base(&self, source: SourceId, game_id: u64, slot: Slot) -> PathBuf {
        self.root.join("images").join(source.id()).join(format!("{game_id}-{}", slot.name()))
    }

    fn with_ext(base: &Path, ext: &str) -> PathBuf {
        let mut name = base.file_name().unwrap_or_default().to_os_string();
        name.push(ext);
        base.with_file_name(name)
    }

    pub fn image_get(&self, source: SourceId, game_id: u64, slot: Slot, now: u64) -> CacheLookup {
        let base = self.image_base(source, game_id, slot);
        if let Ok(text) = fs::read_to_string(Self::with_ext(&base, ".none")) {
            let at: u64 = text.trim().parse().unwrap_or(0);
            if now < at.saturating_add(MISSING_TTL_SECS) {
                return CacheLookup::KnownMissing;
            }
        }
        let Ok(meta_text) = fs::read_to_string(Self::with_ext(&base, ".json")) else { return CacheLookup::Miss };
        let Ok(meta) = serde_json::from_str::<Value>(&meta_text) else { return CacheLookup::Miss };
        let (Some(image_id), Some(fetched_at)) = (meta.get("image_id").and_then(Value::as_u64), meta.get("fetched_at").and_then(Value::as_u64)) else { return CacheLookup::Miss };
        if now > fetched_at.saturating_add(IMAGE_TTL_SECS) {
            return CacheLookup::Miss;
        }
        match fs::read(Self::with_ext(&base, ".img")) {
            Ok(bytes) if !bytes.is_empty() => CacheLookup::Hit { bytes, meta: ImageMeta { image_id, fetched_at } },
            _ => CacheLookup::Miss,
        }
    }

    pub fn image_put(&self, source: SourceId, game_id: u64, slot: Slot, bytes: &[u8], image_id: u64, now: u64) -> io::Result<()> {
        let base = self.image_base(source, game_id, slot);
        if let Some(dir) = base.parent() {
            fs::create_dir_all(dir)?;
        }
        let _ = fs::remove_file(Self::with_ext(&base, ".none"));
        crate::core::files::write_atomic(&Self::with_ext(&base, ".img"), bytes)?;
        let meta = json!({ "image_id": image_id, "fetched_at": now });
        crate::core::files::write_atomic(&Self::with_ext(&base, ".json"), meta.to_string().as_bytes())?;
        self.prune(MAX_CACHE_BYTES);
        Ok(())
    }

    pub fn missing_put(&self, source: SourceId, game_id: u64, slot: Slot, now: u64) -> io::Result<()> {
        let base = self.image_base(source, game_id, slot);
        if let Some(dir) = base.parent() {
            fs::create_dir_all(dir)?;
        }
        crate::core::files::write_atomic(&Self::with_ext(&base, ".none"), now.to_string().as_bytes())
    }

    /// Total size of the downloaded images.
    pub fn image_bytes(&self) -> u64 {
        files_under(&self.root.join("images")).iter().map(|(_, size, _)| size).sum()
    }

    /// Deletes the oldest downloaded images until the folder is under `max_bytes`. Best effort.
    pub fn prune(&self, max_bytes: u64) {
        let mut files = files_under(&self.root.join("images"));
        let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
        if total <= max_bytes {
            return;
        }
        files.sort_by_key(|(_, _, modified)| *modified);
        for (path, size, _) in files {
            if total <= max_bytes {
                break;
            }
            if path.extension().is_some_and(|e| e == "img") && fs::remove_file(&path).is_ok() {
                let _ = fs::remove_file(path.with_extension("json"));
                total = total.saturating_sub(size);
            }
        }
    }
}

fn files_under(dir: &Path) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            out.extend(files_under(&path));
        } else {
            out.push((path, meta.len(), meta.modified().unwrap_or(std::time::UNIX_EPOCH)));
        }
    }
    out
}

// ---- the id table -------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdOrigin {
    /// Found by a title search and judged the same title.
    Search,
    /// The user (or a local entry) said so.
    Confirmed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdEntry {
    pub id: u64,
    pub name: String,
    pub origin: IdOrigin,
    pub confidence: Confidence,
}

/// A look-alike result that was not trusted: offered to the user, never used on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub id: u64,
    pub name: String,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PortIds {
    /// The port's name and `ports.local.json` folder, saved so a waiting look-alike can say how to choose.
    name: Option<String>,
    folder: Option<String>,
    found: BTreeMap<SourceId, IdEntry>,
    pending: BTreeMap<SourceId, Vec<Pending>>,
    searched: BTreeMap<SourceId, u64>,
}

#[derive(Debug, Clone, Default)]
pub struct IdTable {
    path: PathBuf,
    ports: BTreeMap<String, PortIds>,
    ss_roms: BTreeMap<String, u64>,
    dirty: bool,
}

const IDS_VERSION: u64 = 1;

impl IdTable {
    pub fn in_memory() -> IdTable {
        IdTable::default()
    }

    /// A missing or damaged file is an empty table: everything in it can be found again.
    pub fn load(path: &Path) -> IdTable {
        let mut table = IdTable { path: path.to_path_buf(), ..IdTable::default() };
        let Ok(text) = fs::read_to_string(path) else { return table };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { return table };
        if v.get("version").and_then(Value::as_u64) != Some(IDS_VERSION) {
            return table;
        }
        if let Some(ports) = v.get("ports").and_then(Value::as_object) {
            for (key, p) in ports {
                let mut ids = PortIds::default();
                ids.name = p.get("name").and_then(Value::as_str).map(str::to_string);
                ids.folder = p.get("folder").and_then(Value::as_str).map(str::to_string);
                for src in SourceId::ALL {
                    if let Some(e) = p.get(src.id()) {
                        if let (Some(id), Some(name)) = (e.get("id").and_then(Value::as_u64), e.get("name").and_then(Value::as_str)) {
                            let origin = if e.get("origin").and_then(Value::as_str) == Some("confirmed") { IdOrigin::Confirmed } else { IdOrigin::Search };
                            let confidence = Confidence::from_label(e.get("confidence").and_then(Value::as_str).unwrap_or(""));
                            ids.found.insert(src, IdEntry { id, name: name.to_string(), origin, confidence });
                        }
                    }
                    if let Some(list) = p.get("pending").and_then(|x| x.get(src.id())).and_then(Value::as_array) {
                        let items = list
                            .iter()
                            .filter_map(|c| Some(Pending { id: c.get("id")?.as_u64()?, name: c.get("name")?.as_str()?.to_string(), confidence: Confidence::from_label(c.get("confidence")?.as_str()?) }))
                            .collect();
                        ids.pending.insert(src, items);
                    }
                    if let Some(at) = p.get("searched").and_then(|x| x.get(src.id())).and_then(Value::as_u64) {
                        ids.searched.insert(src, at);
                    }
                }
                table.ports.insert(key.clone(), ids);
            }
        }
        if let Some(roms) = v.get("ss_roms").and_then(Value::as_object) {
            for (k, id) in roms {
                if let Some(id) = id.as_u64() {
                    table.ss_roms.insert(k.clone(), id);
                }
            }
        }
        table
    }

    pub fn save(&mut self) -> io::Result<()> {
        if !self.dirty || self.path.as_os_str().is_empty() {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut ports = serde_json::Map::new();
        for (key, ids) in &self.ports {
            let mut p = serde_json::Map::new();
            if let Some(name) = &ids.name {
                p.insert("name".into(), Value::from(name.as_str()));
            }
            if let Some(folder) = &ids.folder {
                p.insert("folder".into(), Value::from(folder.as_str()));
            }
            for (src, e) in &ids.found {
                p.insert(
                    src.id().into(),
                    json!({ "id": e.id, "name": e.name, "origin": if e.origin == IdOrigin::Confirmed { "confirmed" } else { "search" }, "confidence": e.confidence.label() }),
                );
            }
            let pending: serde_json::Map<String, Value> = ids
                .pending
                .iter()
                .filter(|(_, v)| !v.is_empty())
                .map(|(src, v)| (src.id().to_string(), Value::Array(v.iter().map(|c| json!({ "id": c.id, "name": c.name, "confidence": c.confidence.label() })).collect())))
                .collect();
            if !pending.is_empty() {
                p.insert("pending".into(), Value::Object(pending));
            }
            let searched: serde_json::Map<String, Value> = ids.searched.iter().map(|(src, at)| (src.id().to_string(), Value::from(*at))).collect();
            if !searched.is_empty() {
                p.insert("searched".into(), Value::Object(searched));
            }
            ports.insert(key.clone(), Value::Object(p));
        }
        let doc = json!({ "version": IDS_VERSION, "ports": ports, "ss_roms": self.ss_roms });
        let text = serde_json::to_string_pretty(&doc).map_err(io::Error::other)?;
        crate::core::files::write_atomic(&self.path, text.as_bytes())?;
        self.dirty = false;
        Ok(())
    }

    /// The id to use for a port, if one is known and trusted.
    pub fn usable(&self, port_key: &str, src: SourceId) -> Option<&IdEntry> {
        let e = self.ports.get(port_key)?.found.get(&src)?;
        (e.origin == IdOrigin::Confirmed || e.confidence.is_silent()).then_some(e)
    }

    pub fn pending(&self, port_key: &str, src: SourceId) -> &[Pending] {
        self.ports.get(port_key).and_then(|p| p.pending.get(&src)).map_or(&[], Vec::as_slice)
    }

    pub fn searched_recently(&self, port_key: &str, src: SourceId, now: u64) -> bool {
        self.ports.get(port_key).and_then(|p| p.searched.get(&src)).is_some_and(|at| now < at.saturating_add(SEARCH_TTL_SECS))
    }

    /// Records a trusted match from a title search.
    pub fn set_found(&mut self, port_key: &str, src: SourceId, entry: IdEntry, now: u64) {
        let p = self.ports.entry(port_key.to_string()).or_default();
        p.found.insert(src, entry);
        p.pending.remove(&src);
        p.searched.insert(src, now);
        self.dirty = true;
    }

    /// Records that a search found only look-alikes (at most three are kept).
    pub fn set_pending(&mut self, port_key: &str, src: SourceId, candidates: Vec<Pending>, now: u64) {
        let p = self.ports.entry(port_key.to_string()).or_default();
        p.pending.insert(src, candidates.into_iter().take(3).collect());
        p.searched.insert(src, now);
        self.dirty = true;
    }

    /// The user picked this id: it is used from now on and no confidence is needed.
    pub fn confirm(&mut self, port_key: &str, src: SourceId, id: u64, name: &str) {
        let p = self.ports.entry(port_key.to_string()).or_default();
        p.found.insert(src, IdEntry { id, name: name.to_string(), origin: IdOrigin::Confirmed, confidence: Confidence::High });
        p.pending.remove(&src);
        self.dirty = true;
    }

    pub fn forget(&mut self, port_key: &str, src: SourceId) {
        if let Some(p) = self.ports.get_mut(port_key) {
            p.found.remove(&src);
            p.pending.remove(&src);
            p.searched.remove(&src);
            self.dirty = true;
        }
    }

    pub fn rom_game(&self, rom_key: &str) -> Option<u64> {
        self.ss_roms.get(rom_key).copied()
    }

    pub fn set_rom_game(&mut self, rom_key: &str, game_id: u64) {
        if self.ss_roms.insert(rom_key.to_string(), game_id) != Some(game_id) {
            self.dirty = true;
        }
    }

    /// Remembers who a port is, for the screen that lists look-alike matches.
    pub fn set_identity(&mut self, port_key: &str, name: &str, folder: &str) {
        let p = self.ports.entry(port_key.to_string()).or_default();
        if p.name.as_deref() != Some(name) || p.folder.as_deref() != Some(folder) {
            p.name = Some(name.to_string());
            p.folder = Some(folder.to_string());
            self.dirty = true;
        }
    }

    /// The saved name and folder of a port, when known.
    pub fn identity(&self, port_key: &str) -> (Option<&str>, Option<&str>) {
        self.ports.get(port_key).map_or((None, None), |p| (p.name.as_deref(), p.folder.as_deref()))
    }

    /// Is `id` already the confirmed id of this port for `src`?
    pub fn is_confirmed(&self, port_key: &str, src: SourceId, id: u64) -> bool {
        self.ports.get(port_key).and_then(|p| p.found.get(&src)).is_some_and(|e| e.origin == IdOrigin::Confirmed && e.id == id)
    }

    /// Ports with look-alike candidates waiting for the user.
    pub fn ports_with_pending(&self) -> Vec<(&str, SourceId, &[Pending])> {
        let mut out = Vec::new();
        for (key, p) in &self.ports {
            for (src, list) in &p.pending {
                if !list.is_empty() && !p.found.get(src).is_some_and(|e| e.origin == IdOrigin::Confirmed || e.confidence.is_silent()) {
                    out.push((key.as_str(), *src, list.as_slice()));
                }
            }
        }
        out
    }
}

/// Confirms a look-alike for a port straight in the saved id table (what the "Use this match" row does).
pub fn confirm_in_file(cache_dir: &Path, port_key: &str, src: SourceId, id: u64, name: &str) -> io::Result<()> {
    let cache = ArtCache::new(cache_dir);
    let mut table = IdTable::load(&cache.ids_path());
    table.confirm(port_key, src, id, name);
    table.save()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_cache(name: &str) -> (PathBuf, ArtCache) {
        let dir = std::env::temp_dir().join(format!("dpm_artcache_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let cache = ArtCache::new(&dir);
        (dir, cache)
    }

    #[test]
    fn an_image_round_trips_and_is_remembered_until_it_expires() {
        let (dir, cache) = temp_cache("img");
        assert_eq!(cache.image_get(SourceId::SteamGridDb, 5, Slot::Hero, 1_000), CacheLookup::Miss);
        cache.image_put(SourceId::SteamGridDb, 5, Slot::Hero, b"IMAGE", 42, 1_000).unwrap();
        match cache.image_get(SourceId::SteamGridDb, 5, Slot::Hero, 2_000) {
            CacheLookup::Hit { bytes, meta } => assert_eq!((bytes, meta), (b"IMAGE".to_vec(), ImageMeta { image_id: 42, fetched_at: 1_000 })),
            other => panic!("{other:?}"),
        }
        assert_eq!(cache.image_get(SourceId::SteamGridDb, 5, Slot::Logo, 2_000), CacheLookup::Miss, "another slot is separate");
        assert_eq!(cache.image_get(SourceId::TheGamesDb, 5, Slot::Hero, 2_000), CacheLookup::Miss, "another source is separate");
        assert_eq!(cache.image_get(SourceId::SteamGridDb, 5, Slot::Hero, 1_000 + IMAGE_TTL_SECS + 1), CacheLookup::Miss);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_slot_is_believed_for_a_few_days_and_a_new_image_replaces_the_note() {
        let (dir, cache) = temp_cache("none");
        cache.missing_put(SourceId::TheGamesDb, 9, Slot::Logo, 1_000).unwrap();
        assert_eq!(cache.image_get(SourceId::TheGamesDb, 9, Slot::Logo, 1_100), CacheLookup::KnownMissing);
        assert_eq!(cache.image_get(SourceId::TheGamesDb, 9, Slot::Logo, 1_000 + MISSING_TTL_SECS + 1), CacheLookup::Miss);
        cache.image_put(SourceId::TheGamesDb, 9, Slot::Logo, b"L", 1, 1_200).unwrap();
        assert!(matches!(cache.image_get(SourceId::TheGamesDb, 9, Slot::Logo, 1_300), CacheLookup::Hit { .. }));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn damaged_entries_are_misses_not_errors() {
        let (dir, cache) = temp_cache("bad");
        cache.image_put(SourceId::SteamGridDb, 1, Slot::Wide, b"X", 1, 10).unwrap();
        let base = dir.join("art/images/steamgriddb");
        fs::write(base.join("1-wide.json"), "not json").unwrap();
        assert_eq!(cache.image_get(SourceId::SteamGridDb, 1, Slot::Wide, 20), CacheLookup::Miss);
        cache.image_put(SourceId::SteamGridDb, 1, Slot::Wide, b"X", 1, 10).unwrap();
        fs::write(base.join("1-wide.img"), b"").unwrap();
        assert_eq!(cache.image_get(SourceId::SteamGridDb, 1, Slot::Wide, 20), CacheLookup::Miss);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_cache_is_trimmed_oldest_first() {
        let (dir, cache) = temp_cache("prune");
        for i in 0..4u64 {
            cache.image_put(SourceId::SteamGridDb, i, Slot::Hero, &vec![7u8; 1000], i, 10).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(cache.image_bytes() >= 4000);
        cache.prune(2500);
        assert!(cache.image_bytes() <= 2500);
        assert!(matches!(cache.image_get(SourceId::SteamGridDb, 3, Slot::Hero, 20), CacheLookup::Hit { .. }), "the newest survives");
        assert_eq!(cache.image_get(SourceId::SteamGridDb, 0, Slot::Hero, 20), CacheLookup::Miss);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn ids_round_trip_with_pending_candidates_and_search_times() {
        let (dir, cache) = temp_cache("ids");
        let mut t = IdTable::load(&cache.ids_path());
        t.set_found("owner/repo", SourceId::SteamGridDb, IdEntry { id: 77, name: "Ship of Harkinian".into(), origin: IdOrigin::Search, confidence: Confidence::High }, 500);
        t.set_pending("other", SourceId::SteamGridDb, vec![Pending { id: 1, name: "Doom II".into(), confidence: Confidence::Medium }], 600);
        t.set_rom_game("14:abcd:100", 9001);
        t.save().unwrap();
        let again = IdTable::load(&cache.ids_path());
        assert_eq!(again.usable("owner/repo", SourceId::SteamGridDb).map(|e| e.id), Some(77));
        assert!(again.usable("other", SourceId::SteamGridDb).is_none());
        assert_eq!(again.pending("other", SourceId::SteamGridDb).len(), 1);
        assert!(again.searched_recently("other", SourceId::SteamGridDb, 700));
        assert!(!again.searched_recently("other", SourceId::SteamGridDb, 600 + SEARCH_TTL_SECS + 1));
        assert_eq!(again.rom_game("14:abcd:100"), Some(9001));
        assert_eq!(again.ports_with_pending().len(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn identity_is_saved_and_a_match_can_be_confirmed_straight_in_the_file() {
        let (dir, cache) = temp_cache("identity");
        let mut t = IdTable::load(&cache.ids_path());
        t.set_identity("owner/ship", "Ship of Harkinian", "shipwright");
        t.set_pending("owner/ship", SourceId::SteamGridDb, vec![Pending { id: 9, name: "Ship Remastered".into(), confidence: Confidence::Medium }], 5);
        t.save().unwrap();
        let loaded = IdTable::load(&cache.ids_path());
        assert_eq!(loaded.identity("owner/ship"), (Some("Ship of Harkinian"), Some("shipwright")));
        assert_eq!(loaded.ports_with_pending().len(), 1);
        confirm_in_file(&dir, "owner/ship", SourceId::SteamGridDb, 9, "Ship Remastered").unwrap();
        let after = IdTable::load(&cache.ids_path());
        assert!(after.is_confirmed("owner/ship", SourceId::SteamGridDb, 9));
        assert!(after.ports_with_pending().is_empty(), "the waiting row clears");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_low_confidence_entry_is_not_usable_until_the_user_confirms_it() {
        let mut t = IdTable::in_memory();
        t.set_found("p", SourceId::TheGamesDb, IdEntry { id: 5, name: "Ship".into(), origin: IdOrigin::Search, confidence: Confidence::Medium }, 1);
        assert!(t.usable("p", SourceId::TheGamesDb).is_none());
        t.confirm("p", SourceId::TheGamesDb, 5, "Ship");
        assert_eq!(t.usable("p", SourceId::TheGamesDb).map(|e| (e.id, e.origin)), Some((5, IdOrigin::Confirmed)));
        assert!(t.ports_with_pending().is_empty());
        t.forget("p", SourceId::TheGamesDb);
        assert!(t.usable("p", SourceId::TheGamesDb).is_none());
    }

    #[test]
    fn a_damaged_or_foreign_table_loads_empty_and_nothing_is_written_unless_changed() {
        let (dir, cache) = temp_cache("badids");
        fs::create_dir_all(cache.root()).unwrap();
        fs::write(cache.ids_path(), "{ nope").unwrap();
        let mut t = IdTable::load(&cache.ids_path());
        assert!(t.usable("x", SourceId::SteamGridDb).is_none());
        t.save().unwrap();
        assert_eq!(fs::read_to_string(cache.ids_path()).unwrap(), "{ nope", "an unchanged table is not saved");
        fs::write(cache.ids_path(), r#"{"version":9,"ports":{"x":{"steamgriddb":{"id":1,"name":"n","origin":"confirmed"}}}}"#).unwrap();
        assert!(IdTable::load(&cache.ids_path()).usable("x", SourceId::SteamGridDb).is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn nothing_but_ids_names_and_hashes_is_stored() {
        let (dir, cache) = temp_cache("content");
        let mut t = IdTable::load(&cache.ids_path());
        t.set_found("k", SourceId::SteamGridDb, IdEntry { id: 1, name: "N".into(), origin: IdOrigin::Search, confidence: Confidence::High }, 1);
        t.save().unwrap();
        let text = fs::read_to_string(cache.ids_path()).unwrap();
        assert!(!text.contains("http") && !text.to_lowercase().contains("key\":\"") && !text.contains("password"));
        let _ = fs::remove_dir_all(dir);
    }
}
