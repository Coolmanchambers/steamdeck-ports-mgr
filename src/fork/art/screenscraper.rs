//! ScreenScraper API v2 client: identify a game by the hash of the user's own ROM (the original
//! console game behind a port), or by name as a fallback, and list its media.
//!
//! Needs two layers of credentials, see [`crate::fork::secrets`]: the developer id, password and softname
//! (compiled in by a build that has them, none today, so this source says "not configured in this
//! build") and, optionally, the user's own login for their higher limits. Everything travels as query
//! parameters, so no address built here is ever logged or put in an error message.
//!
//! Lookups: `jeuInfos.php` with `crc`, `md5`, `sha1`, `romtaille` (size in bytes) and `systemeid`
//! pins one game; `jeuRecherche.php` with a name returns up to 30 candidates ranked by probability and
//! is never trusted without a same-title check.
//!
//! Limits and errors, from the API page: 401 closed to this account or the servers are saturated, 403
//! developer credentials refused, 404 not found, 423 fully closed, 426 this software is blocked, 429
//! thread limit, 430 daily quota, 431 too many unrecognised ROMs today. The client makes one request
//! at a time with a pause between them (so it never exceeds any account's thread limit, whatever
//! `maxthreads` says), waits once on a 429, and turns the rest into [`SourceError`]s the resolver
//! uses to stop and to remember a back-off.

use std::cell::Cell;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::net::{self, Fetch, MAX_JSON_BYTES};
use super::sgdb::download_image;
use super::source::{KeyCheck, SourceError};
use super::title::{confidences, Confidence};
use crate::fork::rom_library::hash::to_hex;
use crate::fork::rom_library::scan::RomFile;
use crate::fork::rom_library::system::System;

const BASE: &str = "https://api.screenscraper.fr/api2";
const IMAGE_HOSTS: [&str; 1] = ["screenscraper.fr"];
/// Pause between two requests.
const MIN_GAP: Duration = Duration::from_millis(1200);
/// How long to wait before the one retry after a 429.
const THREAD_LIMIT_WAIT: Duration = Duration::from_secs(3);

/// ScreenScraper's id for a console, as listed by its `systemesListe.php`. Without one the lookup is
/// sent with no system, which ScreenScraper allows.
pub fn system_id(system: System) -> Option<u32> {
    Some(match system {
        System::Genesis => 1,
        System::MasterSystem => 2,
        System::Nes => 3,
        System::Snes => 4,
        System::GameBoy => 9,
        System::GameBoyColor => 10,
        System::GameBoyAdvance => 12,
        System::GameCube => 13,
        System::N64 => 14,
        System::Nds => 15,
        System::Wii => 16,
        System::N3ds => 17,
        System::WiiU => 18,
        System::Saturn => 22,
        System::Dreamcast => 23,
        System::Xbox => 32,
        System::Xbox360 => 33,
        System::Ps1 => 57,
        System::Ps2 => 58,
        System::Psp => 61,
        System::Arcade => 75,
        System::Switch => 225,
        System::Unknown => return None,
    })
}

/// What ScreenScraper wants to know about a ROM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomIdentity {
    pub system_id: Option<u32>,
    pub size: u64,
    pub crc32: String,
    pub md5: String,
    pub sha1: Option<String>,
    /// File name only; ScreenScraper answers 400 to a name with a path in it.
    pub file_name: String,
}

impl RomIdentity {
    /// The identities to try for a scanned file, most likely first. A Nintendo 64 image in the wrong
    /// byte order gets two: the big-endian form (what checksum databases list) and the file as it is.
    pub fn from_rom_file(file: &RomFile) -> Vec<RomIdentity> {
        let make = |d: &crate::fork::rom_library::hash::Digests| RomIdentity {
            system_id: system_id(file.system),
            size: file.size,
            crc32: d.crc32_hex(),
            md5: d.md5_hex(),
            sha1: d.sha1.map(|s| to_hex(&s)),
            file_name: file.file_name(),
        };
        let mut out = vec![make(file.canonical())];
        if file.normalized.is_some() {
            out.push(make(&file.raw));
        }
        out
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Media {
    pub kind: String,
    pub region: String,
    pub format: String,
    pub url: String,
}

/// The address is left out of debug output: ScreenScraper's media addresses can carry the account's
/// credentials, and a stray `{:?}` must not put them in a log.
impl std::fmt::Debug for Media {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Media").field("kind", &self.kind).field("region", &self.region).field("format", &self.format).finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    pub id: u64,
    pub name: String,
    pub medias: Vec<Media>,
}

/// The `ssuser` block of an answer: who is asking and what that account may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UserInfo {
    pub max_threads: u32,
    pub requests_today: u64,
    pub max_per_day: u64,
    pub level: u32,
}

#[derive(Clone)]
pub struct DevCreds {
    pub id: String,
    pub password: String,
    pub softname: String,
}

impl std::fmt::Debug for DevCreds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DevCreds").field("softname", &self.softname).finish_non_exhaustive()
    }
}

pub struct ScreenScraper<'a> {
    fetch: &'a dyn Fetch,
    dev: DevCreds,
    user: Option<(String, String)>,
    sleep: &'a dyn Fn(Duration),
    last_call: Cell<Option<Instant>>,
    user_info: Cell<Option<UserInfo>>,
}

fn text_of(v: Option<&Value>) -> Option<String> {
    let v = v?;
    v.as_str().map(str::to_string).or_else(|| v.as_u64().map(|n| n.to_string()))
}

fn number_of(v: Option<&Value>) -> Option<u64> {
    let v = v?;
    v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

fn parse_user(v: &Value) -> Option<UserInfo> {
    let u = v.get("response")?.get("ssuser")?;
    Some(UserInfo {
        max_threads: number_of(u.get("maxthreads")).and_then(|n| u32::try_from(n).ok()).unwrap_or(1).max(1),
        requests_today: number_of(u.get("requeststoday")).unwrap_or(0),
        max_per_day: number_of(u.get("maxrequestsperday")).unwrap_or(0),
        level: number_of(u.get("niveau")).and_then(|n| u32::try_from(n).ok()).unwrap_or(0),
    })
}

/// The game's name for display: US, then world, then the first one given.
fn pick_name(jeu: &Value) -> Option<String> {
    let names = jeu.get("noms")?.as_array()?;
    let by_region = |r: &str| names.iter().find(|n| n.get("region").and_then(Value::as_str) == Some(r)).and_then(|n| n.get("text")?.as_str());
    by_region("us").or_else(|| by_region("wor")).or_else(|| names.first().and_then(|n| n.get("text")?.as_str())).map(str::to_string)
}

fn parse_game(jeu: &Value) -> Option<Game> {
    let id = number_of(jeu.get("id"))?;
    let name = pick_name(jeu)?;
    // Not a real game (ScreenScraper's own placeholder rows).
    if id == 0 || name.to_ascii_uppercase().starts_with("ZZZ(NOTGAME)") {
        return None;
    }
    let medias = jeu
        .get("medias")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|m| {
                    let url = m.get("url")?.as_str()?;
                    if !net::host_allowed(url, &IMAGE_HOSTS) {
                        return None;
                    }
                    Some(Media {
                        kind: m.get("type")?.as_str()?.to_string(),
                        region: text_of(m.get("region")).unwrap_or_default(),
                        format: text_of(m.get("format")).unwrap_or_default().to_ascii_lowercase(),
                        url: url.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Game { id, name, medias })
}

impl<'a> ScreenScraper<'a> {
    pub fn new(fetch: &'a dyn Fetch, dev: DevCreds, user: Option<(String, String)>, sleep: &'a dyn Fn(Duration)) -> ScreenScraper<'a> {
        ScreenScraper { fetch, dev, user, sleep, last_call: Cell::new(None), user_info: Cell::new(None) }
    }

    pub fn user_info(&self) -> Option<UserInfo> {
        self.user_info.get()
    }

    fn url(&self, endpoint: &str, params: &[(&str, String)]) -> String {
        let mut url = format!(
            "{BASE}/{endpoint}?devid={}&devpassword={}&softname={}&output=json",
            net::encode_component(&self.dev.id),
            net::encode_component(&self.dev.password),
            net::encode_component(&self.dev.softname)
        );
        if let Some((id, password)) = &self.user {
            url.push_str(&format!("&ssid={}&sspassword={}", net::encode_component(id), net::encode_component(password)));
        }
        for (name, value) in params {
            url.push_str(&format!("&{name}={}", net::encode_component(value)));
        }
        url
    }

    fn pace(&self) {
        if let Some(last) = self.last_call.get() {
            let elapsed = last.elapsed();
            if elapsed < MIN_GAP {
                (self.sleep)(MIN_GAP - elapsed);
            }
        }
        self.last_call.set(Some(Instant::now()));
    }

    fn request(&self, endpoint: &str, params: &[(&str, String)]) -> Result<Value, SourceError> {
        let url = self.url(endpoint, params);
        let mut retried = false;
        loop {
            self.pace();
            let response = self.fetch.get(&url, &[("Accept", "application/json")], MAX_JSON_BYTES)?;
            if !net::host_allowed(&response.final_url, &IMAGE_HOSTS) {
                return Err(SourceError::BadResponse("ScreenScraper answered from another address; ignored".into()));
            }
            match response.status {
                200 => return self.parse(&response.body),
                400 => return Err(SourceError::BadResponse("ScreenScraper refused the request as badly formed".into())),
                401 => return Err(SourceError::BackedOff { secs: 600, reason: "ScreenScraper is closed to this account or its servers are saturated".into() }),
                403 => return Err(SourceError::Invalid("this app's developer credentials were not accepted".into())),
                404 => return Err(SourceError::NotFound),
                423 => return Err(SourceError::BackedOff { secs: 3600, reason: "ScreenScraper is closed for a serious problem on their side".into() }),
                426 => return Err(SourceError::BackedOff { secs: 86_400, reason: "ScreenScraper blocks this version of the app".into() }),
                429 if !retried => {
                    retried = true;
                    (self.sleep)(THREAD_LIMIT_WAIT);
                }
                429 => return Err(SourceError::BackedOff { secs: 60, reason: "ScreenScraper's thread limit for this account is reached".into() }),
                430 => return Err(SourceError::Limited("ScreenScraper's daily request quota is used up".into())),
                431 => return Err(SourceError::Limited("ScreenScraper saw too many unrecognised ROMs today".into())),
                other => return Err(SourceError::BadResponse(format!("ScreenScraper answered HTTP {other}"))),
            }
        }
    }

    fn parse(&self, body: &[u8]) -> Result<Value, SourceError> {
        let text = String::from_utf8_lossy(body);
        let trimmed = text.trim_start_matches('\u{FEFF}').trim();
        // Some errors come as a plain French sentence with a 200.
        if !trimmed.starts_with('{') {
            let lower = trimmed.to_lowercase();
            return Err(if lower.contains("non trouv") || lower.contains("not found") {
                SourceError::NotFound
            } else if lower.contains("identifiant") || lower.contains("login") || lower.contains("password") {
                SourceError::Invalid("the login was not accepted".into())
            } else if lower.contains("ferm") || lower.contains("closed") {
                SourceError::BackedOff { secs: 600, reason: "ScreenScraper says its API is closed".into() }
            } else {
                SourceError::BadResponse("ScreenScraper sent something that is not JSON".into())
            });
        }
        let v: Value = serde_json::from_str(trimmed).map_err(|_| SourceError::BadResponse("ScreenScraper sent JSON that could not be read".into()))?;
        if let Some(info) = parse_user(&v) {
            self.user_info.set(Some(info));
        }
        Ok(v)
    }

    /// The game whose ROM has these hashes and this size.
    pub fn game_by_rom(&self, rom: &RomIdentity) -> Result<Game, SourceError> {
        let mut params = vec![("crc", rom.crc32.clone()), ("md5", rom.md5.clone())];
        if let Some(sha1) = &rom.sha1 {
            params.push(("sha1", sha1.clone()));
        }
        params.push(("romtaille", rom.size.to_string()));
        if let Some(system) = rom.system_id {
            params.push(("systemeid", system.to_string()));
        }
        params.push(("romnom", rom.file_name.clone()));
        let v = self.request("jeuInfos.php", &params)?;
        v.get("response").and_then(|r| r.get("jeu")).and_then(parse_game).ok_or(SourceError::NotFound)
    }

    /// A game by ScreenScraper's own id, for a game found by name or kept from an earlier run.
    pub fn game_by_id(&self, id: u64) -> Result<Game, SourceError> {
        let v = self.request("jeuInfos.php", &[("gameid", id.to_string())])?;
        v.get("response").and_then(|r| r.get("jeu")).and_then(parse_game).ok_or(SourceError::NotFound)
    }

    /// Up to 30 candidates for `name`, ranked by ScreenScraper.
    pub fn search(&self, name: &str, system: Option<u32>) -> Result<Vec<Game>, SourceError> {
        let mut params = vec![("recherche", name.to_string())];
        if let Some(system) = system {
            params.push(("systemeid", system.to_string()));
        }
        let v = self.request("jeuRecherche.php", &params)?;
        let games = v.get("response").and_then(|r| r.get("jeux")).and_then(Value::as_array).ok_or(SourceError::NotFound)?;
        Ok(games.iter().filter_map(parse_game).collect())
    }

    pub fn download(&self, url: &str) -> Result<Vec<u8>, SourceError> {
        self.pace();
        download_image(self.fetch, url, &IMAGE_HOSTS, "ScreenScraper")
    }

    /// One cheap authenticated call: the user's account info with a login, the server info without.
    pub fn check_login(&self) -> KeyCheck {
        let (endpoint, with_user) = if self.user.is_some() { ("ssuserInfos.php", true) } else { ("ssinfraInfos.php", false) };
        match self.request(endpoint, &[]) {
            Ok(v) if with_user => match parse_user(&v) {
                Some(u) if u.level > 0 || u.max_per_day > 0 => {
                    KeyCheck::Ok(format!("{} of {} requests used today, {} thread{}", u.requests_today, u.max_per_day, u.max_threads, if u.max_threads == 1 { "" } else { "s" }))
                }
                _ => KeyCheck::Invalid("the login was not accepted".into()),
            },
            Ok(_) => KeyCheck::Ok("developer credentials accepted; no login set, so the lowest limits apply".into()),
            Err(SourceError::Invalid(why)) => KeyCheck::Invalid(why),
            Err(SourceError::Offline(why)) => KeyCheck::Offline(why),
            Err(other) => KeyCheck::Unknown(other.to_string()),
        }
    }
}

/// Every candidate ranked for `title`, best first (ScreenScraper's own order breaks ties).
pub fn ranked<'g>(title: &str, games: &'g [Game]) -> Vec<(&'g Game, Confidence)> {
    let names: Vec<&str> = games.iter().map(|g| g.name.as_str()).collect();
    let mut all: Vec<(&Game, Confidence)> = games.iter().zip(confidences(title, &names)).collect();
    all.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    all
}

/// The best candidate for `title` among name-search results, and how sure that is.
pub fn best_match<'g>(title: &str, games: &'g [Game]) -> Option<(&'g Game, Confidence)> {
    ranked(title, games).into_iter().next()
}


/// Media of `kind` in the order ES-DE prefers: the user's region, world, US, Europe, Japan, custom,
/// then anything. Only image formats the decoder reads.
pub fn media_of<'g>(game: &'g Game, kinds: &[&str], region: &str) -> Vec<&'g Media> {
    let chain = [region, "wor", "us", "eu", "jp", "cus", ""];
    let mut out = Vec::new();
    for kind in kinds {
        let of_kind: Vec<&Media> = game.medias.iter().filter(|m| m.kind == *kind && matches!(m.format.as_str(), "png" | "jpg" | "jpeg" | "webp")).collect();
        for want in chain {
            for m in &of_kind {
                if (want.is_empty() || m.region == want) && !out.iter().any(|o: &&Media| std::ptr::eq(*o, *m)) {
                    out.push(*m);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::net::fake::Fake;
    use super::*;
    use crate::fork::rom_library::hash::Digests;
    use crate::fork::rom_library::system::ByteOrder;
    use std::cell::RefCell;
    use std::path::PathBuf;

    const INFO: &str = include_str!("fixtures/ss_jeuinfos.json");
    const SEARCH: &str = include_str!("fixtures/ss_recherche.json");
    const USER: &str = include_str!("fixtures/ss_userinfos.json");

    fn dev() -> DevCreds {
        DevCreds { id: "devidTEST".into(), password: "devpassTEST".into(), softname: "test-soft".into() }
    }

    fn rom() -> RomIdentity {
        RomIdentity {
            system_id: Some(14),
            size: 8_388_608,
            crc32: "AABBCCDD".into(),
            md5: "0123456789abcdef0123456789abcdef".into(),
            sha1: Some("0123456789abcdef0123456789abcdef01234567".into()),
            file_name: "Game (USA).z64".into(),
        }
    }

    struct Sleeps(RefCell<Vec<Duration>>);

    impl Sleeps {
        fn new() -> Sleeps {
            Sleeps(RefCell::new(Vec::new()))
        }
        fn sleeper(&self) -> impl Fn(Duration) + '_ {
            |d| self.0.borrow_mut().push(d)
        }
    }

    #[test]
    fn a_rom_lookup_sends_hashes_size_system_and_credentials_and_parses_the_game() {
        let fake = Fake::new().on_json("jeuInfos.php", 200, INFO);
        let sleeps = Sleeps::new();
        let sleep = sleeps.sleeper();
        let ss = ScreenScraper::new(&fake, dev(), Some(("tim".into(), "pw pw".into())), &sleep);
        let game = ss.game_by_rom(&rom()).unwrap();
        assert_eq!(game.id, 777001);
        assert_eq!(game.name, "The Legend of Fixture: Test of Time");
        let url = &fake.requests()[0];
        for part in [
            "devid=devidTEST", "devpassword=devpassTEST", "softname=test-soft", "output=json", "ssid=tim", "sspassword=pw%20pw", "crc=AABBCCDD",
            "md5=0123456789abcdef0123456789abcdef", "sha1=0123456789abcdef0123456789abcdef01234567", "romtaille=8388608", "systemeid=14", "romnom=Game%20%28USA%29.z64",
        ] {
            assert!(url.contains(part), "{url} lacks {part}");
        }
        assert!(url.starts_with("https://api.screenscraper.fr/api2/jeuInfos.php?"));
        assert_eq!(ss.user_info(), Some(UserInfo { max_threads: 1, requests_today: 12, max_per_day: 20000, level: 1 }));
    }

    #[test]
    fn without_a_user_login_no_ssid_is_sent() {
        let fake = Fake::new().on_json("jeuInfos.php", 200, INFO);
        let sleeps = Sleeps::new();
        let sleep = sleeps.sleeper();
        ScreenScraper::new(&fake, dev(), None, &sleep).game_by_rom(&rom()).unwrap();
        assert!(!fake.requests()[0].contains("ssid=") && !fake.requests()[0].contains("sspassword="));
    }

    #[test]
    fn media_foreign_hosts_are_dropped_and_the_region_chain_is_us_then_world_then_the_rest() {
        let fake = Fake::new().on_json("jeuInfos.php", 200, INFO);
        let sleeps = Sleeps::new();
        let sleep = sleeps.sleeper();
        let game = ScreenScraper::new(&fake, dev(), None, &sleep).game_by_rom(&rom()).unwrap();
        assert_eq!(game.medias.len(), 8, "the evil.example media is dropped");
        let boxes = media_of(&game, &["box-2D"], "us");
        assert_eq!(boxes.iter().map(|m| m.region.as_str()).collect::<Vec<_>>(), vec!["us", "eu"]);
        let boxes_eu = media_of(&game, &["box-2D"], "eu");
        assert_eq!(boxes_eu[0].region, "eu");
        let logos = media_of(&game, &["wheel-hd", "wheel"], "us");
        assert_eq!(logos.iter().map(|m| m.kind.as_str()).collect::<Vec<_>>(), vec!["wheel-hd", "wheel"]);
        assert!(media_of(&game, &["video"], "us").is_empty(), "video is not an image format we use");
        assert_eq!(media_of(&game, &["fanart"], "us").len(), 1, "regionless media is found by the last link of the chain");
    }

    #[test]
    fn a_name_search_returns_candidates_and_drops_placeholder_rows() {
        let fake = Fake::new().on_json("jeuRecherche.php", 200, SEARCH);
        let sleeps = Sleeps::new();
        let sleep = sleeps.sleeper();
        let ss = ScreenScraper::new(&fake, dev(), None, &sleep);
        let games = ss.search("Ship of Harkinian", Some(14)).unwrap();
        assert_eq!(games.iter().map(|g| g.id).collect::<Vec<_>>(), vec![777002, 777003]);
        assert!(fake.requests()[0].contains("recherche=Ship%20of%20Harkinian") && fake.requests()[0].contains("systemeid=14"));
        let (game, conf) = best_match("Ship of Harkinian", &games).unwrap();
        assert_eq!((game.id, conf), (777002, Confidence::High));
        let (_, conf) = best_match("Ship", &games).unwrap();
        assert!(!conf.is_silent());
    }

    #[test]
    fn the_error_codes_become_the_right_back_offs_and_limits() {
        let run = |status: u16| {
            let fake = Fake::new().on_json("jeuInfos.php", status, "");
            let sleeps = Sleeps::new();
            let sleep = sleeps.sleeper();
            ScreenScraper::new(&fake, dev(), None, &sleep).game_by_rom(&rom())
        };
        assert!(matches!(run(401), Err(SourceError::BackedOff { secs: 600, .. })));
        assert!(matches!(run(403), Err(SourceError::Invalid(_))));
        assert_eq!(run(404), Err(SourceError::NotFound));
        assert!(matches!(run(423), Err(SourceError::BackedOff { secs: 3600, .. })));
        assert!(matches!(run(426), Err(SourceError::BackedOff { secs: 86_400, .. })));
        assert!(matches!(run(429), Err(SourceError::BackedOff { secs: 60, .. })));
        assert!(matches!(run(430), Err(SourceError::Limited(_))));
        assert!(matches!(run(431), Err(SourceError::Limited(_))));
        assert!(matches!(run(400), Err(SourceError::BadResponse(_))));
        assert!(matches!(run(500), Err(SourceError::BadResponse(_))));
        for status in [401u16, 423, 429, 430, 431] {
            assert!(run(status).unwrap_err().stops_the_run(), "{status}");
        }
        assert!(!run(404).unwrap_err().stops_the_run());
    }

    #[test]
    fn a_429_waits_once_and_retries_then_gives_up() {
        use super::super::net::{NetError, Response};
        struct Scripted(RefCell<Vec<u16>>);
        impl Fetch for Scripted {
            fn get(&self, _url: &str, _h: &[(&str, &str)], _m: u64) -> Result<Response, NetError> {
                let status = self.0.borrow_mut().remove(0);
                Ok(Response { status, body: if status == 200 { INFO.as_bytes().to_vec() } else { Vec::new() }, final_url: _url.to_string() })
            }
        }
        let sleeps = Sleeps::new();
        let sleep = sleeps.sleeper();
        let fetch = Scripted(RefCell::new(vec![429, 200]));
        let game = ScreenScraper::new(&fetch, dev(), None, &sleep).game_by_rom(&rom()).unwrap();
        assert_eq!(game.id, 777001);
        assert!(sleeps.0.borrow().contains(&THREAD_LIMIT_WAIT), "{:?}", sleeps.0.borrow());

        let fetch = Scripted(RefCell::new(vec![429, 429]));
        assert!(matches!(ScreenScraper::new(&fetch, dev(), None, &sleep).game_by_rom(&rom()), Err(SourceError::BackedOff { secs: 60, .. })));
        assert!(fetch.0.borrow().is_empty(), "exactly two attempts");
    }

    #[test]
    fn requests_are_paced_one_after_another() {
        let fake = Fake::new().on_json("jeuInfos.php", 200, INFO);
        let sleeps = Sleeps::new();
        let sleep = sleeps.sleeper();
        let ss = ScreenScraper::new(&fake, dev(), None, &sleep);
        ss.game_by_rom(&rom()).unwrap();
        assert!(sleeps.0.borrow().is_empty(), "the first request goes straight out");
        ss.game_by_rom(&rom()).unwrap();
        assert_eq!(sleeps.0.borrow().len(), 1);
        assert!(sleeps.0.borrow()[0] > Duration::from_millis(900));
    }

    #[test]
    fn plain_text_errors_with_a_200_are_understood() {
        let run = |body: &str| {
            let fake = Fake::new().on_json("jeuInfos.php", 200, body);
            let sleeps = Sleeps::new();
            let sleep = sleeps.sleeper();
            ScreenScraper::new(&fake, dev(), None, &sleep).game_by_rom(&rom())
        };
        assert_eq!(run("Erreur : Rom/Iso/Dossier non trouvée !"), Err(SourceError::NotFound));
        assert!(matches!(run("Erreur de login : Vérifiez vos identifiants développeur !"), Err(SourceError::Invalid(_))));
        assert!(matches!(run("API fermée pour les non membres"), Err(SourceError::BackedOff { .. })));
        assert!(matches!(run("<html>nginx</html>"), Err(SourceError::BadResponse(_))));
        assert!(matches!(run("{ broken"), Err(SourceError::BadResponse(_))));
        assert_eq!(run(r#"{"response":{}}"#), Err(SourceError::NotFound));
    }

    #[test]
    fn offline_is_offline_and_no_credential_reaches_any_error() {
        let secrets = ["devidTEST", "devpassTEST", "tim-login", "pw-secret-1"];
        let sleeps = Sleeps::new();
        let sleep = sleeps.sleeper();
        let user = Some(("tim-login".to_string(), "pw-secret-1".to_string()));
        let fake = Fake::new();
        let err = ScreenScraper::new(&fake, dev(), user.clone(), &sleep).game_by_rom(&rom()).unwrap_err();
        assert!(matches!(err, SourceError::Offline(_)));
        for status in [400u16, 401, 403, 404, 423, 426, 429, 430, 431, 500] {
            let fake = Fake::new().on_json("jeuInfos.php", status, "");
            let err = ScreenScraper::new(&fake, dev(), user.clone(), &sleep).game_by_rom(&rom()).unwrap_err();
            let text = format!("{err} {err:?}");
            for s in secrets {
                assert!(!text.contains(s), "{status}: {text}");
            }
        }
    }

    #[test]
    fn the_login_check_reads_the_account_or_says_it_is_not_recognised() {
        let sleeps = Sleeps::new();
        let sleep = sleeps.sleeper();
        let fake = Fake::new().on_json("ssuserInfos.php", 200, USER);
        let check = ScreenScraper::new(&fake, dev(), Some(("tim".into(), "pw".into())), &sleep).check_login();
        assert_eq!(check, KeyCheck::Ok("40 of 20000 requests used today, 2 threads".into()));

        let fake = Fake::new().on_json("ssuserInfos.php", 200, r#"{"response":{"ssuser":{"id":"","niveau":"0"}}}"#);
        assert!(matches!(ScreenScraper::new(&fake, dev(), Some(("tim".into(), "pw".into())), &sleep).check_login(), KeyCheck::Invalid(_)));

        let fake = Fake::new().on_json("ssuserInfos.php", 200, r#"{"response":{}}"#);
        assert!(matches!(ScreenScraper::new(&fake, dev(), Some(("tim".into(), "pw".into())), &sleep).check_login(), KeyCheck::Invalid(_)));

        let fake = Fake::new().on_json("ssinfraInfos.php", 200, r#"{"response":{"serveurs":{}}}"#);
        assert!(ScreenScraper::new(&fake, dev(), None, &sleep).check_login().is_ok());
        assert_eq!(fake.requests().len(), 1);

        let fake = Fake::new().on_json("ssinfraInfos.php", 403, "");
        assert!(matches!(ScreenScraper::new(&fake, dev(), None, &sleep).check_login(), KeyCheck::Invalid(_)));
        let fake = Fake::new();
        assert!(matches!(ScreenScraper::new(&fake, dev(), None, &sleep).check_login(), KeyCheck::Offline(_)));
        let fake = Fake::new().on_json("ssinfraInfos.php", 430, "");
        assert!(matches!(ScreenScraper::new(&fake, dev(), None, &sleep).check_login(), KeyCheck::Unknown(_)));
    }

    #[test]
    fn debug_output_hides_credentials_and_media_addresses() {
        let dev = DevCreds { id: "DEVID-SECRET".into(), password: "DEVPASS-SECRET".into(), softname: "soft".into() };
        let media = Media { kind: "box-2D".into(), region: "us".into(), format: "png".into(), url: "https://www.screenscraper.fr/x?ssid=u&sspassword=SECRETPW".into() };
        let game = Game { id: 1, name: "G".into(), medias: vec![media.clone()] };
        let text = format!("{dev:?} {media:?} {game:?}");
        for secret in ["DEVID-SECRET", "DEVPASS-SECRET", "SECRETPW", "screenscraper.fr"] {
            assert!(!text.contains(secret), "{text}");
        }
    }

    #[test]
    fn systems_have_ids_and_unknown_has_none() {
        assert_eq!(system_id(System::N64), Some(14));
        assert_eq!(system_id(System::GameCube), Some(13));
        assert_eq!(system_id(System::Unknown), None);
    }

    #[test]
    fn a_byte_swapped_n64_image_gets_two_identities_big_endian_first() {
        let digests = |crc: u32, md5_byte: u8| Digests { crc32: crc, md5: [md5_byte; 16], sha1: Some([md5_byte; 20]) };
        let file = RomFile {
            path: PathBuf::from("/roms/n64/Game (USA).v64"),
            rel_path: PathBuf::from("n64/Game (USA).v64"),
            system: System::N64,
            size: 8_388_608,
            mtime_ms: 0,
            ext: "v64".into(),
            byte_order: Some(ByteOrder::ByteSwapped),
            raw: digests(0x1111_1111, 0x11),
            normalized: Some(digests(0x2222_2222, 0x22)),
        };
        let ids = RomIdentity::from_rom_file(&file);
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0].crc32, "22222222");
        assert_eq!(ids[1].crc32, "11111111");
        assert_eq!((ids[0].system_id, ids[0].size, ids[0].file_name.as_str()), (Some(14), 8_388_608, "Game (USA).v64"));
        let plain = RomFile { normalized: None, byte_order: None, ..file };
        assert_eq!(RomIdentity::from_rom_file(&plain).len(), 1);
    }

    #[test]
    fn downloads_stay_on_screenscrapers_hosts() {
        let fake = Fake::new().on("screenscraper.fr", 200, b"IMG".to_vec());
        let sleeps = Sleeps::new();
        let sleep = sleeps.sleeper();
        let ss = ScreenScraper::new(&fake, dev(), None, &sleep);
        assert_eq!(ss.download("https://www.screenscraper.fr/image.php?gameid=1&media=ss").unwrap(), b"IMG");
        assert!(ss.download("https://evil.example/x.png").is_err());
    }
}
