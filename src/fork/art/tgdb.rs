//! TheGamesDB API v1 client: look a game up by id (or by exact title), list its images, pick the
//! ones that fit a Steam slot.
//!
//! Base `https://api.thegamesdb.net/v1`; the key travels as `apikey=` on every call (the API has no
//! header option), so no address built here is ever logged or put in an error message. A 403 means a
//! bad key or a spent allowance (the API does not say which). Every answer carries
//! `remaining_monthly_allowance` and `extra_allowance`; the client keeps the last values, stops
//! calling when they reach zero, and the screen shows them.
//!
//! Image variants: the `large` size is upscaled from small originals (a 581x732 box came back as
//! 1080x1360), so the client takes `original` whenever its `resolution` field says it is big enough
//! for the slot, and skips images that would need more than 1.5x enlargement.
//!
//! Slots: boxart front is the portrait; banner (one exactly 1920x620 if there is one) then fanart
//! make the hero and the wide capsule; clearlogo is the logo. The icon has no TheGamesDB source.

use std::cell::Cell;

use serde_json::Value;

use super::fit::{upscale_needed, Slot, MAX_UPSCALE};
use super::net::{self, Fetch, MAX_JSON_BYTES};
use super::sgdb::download_image;
use super::source::{KeyCheck, SourceError};
use super::title::{confidence, Confidence};

const BASE: &str = "https://api.thegamesdb.net/v1";
const IMAGE_HOSTS: [&str; 1] = ["thegamesdb.net"];
/// TheGamesDB's own "PC" platform, where fan ports are listed.
const PLATFORM_PC: u32 = 1;
/// The decoder refuses sources over this per side, so a bigger original is taken as the `large` size.
const MAX_ORIGINAL_SIDE: u32 = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameInfo {
    pub id: u64,
    pub title: String,
    pub platform: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    BoxFront,
    BoxBack,
    Fanart,
    Banner,
    ClearLogo,
    Screenshot,
    TitleScreen,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub id: u64,
    pub kind: ImageKind,
    pub filename: String,
    pub resolution: Option<(u32, u32)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageSet {
    pub original_base: String,
    pub large_base: String,
    pub images: Vec<Image>,
}

/// One image the client would download, in order of preference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub image_id: u64,
    pub url: String,
    pub resolution: Option<(u32, u32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Allowance {
    pub remaining: u64,
    pub extra: u64,
}

impl Allowance {
    pub fn total(self) -> u64 {
        self.remaining.saturating_add(self.extra)
    }
}

pub struct TheGamesDb<'a> {
    fetch: &'a dyn Fetch,
    key: &'a str,
    allowance: Cell<Option<Allowance>>,
}

fn parse_resolution(text: &str) -> Option<(u32, u32)> {
    let (w, h) = text.trim().split_once(['x', 'X'])?;
    let (w, h) = (w.trim().parse::<u32>().ok()?, h.trim().parse::<u32>().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

fn number(v: Option<&Value>) -> Option<u64> {
    let v = v?;
    v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

impl<'a> TheGamesDb<'a> {
    pub fn new(fetch: &'a dyn Fetch, key: &'a str) -> TheGamesDb<'a> {
        TheGamesDb { fetch, key, allowance: Cell::new(None) }
    }

    /// What the last answer said was left (monthly plus extra), if any answer has said so.
    pub fn allowance(&self) -> Option<Allowance> {
        self.allowance.get()
    }

    fn get_json(&self, endpoint: &str, params: &str) -> Result<Value, SourceError> {
        if self.allowance.get().is_some_and(|a| a.total() == 0) {
            return Err(SourceError::Limited("TheGamesDB monthly allowance is used up".into()));
        }
        let url = format!("{BASE}/{endpoint}?apikey={}{}{params}", net::encode_component(self.key), if params.is_empty() { "" } else { "&" });
        let response = self.fetch.get(&url, &[("Accept", "application/json")], MAX_JSON_BYTES)?;
        match response.status {
            200 => {}
            401 | 403 => return Err(SourceError::Invalid("the key was not recognised, or its allowance is used up".into())),
            404 => return Err(SourceError::NotFound),
            406 => return Err(SourceError::BadResponse("TheGamesDB says a parameter is missing".into())),
            429 => return Err(SourceError::Limited("TheGamesDB says too many requests".into())),
            other => return Err(SourceError::BadResponse(format!("TheGamesDB answered HTTP {other}"))),
        }
        let v: Value = serde_json::from_slice(&response.body).map_err(|_| SourceError::BadResponse("TheGamesDB sent something that is not JSON".into()))?;
        if let (Some(remaining), extra) = (number(v.get("remaining_monthly_allowance")), number(v.get("extra_allowance")).unwrap_or(0)) {
            self.allowance.set(Some(Allowance { remaining, extra }));
        }
        if let Some(code) = v.get("code").and_then(Value::as_u64) {
            match code {
                200 => {}
                401 | 403 => return Err(SourceError::Invalid("the key was not recognised, or its allowance is used up".into())),
                404 => return Err(SourceError::NotFound),
                other => return Err(SourceError::BadResponse(format!("TheGamesDB reported code {other}"))),
            }
        }
        Ok(v)
    }

    fn games_of(v: &Value) -> Vec<GameInfo> {
        v.get("data")
            .and_then(|d| d.get("games"))
            .and_then(Value::as_array)
            .map(|games| {
                games
                    .iter()
                    .filter_map(|g| {
                        Some(GameInfo {
                            id: number(g.get("id"))?,
                            title: g.get("game_title")?.as_str()?.to_string(),
                            platform: number(g.get("platform")).and_then(|p| u32::try_from(p).ok()).unwrap_or(0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn by_game_id(&self, id: u64) -> Result<GameInfo, SourceError> {
        let v = self.get_json("Games/ByGameID", &format!("id={id}"))?;
        Self::games_of(&v).into_iter().next().ok_or(SourceError::NotFound)
    }

    /// Games called `name` on the PC platform, in TheGamesDB's own order (most relevant first).
    pub fn search(&self, name: &str) -> Result<Vec<GameInfo>, SourceError> {
        let v = self.get_json("Games/ByGameName", &format!("name={}&filter%5Bplatform%5D={PLATFORM_PC}", net::encode_component(name)))?;
        Ok(Self::games_of(&v))
    }

    pub fn images(&self, game_id: u64) -> Result<ImageSet, SourceError> {
        let v = self.get_json("Games/Images", &format!("games_id={game_id}"))?;
        let data = v.get("data").ok_or_else(|| SourceError::BadResponse("TheGamesDB sent no image data".into()))?;
        let base = |name: &str| data.get("base_url").and_then(|b| b.get(name)).and_then(Value::as_str).filter(|u| net::host_allowed(u, &IMAGE_HOSTS)).unwrap_or("").to_string();
        let mut set = ImageSet { original_base: base("original"), large_base: base("large"), images: Vec::new() };
        // The answer is keyed by game id; accept a plain list too.
        let key = game_id.to_string();
        let list = match data.get("images") {
            Some(Value::Object(by_game)) => by_game.get(&key).and_then(Value::as_array),
            Some(Value::Array(list)) => Some(list),
            _ => None,
        };
        for item in list.into_iter().flatten() {
            let (Some(id), Some(filename)) = (number(item.get("id")), item.get("filename").and_then(Value::as_str)) else { continue };
            if filename.contains("..") || filename.starts_with('/') || filename.contains("://") {
                continue;
            }
            let side = item.get("side").and_then(Value::as_str).unwrap_or("");
            let kind = match (item.get("type").and_then(Value::as_str).unwrap_or(""), side) {
                ("boxart", "front") => ImageKind::BoxFront,
                ("boxart", "back") => ImageKind::BoxBack,
                ("fanart", _) => ImageKind::Fanart,
                ("banner", _) => ImageKind::Banner,
                ("clearlogo", _) => ImageKind::ClearLogo,
                ("screenshot", _) => ImageKind::Screenshot,
                ("titlescreen", _) => ImageKind::TitleScreen,
                _ => ImageKind::Other,
            };
            set.images.push(Image { id, kind, filename: filename.to_string(), resolution: item.get("resolution").and_then(Value::as_str).and_then(parse_resolution) });
        }
        Ok(set)
    }

    pub fn download(&self, url: &str) -> Result<Vec<u8>, SourceError> {
        download_image(self.fetch, url, &IMAGE_HOSTS, "TheGamesDB")
    }

    /// One authenticated call that does not count against the allowance (`API/Limit`). Should that
    /// endpoint turn out not to exist, a one-game lookup is the fallback.
    pub fn check_key(&self) -> KeyCheck {
        let result = match self.get_json("API/Limit", "") {
            Err(SourceError::NotFound) | Err(SourceError::BadResponse(_)) => self.get_json("Games/ByGameID", "id=1"),
            other => other,
        };
        match result {
            Ok(_) => KeyCheck::Ok(self.allowance().map(|a| format!("{} requests left this month", a.total())).unwrap_or_default()),
            Err(SourceError::Invalid(why)) => KeyCheck::Invalid(why),
            Err(SourceError::Offline(why)) => KeyCheck::Offline(why),
            Err(SourceError::NotFound) => KeyCheck::Ok(String::new()),
            Err(other) => KeyCheck::Unknown(other.to_string()),
        }
    }
}

/// Where a game's name search result stands: the first same-title PC entry wins.
pub fn best_match(title: &str, games: &[GameInfo]) -> Option<(GameInfo, Confidence)> {
    let top = games.iter().map(|g| confidence(title, &g.title)).max()?;
    let pick = games.iter().find(|g| confidence(title, &g.title) == top && g.platform == PLATFORM_PC).or_else(|| games.iter().find(|g| confidence(title, &g.title) == top))?;
    Some((pick.clone(), top))
}

impl ImageSet {
    fn url(&self, base: &str, filename: &str) -> Option<String> {
        (!base.is_empty()).then(|| format!("{base}{filename}"))
    }

    /// Images to try for `slot`, best first, each with the size variant to download. Candidates that
    /// would need more than 1.5x enlargement are left out.
    pub fn candidates(&self, slot: Slot) -> Vec<Candidate> {
        let kinds: &[ImageKind] = match slot {
            Slot::Portrait => &[ImageKind::BoxFront],
            Slot::Hero | Slot::Wide => &[ImageKind::Banner, ImageKind::Fanart],
            Slot::Logo => &[ImageKind::ClearLogo],
            Slot::Icon => &[],
        };
        let (tw, th) = slot.size();
        let target_ratio = f64::from(tw) / f64::from(th);
        let mut out: Vec<(usize, f64, Candidate)> = Vec::new();
        for (rank, kind) in kinds.iter().enumerate() {
            for image in self.images.iter().filter(|i| i.kind == *kind) {
                let large = || self.url(&self.large_base, &image.filename);
                let original = || self.url(&self.original_base, &image.filename);
                let (url, resolution) = match image.resolution {
                    Some((w, h)) => {
                        if upscale_needed(slot, w, h) > MAX_UPSCALE + 1e-9 {
                            continue;
                        }
                        // Over the decoder's limit the original cannot be read; `large` is smaller.
                        if w > MAX_ORIGINAL_SIDE || h > MAX_ORIGINAL_SIDE {
                            (large(), None)
                        } else {
                            (original(), Some((w, h)))
                        }
                    }
                    None => (original(), None),
                };
                let Some(url) = url else { continue };
                // Closeness of shape to the slot, so a banner of exactly 1920x620 goes first.
                let off = image.resolution.map_or(1.0, |(w, h)| (f64::from(w) / f64::from(h) / target_ratio).ln().abs());
                out.push((rank, off, Candidate { image_id: image.id, url, resolution }));
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        out.into_iter().map(|(_, _, c)| c).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::net::fake::Fake;
    use super::*;

    const BY_ID: &str = include_str!("fixtures/tgdb_by_id.json");
    const SEARCH: &str = include_str!("fixtures/tgdb_search.json");
    const IMAGES: &str = include_str!("fixtures/tgdb_images.json");
    const LIMIT: &str = include_str!("fixtures/tgdb_limit.json");

    #[test]
    fn by_game_id_parses_the_game_and_reads_the_allowance() {
        let fake = Fake::new().on_json("Games/ByGameID", 200, BY_ID);
        let tgdb = TheGamesDb::new(&fake, "tgdb-test-key-1");
        assert_eq!(tgdb.allowance(), None);
        let game = tgdb.by_game_id(900100).unwrap();
        assert_eq!(game, GameInfo { id: 900100, title: "Ship of Harkinian".into(), platform: 1 });
        assert_eq!(tgdb.allowance(), Some(Allowance { remaining: 987, extra: 0 }));
        assert!(fake.requests()[0].contains("apikey=tgdb-test-key-1") && fake.requests()[0].contains("id=900100"));
    }

    #[test]
    fn images_are_classified_and_the_original_size_is_taken_when_it_is_big_enough() {
        let fake = Fake::new().on_json("Games/Images", 200, IMAGES);
        let tgdb = TheGamesDb::new(&fake, "tgdb-test-key-1");
        let set = tgdb.images(900100).unwrap();
        assert_eq!(set.images.len(), 9);
        assert_eq!(tgdb.allowance().map(Allowance::total), Some(995));

        // Portrait: the 581x732 box needs 1.23x at most, so `original` is used.
        let portrait = set.candidates(Slot::Portrait);
        assert_eq!(portrait.len(), 1);
        assert_eq!(portrait[0].url, "https://cdn.thegamesdb.net/images/original/boxart/front/900100-1.jpg");
        assert_eq!(portrait[0].resolution, Some((581, 732)));

        // Hero: the exact 1920x620 banner first, then the 1800x900 banner, then the big fanart.
        // The 320x180 fanart would need 6x and is left out.
        let hero = set.candidates(Slot::Hero);
        assert_eq!(hero.iter().map(|c| c.image_id).collect::<Vec<_>>(), vec![5, 4, 3]);

        // Logo: both clear logos, original size (the 2927 px one is under the 4096 limit).
        assert_eq!(set.candidates(Slot::Logo).len(), 2);
        // No icon source.
        assert!(set.candidates(Slot::Icon).is_empty());
    }

    #[test]
    fn an_original_over_the_decode_limit_is_taken_as_the_large_size() {
        let set = ImageSet {
            original_base: "https://cdn.thegamesdb.net/images/original/".into(),
            large_base: "https://cdn.thegamesdb.net/images/large/".into(),
            images: vec![Image { id: 1, kind: ImageKind::Fanart, filename: "fanart/x.jpg".into(), resolution: Some((7680, 4320)) }],
        };
        let c = set.candidates(Slot::Hero);
        assert_eq!(c[0].url, "https://cdn.thegamesdb.net/images/large/fanart/x.jpg");
        assert_eq!(c[0].resolution, None);
    }

    #[test]
    fn name_search_is_pc_only_and_picks_the_same_title() {
        let fake = Fake::new().on_json("Games/ByGameName", 200, SEARCH);
        let tgdb = TheGamesDb::new(&fake, "tgdb-test-key-1");
        let games = tgdb.search("Ship of Harkinian").unwrap();
        assert_eq!(games.len(), 3);
        let url = &fake.requests()[0];
        assert!(url.contains("name=Ship%20of%20Harkinian") && url.contains("filter%5Bplatform%5D=1"), "{url}");
        let (game, conf) = best_match("Ship of Harkinian", &games).unwrap();
        assert_eq!((game.id, conf), (900101, Confidence::High));
        let (_, conf) = best_match("Ship", &games).unwrap();
        assert_eq!(conf, Confidence::Medium);
        assert!(best_match("x", &[]).is_none());
    }

    #[test]
    fn status_codes_map_to_typed_errors() {
        let with = |status: u16| {
            let fake = Fake::new().on_json("Games/ByGameID", status, "{}");
            TheGamesDb::new(&fake, "tgdb-test-key-1").by_game_id(1)
        };
        assert!(matches!(with(403), Err(SourceError::Invalid(_))));
        assert!(matches!(with(401), Err(SourceError::Invalid(_))));
        assert_eq!(with(404), Err(SourceError::NotFound));
        assert!(matches!(with(406), Err(SourceError::BadResponse(_))));
        assert!(matches!(with(429), Err(SourceError::Limited(_))));
        assert!(matches!(with(502), Err(SourceError::BadResponse(_))));
    }

    #[test]
    fn a_json_error_code_inside_a_200_is_an_error_too() {
        let fake = Fake::new().on_json("Games/ByGameID", 200, r#"{"code":403,"status":"Invalid API key"}"#);
        assert!(matches!(TheGamesDb::new(&fake, "tgdb-test-key-1").by_game_id(1), Err(SourceError::Invalid(_))));
    }

    #[test]
    fn malformed_answers_are_errors_or_empty_never_panics() {
        let run = |body: &str| {
            let fake = Fake::new().on_json("Games/", 200, body);
            let tgdb = TheGamesDb::new(&fake, "tgdb-test-key-1");
            (tgdb.by_game_id(1), tgdb.images(1))
        };
        let (game, images) = run("not json");
        assert!(matches!(game, Err(SourceError::BadResponse(_))) && matches!(images, Err(SourceError::BadResponse(_))));
        let (game, images) = run(r#"{"code":200,"data":{"count":0,"games":[]}}"#);
        assert_eq!(game, Err(SourceError::NotFound));
        assert!(matches!(images, Ok(set) if set.images.is_empty()));
        let (_, images) = run(r#"{"code":200,"data":{"images":{"1":[{"id":1,"type":"boxart","side":"front","filename":"../../etc/passwd","resolution":"600x900"},{"type":"boxart"}]}}}"#);
        assert!(images.unwrap().images.is_empty(), "path tricks and incomplete entries are dropped");
    }

    #[test]
    fn a_spent_allowance_stops_further_calls() {
        let body = r#"{"code":200,"status":"Success","data":{"count":1,"games":[{"id":1,"game_title":"X","platform":1}]},"remaining_monthly_allowance":0,"extra_allowance":0}"#;
        let fake = Fake::new().on_json("Games/ByGameID", 200, body);
        let tgdb = TheGamesDb::new(&fake, "tgdb-test-key-1");
        assert!(tgdb.by_game_id(1).is_ok());
        assert_eq!(tgdb.allowance().map(Allowance::total), Some(0));
        assert!(matches!(tgdb.by_game_id(2), Err(SourceError::Limited(_))));
        assert_eq!(fake.requests().len(), 1, "no second request once the allowance is gone");
    }

    #[test]
    fn extra_allowance_counts_and_string_numbers_are_read() {
        let body = r#"{"code":200,"data":{"games":[]},"remaining_monthly_allowance":"0","extra_allowance":"25"}"#;
        let fake = Fake::new().on_json("Games/ByGameName", 200, body);
        let tgdb = TheGamesDb::new(&fake, "tgdb-test-key-1");
        tgdb.search("x").unwrap();
        assert_eq!(tgdb.allowance(), Some(Allowance { remaining: 0, extra: 25 }));
        assert!(tgdb.search("y").is_ok(), "the extra allowance keeps it going");
    }

    #[test]
    fn the_key_check_uses_the_limit_endpoint_and_falls_back() {
        let fake = Fake::new().on_json("API/Limit", 200, LIMIT);
        assert_eq!(TheGamesDb::new(&fake, "tgdb-test-key-1").check_key(), KeyCheck::Ok("640 requests left this month".into()));
        assert_eq!(fake.requests().len(), 1);

        let fake = Fake::new().on_json("API/Limit", 404, "{}").on_json("Games/ByGameID", 200, BY_ID);
        assert!(TheGamesDb::new(&fake, "tgdb-test-key-1").check_key().is_ok());
        assert_eq!(fake.requests().len(), 2);

        let fake = Fake::new().on_json("API/Limit", 403, "{}");
        assert!(matches!(TheGamesDb::new(&fake, "tgdb-test-key-1").check_key(), KeyCheck::Invalid(_)));
        let fake = Fake::new();
        assert!(matches!(TheGamesDb::new(&fake, "tgdb-test-key-1").check_key(), KeyCheck::Offline(_)));
        let fake = Fake::new().on_json("API/Limit", 500, "{}").on_json("Games/ByGameID", 500, "{}");
        assert!(matches!(TheGamesDb::new(&fake, "tgdb-test-key-1").check_key(), KeyCheck::Unknown(_)));
    }

    #[test]
    fn downloads_stay_on_the_cdn_and_the_key_never_reaches_a_message() {
        let fake = Fake::new().on("cdn.thegamesdb.net", 200, b"IMG".to_vec());
        let tgdb = TheGamesDb::new(&fake, "tgdb-test-key-1");
        assert_eq!(tgdb.download("https://cdn.thegamesdb.net/images/original/a.jpg").unwrap(), b"IMG");
        assert!(tgdb.download("https://evil.example/a.jpg").is_err());
        let key = "SECRET-TGDB-KEY-777";
        for status in [403u16, 404, 429, 500] {
            let fake = Fake::new().on_json("Games/", status, "{}");
            let err = TheGamesDb::new(&fake, key).by_game_id(1).unwrap_err();
            assert!(!format!("{err} {err:?}").contains(key));
        }
        let fake = Fake::new();
        let err = TheGamesDb::new(&fake, key).images(1).unwrap_err();
        assert!(!format!("{err} {err:?}").contains(key), "an offline error must not echo the address");
    }

    #[test]
    fn the_key_is_encoded_in_the_query() {
        let fake = Fake::new().on_json("Games/ByGameID", 200, BY_ID);
        TheGamesDb::new(&fake, "a b&c").by_game_id(1).unwrap();
        assert!(fake.requests()[0].contains("apikey=a%20b%26c"));
    }
}
