//! Picks the art for a port's Steam slots from every source that is set up, in the research report's
//! priority, and hands back an [`ArtSet`] for [`super::grid::write_art`].
//!
//! | Slot | Order |
//! |---|---|
//! | Portrait | the catalog's own image if it is already 600x900, then SteamGridDB, ScreenScraper `box-2D`, TheGamesDB box front, then the catalog image fitted, then nothing |
//! | Icon | the catalog icon, then SteamGridDB icons, then a square made from the logo or portrait |
//! | Hero | SteamGridDB heroes, TheGamesDB banner then fanart, ScreenScraper fanart then screenshot |
//! | Wide | SteamGridDB grids, TheGamesDB banner then fanart, ScreenScraper `steamgrid` (only if it has the slot's shape), fanart, screenshot; then cropped from the hero |
//! | Logo | SteamGridDB logos, TheGamesDB clear logo, ScreenScraper `wheel-hd` / `wheel` (only with transparency); never composed |
//!
//! Within a slot, an image that already has the slot's shape beats one that has to be cropped or
//! padded, whatever the source; the first source with one wins, and only if none has one does the
//! first composed image win. Everything composed is flagged `generated` in the provenance manifest,
//! and the writer never replaces art the user chose.
//!
//! Game ids: a per-port override (`sgdb_id` / `tgdb_id` in the port's `ports.local.json` entry), then
//! the ids this module saved (`cache/art/ids.json`), then a title search. A title search result is
//! used on its own only when it is the same title (see [`super::title`]); a look-alike is saved as a
//! candidate and reported, never used. TheGamesDB and ScreenScraper are tried only for ports that
//! have an id for them or, for ScreenScraper, a scanned ROM of the original game.
//!
//! A source that fails in a way that will not fix itself during the run (bad key, offline, quota,
//! told to wait) is left alone for the rest of the [`Session`], and the wait is remembered across
//! restarts in `cache/art/state.json`. Failures are returned as text, never logged.
//!
//! Nothing here runs on its own: the Add to Steam screen (or the "Fetch artwork" action) opens a
//! [`Session`] and calls [`Session::resolve`] or [`art_for_port`]. [`Mode::CacheOnly`] makes no network
//! call at all and uses only what an earlier online run saved.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use image::DynamicImage;
use serde_json::Value;

use super::cache::{ArtCache, CacheLookup, IdEntry, IdOrigin, IdTable, Pending};
use super::catalog::{catalog_art_from_files, CatalogArt};
use super::fit::{decode, fit, fit_icon_keep_size, has_transparency, Fitted, Slot};
use super::grid::{ArtSet, Manifest};
use super::net::{Fetch, Net};
use super::screenscraper::{self, DevCreds, RomIdentity, ScreenScraper};
use super::sgdb::{self, SteamGridDb};
use super::source::{SourceError, SourceId, SourceStates};
use super::tgdb::{self, ImageSet, TheGamesDb};
use super::title::Confidence;
use crate::core::models::Port;
use crate::fork::artwork::existing_artwork;
use crate::fork::rom_library::requirements::{load_local_entries, RomSpec};
use crate::fork::rom_library::scan::ScanResult;
use crate::fork::rom_library::{match_port, RomLibraryStore, SlotStatus};
use crate::fork::secrets::{BuildCreds, Secrets, FILE_NAME as SECRETS_FILE};

/// Downloads tried per slot and source before giving up.
const MAX_DOWNLOADS: usize = 3;
/// Images in a row that fail to download before the source is given up for the session.
const MAX_IMAGE_FAILURES: u32 = 3;
/// Region preferred for ScreenScraper media.
pub const DEFAULT_REGION: &str = "us";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Look things up on the sources.
    Online,
    /// Use only what earlier runs saved in `cache/art/`; no network call, no title search.
    CacheOnly,
}

/// What to resolve for one port.
pub struct Request<'a> {
    pub port_key: &'a str,
    /// The name searched for on SteamGridDB and TheGamesDB.
    pub name: &'a str,
    /// The port's `ports.local.json` folder; saved beside look-alike matches so the screen can say how to
    /// choose one. Empty: unknown.
    pub folder: &'a str,
    /// Override from the port's local entry.
    pub sgdb_id: Option<u64>,
    pub tgdb_id: Option<u64>,
    /// Identities of the user's ROM for the original game, most likely first. Empty: no ScreenScraper.
    pub rom: Vec<RomIdentity>,
    /// Portrait and icon from the Ports Launcher catalog.
    pub catalog: CatalogArt,
    pub need: Vec<Slot>,
}

impl<'a> Request<'a> {
    pub fn new(port_key: &'a str, name: &'a str) -> Request<'a> {
        Request { port_key, name, folder: "", sgdb_id: None, tgdb_id: None, rom: Vec::new(), catalog: CatalogArt::default(), need: Slot::ALL.to_vec() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotSource {
    pub slot: Slot,
    /// `catalog`, `steamgriddb`, `thegamesdb` or `screenscraper`.
    pub source: String,
    pub generated: bool,
}

/// What was found for one port.
#[derive(Debug, Default)]
pub struct PortArt {
    pub set: ArtSet,
    /// Things that went wrong (a bad key, a source down, a slot that could not be used). Same shape as
    /// `CatalogArt::problems`, so a caller that prints those lines prints these.
    pub problems: Vec<(Slot, String)>,
    /// Plain-language remarks: where each slot came from, look-alike matches waiting for a decision.
    pub notes: Vec<String>,
    pub sources: Vec<SlotSource>,
}

impl PortArt {
    pub fn source_of(&self, slot: Slot) -> Option<&SlotSource> {
        self.sources.iter().find(|s| s.slot == slot)
    }
}

/// Folders the art code reads and writes: `config_dir` holds `secrets.json`, `ports.local.json` and
/// the ROM library files; `cache_dir` is the app's `cache/`.
#[derive(Clone, Copy)]
pub struct ArtDirs<'a> {
    pub config_dir: &'a Path,
    pub cache_dir: &'a Path,
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// A resolver run: the keys, the cache and the state of each source, shared by every port resolved
/// through it. Dropped sources stay dropped for the whole session.
pub struct Session {
    fetch: Box<dyn Fetch>,
    secrets: Secrets,
    build: BuildCreds,
    cache: ArtCache,
    mode: Mode,
    now: u64,
    sleep: Box<dyn Fn(Duration)>,
    region: String,
    states: RefCell<SourceStates>,
    ids: RefCell<IdTable>,
    dead: RefCell<BTreeMap<SourceId, String>>,
    reported: RefCell<BTreeSet<SourceId>>,
    /// Consecutive single-image failures per source; three in a row give the source up for the session.
    fails: RefCell<BTreeMap<SourceId, u32>>,
    /// The saved ROM scan, read once per session (it can be large); `None` inside: there is none.
    scan: RefCell<Option<Option<Rc<ScanResult>>>>,
}

impl Session {
    /// The real thing: keys from `secrets.json`, this build's credentials, the real network.
    pub fn open(dirs: ArtDirs<'_>, mode: Mode) -> Session {
        let secrets = Secrets::load(&dirs.config_dir.join(SECRETS_FILE));
        Session::with_parts(Box::new(Net::new()), secrets, BuildCreds::from_build(), dirs.cache_dir, mode, unix_now(), Box::new(std::thread::sleep))
    }

    pub fn with_parts(fetch: Box<dyn Fetch>, secrets: Secrets, build: BuildCreds, cache_dir: &Path, mode: Mode, now: u64, sleep: Box<dyn Fn(Duration)>) -> Session {
        let cache = ArtCache::new(cache_dir);
        let states = SourceStates::load(&cache.states_path());
        let ids = IdTable::load(&cache.ids_path());
        Session {
            fetch,
            secrets,
            build,
            cache,
            mode,
            now,
            sleep,
            region: DEFAULT_REGION.to_string(),
            states: RefCell::new(states),
            ids: RefCell::new(ids),
            dead: RefCell::new(BTreeMap::new()),
            reported: RefCell::new(BTreeSet::new()),
            fails: RefCell::new(BTreeMap::new()),
            scan: RefCell::new(None),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// True when at least one source has what it needs to be asked (keys present, this build's
    /// credentials for ScreenScraper). Cached results are used either way.
    pub fn any_source_ready(&self) -> bool {
        SourceId::ALL.iter().any(|s| self.configured(*s))
    }

    fn tgdb_key(&self) -> Option<&str> {
        self.secrets.tgdb_key().or(self.build.tgdb_apikey.as_deref())
    }

    fn configured(&self, src: SourceId) -> bool {
        match src {
            SourceId::SteamGridDb => self.secrets.sgdb_key().is_some(),
            SourceId::TheGamesDb => self.tgdb_key().is_some(),
            SourceId::ScreenScraper => self.build.screenscraper_dev().is_some(),
        }
    }

    fn secret_values(&self) -> Vec<String> {
        let mut all = self.secrets.all_values();
        all.extend(self.build.all_values());
        // Fork (cloud backup): the Google sign-in tokens and email are redacted from the same text.
        all.extend(crate::fork::cloud::tokens::values_beside(self.secrets.path()));
        all
    }

    /// Writes what the run learned (source states, ids). Called after every port; cheap.
    pub fn save(&self) {
        let _ = self.states.borrow().save();
        let _ = self.ids.borrow_mut().save();
    }

    /// Look-alike matches waiting for the user: (port key, source, candidates).
    pub fn pending_matches(&self) -> Vec<(String, SourceId, Vec<Pending>)> {
        self.ids.borrow().ports_with_pending().into_iter().map(|(k, s, c)| (k.to_string(), s, c.to_vec())).collect()
    }

    /// The user picked one of the candidates (or typed an id): it is used from now on.
    pub fn confirm_match(&self, port_key: &str, src: SourceId, id: u64, name: &str) {
        self.ids.borrow_mut().confirm(port_key, src, id, name);
        self.save();
    }

    pub fn resolve(&self, req: &Request<'_>) -> PortArt {
        self.resolve_with(req, &|| false)
    }

    /// [`Session::resolve`] that asks `cancel` before every request, so a Stop is honoured inside a port,
    /// between two calls, and not only between ports. Once it says true the rest of the port is answered
    /// from the cache alone.
    pub fn resolve_with(&self, req: &Request<'_>, cancel: &dyn Fn() -> bool) -> PortArt {
        if !self.any_source_ready() {
            return catalog_only(req);
        }
        let run = Run {
            s: self,
            req,
            cancel,
            sgdb: self.secrets.sgdb_key().map(|k| SteamGridDb::new(self.fetch.as_ref(), k)),
            tgdb: self.tgdb_key().map(|k| TheGamesDb::new(self.fetch.as_ref(), k)),
            ss: self.build.screenscraper_dev().map(|(id, password, softname)| {
                let dev = DevCreds { id: id.to_string(), password: password.to_string(), softname: softname.to_string() };
                let user = self.secrets.screenscraper_login().map(|(u, p)| (u.to_string(), p.to_string()));
                ScreenScraper::new(self.fetch.as_ref(), dev, user, self.sleep.as_ref())
            }),
            tgdb_images: RefCell::new(BTreeMap::new()),
            ss_game: RefCell::new(None),
            current: Cell::new(Slot::Portrait),
            problems: RefCell::new(Vec::new()),
            notes: RefCell::new(Vec::new()),
        };
        let out = run.resolve();
        self.save();
        out
    }
}

struct Run<'s, 'r> {
    s: &'s Session,
    req: &'r Request<'r>,
    cancel: &'r dyn Fn() -> bool,
    sgdb: Option<SteamGridDb<'s>>,
    tgdb: Option<TheGamesDb<'s>>,
    ss: Option<ScreenScraper<'s>>,
    tgdb_images: RefCell<BTreeMap<u64, Option<Rc<ImageSet>>>>,
    ss_game: RefCell<Option<Option<Rc<screenscraper::Game>>>>,
    current: Cell<Slot>,
    problems: RefCell<Vec<(Slot, String)>>,
    notes: RefCell<Vec<String>>,
}

/// A candidate image from a source: its id, where to get it, and whether it must already have the
/// slot's shape (ScreenScraper's `steamgrid`, whose shape is not documented).
struct Cand {
    image_id: u64,
    url: String,
    require_native: bool,
}

/// Why a downloaded image was not used.
enum EvalError {
    /// A logo with no transparency.
    Opaque,
    /// Not decodable, too small, or not the slot's shape.
    Unfit,
}

/// Turns downloaded bytes into a slot image or says why not.
fn evaluate(slot: Slot, bytes: &[u8], require_native: bool) -> Result<Fitted, EvalError> {
    let img = decode(bytes).map_err(|_| EvalError::Unfit)?;
    let fitted = if slot == Slot::Icon { fit_icon_keep_size(&img) } else { fit(slot, &img) }.map_err(|_| EvalError::Unfit)?;
    if slot == Slot::Logo && !has_transparency(&fitted.image) {
        return Err(EvalError::Opaque);
    }
    if require_native && fitted.generated {
        return Err(EvalError::Unfit);
    }
    Ok(fitted)
}

impl Run<'_, '_> {
    fn now(&self) -> u64 {
        self.s.now
    }

    fn problem(&self, text: String) {
        let slot = self.current.get();
        let mut problems = self.problems.borrow_mut();
        if !problems.iter().any(|(_, t)| *t == text) {
            problems.push((slot, text));
        }
    }

    fn note(&self, text: String) {
        let mut notes = self.notes.borrow_mut();
        if !notes.contains(&text) {
            notes.push(text);
        }
    }

    /// Can `src` be asked right now? Says why not (once per session) when it cannot.
    fn can_ask(&self, src: SourceId) -> bool {
        if self.s.mode == Mode::CacheOnly {
            return false;
        }
        if (self.cancel)() {
            return false;
        }
        let ready = match src {
            SourceId::SteamGridDb => self.sgdb.is_some(),
            SourceId::TheGamesDb => self.tgdb.is_some(),
            SourceId::ScreenScraper => self.ss.is_some(),
        };
        if !ready {
            return false;
        }
        if self.s.dead.borrow().contains_key(&src) {
            return false;
        }
        if let Some((secs, reason)) = self.s.states.borrow().blocked(src, self.now()) {
            if self.s.reported.borrow_mut().insert(src) {
                self.problem(format!("{}: {}", src.label(), SourceError::BackedOff { secs, reason }));
            }
            return false;
        }
        true
    }

    /// Records the outcome of one call and passes the value on.
    fn call<T>(&self, src: SourceId, result: Result<T, SourceError>) -> Option<T> {
        match result {
            Ok(v) => {
                self.s.states.borrow_mut().record_ok(src, self.now());
                self.s.fails.borrow_mut().remove(&src);
                Some(v)
            }
            Err(SourceError::NotFound) => None,
            Err(e) => {
                let secrets = self.s.secret_values();
                self.s.states.borrow_mut().record_error(src, &e, self.now(), &secrets);
                if e.stops_the_run() {
                    self.s.dead.borrow_mut().insert(src, e.to_string());
                    self.s.reported.borrow_mut().insert(src);
                }
                self.problem(format!("{}: {e}", src.label()));
                None
            }
        }
    }

    // ---- game ids ----------------------------------------------------------------------------

    fn title_search_id(&self, src: SourceId, over: Option<u64>) -> Option<u64> {
        if let Some(id) = over {
            // A choice written in `ports.local.json` settles the matter: a look-alike waiting for this port
            // is no longer waiting.
            let mut ids = self.s.ids.borrow_mut();
            if !ids.is_confirmed(self.req.port_key, src, id) {
                ids.confirm(self.req.port_key, src, id, "chosen in ports.local.json");
            }
            return Some(id);
        }
        let key = self.req.port_key;
        if let Some(e) = self.s.ids.borrow().usable(key, src) {
            return Some(e.id);
        }
        if self.s.ids.borrow().searched_recently(key, src, self.now()) || !self.can_ask(src) {
            return None;
        }
        let name = self.req.name;
        let found: Vec<(u64, String, Confidence)> = match src {
            SourceId::SteamGridDb => {
                let games = self.call(src, self.sgdb.as_ref()?.search(name))?;
                sgdb::ranked(name, &games).into_iter().map(|(g, c)| (g.id, g.name.clone(), c)).collect()
            }
            SourceId::TheGamesDb => {
                let games = self.call(src, self.tgdb.as_ref()?.search(name))?;
                tgdb::ranked(name, &games).into_iter().map(|(g, c)| (g.id, g.title, c)).collect()
            }
            SourceId::ScreenScraper => return None,
        };
        let now = self.now();
        let mut ids = self.s.ids.borrow_mut();
        match found.first() {
            Some((id, title, conf)) if conf.is_silent() => {
                ids.set_found(key, src, IdEntry { id: *id, name: title.clone(), origin: IdOrigin::Search, confidence: *conf }, now);
                Some(*id)
            }
            Some(_) => {
                let candidates: Vec<Pending> = found.iter().filter(|(_, _, c)| *c >= Confidence::Medium).map(|(id, n, c)| Pending { id: *id, name: n.clone(), confidence: *c }).collect();
                if let Some(first) = candidates.first() {
                    self.note(format!(
                        "{}: \"{}\" looks like \"{}\" but is not the same title, so it is not used. Set {} for this port in ports.local.json to choose it.",
                        src.label(),
                        first.name,
                        name,
                        if src == SourceId::SteamGridDb { "sgdb_id" } else { "tgdb_id" }
                    ));
                }
                ids.set_pending(key, src, candidates, now);
                None
            }
            None => {
                ids.set_pending(key, src, Vec::new(), now);
                None
            }
        }
    }

    // ---- one image from one source -----------------------------------------------------------

    /// Cache first, then the candidates (at most [`MAX_DOWNLOADS`] downloads): the first that has the
    /// slot's shape wins, else the first composed one.
    fn fill(&self, src: SourceId, game_id: u64, slot: Slot, list: &dyn Fn() -> Option<Vec<Cand>>, download: &dyn Fn(&str) -> Result<Vec<u8>, SourceError>) -> Option<Fitted> {
        let now = self.now();
        match self.s.cache.image_get(src, game_id, slot, now) {
            CacheLookup::Hit { bytes, .. } => {
                if let Ok(f) = evaluate(slot, &bytes, false) {
                    return Some(f.with_source(src.id()));
                }
            }
            CacheLookup::KnownMissing => return None,
            CacheLookup::Miss => {}
        }
        if !self.can_ask(src) {
            return None;
        }
        let cands = list()?;
        let mut best: Option<(Fitted, Vec<u8>, u64)> = None;
        let mut net_trouble = false;
        // Downloads that count against the budget: every one except a logo thrown out for having no
        // transparency (SteamGridDB lists opaque ones among the real ones); those are only capped overall.
        let mut spent = 0;
        for (n, cand) in cands.iter().enumerate() {
            if spent >= MAX_DOWNLOADS || n >= MAX_DOWNLOADS * 2 {
                break;
            }
            let bytes = match download(&cand.url) {
                Ok(b) => {
                    self.s.fails.borrow_mut().remove(&src);
                    b
                }
                Err(SourceError::NotFound) => {
                    spent += 1;
                    continue;
                }
                Err(e) => {
                    spent += 1;
                    net_trouble = true;
                    // One image that will not come is not the source's fault: skip it. Three in a row, or
                    // anything that speaks for the whole source (a limit, a bad key), ends the session's use of it.
                    let one_image = matches!(e, SourceError::Offline(_) | SourceError::BadResponse(_));
                    let streak = {
                        let mut fails = self.s.fails.borrow_mut();
                        let n = fails.entry(src).or_insert(0);
                        *n += 1;
                        *n
                    };
                    if one_image && streak < MAX_IMAGE_FAILURES {
                        self.problem(format!("{}: one image could not be downloaded ({e})", src.label()));
                        continue;
                    }
                    self.call::<()>(src, Err(e));
                    if one_image {
                        self.s.dead.borrow_mut().entry(src).or_insert_with(|| "images keep failing to download".to_string());
                    }
                    if !self.can_ask(src) {
                        break;
                    }
                    continue;
                }
            };
            match evaluate(slot, &bytes, cand.require_native) {
                Ok(f) if !f.generated => {
                    best = Some((f, bytes, cand.image_id));
                    break;
                }
                Ok(f) => {
                    spent += 1;
                    if best.is_none() {
                        best = Some((f, bytes, cand.image_id));
                    }
                }
                Err(EvalError::Opaque) => {}
                Err(EvalError::Unfit) => spent += 1,
            }
        }
        match best {
            Some((f, bytes, image_id)) => {
                let _ = self.s.cache.image_put(src, game_id, slot, &bytes, image_id, now);
                Some(f.with_source(src.id()))
            }
            None => {
                if !net_trouble {
                    let _ = self.s.cache.missing_put(src, game_id, slot, now);
                }
                None
            }
        }
    }

    fn from_sgdb(&self, slot: Slot) -> Option<Fitted> {
        let id = self.title_search_id(SourceId::SteamGridDb, self.req.sgdb_id)?;
        let sgdb = self.sgdb.as_ref();
        let list = || -> Option<Vec<Cand>> {
            let assets = match sgdb?.assets(id, sgdb::filter_for(slot)) {
                // No images for this game is an answer (remembered for a few days), not a failure.
                Err(SourceError::NotFound) => Vec::new(),
                other => self.call(SourceId::SteamGridDb, other)?,
            };
            Some(assets.into_iter().map(|a| Cand { image_id: a.id, url: a.url, require_native: false }).collect())
        };
        let download = |url: &str| match sgdb {
            Some(c) => c.download(url),
            None => Err(SourceError::NotConfigured("no SteamGridDB key".into())),
        };
        self.fill(SourceId::SteamGridDb, id, slot, &list, &download)
    }

    fn tgdb_images(&self, id: u64) -> Option<Rc<ImageSet>> {
        if let Some(memo) = self.tgdb_images.borrow().get(&id) {
            return memo.clone();
        }
        let set = self.tgdb.as_ref().and_then(|c| match c.images(id) {
            Err(SourceError::NotFound) => Some(ImageSet::default()),
            other => self.call(SourceId::TheGamesDb, other),
        });
        let set = set.map(Rc::new);
        self.tgdb_images.borrow_mut().insert(id, set.clone());
        set
    }

    fn from_tgdb(&self, slot: Slot) -> Option<Fitted> {
        if slot == Slot::Icon {
            return None;
        }
        let id = self.title_search_id(SourceId::TheGamesDb, self.req.tgdb_id)?;
        let list = || -> Option<Vec<Cand>> {
            let set = self.tgdb_images(id)?;
            Some(set.candidates(slot).into_iter().map(|c| Cand { image_id: c.image_id, url: c.url, require_native: false }).collect())
        };
        let download = |url: &str| match self.tgdb.as_ref() {
            Some(c) => c.download(url),
            None => Err(SourceError::NotConfigured("no TheGamesDB key".into())),
        };
        self.fill(SourceId::TheGamesDb, id, slot, &list, &download)
    }

    /// ScreenScraper's game for the user's ROM, found by hash, else by the ROM's file name.
    fn ss_game(&self) -> Option<Rc<screenscraper::Game>> {
        if let Some(memo) = self.ss_game.borrow().as_ref() {
            return memo.clone();
        }
        let found = self.find_ss_game().map(Rc::new);
        *self.ss_game.borrow_mut() = Some(found.clone());
        found
    }

    fn rom_key(rom: &RomIdentity) -> String {
        format!("{}:{}:{}", rom.system_id.unwrap_or(0), rom.md5, rom.size)
    }

    fn find_ss_game(&self) -> Option<screenscraper::Game> {
        let ss = self.ss.as_ref()?;
        let first = self.req.rom.first()?;
        let known = self.req.rom.iter().find_map(|r| self.s.ids.borrow().rom_game(&Self::rom_key(r)));
        if let Some(id) = known {
            if !self.can_ask(SourceId::ScreenScraper) {
                return None;
            }
            return self.call(SourceId::ScreenScraper, ss.game_by_id(id));
        }
        for rom in &self.req.rom {
            if !self.can_ask(SourceId::ScreenScraper) {
                return None;
            }
            match ss.game_by_rom(rom) {
                Ok(game) => {
                    self.s.states.borrow_mut().record_ok(SourceId::ScreenScraper, self.now());
                    self.s.ids.borrow_mut().set_rom_game(&Self::rom_key(rom), game.id);
                    return Some(game);
                }
                Err(SourceError::NotFound) => continue,
                Err(e) => {
                    self.call::<()>(SourceId::ScreenScraper, Err(e));
                    return None;
                }
            }
        }
        // No hash match: search by the ROM's own file name, and only trust the same title.
        if !self.can_ask(SourceId::ScreenScraper) {
            return None;
        }
        let stem = first.file_name.rsplit_once('.').map_or(first.file_name.as_str(), |(s, _)| s);
        let games = self.call(SourceId::ScreenScraper, ss.search(stem, first.system_id))?;
        match screenscraper::best_match(stem, &games) {
            Some((game, conf)) if conf.is_silent() => {
                let game = game.clone();
                if game.medias.is_empty() {
                    return self.call(SourceId::ScreenScraper, ss.game_by_id(game.id));
                }
                Some(game)
            }
            Some((game, _)) => {
                self.note(format!("ScreenScraper: \"{}\" looks like \"{stem}\" but is not the same title, so it is not used.", game.name));
                None
            }
            None => None,
        }
    }

    fn from_screenscraper(&self, slot: Slot) -> Option<Fitted> {
        if slot == Slot::Icon || self.req.rom.is_empty() {
            return None;
        }
        // The game id is known from an earlier run: the cache can answer without a call.
        let cached_id = self.req.rom.iter().find_map(|r| self.s.ids.borrow().rom_game(&Self::rom_key(r)));
        let id = match cached_id {
            Some(id) => id,
            None => {
                if self.s.mode == Mode::CacheOnly || !self.can_ask(SourceId::ScreenScraper) {
                    return None;
                }
                self.ss_game()?.id
            }
        };
        let list = || -> Option<Vec<Cand>> {
            let game = self.ss_game()?;
            let region = self.s.region.as_str();
            let kinds: &[&str] = match slot {
                Slot::Portrait => &["box-2D"],
                Slot::Logo => &["wheel-hd", "wheel"],
                Slot::Hero => &["fanart", "ss"],
                Slot::Wide => &["steamgrid", "fanart", "ss"],
                Slot::Icon => &[],
            };
            let cands = screenscraper::media_of(&game, kinds, region)
                .into_iter()
                .enumerate()
                .map(|(i, m)| Cand { image_id: i as u64, url: m.url.clone(), require_native: m.kind == "steamgrid" })
                .collect();
            Some(cands)
        };
        let download = |url: &str| match self.ss.as_ref() {
            Some(c) => c.download(url),
            None => Err(SourceError::NotConfigured("this build has no ScreenScraper developer credentials".into())),
        };
        self.fill(SourceId::ScreenScraper, id, slot, &list, &download)
    }

    fn from_source(&self, src: SourceId, slot: Slot) -> Option<Fitted> {
        match src {
            SourceId::SteamGridDb => self.from_sgdb(slot),
            SourceId::TheGamesDb => self.from_tgdb(slot),
            SourceId::ScreenScraper => self.from_screenscraper(slot),
        }
    }

    /// The first image with the slot's shape from `order`, else the first composed one.
    fn best_of(&self, slot: Slot, order: &[SourceId]) -> Option<Fitted> {
        self.current.set(slot);
        let mut composed = None;
        for src in order {
            if let Some(f) = self.from_source(*src, slot) {
                if !f.generated {
                    return Some(f);
                }
                composed.get_or_insert(f);
            }
        }
        composed
    }

    // ---- the slots ---------------------------------------------------------------------------

    fn resolve(&self) -> PortArt {
        if !self.req.folder.is_empty() {
            self.s.ids.borrow_mut().set_identity(self.req.port_key, self.req.name, self.req.folder);
        }
        let need = |slot: Slot| self.req.need.contains(&slot);
        let mut set = ArtSet::default();
        let (catalog_portrait, catalog_icon) = (self.req.catalog.set.portrait.clone(), self.req.catalog.set.icon.clone());

        if need(Slot::Hero) {
            if let Some(f) = self.best_of(Slot::Hero, &[SourceId::SteamGridDb, SourceId::TheGamesDb, SourceId::ScreenScraper]) {
                set.set(f);
            }
        }
        if need(Slot::Wide) {
            let mut wide = self.best_of(Slot::Wide, &[SourceId::SteamGridDb, SourceId::TheGamesDb, SourceId::ScreenScraper]);
            if wide.as_ref().is_none_or(|w| w.generated) {
                if let Some(hero) = &set.hero {
                    // A hero is three times wider than tall; cropping it gives the wide capsule without a new download.
                    if wide.is_none() {
                        wide = derive(Slot::Wide, hero);
                    }
                }
            }
            if let Some(f) = wide {
                set.set(f);
            }
        }
        if need(Slot::Logo) {
            if let Some(f) = self.best_of(Slot::Logo, &[SourceId::SteamGridDb, SourceId::TheGamesDb, SourceId::ScreenScraper]) {
                set.set(f);
            }
        }
        if need(Slot::Portrait) {
            self.current.set(Slot::Portrait);
            match catalog_portrait {
                Some(c) if !c.generated => set.set(c),
                other => {
                    let online = self.best_of(Slot::Portrait, &[SourceId::SteamGridDb, SourceId::ScreenScraper, SourceId::TheGamesDb]);
                    match (online, other) {
                        (Some(f), _) if !f.generated => set.set(f),
                        // Both are composed: the catalog's own picture is the better bet.
                        (Some(_), Some(c)) => set.set(c),
                        (Some(f), None) => set.set(f),
                        (None, Some(c)) => set.set(c),
                        (None, None) => {}
                    }
                }
            }
        }
        if need(Slot::Icon) {
            self.current.set(Slot::Icon);
            let icon = catalog_icon.or_else(|| self.from_sgdb(Slot::Icon)).or_else(|| set.logo.as_ref().and_then(|l| derive(Slot::Icon, l))).or_else(|| set.portrait.as_ref().and_then(|p| derive(Slot::Icon, p)));
            if let Some(f) = icon {
                set.set(f);
            }
        }

        let mut sources = Vec::new();
        let mut notes = self.notes.take();
        for slot in Slot::ALL {
            if let Some(f) = set.get(slot) {
                sources.push(SlotSource { slot, source: f.source.clone(), generated: f.generated });
                notes.push(format!("{}: {}{}", slot.name(), f.source, if f.generated { " (composed)" } else { "" }));
            } else if need(slot) {
                notes.push(format!("{}: none found", slot.name()));
            }
        }
        let mut problems: Vec<(Slot, String)> = self.req.catalog.problems.iter().map(|(s, e)| (*s, e.to_string())).collect();
        problems.extend(self.problems.take());
        PortArt { set, problems, notes, sources }
    }
}

/// What a port gets when no online source is set up: the catalog's portrait and icon, nothing else,
/// the same as [`catalog_art_from_files`] gave before the online sources existed.
fn catalog_only(req: &Request<'_>) -> PortArt {
    let mut set = ArtSet::default();
    let mut sources = Vec::new();
    for slot in [Slot::Portrait, Slot::Icon] {
        if let Some(f) = req.catalog.set.get(slot).filter(|_| req.need.contains(&slot)) {
            sources.push(SlotSource { slot, source: f.source.clone(), generated: f.generated });
            set.set(f.clone());
        }
    }
    let problems = req.catalog.problems.iter().map(|(s, e)| (*s, e.to_string())).collect();
    PortArt { set, problems, notes: Vec::new(), sources }
}

/// A slot image cropped or padded from another slot's image of the same game, flagged as composed.
fn derive(slot: Slot, from: &Fitted) -> Option<Fitted> {
    let mut out = fit(slot, &DynamicImage::ImageRgba8(from.image.clone())).ok()?;
    out.source = from.source.clone();
    out.generated = true;
    Some(out)
}

// ---- entry points for the screens ----------------------------------------------------------------

/// The identity of a port as the art code needs it.
#[derive(Clone, Copy)]
pub struct PortRef<'a> {
    pub key: &'a str,
    pub folder: &'a str,
    pub name: &'a str,
    /// The catalog's `instructions` text, which says which game files the port needs. `None`: no
    /// ScreenScraper lookup.
    pub instructions: Option<&'a str>,
}

impl<'a> PortRef<'a> {
    pub fn from_port(port: &'a Port) -> PortRef<'a> {
        PortRef { key: port.key(), folder: &port.folder, name: port.display_name(), instructions: Some(&port.instructions) }
    }
}

fn id_field(entry: Option<&Value>, name: &str) -> Option<u64> {
    let v = entry?.get(name)?;
    v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).filter(|id| *id > 0)
}

/// The user's scanned ROM of the game a port needs, hash-confirmed only (a name-only match is not
/// good enough to ask ScreenScraper about). Empty when there is none.
pub fn rom_identities(config_dir: &Path, port: &PortRef<'_>, local_entry: Option<&Value>) -> Vec<RomIdentity> {
    match load_scan(config_dir) {
        Some(scan) => rom_identities_in(&scan, port, local_entry),
        None => Vec::new(),
    }
}

/// The ROM library's saved scan, if the user has scanned a folder.
fn load_scan(config_dir: &Path) -> Option<ScanResult> {
    RomLibraryStore::load(&config_dir.join("rom_library.json")).ok()?.load_scan_cache()
}

fn rom_identities_in(scan: &ScanResult, port: &PortRef<'_>, local_entry: Option<&Value>) -> Vec<RomIdentity> {
    let Some(instructions) = port.instructions else { return Vec::new() };
    // `RomSpec::for_port` wants a catalog entry; only its name, folder and instructions matter here.
    let Ok(stub) = crate::core::models::port_from_value(&serde_json::json!({ "name": port.name, "folder": port.folder, "instructions": instructions })) else { return Vec::new() };
    let spec = RomSpec::for_port(&stub, local_entry);
    let matched = match_port(port.key, port.name, &spec, &scan.files);
    for slot in &matched.slots {
        if let SlotStatus::Found(found) = &slot.status {
            if found.is_low_confidence() {
                continue;
            }
            if let Some(file) = scan.files.iter().find(|f| f.path == found.path) {
                return RomIdentity::from_rom_file(file);
            }
        }
    }
    Vec::new()
}

/// Slots of a shortcut that still want art: empty ones, and ones holding only composed art this app
/// wrote (a real image may have turned up since). Art the user chose is never listed.
pub fn slots_needing_art(grid_dir: &Path, app_id: u32) -> Vec<Slot> {
    let manifest = Manifest::load(grid_dir);
    Slot::ALL
        .into_iter()
        .filter(|slot| match existing_artwork(grid_dir, app_id, slot.kind()) {
            Ok(files) if files.is_empty() => true,
            Ok(files) => files.iter().all(|p| manifest.is_ours(p) && p.file_name().and_then(|n| n.to_str()).and_then(|n| manifest.entries().get(n)).is_some_and(|e| e.generated)),
            Err(_) => false,
        })
        .collect()
}

/// What to resolve for a shortcut: the slots that want art (see [`slots_needing_art`]) plus the portrait
/// and the icon. Those two come from the catalog without any lookup, and the Add to Steam step needs the
/// icon's path back even when our own icon is already in place; the writer still leaves anything the user
/// chose alone.
pub fn slots_to_resolve(grid_dir: &Path, app_id: u32) -> Vec<Slot> {
    let mut slots = slots_needing_art(grid_dir, app_id);
    for always in [Slot::Portrait, Slot::Icon] {
        if !slots.contains(&always) {
            slots.push(always);
        }
    }
    slots
}

/// The one-call form for a screen: the catalog portrait and icon (from the files the app cached),
/// improved by whatever sources are set up. With no source set up, and nothing cached, the result is
/// exactly what [`catalog_art_from_files`] gives.
///
/// `grid` is `(grid folder, shortcut app id)`; given, it limits the lookup to slots that want art.
/// `Mode::Online` makes network calls on the calling thread, so call it from a worker thread; use
/// `Mode::CacheOnly` on the UI thread.
pub fn art_for_port(dirs: ArtDirs<'_>, port: PortRef<'_>, portrait: Option<&Path>, icon: Option<&Path>, grid: Option<(&Path, u32)>, mode: Mode) -> PortArt {
    let session = Session::open(dirs, mode);
    session.resolve_port(dirs.config_dir, port, portrait, icon, grid)
}

impl Session {
    /// [`art_for_port`] on an existing session, for resolving many ports in a row.
    pub fn resolve_port(&self, config_dir: &Path, port: PortRef<'_>, portrait: Option<&Path>, icon: Option<&Path>, grid: Option<(&Path, u32)>) -> PortArt {
        self.resolve_port_with(config_dir, port, portrait, icon, grid, &|| false)
    }

    /// [`Session::resolve_port`] with a cancel check (see [`Session::resolve_with`]).
    pub fn resolve_port_with(&self, config_dir: &Path, port: PortRef<'_>, portrait: Option<&Path>, icon: Option<&Path>, grid: Option<(&Path, u32)>, cancel: &dyn Fn() -> bool) -> PortArt {
        let catalog = catalog_art_from_files(portrait, icon);
        let local = load_local_entries(&config_dir.join("ports.local.json")).remove(port.folder);
        let mut req = Request::new(port.key, port.name);
        req.folder = port.folder;
        req.catalog = catalog;
        req.sgdb_id = id_field(local.as_ref(), "sgdb_id");
        req.tgdb_id = id_field(local.as_ref(), "tgdb_id");
        if let Some((grid_dir, app_id)) = grid {
            req.need = slots_to_resolve(grid_dir, app_id);
        }
        if self.configured(SourceId::ScreenScraper) {
            let scan = self.scan.borrow_mut().get_or_insert_with(|| load_scan(config_dir).map(Rc::new)).clone();
            if let Some(scan) = scan {
                req.rom = rom_identities_in(&scan, &port, local.as_ref());
            }
        }
        self.resolve_with(&req, cancel)
    }
}
