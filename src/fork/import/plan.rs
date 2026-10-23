//! What importing a port means, spelled out before anything happens: where it goes, what is checked,
//! what is copied, which Steam shortcut changes and what is left behind. The dry run shows exactly this;
//! the real engine does exactly this.

use super::scan::{Found, Size, SteamRef};
use crate::fork::rom_screens::util::human_bytes;
use crate::fork::screens::short_path;
use crate::fork::store::SteamProcessCheck;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// A copy needs this much free space beyond the port's own size, like the game-files copy does.
pub const SPACE_MARGIN: u64 = 1_000_000_000;

/// When a Steam shortcut change can be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteamWrite {
    /// Steam is closed (Desktop Mode): the shortcut is repointed during the import.
    Now,
    /// Steam is running (always in Game Mode): the change is queued and applied in Desktop Mode.
    Queued,
}

/// `Queued` unless Steam is provably closed outside Game Mode. A failed check also queues: nothing is
/// written to a file Steam may be holding.
pub fn steam_write_mode(steam: &dyn SteamProcessCheck, game_mode: bool) -> SteamWrite {
    if game_mode {
        return SteamWrite::Queued;
    }
    match steam.running_steam_processes() {
        Ok(p) if p.is_empty() => SteamWrite::Now,
        _ => SteamWrite::Queued,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveRequest {
    pub port_key: String,
    pub port_name: String,
    pub catalog_folder: String,
    pub from: PathBuf,
    pub to: PathBuf,
    pub size: Size,
    pub steam: Vec<SteamRef>,
}

/// One request per found port, each going to `<ports folder>/<catalog folder>`.
pub fn build_requests(found: &[&Found], ports_folder: &Path) -> Vec<MoveRequest> {
    found
        .iter()
        .map(|f| MoveRequest {
            port_key: f.port_key.clone(),
            port_name: f.port_name.clone(),
            catalog_folder: f.catalog_folder.clone(),
            from: f.path.clone(),
            to: ports_folder.join(&f.catalog_folder),
            size: f.size,
            steam: f.steam.clone(),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    BadFolderName(String),
    DestinationExists(PathBuf),
    SourceMissing(PathBuf),
    /// The old folder holds the ports folder or the other way round.
    Nested,
    NotEnoughSpace { need: u64, free: u64 },
}

impl Block {
    pub fn text(&self, home: &Path) -> String {
        match self {
            Block::BadFolderName(name) => format!("the catalog folder name \"{name}\" is not usable"),
            Block::DestinationExists(p) => format!("{} already exists, so nothing is copied over it", short_path(p, home)),
            Block::SourceMissing(p) => format!("{} is not there any more", short_path(p, home)),
            Block::Nested => "the old folder and the ports folder contain each other".to_string(),
            Block::NotEnoughSpace { need, free } => {
                format!("not enough free space: needs {} plus {} spare, {} free", human_bytes(*need), human_bytes(SPACE_MARGIN), human_bytes(*free))
            }
        }
    }

    pub fn is_space(&self) -> bool {
        matches!(self, Block::NotEnoughSpace { .. })
    }
}

/// One Steam shortcut and where it will point afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamStep {
    pub app_id: Option<u32>,
    pub name: String,
    pub old_exe: String,
    pub new_exe: String,
    pub old_start_dir: String,
    pub new_start_dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortPlan {
    pub request: MoveRequest,
    pub block: Option<Block>,
    pub steam: Vec<SteamStep>,
    pub steam_write: SteamWrite,
}

/// `path` re-rooted from the old install folder to the new one; unchanged when it is not inside it.
fn reroot(path: &str, from: &Path, to: &Path) -> String {
    if path.is_empty() {
        return String::new();
    }
    let real = fs::canonicalize(from).ok();
    let p = Path::new(path);
    let rel = p.strip_prefix(from).ok().or_else(|| real.as_deref().and_then(|r| p.strip_prefix(r).ok()));
    match rel {
        Some(rel) if rel.as_os_str().is_empty() => to.to_string_lossy().into_owned(),
        Some(rel) => to.join(rel).to_string_lossy().into_owned(),
        None => path.to_string(),
    }
}

fn single_component(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    matches!((parts.next(), parts.next()), (Some(Component::Normal(_)), None))
}

fn nested(a: &Path, b: &Path) -> bool {
    let a = fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let b = fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    a.starts_with(&b) || b.starts_with(&a)
}

/// Checks one request against the disk. `free` is the free space left for the ports folder (`None` when
/// unknown, which never blocks). Reads only.
pub fn plan_port(request: &MoveRequest, free: Option<u64>, steam_write: SteamWrite) -> PortPlan {
    let block = if !single_component(&request.catalog_folder) {
        Some(Block::BadFolderName(request.catalog_folder.clone()))
    } else if !request.from.is_dir() {
        Some(Block::SourceMissing(request.from.clone()))
    } else if fs::symlink_metadata(&request.to).is_ok() {
        Some(Block::DestinationExists(request.to.clone()))
    } else if request.to.parent().is_some_and(|ports| nested(&request.from, ports)) {
        Some(Block::Nested)
    } else {
        free.filter(|&f| f < request.size.bytes.saturating_add(SPACE_MARGIN)).map(|free| Block::NotEnoughSpace { need: request.size.bytes, free })
    };
    let steam = request
        .steam
        .iter()
        .map(|s| SteamStep {
            app_id: s.app_id,
            name: s.name.clone(),
            old_exe: s.exe.clone(),
            new_exe: reroot(&s.exe, &request.from, &request.to),
            old_start_dir: s.start_dir.clone(),
            new_start_dir: reroot(&s.start_dir, &request.from, &request.to),
        })
        .collect();
    PortPlan { request: request.clone(), block, steam, steam_write }
}

/// Plans a whole batch: each port that will be copied uses up space the next one cannot count on.
pub fn plan_all(requests: &[MoveRequest], free: Option<u64>, steam_write: SteamWrite) -> Vec<PortPlan> {
    let mut left = free;
    requests
        .iter()
        .map(|r| {
            let plan = plan_port(r, left, steam_write);
            if plan.block.is_none() {
                left = left.map(|l| l.saturating_sub(r.size.bytes));
            }
            plan
        })
        .collect()
}

fn size_text(size: Size) -> String {
    let at_least = if size.complete { "" } else { "at least " };
    format!("{at_least}{}", human_bytes(size.bytes))
}

fn files_text(size: Size) -> String {
    format!("{}{} file{}", if size.complete { "" } else { "at least " }, size.files, if size.files == 1 { "" } else { "s" })
}

impl PortPlan {
    /// The steps in order, in plain words, with paths shortened against `home`.
    pub fn steps(&self, home: &Path) -> Vec<String> {
        let r = &self.request;
        if let Some(block) = &self.block {
            return vec![format!("Nothing is done: {}.", block.text(home))];
        }
        let mut steps = vec![
            format!("Check the game files {} needs in {}; stop and ask if one is missing or broken.", r.port_name, short_path(&r.from, home)),
            format!("Copy {} ({}) to {}.", files_text(r.size), size_text(r.size), short_path(&r.to, home)),
            "Check the copy: every file is there with the same size and content, and the game files again.".to_string(),
            "Switch the app over to the new folder.".to_string(),
        ];
        if self.steam.is_empty() {
            steps.push("No Steam shortcut points at it.".to_string());
        }
        for s in &self.steam {
            let id = if s.app_id.is_some() { "keeps its app ID" } else { "no stored app ID yet" };
            let what = if s.old_exe == s.new_exe { format!("start folder to {}", short_path(Path::new(&s.new_start_dir), home)) } else { format!("program to {}", short_path(Path::new(&s.new_exe), home)) };
            steps.push(match self.steam_write {
                SteamWrite::Now => format!("Repoint Steam shortcut \"{}\" ({id}) {what}.", s.name),
                SteamWrite::Queued => format!("Steam is running: queue the change of Steam shortcut \"{}\" ({id}) {what} for Desktop Mode.", s.name),
            });
        }
        steps.push(format!("Keep the old folder {}; you are asked afterwards whether to remove it. Nothing is deleted automatically.", short_path(&r.from, home)));
        steps
    }

    pub fn can_run(&self) -> bool {
        self.block.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::super::scan::fixtures::*;
    use super::super::scan::{run_scan, ScanInput};
    use super::*;
    use crate::fork::import::folders::ScanFolders;
    use crate::fork::settings::ForkSettings;

    fn requests(h: &Home) -> Vec<MoveRequest> {
        let input = ScanInput {
            roots: ScanFolders::default().roots(&h.home, &h.library),
            catalog: catalog(),
            library_dir: h.library.clone(),
            deeper: false,
            shortcuts: super::super::scan::read_shortcuts(&h.home, &ForkSettings::load(&h.home.join("s.json"))),
        };
        let scan = run_scan(&input, &|| false, &|_| {});
        let refs: Vec<&Found> = scan.found.iter().collect();
        build_requests(&refs, &h.library)
    }

    #[test]
    fn a_request_goes_to_the_catalog_folder_inside_the_ports_folder() {
        let h = fake_home("plan_dest");
        let reqs = requests(&h);
        let soh = reqs.iter().find(|r| r.port_name == "Ship of Harkinian").unwrap();
        assert_eq!(soh.to, h.library.join("SoH"));
        assert_eq!(soh.from, h.home.join("Applications/SoH"));
    }

    #[test]
    fn the_plan_spells_out_check_copy_verify_switch_steam_and_keep() {
        let h = fake_home("plan_steps");
        let reqs = requests(&h);
        let soh = reqs.iter().find(|r| r.port_name == "Ship of Harkinian").unwrap();
        let plan = plan_port(soh, Some(500_000_000_000), SteamWrite::Now);
        assert!(plan.block.is_none() && plan.can_run());
        let steps = plan.steps(&h.home);
        assert_eq!(steps.len(), 6, "{steps:#?}");
        assert!(steps[0].starts_with("Check the game files Ship of Harkinian needs in ~/Applications/SoH"));
        assert_eq!(steps[1], "Copy 2 files (8 KB) to ~/DeckPorts/Library/SoH.");
        assert!(steps[2].starts_with("Check the copy"));
        assert!(steps[3].starts_with("Switch the app over"));
        assert!(steps[4].starts_with("Repoint Steam shortcut \"Ship of Harkinian\" (keeps its app ID) program to ~/DeckPorts/Library/SoH/soh.appimage"), "{}", steps[4]);
        assert!(steps[5].contains("Keep the old folder ~/Applications/SoH") && steps[5].contains("Nothing is deleted automatically"));
    }

    #[test]
    fn the_steam_step_keeps_the_app_id_and_rewrites_both_paths_under_the_new_folder() {
        let h = fake_home("plan_steam");
        let reqs = requests(&h);
        let soh = reqs.iter().find(|r| r.port_name == "Ship of Harkinian").unwrap();
        let plan = plan_port(soh, None, SteamWrite::Now);
        assert_eq!(plan.steam.len(), 1);
        let s = &plan.steam[0];
        assert_eq!(s.app_id, Some(0xAAAA_0001));
        assert_eq!(Path::new(&s.new_exe), h.library.join("SoH/soh.appimage"));
        assert_eq!(Path::new(s.new_start_dir.trim_end_matches('/')), h.library.join("SoH"));
        assert!(Path::new(&s.old_exe).ends_with("Applications/SoH/soh.appimage"));
    }

    #[test]
    fn with_steam_running_the_change_is_queued_and_the_plan_says_so() {
        let h = fake_home("plan_queue");
        let reqs = requests(&h);
        let soh = reqs.iter().find(|r| r.port_name == "Ship of Harkinian").unwrap();
        let steps = plan_port(soh, None, SteamWrite::Queued).steps(&h.home);
        assert!(steps.iter().any(|s| s.starts_with("Steam is running: queue the change of Steam shortcut \"Ship of Harkinian\"") && s.ends_with("for Desktop Mode.")), "{steps:#?}");
    }

    #[test]
    fn a_port_with_no_shortcut_says_so() {
        let h = fake_home("plan_noshortcut");
        let reqs = requests(&h);
        let dusk = reqs.iter().find(|r| r.port_name == "Dusklight").unwrap();
        let steps = plan_port(dusk, None, SteamWrite::Now).steps(&h.home);
        assert!(steps.contains(&"No Steam shortcut points at it.".to_string()));
    }

    #[test]
    fn write_mode_is_queued_in_game_mode_while_steam_runs_and_when_it_cannot_be_told() {
        let closed = || -> std::io::Result<Vec<String>> { Ok(vec![]) };
        let running = || -> std::io::Result<Vec<String>> { Ok(vec!["steam".into()]) };
        let broken = || -> std::io::Result<Vec<String>> { Err(std::io::Error::other("no /proc")) };
        assert_eq!(steam_write_mode(&closed, false), SteamWrite::Now);
        assert_eq!(steam_write_mode(&closed, true), SteamWrite::Queued, "Game Mode never writes");
        assert_eq!(steam_write_mode(&running, false), SteamWrite::Queued);
        assert_eq!(steam_write_mode(&broken, false), SteamWrite::Queued);
    }

    #[test]
    fn an_existing_destination_blocks_and_nothing_is_planned() {
        let h = fake_home("plan_exists");
        fs::create_dir_all(h.library.join("SoH")).unwrap();
        let reqs = requests(&h);
        let soh = reqs.iter().find(|r| r.port_name == "Ship of Harkinian");
        // The scan skips it (already installed), so build the request by hand.
        assert!(soh.is_none());
        let by_hand = MoveRequest {
            port_key: "k".into(),
            port_name: "Ship of Harkinian".into(),
            catalog_folder: "SoH".into(),
            from: h.home.join("Applications/SoH"),
            to: h.library.join("SoH"),
            size: Size::default(),
            steam: vec![],
        };
        let plan = plan_port(&by_hand, None, SteamWrite::Now);
        assert_eq!(plan.block, Some(Block::DestinationExists(h.library.join("SoH"))));
        let steps = plan.steps(&h.home);
        assert_eq!(steps.len(), 1);
        assert!(steps[0].starts_with("Nothing is done: ~/DeckPorts/Library/SoH already exists"), "{steps:?}");
    }

    #[test]
    fn not_enough_free_space_blocks_with_the_numbers_and_unknown_space_does_not() {
        let h = fake_home("plan_space");
        let reqs = requests(&h);
        let soh = reqs.iter().find(|r| r.port_name == "Ship of Harkinian").unwrap();
        let tight = plan_port(soh, Some(SPACE_MARGIN + 7_999), SteamWrite::Now);
        assert_eq!(tight.block, Some(Block::NotEnoughSpace { need: 8000, free: SPACE_MARGIN + 7_999 }));
        assert!(tight.block.as_ref().unwrap().is_space());
        assert!(tight.block.unwrap().text(&h.home).contains("needs 8 KB plus 1.0 GB spare"));
        assert!(plan_port(soh, Some(SPACE_MARGIN + 8_000), SteamWrite::Now).block.is_none());
        assert!(plan_port(soh, None, SteamWrite::Now).block.is_none());
    }

    #[test]
    fn a_batch_counts_the_space_the_earlier_ports_use_up() {
        let h = fake_home("plan_batch");
        let mut reqs = requests(&h);
        reqs.sort_by(|a, b| a.port_name.cmp(&b.port_name));
        for r in &mut reqs {
            r.size.bytes = 4_000_000_000;
        }
        // Room for one 4 GB port plus the margin, not two.
        let plans = plan_all(&reqs, Some(5_500_000_000), SteamWrite::Now);
        assert_eq!(plans.len(), 2);
        assert!(plans[0].block.is_none());
        assert!(plans[1].block.as_ref().is_some_and(Block::is_space), "{:?}", plans[1].block);
    }

    #[test]
    fn a_missing_source_and_a_bad_catalog_folder_and_nesting_are_refused() {
        let h = fake_home("plan_refuse");
        let base = MoveRequest {
            port_key: "k".into(),
            port_name: "X".into(),
            catalog_folder: "x".into(),
            from: h.home.join("Applications/SoH"),
            to: h.library.join("x"),
            size: Size::default(),
            steam: vec![],
        };
        let missing = MoveRequest { from: h.home.join("gone"), ..base.clone() };
        assert!(matches!(plan_port(&missing, None, SteamWrite::Now).block, Some(Block::SourceMissing(_))));
        for bad in ["../evil", "a/b", ".", ""] {
            let r = MoveRequest { catalog_folder: bad.into(), to: h.library.join("zz"), ..base.clone() };
            assert!(matches!(plan_port(&r, None, SteamWrite::Now).block, Some(Block::BadFolderName(_))), "{bad}");
        }
        // The old folder is the folder that holds the ports folder.
        let outer = MoveRequest { from: h.home.join("DeckPorts"), ..base.clone() };
        assert_eq!(plan_port(&outer, None, SteamWrite::Now).block, Some(Block::Nested));
    }

    #[test]
    fn a_shortcut_is_rerooted_only_when_it_points_inside_the_old_folder() {
        let from = Path::new("/home/d/Applications/SoH");
        let to = Path::new("/run/media/sd/Ports/SoH");
        assert_eq!(reroot("/home/d/Applications/SoH/bin/soh", from, to), "/run/media/sd/Ports/SoH/bin/soh");
        assert_eq!(reroot("/home/d/Applications/SoH", from, to), "/run/media/sd/Ports/SoH");
        assert_eq!(reroot("/usr/bin/env", from, to), "/usr/bin/env");
        assert_eq!(reroot("", from, to), "");
    }
}
