//! Tests of the resolver: priority, fallbacks, caching, back-off, "not configured in this build", and
//! that nothing secret is stored or said. Every answer is a saved fixture or generated in the test;
//! nothing here touches the network and no fixture holds a key.

use super::cache::{CacheLookup, IdTable};
use super::catalog::{catalog_art, CatalogArt};
use super::fit::Slot;
use super::grid::{write_art, ArtSet, Manifest, SlotOutcome, WriteOptions};
use super::net::fake::Fake;
use super::net::{Fetch, NetError, Response};
use super::resolve::*;
use super::screenscraper::RomIdentity;
use super::source::SourceId;
use crate::fork::rom_library::testutil::{n64_rom, write};
use crate::fork::rom_library::RomLibraryStore;
use crate::fork::secrets::{BuildCreds, Field, Secrets};
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::rc::Rc;

const SGDB_SEARCH: &str = include_str!("fixtures/sgdb_search.json");
const TGDB_IMAGES: &str = include_str!("fixtures/tgdb_images.json");
const TGDB_SEARCH: &str = include_str!("fixtures/tgdb_search.json");
const SS_INFO: &str = include_str!("fixtures/ss_jeuinfos.json");
const SS_SEARCH: &str = include_str!("fixtures/ss_recherche.json");

const SGDB_KEY: &str = "SGDB-TEST-KEY-AAAA1111";
const TGDB_KEY: &str = "TGDB-TEST-KEY-BBBB2222";

/// Lets a test keep a handle on the fake after the session took ownership of its box.
struct Shared(Rc<Fake>);

impl Fetch for Shared {
    fn get(&self, url: &str, headers: &[(&str, &str)], max_bytes: u64) -> Result<Response, NetError> {
        self.0.get(url, headers, max_bytes)
    }
}

fn png(w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    RgbaImage::from_pixel(w, h, Rgba(color)).write_to(&mut Cursor::new(&mut out), ImageFormat::Png).unwrap();
    out
}

/// A logo-shaped picture with a transparent half.
fn logo_png(w: u32, h: u32) -> Vec<u8> {
    let mut img = RgbaImage::from_pixel(w, h, Rgba([255, 255, 255, 255]));
    for y in 0..h / 2 {
        for x in 0..w {
            img.put_pixel(x, y, Rgba([0, 0, 0, 0]));
        }
    }
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).unwrap();
    out
}

fn sgdb_list(items: &[(u64, &str)]) -> String {
    let data: Vec<String> = items.iter().map(|(id, name)| format!(r#"{{"id":{id},"score":5,"style":"alternate","url":"https://cdn2.steamgriddb.com/img/{name}","tags":[]}}"#)).collect();
    format!(r#"{{"success":true,"data":[{}]}}"#, data.join(","))
}

fn secrets_at(dir: &Path, sgdb: Option<&str>, tgdb: Option<&str>) -> Secrets {
    let mut s = Secrets::load(&dir.join("secrets.json"));
    if let Some(k) = sgdb {
        s.set(Field::SteamGridDbKey, Some(k)).unwrap();
    }
    if let Some(k) = tgdb {
        s.set(Field::TheGamesDbKey, Some(k)).unwrap();
    }
    s
}

fn ss_build() -> BuildCreds {
    BuildCreds { ss_devid: Some("DEVID-SECRET-1".into()), ss_devpassword: Some("DEVPASS-SECRET-2".into()), ss_softname: None, tgdb_apikey: None }
}

fn session(dir: &Path, fake: &Rc<Fake>, secrets: Secrets, build: BuildCreds, mode: Mode, now: u64) -> Session {
    Session::with_parts(Box::new(Shared(fake.clone())), secrets, build, &dir.join("cache"), mode, now, Box::new(|_| {}))
}

fn catalog_set() -> CatalogArt {
    catalog_art(Some(&png(600, 900, [10, 20, 30, 255])), Some(&png(256, 256, [200, 0, 0, 255])))
}

fn request(name: &str) -> Request<'_> {
    let mut req = Request::new("owner/ship", name);
    req.catalog = catalog_set();
    req
}

fn sample_rom() -> RomIdentity {
    RomIdentity { system_id: Some(14), size: 8_388_608, crc32: "AABBCCDD".into(), md5: "0123456789abcdef0123456789abcdef".into(), sha1: None, file_name: "Game (USA).z64".into() }
}

fn sgdb_fake() -> Fake {
    Fake::new()
        .on_json("/search/autocomplete/", 200, SGDB_SEARCH)
        .on_json("/heroes/game/900001", 200, &sgdb_list(&[(1, "hero1.png")]))
        .on_json("dimensions=920x430", 200, &sgdb_list(&[(2, "wide1.png")]))
        .on_json("dimensions=600x900", 200, &sgdb_list(&[(4, "portrait1.png")]))
        .on_json("/logos/game/900001", 200, &sgdb_list(&[(3, "logo1.png")]))
        .on_json("/icons/game/900001", 200, &sgdb_list(&[(5, "icon1.png")]))
        .on("/img/hero1.png", 200, png(1920, 620, [1, 2, 3, 255]))
        .on("/img/wide1.png", 200, png(920, 430, [4, 5, 6, 255]))
        .on("/img/portrait1.png", 200, png(600, 900, [7, 8, 9, 255]))
        .on("/img/logo1.png", 200, logo_png(800, 300))
        .on("/img/icon1.png", 200, png(256, 256, [9, 9, 9, 255]))
}

#[test]
fn slots_come_from_the_catalog_first_then_steamgriddb_and_nothing_else_is_asked() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(sgdb_fake());
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 1_000);
    let art = s.resolve(&request("Ship of Harkinian"));
    let src = |slot| art.source_of(slot).map(|s| (s.source.as_str(), s.generated));
    assert_eq!(src(Slot::Portrait), Some(("catalog", false)));
    assert_eq!(src(Slot::Icon), Some(("catalog", false)));
    assert_eq!(src(Slot::Hero), Some(("steamgriddb", false)));
    assert_eq!(src(Slot::Wide), Some(("steamgriddb", false)));
    assert_eq!(src(Slot::Logo), Some(("steamgriddb", false)));
    assert_eq!(art.set.hero.as_ref().unwrap().image.dimensions(), (1920, 620));
    assert_eq!(art.set.wide.as_ref().unwrap().image.dimensions(), (920, 430));
    assert!(art.set.logo.as_ref().unwrap().image.pixels().any(|p| p[3] == 0), "the logo keeps its transparency");
    assert!(art.problems.is_empty(), "{:?}", art.problems);
    assert_eq!(fake.count("thegamesdb"), 0);
    assert_eq!(fake.count("screenscraper"), 0);
    assert_eq!(fake.count("dimensions=600x900"), 0, "the portrait came from the catalog, no request was needed");
    assert_eq!(fake.count("/icons/game"), 0, "so did the icon");
    assert_eq!(fake.count("/search/autocomplete/"), 1);
    assert_eq!(fake.header_of("/heroes/game/900001", "Authorization"), Some(format!("Bearer {SGDB_KEY}")));
}

#[test]
fn with_no_source_set_up_the_result_is_exactly_the_catalog_art() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(Fake::new());
    let s = session(dir.path(), &fake, Secrets::in_memory(), BuildCreds::none(), Mode::Online, 1_000);
    assert!(!s.any_source_ready());
    let art = s.resolve(&request("Ship of Harkinian"));
    assert_eq!(art.sources.iter().map(|s| s.slot).collect::<Vec<_>>(), vec![Slot::Portrait, Slot::Icon]);
    assert!(art.set.hero.is_none() && art.set.logo.is_none());
    assert!(fake.requests().is_empty());
    // A catalog without an icon gets none made up when nothing is set up.
    let mut req = Request::new("k", "Game");
    req.catalog = catalog_art(Some(&png(600, 900, [1, 1, 1, 255])), None);
    assert!(s.resolve(&req).set.icon.is_none());
}

#[test]
fn an_off_size_catalog_portrait_gives_way_to_a_native_one_from_a_source() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(sgdb_fake());
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 1_000);
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.catalog = catalog_art(Some(&png(800, 600, [10, 20, 30, 255])), None);
    req.need = vec![Slot::Portrait];
    let art = s.resolve(&req);
    assert_eq!(art.source_of(Slot::Portrait).map(|s| (s.source.as_str(), s.generated)), Some(("steamgriddb", false)));
    // And with no source that has one, the composed catalog image is still used.
    let fake = Rc::new(Fake::new().on_json("/search/autocomplete/", 200, SGDB_SEARCH).on_json("dimensions=600x900", 200, r#"{"success":true,"data":[]}"#));
    let dir = tempfile::tempdir().unwrap();
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 5_000_000);
    let mut req = Request::new("other/port", "Ship of Harkinian");
    req.catalog = catalog_art(Some(&png(800, 600, [10, 20, 30, 255])), None);
    req.need = vec![Slot::Portrait];
    let art = s.resolve(&req);
    assert_eq!(art.source_of(Slot::Portrait).map(|s| (s.source.as_str(), s.generated)), Some(("catalog", true)));
}

#[test]
fn when_steamgriddb_has_nothing_thegamesdb_fills_in_after_it() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(
        Fake::new()
            .on_json("/search/autocomplete/", 200, SGDB_SEARCH)
            .on_json("/heroes/game/900001", 200, r#"{"success":true,"data":[]}"#)
            .on_json("/logos/game/900001", 200, r#"{"success":true,"data":[]}"#)
            .on_json("Games/Images", 200, TGDB_IMAGES)
            .on("images/original/banners/900100-2.jpg", 200, png(1920, 620, [50, 60, 70, 255]))
            .on("images/original/banners/900100-1.jpg", 200, png(1800, 900, [1, 1, 1, 255]))
            .on("images/original/fanart/original/900100-1.jpg", 200, png(3000, 1700, [1, 1, 1, 255]))
            .on("images/original/clearlogo/900100.png", 200, logo_png(2000, 2000))
            .on("images/original/clearlogo/900100-2.png", 200, logo_png(1200, 1200)),
    );
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), Some(TGDB_KEY)), BuildCreds::none(), Mode::Online, 1_000);
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.tgdb_id = Some(900100);
    req.need = vec![Slot::Hero, Slot::Logo];
    let art = s.resolve(&req);
    let hero = art.source_of(Slot::Hero).unwrap();
    assert_eq!((hero.source.as_str(), hero.generated), ("thegamesdb", false), "the exact 1920x620 banner is native");
    assert_eq!(art.source_of(Slot::Logo).map(|s| s.source.as_str()), Some("thegamesdb"));
    assert_eq!(fake.count("/heroes/game/900001"), 1);
    assert_eq!(fake.count("Games/Images"), 1, "images are listed once per game");
    let urls = fake.requests();
    let first_sgdb = urls.iter().position(|u| u.contains("/heroes/")).unwrap();
    let first_tgdb = urls.iter().position(|u| u.contains("Games/Images")).unwrap();
    assert!(first_sgdb < first_tgdb, "SteamGridDB is asked first");
    assert_eq!(fake.count("screenscraper"), 0);
}

#[test]
fn a_native_image_from_a_later_source_beats_a_composed_one_from_an_earlier_source() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(
        Fake::new()
            .on_json("/search/autocomplete/", 200, SGDB_SEARCH)
            .on_json("dimensions=920x430", 200, r#"{"success":true,"data":[]}"#)
            .on_json("jeuInfos.php", 200, SS_INFO)
            .on("media=steamgrid", 200, png(920, 430, [90, 0, 0, 255]))
            .on("media=fanart", 200, png(1920, 1080, [0, 90, 0, 255]))
            .on("media=ss&", 200, png(1920, 1080, [0, 0, 90, 255])),
    );
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), ss_build(), Mode::Online, 1_000);
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.rom = vec![sample_rom()];
    req.need = vec![Slot::Wide];
    let art = s.resolve(&req);
    let wide = art.source_of(Slot::Wide).unwrap();
    assert_eq!((wide.source.as_str(), wide.generated), ("screenscraper", false));
    assert_eq!(art.set.wide.as_ref().unwrap().image.get_pixel(5, 5)[0], 90, "it is the steamgrid image");
}

#[test]
fn a_steamgrid_image_without_the_slots_shape_is_not_taken_for_the_wide_capsule() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(
        Fake::new()
            .on_json("/search/autocomplete/", 200, SGDB_SEARCH)
            .on_json("dimensions=920x430", 200, r#"{"success":true,"data":[]}"#)
            .on_json("jeuInfos.php", 200, SS_INFO)
            .on("media=steamgrid", 200, png(1840, 1200, [90, 0, 0, 255]))
            .on("media=fanart", 200, png(1920, 1080, [0, 90, 0, 255])),
    );
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), ss_build(), Mode::Online, 1_000);
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.rom = vec![sample_rom()];
    req.need = vec![Slot::Wide];
    let art = s.resolve(&req);
    let wide = art.source_of(Slot::Wide).unwrap();
    assert!(wide.generated, "composed from the fan art instead");
    assert_eq!(art.set.wide.as_ref().unwrap().image.get_pixel(5, 5)[1], 90);
}

#[test]
fn composed_art_is_flagged_in_the_manifest_and_a_wide_is_cropped_from_the_hero_when_nothing_else_exists() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(
        Fake::new()
            .on_json("/search/autocomplete/", 200, SGDB_SEARCH)
            .on_json("/heroes/game/900001", 200, &sgdb_list(&[(1, "hero1.png")]))
            .on_json("dimensions=920x430", 200, r#"{"success":true,"data":[]}"#)
            .on("/img/hero1.png", 200, png(1920, 620, [1, 2, 3, 255])),
    );
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 1_000);
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.need = vec![Slot::Hero, Slot::Wide];
    let art = s.resolve(&req);
    let (hero, wide) = (art.source_of(Slot::Hero).unwrap(), art.source_of(Slot::Wide).unwrap());
    assert_eq!((hero.generated, wide.generated, wide.source.as_str()), (false, true, "steamgriddb"));
    let grid = dir.path().join("grid");
    let id = 2_573_349_430u32;
    let report = write_art(&grid, id, &art.set, &WriteOptions::default()).unwrap();
    assert!(matches!(report.outcome(Slot::Wide), Some(SlotOutcome::Written(_))));
    let manifest = Manifest::load(&grid);
    assert!(!manifest.entries()[&format!("{id}_hero.png")].generated);
    let wide_entry = &manifest.entries()[&format!("{id}.png")];
    assert!(wide_entry.generated && wide_entry.source == "steamgriddb");
}

#[test]
fn a_look_alike_title_is_not_used_remembered_as_a_candidate_and_not_searched_again() {
    let dir = tempfile::tempdir().unwrap();
    let only_lookalike = r#"{"success":true,"data":[{"id":900002,"name":"Ship of Harkinian: Remastered","types":[],"verified":true}]}"#;
    let fake = Rc::new(Fake::new().on_json("/search/autocomplete/", 200, only_lookalike).on_json("/heroes/game/900002", 200, r#"{"success":true,"data":[]}"#));
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 1_000);
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.need = vec![Slot::Hero, Slot::Logo];
    let art = s.resolve(&req);
    assert!(art.set.is_empty());
    assert_eq!(fake.count("/heroes/"), 0, "no art was asked for under a game that is not the same title");
    assert!(art.notes.iter().any(|n| n.contains("not the same title") && n.contains("sgdb_id")), "{:?}", art.notes);
    let pending = s.pending_matches();
    assert_eq!(pending.len(), 1);
    assert_eq!((pending[0].0.as_str(), pending[0].2[0].id), ("owner/ship", 900002));
    // A later session does not repeat the search within a few days.
    let again = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 2_000);
    again.resolve(&req);
    assert_eq!(fake.count("/search/autocomplete/"), 1);
    // Confirming the candidate makes it usable.
    again.confirm_match("owner/ship", SourceId::SteamGridDb, 900002, "Ship of Harkinian: Remastered");
    let confirmed = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 3_000);
    confirmed.resolve(&req);
    assert_eq!(fake.count("/heroes/game/900002"), 1, "{:?}", fake.requests());
}

#[test]
fn a_local_override_id_skips_the_title_search_entirely() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(sgdb_fake());
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 1_000);
    let mut req = Request::new("owner/fanport", "A Fan Port With An Odd Title");
    req.sgdb_id = Some(900001);
    req.need = vec![Slot::Hero];
    let art = s.resolve(&req);
    assert!(art.source_of(Slot::Hero).is_some());
    assert_eq!(fake.count("/search/autocomplete/"), 0);
    assert_eq!(fake.count("/heroes/game/900001"), 1);
}

#[test]
fn the_second_run_needs_no_search_and_no_download_and_cache_only_makes_no_call_at_all() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(sgdb_fake());
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.need = vec![Slot::Hero, Slot::Wide, Slot::Logo];
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 1_000);
    let first = s.resolve(&req);
    assert_eq!(first.sources.len(), 3);
    assert!(!fake.requests().is_empty());

    // Same cache folder, network gone: online mode still works from the cache.
    let offline = Rc::new(Fake::new());
    let s2 = session(dir.path(), &offline, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 2_000);
    let second = s2.resolve(&req);
    assert_eq!(second.sources.len(), 3, "{:?}", second.problems);
    assert!(offline.requests().is_empty(), "everything came from the cache: {:?}", offline.requests());
    assert_eq!(second.set.hero.as_ref().unwrap().image, first.set.hero.as_ref().unwrap().image);

    // Cache-only mode, with the key present.
    let s4 = session(dir.path(), &offline, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::CacheOnly, 3_000);
    assert_eq!(s4.resolve(&req).sources.len(), 3);
    assert!(offline.requests().is_empty());
    // Nothing set up: only the catalog is offered.
    let s3 = session(dir.path(), &offline, Secrets::in_memory(), BuildCreds::none(), Mode::CacheOnly, 3_000);
    assert!(s3.resolve(&req).sources.is_empty());
}

#[test]
fn cache_only_never_searches_for_an_unknown_game() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(sgdb_fake());
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::CacheOnly, 1_000);
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.need = vec![Slot::Hero];
    assert!(s.resolve(&req).set.is_empty());
    assert!(fake.requests().is_empty());
}

#[test]
fn nothing_found_for_a_slot_is_remembered_for_a_few_days() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(Fake::new().on_json("/search/autocomplete/", 200, SGDB_SEARCH).on_json("/logos/game/900001", 200, r#"{"success":true,"data":[]}"#));
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.need = vec![Slot::Logo];
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 1_000);
    assert!(s.resolve(&req).set.is_empty());
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 2_000);
    let art = s.resolve(&req);
    assert!(art.set.is_empty());
    assert_eq!(fake.count("/logos/game/900001"), 1);
    assert!(art.notes.iter().any(|n| n.contains("logo: none found")));
}

#[test]
fn a_logo_without_transparency_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(
        Fake::new()
            .on_json("/search/autocomplete/", 200, SGDB_SEARCH)
            .on_json("/logos/game/900001", 200, &sgdb_list(&[(3, "logo1.png")]))
            .on("/img/logo1.png", 200, png(800, 300, [255, 255, 255, 255])),
    );
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 1_000);
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.need = vec![Slot::Logo];
    assert!(s.resolve(&req).set.logo.is_none());
}

#[test]
fn a_rejected_key_stops_that_source_for_the_whole_session_and_says_so_once() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(Fake::new().on_json("/search/autocomplete/", 401, "{}"));
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 1_000);
    let first = s.resolve(&request("Ship of Harkinian"));
    assert!(first.problems.iter().any(|(_, p)| p.contains("SteamGridDB") && p.contains("key was not accepted")), "{:?}", first.problems);
    let second = s.resolve(&Request::new("other/port", "Another Port"));
    assert_eq!(fake.count("/search/autocomplete/"), 1, "the second port made no call");
    assert!(second.problems.is_empty());
    assert_eq!(first.source_of(Slot::Portrait).map(|s| s.source.as_str()), Some("catalog"), "the catalog art is still delivered");
}

#[test]
fn offline_is_a_problem_not_a_crash_and_the_catalog_art_still_comes_through() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(Fake::new());
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), Some(TGDB_KEY)), BuildCreds::none(), Mode::Online, 1_000);
    let art = s.resolve(&request("Ship of Harkinian"));
    assert!(art.problems.iter().any(|(_, p)| p.contains("offline")), "{:?}", art.problems);
    assert!(art.set.portrait.is_some() && art.set.icon.is_some());
    assert_eq!(fake.count("/search/autocomplete/"), 1);
}

#[test]
fn a_daily_quota_is_remembered_across_sessions_and_the_source_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(Fake::new().on_json("jeuInfos.php", 430, "").on_json("jeuRecherche.php", 430, ""));
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.rom = vec![sample_rom()];
    req.need = vec![Slot::Hero];
    let day = 1_800_000_000 - 1_800_000_000 % 86_400;
    let s = session(dir.path(), &fake, Secrets::in_memory(), ss_build(), Mode::Online, day + 40_000);
    let art = s.resolve(&req);
    assert!(art.problems.iter().any(|(_, p)| p.contains("ScreenScraper") && p.contains("daily")), "{:?}", art.problems);
    assert_eq!(fake.count("jeuInfos.php"), 1);
    let later = session(dir.path(), &fake, Secrets::in_memory(), ss_build(), Mode::Online, day + 40_100);
    let art = later.resolve(&req);
    assert_eq!(fake.count("jeuInfos.php"), 1, "still blocked until the next UTC day");
    assert!(art.problems.iter().any(|(_, p)| p.contains("waiting")), "{:?}", art.problems);
    let tomorrow = session(dir.path(), &fake, Secrets::in_memory(), ss_build(), Mode::Online, day + 86_400 + 60);
    tomorrow.resolve(&req);
    assert_eq!(fake.count("jeuInfos.php"), 2, "the block is over after midnight UTC");
}

#[test]
fn screenscraper_is_not_asked_without_the_builds_credentials_even_with_a_rom_and_a_login() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(sgdb_fake().on_json("jeuInfos.php", 200, SS_INFO));
    let mut secrets = secrets_at(dir.path(), Some(SGDB_KEY), None);
    secrets.set(Field::ScreenScraperUser, Some("tim")).unwrap();
    secrets.set(Field::ScreenScraperPassword, Some("pw-123456")).unwrap();
    let s = session(dir.path(), &fake, secrets, BuildCreds::none(), Mode::Online, 1_000);
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.rom = vec![sample_rom()];
    req.need = vec![Slot::Hero, Slot::Logo];
    let art = s.resolve(&req);
    assert_eq!(art.sources.len(), 2);
    assert_eq!(fake.count("screenscraper"), 0);
}

#[test]
fn a_rom_hash_pins_the_game_and_its_media_fill_slots_and_the_id_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(
        Fake::new()
            .on_json("jeuInfos.php", 200, SS_INFO)
            .on("box-2D(us)", 200, png(800, 600, [10, 10, 10, 255]))
            .on("box-2D(eu)", 200, png(600, 900, [99, 99, 99, 255]))
            .on("media=wheel-hd", 200, logo_png(600, 300))
            .on("media=wheel(wor)", 200, png(600, 300, [255, 255, 255, 255])),
    );
    let s = session(dir.path(), &fake, Secrets::in_memory(), ss_build(), Mode::Online, 1_000);
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.rom = vec![sample_rom()];
    req.need = vec![Slot::Portrait, Slot::Logo];
    let art = s.resolve(&req);
    // The US box is landscape (composed) and the EU one is native: the native one wins.
    let portrait = art.source_of(Slot::Portrait).unwrap();
    assert_eq!((portrait.source.as_str(), portrait.generated), ("screenscraper", false));
    assert_eq!(art.set.portrait.as_ref().unwrap().image.get_pixel(3, 3)[0], 99);
    assert_eq!(art.source_of(Slot::Logo).map(|s| s.source.as_str()), Some("screenscraper"));
    let url = fake.requests()[0].clone();
    assert!(url.contains("crc=AABBCCDD") && url.contains("systemeid=14") && url.contains("romtaille=8388608"));
    assert_eq!(fake.count("jeuInfos.php"), 1, "one lookup serves every slot");

    // Next run, even cache-only: the hash is known, the images are cached.
    let offline = Rc::new(Fake::new());
    let s2 = session(dir.path(), &offline, Secrets::in_memory(), ss_build(), Mode::CacheOnly, 2_000);
    let again = s2.resolve(&req);
    assert_eq!(again.sources.len(), 2, "{:?}", again.problems);
    assert!(offline.requests().is_empty());
}

#[test]
fn without_a_hash_match_the_roms_file_name_is_searched_and_only_the_same_title_is_trusted() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(Fake::new().on_json("jeuInfos.php", 404, "").on_json("jeuRecherche.php", 200, SS_SEARCH).on("box-2D(us)", 200, png(600, 900, [5, 5, 5, 255])));
    let s = session(dir.path(), &fake, Secrets::in_memory(), ss_build(), Mode::Online, 1_000);
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    let mut rom = sample_rom();
    rom.file_name = "Ship of Harkinian (USA).z64".into();
    req.rom = vec![rom];
    req.need = vec![Slot::Portrait];
    let art = s.resolve(&req);
    assert_eq!(art.source_of(Slot::Portrait).map(|s| s.source.as_str()), Some("screenscraper"));
    assert!(fake.requests().iter().any(|u| u.contains("recherche=Ship%20of%20Harkinian%20%28USA%29")));

    let dir2 = tempfile::tempdir().unwrap();
    let fake2 = Rc::new(Fake::new().on_json("jeuInfos.php", 404, "").on_json("jeuRecherche.php", 200, SS_SEARCH).on("box-2D(us)", 200, png(600, 900, [5, 5, 5, 255])));
    let s2 = session(dir2.path(), &fake2, Secrets::in_memory(), ss_build(), Mode::Online, 1_000);
    let mut req2 = Request::new("owner/ship", "Ship");
    let mut rom2 = sample_rom();
    rom2.file_name = "Ship (USA).z64".into();
    req2.rom = vec![rom2];
    req2.need = vec![Slot::Portrait];
    let art2 = s2.resolve(&req2);
    assert!(art2.set.is_empty(), "\"Ship\" is not \"Ship of Harkinian\"");
    assert!(art2.notes.iter().any(|n| n.contains("not the same title")), "{:?}", art2.notes);
}

#[test]
fn thegamesdb_name_search_goes_by_the_same_title_pc_entry() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(Fake::new().on_json("Games/ByGameName", 200, TGDB_SEARCH).on_json("Games/Images", 200, TGDB_IMAGES));
    let s = session(dir.path(), &fake, secrets_at(dir.path(), None, Some(TGDB_KEY)), BuildCreds::none(), Mode::Online, 1_000);
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.need = vec![Slot::Hero];
    let art = s.resolve(&req);
    // The search gave id 900101 (the PC entry); the images fixture is keyed by 900100, so nothing is listed.
    assert!(art.set.hero.is_none());
    assert!(fake.requests().iter().any(|u| u.contains("games_id=900101")), "{:?}", fake.requests());
    let table = IdTable::load(&dir.path().join("cache/art/ids.json"));
    assert_eq!(table.usable("owner/ship", SourceId::TheGamesDb).map(|e| e.id), Some(900101));
}

#[test]
fn user_art_is_never_listed_as_needed_and_composed_art_of_ours_is() {
    let dir = tempfile::tempdir().unwrap();
    let grid = dir.path().join("grid");
    fs::create_dir_all(&grid).unwrap();
    let id = 2_573_349_430u32;
    assert_eq!(slots_needing_art(&grid, id).len(), 5);
    // The user picked a hero in Steam.
    fs::write(grid.join(format!("{id}_hero.jpg")), b"the user's own picture").unwrap();
    let needed = slots_needing_art(&grid, id);
    assert!(!needed.contains(&Slot::Hero) && needed.contains(&Slot::Wide));
    // We wrote a composed wide capsule: it may be upgraded. A native portrait may not.
    let composed = super::fit::fit(Slot::Wide, &DynamicImage::ImageRgba8(RgbaImage::from_pixel(1920, 1080, Rgba([1, 2, 3, 255])))).unwrap().with_source("steamgriddb");
    assert!(composed.generated);
    let mut set = ArtSet::default();
    set.set(composed);
    write_art(&grid, id, &set, &WriteOptions::default()).unwrap();
    assert!(slots_needing_art(&grid, id).contains(&Slot::Wide));
    let native = super::fit::fit(Slot::Portrait, &DynamicImage::ImageRgba8(RgbaImage::from_pixel(600, 900, Rgba([1, 2, 3, 255])))).unwrap().with_source("steamgriddb");
    let mut set = ArtSet::default();
    set.set(native);
    write_art(&grid, id, &set, &WriteOptions::default()).unwrap();
    assert!(!slots_needing_art(&grid, id).contains(&Slot::Portrait));
    assert_eq!(fs::read(grid.join(format!("{id}_hero.jpg"))).unwrap(), b"the user's own picture");
}

#[test]
fn the_writer_still_refuses_to_replace_a_hero_the_user_chose_even_if_a_source_has_one() {
    let dir = tempfile::tempdir().unwrap();
    let grid = dir.path().join("grid");
    fs::create_dir_all(&grid).unwrap();
    let id = 77u32;
    fs::write(grid.join(format!("{id}_hero.png")), b"mine").unwrap();
    let fake = Rc::new(sgdb_fake());
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 1_000);
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.need = vec![Slot::Hero];
    let art = s.resolve(&req);
    assert!(art.set.hero.is_some());
    let report = write_art(&grid, id, &art.set, &WriteOptions::default()).unwrap();
    assert!(matches!(report.outcome(Slot::Hero), Some(SlotOutcome::SkippedUserArt(_))));
    assert_eq!(fs::read(grid.join(format!("{id}_hero.png"))).unwrap(), b"mine");
}

#[test]
fn no_key_or_login_reaches_the_cache_the_state_file_a_message_or_a_note() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(
        sgdb_fake()
            .on_json("Games/Images", 200, TGDB_IMAGES)
            .on_json("jeuInfos.php", 200, SS_INFO)
            .on("media=", 200, png(1920, 1080, [3, 3, 3, 255]))
            .on("images/original", 200, png(1920, 1080, [3, 3, 3, 255])),
    );
    let mut secrets = secrets_at(dir.path(), Some(SGDB_KEY), Some(TGDB_KEY));
    secrets.set(Field::ScreenScraperUser, Some("tim-login-name")).unwrap();
    secrets.set(Field::ScreenScraperPassword, Some("PASSWORD-SECRET-3")).unwrap();
    let s = session(dir.path(), &fake, secrets, ss_build(), Mode::Online, 1_000);
    let mut req = request("Ship of Harkinian");
    req.tgdb_id = Some(900100);
    req.rom = vec![sample_rom()];
    let art = s.resolve(&req);
    // Provoke errors too, in a fresh cache so they are not answered from it.
    let dir2 = tempfile::tempdir().unwrap();
    let broken = Rc::new(Fake::new().on_json("/search/", 401, "{}").on_json("Games/", 403, "{}").on_json("jeuInfos.php", 403, ""));
    let s2 = session(dir2.path(), &broken, secrets_at(dir2.path(), Some(SGDB_KEY), Some(TGDB_KEY)), ss_build(), Mode::Online, 9_000_000);
    let mut req2 = Request::new("other/port", "Another Port");
    req2.tgdb_id = Some(5);
    req2.rom = vec![sample_rom()];
    let art2 = s2.resolve(&req2);

    let secrets_list = ["SGDB-TEST-KEY-AAAA1111", "TGDB-TEST-KEY-BBBB2222", "tim-login-name", "PASSWORD-SECRET-3", "DEVID-SECRET-1", "DEVPASS-SECRET-2"];
    let mut texts: Vec<String> = Vec::new();
    for a in [&art, &art2] {
        texts.extend(a.problems.iter().map(|(_, p)| p.clone()));
        texts.extend(a.notes.clone());
    }
    fn walk(dir: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_none_or(|e| e != "img") {
                out.push(String::from_utf8_lossy(&fs::read(&path).unwrap()).into_owned());
            }
        }
    }
    walk(&dir.path().join("cache"), &mut texts);
    walk(&dir2.path().join("cache"), &mut texts);
    assert!(texts.len() > 6, "the check must have something to look at: {}", texts.len());
    for secret in secrets_list {
        for text in &texts {
            assert!(!text.contains(secret), "{secret} found in: {text}");
        }
    }
    assert!(dir2.path().join("cache/art/state.json").exists(), "the failures were recorded");
}

#[test]
fn rom_identities_come_from_the_saved_scan_and_only_for_hash_matches() {
    let tmp = tempfile::tempdir().unwrap();
    let (config, roms) = (tmp.path().join("app"), tmp.path().join("roms"));
    fs::create_dir_all(&config).unwrap();
    let z64 = n64_rom(11, 8192);
    write(&roms.join("n64").join("renamed.z64"), &z64);
    write(&roms.join("n64").join("Name Only (USA).z64"), &n64_rom(12, 8192));
    let crc = format!("{:08X}", crate::fork::rom_library::hash::crc32_of(&z64));

    let mut store = RomLibraryStore::load(&config.join("rom_library.json")).unwrap();
    store.set_rom_dir(Some(roms.clone())).unwrap();
    let scan = crate::fork::rom_library::scan::scan(&roms, &Default::default(), None, &std::sync::atomic::AtomicBool::new(false), &mut |_| {}).unwrap();
    store.save_scan_cache(&scan).unwrap();

    let hash_text = format!("Required files: Game (USA).z64 (CRC32: {crc})");
    let port = PortRef { key: "owner/game", folder: "game", name: "Game", instructions: Some(&hash_text) };
    let ids = rom_identities(&config, &port, None);
    assert_eq!(ids.len(), 1);
    assert_eq!((ids[0].system_id, ids[0].size, ids[0].crc32.as_str(), ids[0].file_name.as_str()), (Some(14), 8192, crc.as_str(), "renamed.z64"));

    let name_only = PortRef { key: "owner/game", folder: "game", name: "Game", instructions: Some("Required files: Name Only (USA).z64") };
    assert!(rom_identities(&config, &name_only, None).is_empty(), "a name-only match is not good enough to ask ScreenScraper about");
    let none = PortRef { instructions: None, ..port };
    assert!(rom_identities(&config, &none, None).is_empty());
    assert!(rom_identities(&tmp.path().join("nowhere"), &port, None).is_empty());
}

#[test]
fn resolve_port_reads_overrides_from_the_local_entry_and_limits_itself_to_slots_that_want_art() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("app");
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join("ports.local.json"), r#"{"ports":[{"name":"Fan Port","folder":"fanport","sgdb_id":"900001","tgdb_id":900100}]}"#).unwrap();
    let grid = dir.path().join("grid");
    fs::create_dir_all(&grid).unwrap();
    let id = 99u32;
    fs::write(grid.join(format!("{id}_hero.png")), b"mine").unwrap();
    let fake = Rc::new(sgdb_fake());
    let s = session(&config, &fake, secrets_at(&config, Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 1_000);
    let port = PortRef { key: "fanport", folder: "fanport", name: "Fan Port", instructions: None };
    let art = s.resolve_port(&config, port, None, None, Some((&grid, id)));
    assert_eq!(fake.count("/search/autocomplete/"), 0, "the override id was used");
    assert_eq!(fake.count("/heroes/"), 0, "the user's hero is left alone, so it is not even looked up");
    assert!(art.set.hero.is_none());
    assert!(art.set.logo.is_some() && art.set.wide.is_some());
}

#[test]
fn a_session_survives_garbage_in_every_answer() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(
        Fake::new()
            .on_json("/search/autocomplete/", 200, SGDB_SEARCH)
            .on_json("/heroes/", 200, &sgdb_list(&[(1, "hero1.png")]))
            .on("/img/hero1.png", 200, b"<html>not an image</html>".to_vec()),
    );
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 1_000);
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.need = vec![Slot::Hero];
    let art = s.resolve(&req);
    assert!(art.set.hero.is_none());
    let cache = super::cache::ArtCache::new(&dir.path().join("cache"));
    assert!(matches!(cache.image_get(SourceId::SteamGridDb, 900001, Slot::Hero, 1_100), CacheLookup::KnownMissing | CacheLookup::Miss));
    assert!(!matches!(cache.image_get(SourceId::SteamGridDb, 900001, Slot::Hero, 1_100), CacheLookup::Hit { .. }), "an undecodable image is not cached as a hit");
}

#[test]
fn art_for_port_with_no_source_set_up_is_the_catalog_art_and_touches_nothing_else() {
    let tmp = tempfile::tempdir().unwrap();
    let (config, cache) = (tmp.path().join("app"), tmp.path().join("app").join("cache"));
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("Game.png"), png(600, 900, [10, 20, 30, 255])).unwrap();
    fs::write(cache.join("Game_icon.png"), png(256, 256, [200, 0, 0, 255])).unwrap();
    let dirs = ArtDirs { config_dir: &config, cache_dir: &cache };
    let port = PortRef { key: "owner/game", folder: "Game", name: "Game", instructions: None };
    let art = art_for_port(dirs, port, Some(&cache.join("Game.png")), Some(&cache.join("Game_icon.png")), None, Mode::Online);
    assert_eq!(art.sources.iter().map(|s| (s.slot, s.source.as_str())).collect::<Vec<_>>(), vec![(Slot::Portrait, "catalog"), (Slot::Icon, "catalog")]);
    assert!(art.problems.is_empty());
    assert!(!cache.join("art").exists(), "no key, so nothing was looked up and nothing was saved");
    // A missing file is a problem for that slot, as before.
    let art = art_for_port(dirs, port, Some(&cache.join("nope.png")), None, None, Mode::CacheOnly);
    assert_eq!(art.problems.len(), 1);
    assert!(art.set.is_empty());
}

#[test]
fn too_many_requests_backs_a_source_off_for_minutes_not_until_midnight() {
    let dir = tempfile::tempdir().unwrap();
    let busy = Rc::new(Fake::new().on_json("/search/autocomplete/", 429, "{}"));
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.need = vec![Slot::Hero];
    let s = session(dir.path(), &busy, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 10_000);
    let art = s.resolve(&req);
    assert!(art.problems.iter().any(|(_, p)| p.contains("SteamGridDB") && p.contains("too many requests")), "{:?}", art.problems);
    // Four minutes later it is still being left alone, without a call.
    let soon = session(dir.path(), &busy, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 10_240);
    soon.resolve(&req);
    assert_eq!(busy.count("/search/autocomplete/"), 1);
    // Ten minutes later it is asked again.
    let fine = Rc::new(sgdb_fake());
    let later = session(dir.path(), &fine, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 10_600);
    let art = later.resolve(&req);
    assert!(art.source_of(Slot::Hero).is_some(), "{:?}", art.problems);
    assert_eq!(fine.count("/search/autocomplete/"), 1);
}

#[test]
fn two_same_titles_with_different_years_are_not_picked_and_wait_for_the_user() {
    let dir = tempfile::tempdir().unwrap();
    let two = r#"{"success":true,"data":[{"id":71,"name":"DOOM (2016)","types":[],"verified":true},{"id":72,"name":"Doom (1993)","types":[],"verified":true}]}"#;
    let fake = Rc::new(Fake::new().on_json("/search/autocomplete/", 200, two));
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 1_000);
    let mut req = Request::new("owner/doom", "Doom");
    req.folder = "doom-port";
    req.need = vec![Slot::Hero];
    let art = s.resolve(&req);
    assert!(art.set.is_empty());
    assert_eq!(fake.count("/heroes/"), 0, "neither game was guessed");
    let pending = s.pending_matches();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].2.iter().map(|c| c.id).collect::<Vec<_>>(), vec![71, 72]);
    let table = IdTable::load(&dir.path().join("cache/art/ids.json"));
    assert_eq!(table.identity("owner/doom"), (Some("Doom"), Some("doom-port")), "the folder is saved so the screen can say how to choose");
}

#[test]
fn an_override_in_ports_local_json_clears_the_waiting_look_alike() {
    let dir = tempfile::tempdir().unwrap();
    let lookalike = r#"{"success":true,"data":[{"id":900002,"name":"Ship of Harkinian: Remastered","types":[],"verified":true}]}"#;
    let fake = Rc::new(Fake::new().on_json("/search/autocomplete/", 200, lookalike).on_json("/heroes/game/777", 200, r#"{"success":true,"data":[]}"#));
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.need = vec![Slot::Hero];
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 1_000);
    s.resolve(&req);
    assert_eq!(s.pending_matches().len(), 1);
    let again = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 2_000);
    req.sgdb_id = Some(777);
    again.resolve(&req);
    assert!(again.pending_matches().is_empty(), "the row clears once the user has chosen");
    assert_eq!(fake.count("/heroes/game/777"), 1);
}

#[test]
fn one_image_that_will_not_download_does_not_end_the_source_but_three_in_a_row_do() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(
        Fake::new()
            .on_json("/search/autocomplete/", 200, SGDB_SEARCH)
            .on_json("/heroes/game/900001", 200, &sgdb_list(&[(1, "bad1.png"), (2, "good2.png")]))
            .on_json("dimensions=920x430", 200, &sgdb_list(&[(3, "wide3.png")]))
            .on("/img/bad1.png", 500, Vec::new())
            .on("/img/good2.png", 200, png(1920, 620, [1, 2, 3, 255]))
            .on("/img/wide3.png", 200, png(920, 430, [4, 5, 6, 255])),
    );
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 1_000);
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.need = vec![Slot::Hero, Slot::Wide];
    let art = s.resolve(&req);
    assert_eq!(art.source_of(Slot::Hero).map(|s| s.source.as_str()), Some("steamgriddb"), "the second hero came through");
    assert_eq!(art.source_of(Slot::Wide).map(|s| s.source.as_str()), Some("steamgriddb"), "and the source was still in use for the next slot");

    // Three broken images in a row: now the source is given up for the session.
    let dir2 = tempfile::tempdir().unwrap();
    let fake2 = Rc::new(
        Fake::new()
            .on_json("/search/autocomplete/", 200, SGDB_SEARCH)
            .on_json("/heroes/game/900001", 200, &sgdb_list(&[(1, "b1.png"), (2, "b2.png"), (3, "b3.png")]))
            .on_json("/logos/game/900001", 200, &sgdb_list(&[(4, "logo4.png")]))
            .on("/img/b", 502, Vec::new())
            .on("/img/logo4.png", 200, logo_png(800, 300)),
    );
    let s2 = session(dir2.path(), &fake2, secrets_at(dir2.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 1_000);
    let mut req2 = Request::new("owner/ship", "Ship of Harkinian");
    req2.need = vec![Slot::Hero, Slot::Logo];
    let art2 = s2.resolve(&req2);
    assert!(art2.set.hero.is_none() && art2.set.logo.is_none(), "after the third failure SteamGridDB is left alone");
    assert_eq!(fake2.count("/logos/game"), 0);
}

#[test]
fn stop_is_honoured_inside_a_port_between_two_requests() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(sgdb_fake());
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 1_000);
    let stop = Rc::new(std::cell::Cell::new(false));
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.need = vec![Slot::Hero, Slot::Wide, Slot::Logo];
    // Stop is pressed as soon as the title search has been answered.
    let cancel = {
        let (stop, fake) = (stop.clone(), fake.clone());
        move || {
            if fake.count("/search/autocomplete/") >= 1 {
                stop.set(true);
            }
            stop.get()
        }
    };
    let art = s.resolve_with(&req, &cancel);
    assert!(art.set.is_empty());
    assert_eq!(fake.count("/heroes/"), 0);
    assert_eq!(fake.requests().len(), 1, "only the search went out: {:?}", fake.requests());
}

#[test]
fn an_opaque_logo_does_not_use_up_the_download_budget_and_a_game_with_no_images_is_remembered() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Rc::new(
        Fake::new()
            .on_json("/search/autocomplete/", 200, SGDB_SEARCH)
            .on_json("/logos/game/900001", 200, &sgdb_list(&[(1, "o1.png"), (2, "o2.png"), (3, "o3.png"), (4, "real.png")]))
            .on("/img/o", 200, png(800, 300, [255, 255, 255, 255]))
            .on("/img/real.png", 200, logo_png(800, 300)),
    );
    let s = session(dir.path(), &fake, secrets_at(dir.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 1_000);
    let mut req = Request::new("owner/ship", "Ship of Harkinian");
    req.need = vec![Slot::Logo];
    assert!(s.resolve(&req).set.logo.is_some(), "the fourth, transparent logo was reached");

    // A 404 for the image list means "none", and is remembered like an empty list.
    let dir2 = tempfile::tempdir().unwrap();
    let gone = Rc::new(Fake::new().on_json("/search/autocomplete/", 200, SGDB_SEARCH).on_json("/heroes/game/900001", 404, "{}"));
    let mut req2 = Request::new("owner/ship", "Ship of Harkinian");
    req2.need = vec![Slot::Hero];
    let s2 = session(dir2.path(), &gone, secrets_at(dir2.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 1_000);
    let art2 = s2.resolve(&req2);
    assert!(art2.problems.is_empty(), "{:?}", art2.problems);
    let s3 = session(dir2.path(), &gone, secrets_at(dir2.path(), Some(SGDB_KEY), None), BuildCreds::none(), Mode::Online, 2_000);
    s3.resolve(&req2);
    assert_eq!(gone.count("/heroes/"), 1);
}
