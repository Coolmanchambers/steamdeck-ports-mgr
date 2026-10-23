//! Looking at a GitHub project before it is added: does it exist, what is it, and does its newest release
//! have a file for Linux? At most two calls to GitHub's API per check (the project, then its releases), through
//! the app's one HTTP door ([`Fetch`]), so tests plug in a fake and nothing here needs the network. Nothing is
//! downloaded and nothing is written.
//!
//! "Linux file" is judged here, not by the installer's own picker, so the answer does not depend on which
//! system the app runs on: an `.AppImage` or an archive (`.tar.gz`, `.zip`, ...) that does not say it is for
//! Windows, macOS, Android or an ARM chip, or any file that says Linux.

use super::parse::Repo;
use crate::core::asset_select::{self, is_draft};
use crate::fork::art::net::{Fetch, NetError, MAX_JSON_BYTES};
use serde_json::Value;

const API: &str = "https://api.github.com/repos";
const HEADERS: [(&str, &str); 1] = [("Accept", "application/vnd.github+json")];
/// How many of the newest releases are looked at (the stable one is chosen from them).
const RELEASES: usize = 5;

/// What a check found out about a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    /// The project as GitHub names it (case and renames settled).
    pub repo: Repo,
    pub name: String,
    pub description: String,
    pub version: String,
    /// `2026-10-17`, when GitHub says when the release was published.
    pub date: Option<String>,
    /// The file the newest release offers for Linux (the best one when there are several).
    pub linux_file: String,
}

/// Why a project could not be taken, each with the sentence the screen shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckError {
    /// No answer from GitHub at all: no network.
    Offline,
    /// GitHub has no such project (or it is private).
    NotFound(String),
    /// GitHub says too many questions (60 an hour without a login).
    RateLimited,
    /// Any other answer GitHub gave (the status code).
    Server(u16),
    /// The project exists but has no published release.
    NoReleases(String),
    /// The newest release (its version) has no file for Linux.
    NoLinuxFile(String),
    /// GitHub answered with something unreadable.
    Unreadable,
}

impl CheckError {
    pub fn text(&self) -> String {
        match self {
            CheckError::Offline => "Could not reach GitHub. Check the internet connection and try again.".to_string(),
            CheckError::NotFound(slug) => format!("GitHub has no project called {slug}. Check the link for typos; a private project can't be used."),
            CheckError::RateLimited => "GitHub is asking to wait a while (too many checks from this connection). Try again in about an hour.".to_string(),
            CheckError::Server(code) => format!("GitHub answered with an error (code {code}). Try again later."),
            CheckError::NoReleases(slug) => format!("{slug} has no releases, so there is nothing to download. Look for a project page with a Releases section that has files."),
            CheckError::NoLinuxFile(version) => format!("The newest release ({version}) has no file for Linux. Look for a download ending in .AppImage, .tar.gz or .zip that says Linux."),
            CheckError::Unreadable => "GitHub's answer could not be read. Try again later.".to_string(),
        }
    }
}

fn get(fetch: &dyn Fetch, url: &str) -> Result<Value, CheckError> {
    let response = fetch.get(url, &HEADERS, MAX_JSON_BYTES).map_err(|e| match e {
        NetError::Offline(_) | NetError::Refused(_) => CheckError::Offline,
        NetError::TooLarge => CheckError::Unreadable,
    })?;
    match response.status {
        200..=299 => serde_json::from_slice(&response.body).map_err(|_| CheckError::Unreadable),
        // The parts of the address the caller names (owner/project) say which project was meant.
        404 | 410 => Err(CheckError::NotFound(String::new())),
        403 | 429 => Err(CheckError::RateLimited),
        code => Err(CheckError::Server(code)),
    }
}

fn not_found(e: CheckError, repo: &Repo) -> CheckError {
    match e {
        CheckError::NotFound(_) => CheckError::NotFound(repo.slug()),
        other => other,
    }
}

const FOREIGN: [&str; 14] = ["win", "win32", "win64", "windows", "mac", "macos", "osx", "darwin", "android", "ios", "arm64", "aarch64", "armv7", "armhf"];
const ARCHIVES: [&str; 9] = [".appimage", ".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.zst", ".tar.bz2", ".zip", ".7z"];
const NOT_PROGRAMS: [&str; 12] = [".txt", ".md", ".sha256", ".sha1", ".sha512", ".md5", ".sig", ".asc", ".json", ".yml", ".zsync", ".pdf"];

/// Whether a release file looks like one for a Linux PC (x86-64: the Deck's chip).
pub fn is_linux_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    if NOT_PROGRAMS.iter().any(|e| lower.ends_with(e)) || [".exe", ".msi", ".dmg", ".pkg", ".deb", ".rpm", ".apk"].iter().any(|e| lower.ends_with(e)) {
        return false;
    }
    let squashed: String = lower.chars().filter(|c| c.is_alphanumeric()).collect();
    if squashed.contains("sourcecode") {
        return false;
    }
    let tokens: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).collect();
    if tokens.iter().any(|t| FOREIGN.contains(t)) {
        return false;
    }
    ARCHIVES.iter().any(|e| lower.ends_with(e)) || tokens.contains(&"linux")
}

/// How good a Linux file looks: 0 an AppImage, 1 an archive that says Linux, 2 any other.
fn rank(name: &str) -> u8 {
    let lower = name.to_lowercase();
    if lower.ends_with(".appimage") {
        0
    } else if lower.contains("linux") {
        1
    } else {
        2
    }
}

/// The best Linux file of a release, if it has one.
pub fn best_linux_file(release: &Value) -> Option<String> {
    let assets = release.get("assets").and_then(Value::as_array)?;
    let mut names: Vec<&str> = assets.iter().filter_map(|a| a.get("name").and_then(Value::as_str)).filter(|n| is_linux_file(n)).collect();
    names.sort_by_key(|n| (rank(n), n.len()));
    names.first().map(|n| n.to_string())
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string()
}

/// Asks GitHub about `repo`: the project, then its newest release.
pub fn check(fetch: &dyn Fetch, repo: &Repo) -> Result<Checked, CheckError> {
    let project = get(fetch, &format!("{API}/{}/{}", repo.owner, repo.name)).map_err(|e| not_found(e, repo))?;
    // GitHub follows renames and case: its own name for the project is the one to keep.
    let named = text(&project, "full_name");
    let settled = super::parse::parse_link(&named).unwrap_or_else(|_| repo.clone());
    let name = Some(text(&project, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| settled.name.clone());
    let description = text(&project, "description");

    let releases = get(fetch, &format!("{API}/{}/{}/releases?per_page={RELEASES}", settled.owner, settled.name)).map_err(|e| not_found(e, repo))?;
    let releases: Vec<Value> = releases.as_array().cloned().unwrap_or_default().into_iter().filter(|r| !is_draft(r)).collect();
    let Some(release) = asset_select::newest_release(releases) else { return Err(CheckError::NoReleases(settled.slug())) };
    let version = Some(text(&release, "tag_name")).filter(|t| !t.is_empty()).or_else(|| Some(text(&release, "name")).filter(|t| !t.is_empty())).unwrap_or_else(|| "the newest release".to_string());
    let Some(linux_file) = best_linux_file(&release) else { return Err(CheckError::NoLinuxFile(version)) };
    let date = Some(text(&release, "published_at")).filter(|d| d.len() >= 10).map(|d| d[..10].to_string());
    Ok(Checked { repo: settled, name, description, version, date, linux_file })
}


/// Answers from saved files instead of GitHub: the test switch `DPM_GITHUB_REPLAY=<folder>` (like the
/// updater's `DPM_RELEASE_BASE`), so a walk of the screens under a virtual display can show every answer
/// GitHub can give without the network. `<folder>/<owner>/<project>.project` answers the project call and
/// `.releases` the releases call. A file starts with a line `<status> [<delay in ms>]` (status 0 = no
/// answer at all), then the body. A missing file is a 404.
pub struct ReplayFetch {
    pub dir: std::path::PathBuf,
}

impl ReplayFetch {
    /// The replay folder named by the test switch, if it is set.
    pub fn from_env() -> Option<ReplayFetch> {
        std::env::var_os("DPM_GITHUB_REPLAY").map(|d| ReplayFetch { dir: d.into() }).filter(|r| r.dir.is_dir())
    }
}

impl Fetch for ReplayFetch {
    fn get(&self, url: &str, _headers: &[(&str, &str)], _max: u64) -> Result<crate::fork::art::net::Response, NetError> {
        let rest = url.strip_prefix(API).and_then(|r| r.strip_prefix('/')).unwrap_or_default();
        let mut parts = rest.split('/');
        let (owner, name) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
        let kind = if parts.next().is_some() { "releases" } else { "project" };
        let safe = |s: &str| !s.is_empty() && s != ".." && !s.contains(['/', '\\']);
        let answer = |status: u16, body: Vec<u8>| crate::fork::art::net::Response { status, body, final_url: url.to_string() };
        if !safe(owner) || !safe(name) {
            return Ok(answer(404, Vec::new()));
        }
        let Ok(text) = std::fs::read(self.dir.join(owner).join(format!("{name}.{kind}"))) else { return Ok(answer(404, Vec::new())) };
        let split = text.iter().position(|b| *b == b'\n').unwrap_or(text.len());
        let head = String::from_utf8_lossy(&text[..split]).to_string();
        let mut fields = head.split_whitespace();
        let status: u16 = fields.next().and_then(|s| s.parse().ok()).unwrap_or(404);
        if let Some(delay) = fields.next().and_then(|s| s.parse::<u64>().ok()) {
            std::thread::sleep(std::time::Duration::from_millis(delay));
        }
        if status == 0 {
            return Err(NetError::Offline("replayed".to_string()));
        }
        Ok(answer(status, text.get(split + 1..).unwrap_or_default().to_vec()))
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! A GitHub that answers from a list, for tests of the checks and the screens that use them.
    use super::*;
    use crate::fork::art::net::Response;
    use std::cell::RefCell;

    pub struct FakeGithub {
        pub answers: Vec<(String, Result<(u16, String), NetError>)>,
        pub asked: RefCell<Vec<String>>,
    }

    impl FakeGithub {
        pub fn new(answers: Vec<(&str, Result<(u16, String), NetError>)>) -> FakeGithub {
            FakeGithub { answers: answers.into_iter().map(|(u, a)| (u.to_string(), a)).collect(), asked: RefCell::new(Vec::new()) }
        }
    }

    impl Fetch for FakeGithub {
        fn get(&self, url: &str, _headers: &[(&str, &str)], _max: u64) -> Result<Response, NetError> {
            self.asked.borrow_mut().push(url.to_string());
            match self.answers.iter().find(|(u, _)| url.contains(u.as_str())) {
                Some((_, Ok((status, body)))) => Ok(Response { status: *status, body: body.clone().into_bytes(), final_url: url.to_string() }),
                Some((_, Err(e))) => Err(e.clone()),
                None => Ok(Response { status: 404, body: Vec::new(), final_url: url.to_string() }),
            }
        }
    }

    pub fn ok(body: &str) -> Result<(u16, String), NetError> {
        Ok((200, body.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::fake::*;
    use super::*;
    use crate::fork::custom_repo::parse::parse_link;
    use serde_json::json;

    fn repo() -> Repo {
        parse_link("github.com/HarbourMasters/Shipwright").unwrap()
    }

    fn project() -> String {
        json!({"full_name": "HarbourMasters/Shipwright", "name": "Shipwright", "description": "  Ocarina of Time, ported.  "}).to_string()
    }

    fn release(tag: &str, prerelease: bool, files: &[&str]) -> Value {
        json!({"tag_name": tag, "prerelease": prerelease, "draft": false, "published_at": "2026-10-17T01:16:33Z", "assets": files.iter().map(|f| json!({"name": f})).collect::<Vec<_>>()})
    }

    fn releases(list: Vec<Value>) -> String {
        Value::Array(list).to_string()
    }

    fn github(project_answer: Result<(u16, String), NetError>, releases_answer: Result<(u16, String), NetError>) -> FakeGithub {
        FakeGithub::new(vec![("/releases?", releases_answer), ("/repos/HarbourMasters/Shipwright", project_answer)])
    }

    #[test]
    fn a_project_with_a_linux_release_is_described_and_costs_two_calls() {
        let g = github(ok(&project()), ok(&releases(vec![release("9.1.0", false, &["Shipwright-win.zip", "SoH-Linux.AppImage", "SoH-Linux.AppImage.zsync", "Shipwright-mac.zip"])])));
        let c = check(&g, &repo()).unwrap();
        assert_eq!(c.repo.slug(), "HarbourMasters/Shipwright");
        assert_eq!((c.name.as_str(), c.description.as_str(), c.version.as_str()), ("Shipwright", "Ocarina of Time, ported.", "9.1.0"));
        assert_eq!(c.date.as_deref(), Some("2026-10-17"));
        assert_eq!(c.linux_file, "SoH-Linux.AppImage");
        let asked = g.asked.borrow();
        assert_eq!(asked.len(), 2, "{asked:?}");
        assert_eq!(asked[0], "https://api.github.com/repos/HarbourMasters/Shipwright");
        assert!(asked[1].starts_with("https://api.github.com/repos/HarbourMasters/Shipwright/releases?per_page="));
    }

    #[test]
    fn github_renames_and_case_are_followed() {
        let renamed = json!({"full_name": "NewOwner/ShipwrightRenamed", "name": "ShipwrightRenamed", "description": null}).to_string();
        let g = FakeGithub::new(vec![("/repos/NewOwner/ShipwrightRenamed/releases", ok(&releases(vec![release("1", false, &["a-linux.tar.gz"])]))), ("/repos/HarbourMasters/Shipwright", ok(&renamed))]);
        let c = check(&g, &repo()).unwrap();
        assert_eq!(c.repo.slug(), "NewOwner/ShipwrightRenamed");
        assert_eq!(c.description, "");
    }

    #[test]
    fn the_newest_stable_release_is_the_one_checked_not_a_beta_and_drafts_are_ignored() {
        let mut draft = release("10.0.0-draft", false, &["x-linux.AppImage"]);
        draft["draft"] = json!(true);
        let g = github(ok(&project()), ok(&releases(vec![draft, release("9.2.0-beta", true, &["b-linux.AppImage"]), release("9.1.0", false, &["a-linux.AppImage"])])));
        let c = check(&g, &repo()).unwrap();
        assert_eq!((c.version.as_str(), c.linux_file.as_str()), ("9.1.0", "a-linux.AppImage"));
    }

    #[test]
    fn what_counts_as_a_linux_file() {
        for yes in ["Game-Linux-x86_64.AppImage", "game.tar.gz", "game-linux.zip", "Dusklight-v2.0.3-linux-x86_64.AppImage", "Severed_Chains_Steam_Deck.tar.gz", "game.bin.linux"] {
            assert!(is_linux_file(yes), "{yes}");
        }
        for no in ["game-win64.zip", "game-windows.7z", "Game.exe", "game-macos.tar.gz", "game-darwin.zip", "game-osx.zip", "game.dmg", "game.deb", "game.apk", "Game-Source code.zip", "game.tar.gz.sha256", "game.AppImage.zsync", "readme.txt", "game-linux-arm64.tar.gz", "game-aarch64.AppImage", "notes"] {
            assert!(!is_linux_file(no), "{no}");
        }
        assert!(is_linux_file("winter-linux.AppImage"), "winter is not win");
    }

    #[test]
    fn an_appimage_is_preferred_over_an_archive_over_other_files() {
        let r = release("1", false, &["g.zip", "g-linux.tar.gz", "g-x86_64.AppImage"]);
        assert_eq!(best_linux_file(&r).as_deref(), Some("g-x86_64.AppImage"));
        let r = release("1", false, &["g.zip", "g-linux.tar.gz"]);
        assert_eq!(best_linux_file(&r).as_deref(), Some("g-linux.tar.gz"));
        assert_eq!(best_linux_file(&release("1", false, &["g-win.zip"])), None);
        assert_eq!(best_linux_file(&json!({"tag_name": "1"})), None, "a release without assets");
    }

    #[test]
    fn every_failure_is_a_plain_sentence() {
        let none = github(Ok((404, String::new())), ok("[]"));
        assert_eq!(check(&none, &repo()), Err(CheckError::NotFound("HarbourMasters/Shipwright".into())));
        assert!(CheckError::NotFound("a/b".into()).text().starts_with("GitHub has no project called a/b."));

        let off = github(Err(NetError::Offline("could not connect".into())), ok("[]"));
        assert_eq!(check(&off, &repo()), Err(CheckError::Offline));
        assert!(CheckError::Offline.text().contains("internet connection"));

        for status in [403, 429] {
            let limited = github(Ok((status, String::new())), ok("[]"));
            assert_eq!(check(&limited, &repo()), Err(CheckError::RateLimited), "{status}");
        }
        assert!(CheckError::RateLimited.text().contains("about an hour"));

        let down = github(Ok((502, String::new())), ok("[]"));
        assert_eq!(check(&down, &repo()), Err(CheckError::Server(502)));
        assert!(CheckError::Server(502).text().contains("code 502"));

        let empty = github(ok(&project()), ok("[]"));
        assert_eq!(check(&empty, &repo()), Err(CheckError::NoReleases("HarbourMasters/Shipwright".into())));
        let only_drafts = {
            let mut d = release("1", false, &["a-linux.AppImage"]);
            d["draft"] = json!(true);
            github(ok(&project()), ok(&releases(vec![d])))
        };
        assert!(matches!(check(&only_drafts, &repo()), Err(CheckError::NoReleases(_))));
        assert!(CheckError::NoReleases("a/b".into()).text().contains("no releases"));

        let windows_only = github(ok(&project()), ok(&releases(vec![release("2.0", false, &["g-win.zip", "g-mac.zip"])])));
        assert_eq!(check(&windows_only, &repo()), Err(CheckError::NoLinuxFile("2.0".into())));
        assert!(CheckError::NoLinuxFile("2.0".into()).text().contains("newest release (2.0)"));

        let garbage = github(ok("not json"), ok("[]"));
        assert_eq!(check(&garbage, &repo()), Err(CheckError::Unreadable));
        let garbage_releases = github(ok(&project()), ok("{\"message\": \"x\"}"));
        assert!(matches!(check(&garbage_releases, &repo()), Err(CheckError::NoReleases(_))), "an object where a list belongs is no release");
        assert!(CheckError::Unreadable.text().contains("could not be read"));
    }

    #[test]
    fn a_release_missing_its_project_is_still_not_found() {
        let g = github(ok(&project()), Ok((404, String::new())));
        assert_eq!(check(&g, &repo()), Err(CheckError::NotFound("HarbourMasters/Shipwright".into())));
    }

    #[test]
    fn no_error_text_names_a_file_or_shows_code() {
        for e in [CheckError::Offline, CheckError::NotFound("a/b".into()), CheckError::RateLimited, CheckError::Server(500), CheckError::NoReleases("a/b".into()), CheckError::NoLinuxFile("1".into()), CheckError::Unreadable] {
            let t = e.text().to_lowercase();
            assert!(!t.contains("json") && !t.contains("err(") && !t.contains("::") && !t.contains("api.github"), "{t}");
        }
    }

    #[test]
    fn the_replay_switch_answers_from_files_with_a_status_and_a_missing_file_is_a_404() {
        let dir = std::env::temp_dir().join(format!("dpm_replay_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("o")).unwrap();
        std::fs::write(dir.join("o/p.project"), format!("200\n{}", json!({"full_name": "o/p", "name": "P", "description": "d"}))).unwrap();
        std::fs::write(dir.join("o/p.releases"), format!("200\n{}", releases(vec![release("1.0", false, &["p-linux.AppImage"])]))).unwrap();
        std::fs::write(dir.join("o/q.project"), "0\n").unwrap();
        std::fs::write(dir.join("o/r.project"), "429\n").unwrap();
        let replay = ReplayFetch { dir: dir.clone() };
        let c = check(&replay, &parse_link("github.com/o/p").unwrap()).unwrap();
        assert_eq!((c.name.as_str(), c.version.as_str(), c.linux_file.as_str()), ("P", "1.0", "p-linux.AppImage"));
        assert_eq!(check(&replay, &parse_link("github.com/o/q").unwrap()), Err(CheckError::Offline));
        assert_eq!(check(&replay, &parse_link("github.com/o/r").unwrap()), Err(CheckError::RateLimited));
        assert_eq!(check(&replay, &parse_link("github.com/o/missing").unwrap()), Err(CheckError::NotFound("o/missing".into())));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Talks to the real GitHub; run by hand: `cargo test live_github -- --ignored`.
    #[test]
    #[ignore]
    fn live_github_describes_a_real_project() {
        let c = check(&crate::fork::art::net::Net::new(), &parse_link("github.com/HarbourMasters/Shipwright").unwrap()).expect("a real project with a Linux release");
        assert!(!c.linux_file.is_empty() && !c.version.is_empty(), "{c:?}");
        let missing = check(&crate::fork::art::net::Net::new(), &parse_link("github.com/this-owner-does-not-exist-12345/nope").unwrap());
        assert!(matches!(missing, Err(CheckError::NotFound(_))), "{missing:?}");
    }
}
