//! SteamGridDB API v2 client: find a game by title, list its grids, heroes, logos and icons.
//!
//! Base `https://www.steamgriddb.com/api/v2`, `Authorization: Bearer <key>` on every call (the user's
//! own free key, pasted into the app). Every list request carries `types=static` (no animated art),
//! `nsfw=false`, `humor=false` and `epilepsy=false`, plus the dimension filter of the slot, so the
//! shapes arrive ready for Steam. HTTP 401 and 403 mean a bad key, 404 means no such game or no
//! images, 429 means slow down.
//!
//! No rate limit is published. The resolver makes a handful of calls per port, one at a time.
//!
//! Image addresses in the answers are only followed when they are https on a `steamgriddb.com` host.

use serde_json::Value;

use super::fit::Slot;
use super::net::{self, Fetch, MAX_IMAGE_BYTES, MAX_JSON_BYTES};
use super::source::{KeyCheck, SourceError};
use super::title::{confidence, Confidence};

const BASE: &str = "https://www.steamgriddb.com/api/v2";
const IMAGE_HOSTS: [&str; 1] = ["steamgriddb.com"];
/// Per request, the images listed; the resolver tries the first few.
const LIST_LIMIT: u32 = 12;

/// A game in SteamGridDB's catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    pub id: u64,
    pub name: String,
    pub verified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Grid,
    Hero,
    Logo,
    Icon,
}

impl Kind {
    fn path(self) -> &'static str {
        match self {
            Kind::Grid => "grids",
            Kind::Hero => "heroes",
            Kind::Logo => "logos",
            Kind::Icon => "icons",
        }
    }
}

/// What to ask SteamGridDB for to fill a slot. The portrait and the wide capsule are both grids, told
/// apart by their dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Filter {
    pub kind: Kind,
    pub dimensions: Option<&'static str>,
    pub mimes: Option<&'static str>,
}

pub fn filter_for(slot: Slot) -> Filter {
    match slot {
        Slot::Portrait => Filter { kind: Kind::Grid, dimensions: Some("600x900"), mimes: None },
        Slot::Wide => Filter { kind: Kind::Grid, dimensions: Some("920x430"), mimes: None },
        Slot::Hero => Filter { kind: Kind::Hero, dimensions: Some("1920x620,3840x1240"), mimes: None },
        // Logos come as PNG or WebP with transparency; no dimension filter exists for them.
        Slot::Logo => Filter { kind: Kind::Logo, dimensions: None, mimes: Some("image/png,image/webp") },
        // The decoder reads PNG, JPEG and WebP only, so .ico icons are left out.
        Slot::Icon => Filter { kind: Kind::Icon, dimensions: None, mimes: Some("image/png") },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub id: u64,
    pub score: i64,
    pub style: String,
    pub url: String,
}

pub struct SteamGridDb<'a> {
    fetch: &'a dyn Fetch,
    key: &'a str,
}

impl<'a> SteamGridDb<'a> {
    pub fn new(fetch: &'a dyn Fetch, key: &'a str) -> SteamGridDb<'a> {
        SteamGridDb { fetch, key }
    }

    fn get_json(&self, path_and_query: &str) -> Result<Value, SourceError> {
        let url = format!("{BASE}{path_and_query}");
        let bearer = format!("Bearer {}", self.key);
        let response = self.fetch.get(&url, &[("Authorization", bearer.as_str()), ("Accept", "application/json")], MAX_JSON_BYTES)?;
        match response.status {
            200 | 207 => {}
            401 | 403 => return Err(SourceError::Invalid("the key was not recognised".into())),
            404 => return Err(SourceError::NotFound),
            429 => return Err(SourceError::Limited("SteamGridDB says too many requests; try again in a few minutes".into())),
            other => return Err(SourceError::BadResponse(format!("SteamGridDB answered HTTP {other}"))),
        }
        let v: Value = serde_json::from_slice(&response.body).map_err(|_| SourceError::BadResponse("SteamGridDB sent something that is not JSON".into()))?;
        if v.get("success").and_then(Value::as_bool) == Some(false) {
            let why = v.get("errors").and_then(Value::as_array).and_then(|e| e.first()).and_then(Value::as_str).unwrap_or("no reason given");
            let why = net::strip_query(why);
            return Err(if why.to_ascii_lowercase().contains("not found") { SourceError::NotFound } else { SourceError::BadResponse(format!("SteamGridDB: {why}")) });
        }
        Ok(v)
    }

    /// Title search (`/search/autocomplete/{term}`), best matches first as SteamGridDB orders them.
    pub fn search(&self, term: &str) -> Result<Vec<Game>, SourceError> {
        let v = self.get_json(&format!("/search/autocomplete/{}", net::encode_component(term)))?;
        let games = v.get("data").and_then(Value::as_array).ok_or_else(|| SourceError::BadResponse("SteamGridDB sent no list of games".into()))?;
        Ok(games
            .iter()
            .filter_map(|g| {
                Some(Game { id: g.get("id")?.as_u64()?, name: g.get("name")?.as_str()?.to_string(), verified: g.get("verified").and_then(Value::as_bool).unwrap_or(false) })
            })
            .collect())
    }

    /// Images of one kind for a game, best score first as SteamGridDB orders them.
    pub fn assets(&self, game_id: u64, filter: Filter) -> Result<Vec<Asset>, SourceError> {
        let mut query = format!("types=static&nsfw=false&humor=false&epilepsy=false&limit={LIST_LIMIT}");
        if let Some(dimensions) = filter.dimensions {
            query.push_str(&format!("&dimensions={}", net::encode_component(dimensions)));
        }
        if let Some(mimes) = filter.mimes {
            query.push_str(&format!("&mimes={}", net::encode_component(mimes)));
        }
        let v = self.get_json(&format!("/{}/game/{game_id}?{query}", filter.kind.path()))?;
        let list = v.get("data").and_then(Value::as_array).ok_or_else(|| SourceError::BadResponse("SteamGridDB sent no list of images".into()))?;
        let mut assets: Vec<Asset> = list
            .iter()
            .filter_map(|a| {
                let url = a.get("url")?.as_str()?;
                if !net::host_allowed(url, &IMAGE_HOSTS) {
                    return None;
                }
                Some(Asset {
                    id: a.get("id")?.as_u64()?,
                    score: a.get("score").and_then(Value::as_i64).unwrap_or(0),
                    style: a.get("style").and_then(Value::as_str).unwrap_or("").to_string(),
                    url: url.to_string(),
                })
            })
            .collect();
        // Stable: equal scores keep SteamGridDB's own order.
        assets.sort_by_key(|a| std::cmp::Reverse(a.score));
        Ok(assets)
    }

    /// The image bytes. The address must be https on a SteamGridDB host.
    pub fn download(&self, url: &str) -> Result<Vec<u8>, SourceError> {
        download_image(self.fetch, url, &IMAGE_HOSTS, "SteamGridDB")
    }

    /// One cheap authenticated call: a title search for a well-known game.
    pub fn check_key(&self) -> KeyCheck {
        match self.search("portal") {
            Ok(_) => KeyCheck::Ok(String::new()),
            Err(SourceError::Invalid(why)) => KeyCheck::Invalid(why),
            Err(SourceError::Offline(why)) => KeyCheck::Offline(why),
            Err(SourceError::NotFound) => KeyCheck::Ok(String::new()),
            Err(SourceError::Limited(why)) => KeyCheck::Unknown(why),
            Err(other) => KeyCheck::Unknown(other.to_string()),
        }
    }
}

/// Shared by the three clients: fetch an image from a host the source owns.
pub(crate) fn download_image(fetch: &dyn Fetch, url: &str, hosts: &[&str], who: &str) -> Result<Vec<u8>, SourceError> {
    if !net::host_allowed(url, hosts) {
        return Err(SourceError::BadResponse(format!("{who} pointed at an address it does not own; not downloaded")));
    }
    let response = fetch.get(url, &[], MAX_IMAGE_BYTES)?;
    match response.status {
        200 => Ok(response.body),
        404 => Err(SourceError::NotFound),
        429 => Err(SourceError::Limited(format!("{who} says too many requests"))),
        other => Err(SourceError::BadResponse(format!("{who} image answered HTTP {other}"))),
    }
}

/// The best game for `title` among `games`, and how sure that is. The first same-title game wins,
/// a verified one over an unverified one; with no same-title game the first look-alike is returned
/// with its (lower) confidence.
pub fn best_match<'g>(title: &str, games: &'g [Game]) -> Option<(&'g Game, Confidence)> {
    let scored: Vec<(&Game, Confidence)> = games.iter().map(|g| (g, confidence(title, &g.name))).collect();
    let top = scored.iter().map(|(_, c)| *c).max()?;
    scored.into_iter().filter(|(_, c)| *c == top).min_by_key(|(g, _)| !g.verified)
}

#[cfg(test)]
mod tests {
    use super::super::net::fake::Fake;
    use super::*;

    const SEARCH: &str = include_str!("fixtures/sgdb_search.json");
    const GRIDS: &str = include_str!("fixtures/sgdb_grids.json");

    #[test]
    fn search_parses_games_and_sends_the_bearer_key() {
        let fake = Fake::new().on_json("/search/autocomplete/", 200, SEARCH);
        let games = SteamGridDb::new(&fake, "TESTKEY-0000").search("Ship of Harkinian").unwrap();
        assert_eq!(games.len(), 3);
        assert_eq!(games[0], Game { id: 900001, name: "Ship of Harkinian".into(), verified: true });
        assert_eq!(fake.header_of("/search/autocomplete/Ship%20of%20Harkinian", "Authorization").as_deref(), Some("Bearer TESTKEY-0000"));
        assert!(fake.requests()[0].starts_with("https://www.steamgriddb.com/api/v2/search/autocomplete/"));
    }

    #[test]
    fn assets_ask_for_static_safe_art_in_the_slot_shape_and_sort_by_score() {
        let fake = Fake::new().on_json("/grids/game/900001", 200, GRIDS);
        let sgdb = SteamGridDb::new(&fake, "k-123456789");
        let assets = sgdb.assets(900001, filter_for(Slot::Portrait)).unwrap();
        let url = &fake.requests()[0];
        for part in ["types=static", "nsfw=false", "humor=false", "epilepsy=false", "dimensions=600x900"] {
            assert!(url.contains(part), "{url} lacks {part}");
        }
        // The fixture lists a 3-score item first; the 9-score one wins, and the off-host image is dropped.
        assert_eq!(assets.iter().map(|a| a.id).collect::<Vec<_>>(), vec![801, 802]);
        assert_eq!(assets[0].style, "alternate");
    }

    #[test]
    fn every_slot_has_its_own_filter() {
        assert_eq!(filter_for(Slot::Wide).dimensions, Some("920x430"));
        assert_eq!(filter_for(Slot::Hero).dimensions, Some("1920x620,3840x1240"));
        assert_eq!(filter_for(Slot::Hero).kind, Kind::Hero);
        assert_eq!(filter_for(Slot::Logo).kind, Kind::Logo);
        assert_eq!(filter_for(Slot::Icon).mimes, Some("image/png"));
        let fake = Fake::new().on_json("/heroes/game/", 200, r#"{"success":true,"data":[]}"#);
        SteamGridDb::new(&fake, "k-123456789").assets(1, filter_for(Slot::Hero)).unwrap();
        assert!(fake.requests()[0].contains("dimensions=1920x620%2C3840x1240"));
    }

    #[test]
    fn status_codes_map_to_typed_errors() {
        let sgdb_with = |status: u16| {
            let fake = Fake::new().on_json("/search/", status, "{}");
            SteamGridDb::new(&fake, "k-123456789").search("x")
        };
        assert!(matches!(sgdb_with(401), Err(SourceError::Invalid(_))));
        assert!(matches!(sgdb_with(403), Err(SourceError::Invalid(_))));
        assert_eq!(sgdb_with(404), Err(SourceError::NotFound));
        assert!(matches!(sgdb_with(429), Err(SourceError::Limited(_))));
        assert!(matches!(sgdb_with(500), Err(SourceError::BadResponse(_))));
        assert!(matches!(sgdb_with(430), Err(SourceError::BadResponse(_))));
    }

    #[test]
    fn malformed_and_unsuccessful_answers_are_errors_not_panics() {
        let run = |body: &str| {
            let fake = Fake::new().on_json("/search/", 200, body);
            SteamGridDb::new(&fake, "k-123456789").search("x")
        };
        assert!(matches!(run("<html>oops</html>"), Err(SourceError::BadResponse(_))));
        assert!(matches!(run(""), Err(SourceError::BadResponse(_))));
        assert!(matches!(run(r#"{"success":true}"#), Err(SourceError::BadResponse(_))));
        assert_eq!(run(r#"{"success":false,"errors":["Game not found"]}"#), Err(SourceError::NotFound));
        assert!(matches!(run(r#"{"success":false,"errors":["Something else"]}"#), Err(SourceError::BadResponse(_))));
        // Entries missing fields are skipped, not fatal.
        assert_eq!(run(r#"{"success":true,"data":[{"name":"no id"},{"id":5,"name":"ok"}]}"#).unwrap().len(), 1);
    }

    #[test]
    fn offline_is_offline() {
        let fake = Fake::new().offline_on("/search/");
        assert!(matches!(SteamGridDb::new(&fake, "k-123456789").search("x"), Err(SourceError::Offline(_))));
    }

    #[test]
    fn the_key_check_reports_ok_invalid_offline_and_limits() {
        let check = |status: u16| {
            let fake = Fake::new().on_json("/search/", status, r#"{"success":true,"data":[]}"#);
            SteamGridDb::new(&fake, "k-123456789").check_key()
        };
        assert_eq!(check(200), KeyCheck::Ok(String::new()));
        assert!(matches!(check(401), KeyCheck::Invalid(_)));
        assert!(matches!(check(429), KeyCheck::Unknown(_)));
        assert!(matches!(check(503), KeyCheck::Unknown(_)));
        let fake = Fake::new();
        assert!(matches!(SteamGridDb::new(&fake, "k-123456789").check_key(), KeyCheck::Offline(_)));
        assert_eq!(fake.requests().len(), 1, "one cheap call, nothing more");
    }

    #[test]
    fn downloads_are_limited_to_the_sources_own_hosts() {
        let fake = Fake::new().on("steamgriddb.com", 200, b"PNGDATA".to_vec());
        let sgdb = SteamGridDb::new(&fake, "k-123456789");
        assert_eq!(sgdb.download("https://cdn2.steamgriddb.com/grid/a.png").unwrap(), b"PNGDATA");
        assert!(matches!(sgdb.download("https://evil.example/a.png"), Err(SourceError::BadResponse(_))));
        assert!(matches!(sgdb.download("http://cdn2.steamgriddb.com/a.png"), Err(SourceError::BadResponse(_))));
        assert_eq!(fake.requests().len(), 1);
        let gone = Fake::new().on("steamgriddb.com", 404, Vec::new());
        assert_eq!(SteamGridDb::new(&gone, "k-123456789").download("https://cdn2.steamgriddb.com/x.png"), Err(SourceError::NotFound));
    }

    #[test]
    fn best_match_prefers_the_same_title_then_a_verified_one() {
        let games = vec![
            Game { id: 1, name: "Ship of Harkinian: Remastered".into(), verified: true },
            Game { id: 2, name: "Ship of Harkinian".into(), verified: false },
            Game { id: 3, name: "Ship of Harkinian (Unofficial)".into(), verified: true },
        ];
        let (game, conf) = best_match("Ship of Harkinian", &games).unwrap();
        assert_eq!((game.id, conf), (3, Confidence::High));
        let (game, conf) = best_match("Ship of Harkinian Remastered Edition", &games).unwrap();
        assert_eq!(conf, Confidence::Medium);
        assert!(game.id == 1 || game.id == 2 || game.id == 3);
        let (_, conf) = best_match("Doom", &games).unwrap();
        assert_eq!(conf, Confidence::Low);
        assert!(best_match("Doom", &[]).is_none());
    }

    #[test]
    fn no_key_text_reaches_an_error_message() {
        let key = "VERYSECRETKEY-98765";
        for status in [401u16, 403, 404, 429, 500] {
            let fake = Fake::new().on_json("/search/", status, "{}");
            let err = SteamGridDb::new(&fake, key).search("x").unwrap_err();
            assert!(!format!("{err} {err:?}").contains(key));
        }
        let fake = Fake::new();
        let err = SteamGridDb::new(&fake, key).search("x").unwrap_err();
        assert!(!format!("{err} {err:?}").contains(key));
    }
}
