//! Small cover images for the card screens. A cover is the catalog's `image` for the port, fetched in the
//! background into the same cache the main grid uses (`cache/<folder>.png`, so a cover the grid already has
//! costs nothing), decoded off the UI thread and shrunk to a thumbnail (at most [`MAX_EDGE`] pixels on a
//! side). The UI thread only collects the finished pixels with [`ThumbLoader::drain`] on a timer and turns
//! them into images; it never decodes or waits on the network.
//!
//! Offline, a cover that is not cached yet simply never arrives (the fetch gives up after its 10 s timeout)
//! and the screen keeps its name tile. A cover is asked for once per run, whether or not it arrived.

use crate::core::image_cache;
use image::GenericImageView;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

/// The longest side of a thumbnail. Covers are shown at under 200 px, so this keeps them sharp and small.
pub const MAX_EDGE: u32 = 256;

/// Which cover a card or tile shows: the cache key (the port's catalog folder) and where to fetch it from
/// when it is not cached yet.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArtRef {
    pub folder: String,
    pub url: Option<String>,
}

impl ArtRef {
    /// The cover of a catalog port; with no `image` link there is nothing to fetch, only a cached file to find.
    pub fn of_port(port: &crate::core::models::Port) -> ArtRef {
        let url = port.image.clone().filter(|u| crate::core::launch::is_web_url(u));
        ArtRef { folder: port.folder.clone(), url }
    }
}

/// Decoded pixels, straight RGBA, `width * height * 4` bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumb {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Decodes an image and shrinks it to fit `max_edge`; an image already that small is kept as it is.
/// Biggest picture accepted, in pixels a side, and the most memory its decoding may ask for: a cover is a few
/// hundred pixels, so a huge or hostile file is refused before it is decoded.
const MAX_SIDE: u32 = 8192;
const MAX_ALLOC: u64 = 64 * 1024 * 1024;

pub fn decode(bytes: &[u8], max_edge: u32) -> Option<Thumb> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let img = if w.max(h) > max_edge { img.thumbnail(max_edge, max_edge) } else { img };
    let rgba = img.into_rgba8();
    let (width, height) = rgba.dimensions();
    Some(Thumb { width, height, rgba: rgba.into_raw() })
}

/// The cached cover for `folder`, decoded and shrunk. `None` when it is not cached or is not an image.
pub fn load_cached(cache_dir: &Path, folder: &str, max_edge: u32) -> Option<Thumb> {
    let path = image_cache::cached_image_path(cache_dir, folder).ok()?;
    decode(&std::fs::read(path).ok()?, max_edge)
}

/// How a cover that is not cached gets fetched; the app uses the real one, tests a fake.
pub trait Fetcher: Send + Sync + 'static {
    fn fetch(&self, url: &str, cache_dir: &Path, folder: &str);
}

/// The app's cover download (`core::image_cache`, 10 s timeout, only keeps real images).
pub struct NetFetcher;

impl Fetcher for NetFetcher {
    fn fetch(&self, url: &str, cache_dir: &Path, folder: &str) {
        image_cache::cache_image(url, cache_dir, folder);
    }
}

/// A cover that finished loading (or that could not be had: `thumb` is `None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub folder: String,
    pub thumb: Option<Thumb>,
}

struct Shared {
    cache_dir: PathBuf,
    fetcher: Box<dyn Fetcher>,
    asked: Mutex<HashSet<String>>,
    ready: Mutex<Vec<Loaded>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Loads covers on a background thread. Clones share the same state.
#[derive(Clone)]
pub struct ThumbLoader {
    shared: Arc<Shared>,
}

impl ThumbLoader {
    pub fn new(cache_dir: PathBuf) -> ThumbLoader {
        ThumbLoader::with_fetcher(cache_dir, Box::new(NetFetcher))
    }

    pub fn with_fetcher(cache_dir: PathBuf, fetcher: Box<dyn Fetcher>) -> ThumbLoader {
        ThumbLoader { shared: Arc::new(Shared { cache_dir, fetcher, asked: Mutex::new(HashSet::new()), ready: Mutex::new(Vec::new()) }) }
    }

    /// Starts loading the covers not asked for before, one background thread for the batch. Cached covers come
    /// first (no network), then the ones to fetch.
    pub fn request(&self, refs: &[ArtRef]) {
        let fresh: Vec<ArtRef> = {
            let mut asked = lock(&self.shared.asked);
            refs.iter().filter(|r| asked.insert(r.folder.clone())).cloned().collect()
        };
        if fresh.is_empty() {
            return;
        }
        let shared = self.shared.clone();
        std::thread::spawn(move || {
            let is_cached = |r: &ArtRef| image_cache::cached_image_path(&shared.cache_dir, &r.folder).is_ok_and(|p| p.exists());
            let (cached, missing): (Vec<ArtRef>, Vec<ArtRef>) = fresh.into_iter().partition(|r| is_cached(r));
            for r in cached.into_iter().chain(missing) {
                // One bad cover (a panic in a download or a decode) must not stall the ones after it.
                let one = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if let (Some(url), false) = (&r.url, is_cached(&r)) {
                        shared.fetcher.fetch(url, &shared.cache_dir, &r.folder);
                    }
                    load_cached(&shared.cache_dir, &r.folder, MAX_EDGE)
                }));
                let thumb = one.ok().flatten();
                lock(&shared.ready).push(Loaded { folder: r.folder, thumb });
            }
        });
    }

    /// The covers that finished since the last call (each once).
    pub fn drain(&self) -> Vec<Loaded> {
        std::mem::take(&mut *lock(&self.shared.ready))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    fn temp_dir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("dpm_thumbs_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn png(w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbaImage::from_pixel(w, h, image::Rgba(color)).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    fn collect(loader: &ThumbLoader, n: usize) -> Vec<Loaded> {
        let start = Instant::now();
        let mut got = Vec::new();
        while got.len() < n {
            got.extend(loader.drain());
            assert!(start.elapsed() < Duration::from_secs(10), "covers never arrived: {got:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
        got
    }

    /// Writes the cache file the way the real download does.
    struct WritesPng;

    impl Fetcher for WritesPng {
        fn fetch(&self, _url: &str, cache_dir: &Path, folder: &str) {
            std::fs::write(image_cache::cached_image_path(cache_dir, folder).unwrap(), png(600, 900, [10, 20, 30, 255])).unwrap();
        }
    }

    struct Offline;

    impl Fetcher for Offline {
        fn fetch(&self, _url: &str, _cache_dir: &Path, _folder: &str) {}
    }

    #[test]
    fn a_big_cover_is_shrunk_to_fit_and_keeps_its_shape() {
        let t = decode(&png(600, 900, [1, 2, 3, 255]), MAX_EDGE).unwrap();
        assert_eq!((t.width, t.height), (171, 256), "2:3 stays 2:3, longest side 256");
        assert_eq!(t.rgba.len(), 171 * 256 * 4);
        assert_eq!(&t.rgba[..4], &[1, 2, 3, 255]);
    }

    #[test]
    fn a_small_cover_is_not_blown_up() {
        let t = decode(&png(60, 90, [9, 9, 9, 255]), MAX_EDGE).unwrap();
        assert_eq!((t.width, t.height), (60, 90));
    }

    #[test]
    fn a_wide_cover_is_limited_by_its_width() {
        let t = decode(&png(1000, 500, [5, 5, 5, 255]), 100).unwrap();
        assert_eq!((t.width, t.height), (100, 50));
    }

    #[test]
    fn something_that_is_not_an_image_gives_nothing() {
        assert!(decode(b"<html>not found</html>", MAX_EDGE).is_none());
        assert!(decode(b"", MAX_EDGE).is_none());
    }

    #[test]
    fn a_cached_cover_loads_without_the_network() {
        let dir = temp_dir("cached");
        std::fs::write(dir.join("Dusk.png"), png(300, 300, [7, 7, 7, 255])).unwrap();
        let t = load_cached(&dir, "Dusk", MAX_EDGE).unwrap();
        assert_eq!((t.width, t.height), (256, 256));
        assert!(load_cached(&dir, "Missing", MAX_EDGE).is_none());
        assert!(load_cached(&dir, "../escape", MAX_EDGE).is_none(), "a folder name cannot leave the cache");
    }

    #[test]
    fn a_cached_cover_is_never_downloaded_again() {
        let dir = temp_dir("no_refetch");
        std::fs::write(dir.join("Dusk.png"), png(100, 100, [1, 1, 1, 255])).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let loader = ThumbLoader::with_fetcher(dir.clone(), Box::new(Counts(calls.clone())));
        loader.request(&[ArtRef { folder: "Dusk".into(), url: Some("https://example.invalid/dusk.png".into()) }]);
        assert!(collect(&loader, 1)[0].thumb.is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 0, "no download for a cover already in the cache");
        assert_eq!(std::fs::read(dir.join("Dusk.png")).unwrap(), png(100, 100, [1, 1, 1, 255]), "the cached file is untouched");
    }

    struct Counts(Arc<AtomicUsize>);

    impl Fetcher for Counts {
        fn fetch(&self, _url: &str, _cache_dir: &Path, _folder: &str) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn a_missing_cover_is_fetched_into_the_cache_and_then_loaded() {
        let dir = temp_dir("fetch");
        let loader = ThumbLoader::with_fetcher(dir.clone(), Box::new(WritesPng));
        loader.request(&[ArtRef { folder: "Soh".into(), url: Some("https://example.invalid/soh.png".into()) }]);
        let got = collect(&loader, 1);
        assert_eq!(got[0].folder, "Soh");
        let t = got[0].thumb.as_ref().unwrap();
        assert_eq!((t.width, t.height), (171, 256));
        assert!(dir.join("Soh.png").exists(), "kept in the shared cache for the main grid too");
    }

    #[test]
    fn offline_a_missing_cover_arrives_as_nothing_and_is_not_asked_for_twice() {
        let dir = temp_dir("offline");
        let loader = ThumbLoader::with_fetcher(dir, Box::new(Offline));
        let r = ArtRef { folder: "Soh".into(), url: Some("https://example.invalid/soh.png".into()) };
        loader.request(std::slice::from_ref(&r));
        let got = collect(&loader, 1);
        assert_eq!(got, [Loaded { folder: "Soh".into(), thumb: None }]);
        loader.request(&[r]);
        std::thread::sleep(Duration::from_millis(50));
        assert!(loader.drain().is_empty(), "asked once per run");
    }

    #[test]
    fn a_port_without_art_still_loads_nothing_and_does_not_hang() {
        let dir = temp_dir("noart");
        let loader = ThumbLoader::with_fetcher(dir, Box::new(Offline));
        loader.request(&[ArtRef { folder: "Plain".into(), url: None }]);
        assert_eq!(collect(&loader, 1)[0].thumb, None);
    }

    #[test]
    fn cached_covers_are_delivered_before_the_ones_that_need_the_network() {
        let dir = temp_dir("order");
        std::fs::write(dir.join("Have.png"), png(50, 50, [1, 1, 1, 255])).unwrap();
        let loader = ThumbLoader::with_fetcher(dir, Box::new(WritesPng));
        loader.request(&[
            ArtRef { folder: "Need".into(), url: Some("https://example.invalid/need.png".into()) },
            ArtRef { folder: "Have".into(), url: Some("https://example.invalid/have.png".into()) },
        ]);
        let got = collect(&loader, 2);
        assert_eq!(got[0].folder, "Have");
        assert_eq!(got[1].folder, "Need");
    }

    #[test]
    fn the_cover_of_a_catalog_port_uses_its_folder_and_only_a_web_link() {
        use serde_json::json;
        let mk = |image: serde_json::Value| crate::core::models::port_from_value(&json!({"name": "Dusklight", "folder": "dusklight", "source": "https://github.com/a/dusk", "image": image})).unwrap();
        let with = ArtRef::of_port(&mk(json!("https://example.com/dusk.png")));
        assert_eq!((with.folder.as_str(), with.url.as_deref()), ("dusklight", Some("https://example.com/dusk.png")));
        let local = ArtRef::of_port(&mk(json!("file:///etc/passwd")));
        assert_eq!(local.url, None, "only web links are ever fetched");
    }

    #[test]
    fn a_picture_wider_than_any_cover_is_refused_before_it_is_decoded() {
        assert!(decode(&png(9000, 1, [1, 1, 1, 255]), MAX_EDGE).is_none());
        assert!(decode(&png(1, 9000, [1, 1, 1, 255]), MAX_EDGE).is_none());
        assert!(decode(&png(8000, 2, [1, 1, 1, 255]), MAX_EDGE).is_some(), "just inside the limit still loads");
    }

    struct Panics;

    impl Fetcher for Panics {
        fn fetch(&self, _url: &str, _cache_dir: &Path, folder: &str) {
            if folder == "Bad" {
                panic!("a download blew up");
            }
        }
    }

    #[test]
    fn one_cover_that_blows_up_does_not_stall_the_ones_after_it() {
        let dir = temp_dir("panics");
        std::fs::write(dir.join("Good.png"), png(40, 40, [3, 3, 3, 255])).unwrap();
        let loader = ThumbLoader::with_fetcher(dir, Box::new(Panics));
        loader.request(&[ArtRef { folder: "Bad".into(), url: Some("https://example.invalid/bad.png".into()) }, ArtRef { folder: "Good".into(), url: None }]);
        let got = collect(&loader, 2);
        let bad = got.iter().find(|l| l.folder == "Bad").unwrap();
        assert!(bad.thumb.is_none(), "the failed one arrives as nothing");
        assert!(got.iter().find(|l| l.folder == "Good").unwrap().thumb.is_some());
    }
}
