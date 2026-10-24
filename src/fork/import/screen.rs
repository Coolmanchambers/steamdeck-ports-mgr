//! What the Import ports screens say and do, without any UI code. The cards (found, one per port, which
//! port is this, move) are in `cards.rs`; this file holds the state they share with the flow
//! (`ImportScreen`), the ports a folder could be (`PortChoice`, `PortListScreen`), the texts of the plan and
//! blocked lists, and the scan folders list. The screens in `src/app/import_cards_screen.rs` and
//! `src/app/import_screen.rs` draw them.

use super::cards::Cards;
use super::folders::ScanFolders;
use super::plan::{Block, PortPlan};
use super::scan::{Found, ImportScan};
use crate::core::models::Port;
use crate::fork::adopt::{shared_words, WELL_KNOWN};
use crate::fork::rom_screens::util::ellipsize;
use crate::fork::screens::{listed, short_path};
use std::path::{Path, PathBuf};

/// One run of Import ports: what the scan found, what the user said about each find, where it goes.
pub struct ImportScreen {
    pub home: PathBuf,
    pub ports_folder: PathBuf,
    pub folders: ScanFolders,
    pub scan: ImportScan,
    pub cards: Cards,
    pub from_wizard: bool,
}

impl ImportScreen {
    pub fn new(home: &Path, ports_folder: &Path, folders: ScanFolders, scan: ImportScan, cards: Cards, from_wizard: bool) -> ImportScreen {
        ImportScreen { home: home.to_path_buf(), ports_folder: ports_folder.to_path_buf(), folders, scan, cards, from_wizard }
    }

    pub fn selected_count(&self) -> usize {
        self.cards.accepted_count()
    }

    /// The finds the user said yes to, in order.
    pub fn selected_found(&self) -> Vec<&Found> {
        self.cards.accepted_found().into_iter().filter_map(|i| self.scan.found.get(i)).collect()
    }

    /// A folder for a text: `~/...` under the home folder, else just its name.
    pub fn short_name(&self, path: &Path) -> String {
        let short = short_path(path, &self.home);
        if short.starts_with('~') {
            short
        } else {
            path.file_name().map_or(short, |n| n.to_string_lossy().into_owned())
        }
    }

    /// The port with this key, from everything the scan could match.
    pub fn port(&self, key: &str) -> Option<&Port> {
        self.scan.ports.iter().find(|p| p.key() == key)
    }

    /// The user chose a port for a folder: it becomes a find ("you chose") like any other, with the same plan
    /// and checks. Returns its position in `scan.found`.
    pub fn add_found(&mut self, found: Found) -> usize {
        self.scan.found.push(found);
        self.scan.found.len() - 1
    }

    /// The user said a folder is not a port: it leaves the scan's to-choose lists and is noted.
    pub fn leave_alone(&mut self, path: &Path) {
        self.scan.ambiguous.retain(|a| a.path != path);
        self.scan.unrecognised.retain(|p| p != path);
        if !self.scan.left_alone.iter().any(|p| p == path) {
            self.scan.left_alone.push(path.to_path_buf());
        }
    }
}

/// The ports a folder could be, short: two catalog entries for one game ("Majora's Mask (2ship2harkinian)",
/// "Majora's Mask (Zelda64Recomp)") read by their projects, "2ship2harkinian or Zelda64Recomp" (the
/// chooser then shows the full names).
fn could_be(names: &[String]) -> String {
    let split = |n: &str| -> Option<(String, String)> {
        let open = n.strip_suffix(')')?.rfind(" (")?;
        Some((n[..open].to_string(), n[open + 2..n.len() - 1].to_string()))
    };
    let parts: Vec<Option<(String, String)>> = names.iter().map(|n| split(n)).collect();
    if names.len() > 1 && parts.iter().all(Option::is_some) {
        let parts: Vec<(String, String)> = parts.into_iter().flatten().collect();
        if parts.iter().all(|(game, _)| *game == parts[0].0) {
            let notes: Vec<String> = parts.iter().map(|(_, note)| note.clone()).collect();
            return or_list(&notes);
        }
    }
    or_list(names)
}

/// "A", "A or B", "A, B or C".
fn or_list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

// ---------------------------------------------------------------------------------------------
// Choosing the port for a folder
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceTarget {
    pub path: PathBuf,
    /// Keys of the ports it could be (the ambiguous matches); empty for an unrecognised folder.
    pub could_be: Vec<String>,
}

/// A port as the choosers list it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortChoice {
    pub key: String,
    pub name: String,
    pub installed: bool,
    /// Another folder of this scan already found as this port (shortened for the label).
    pub found_in: Option<String>,
}

impl PortChoice {
    fn of(port: &Port, screen: &ImportScreen, target: &Path) -> PortChoice {
        let found_in = screen.scan.found.iter().find(|f| f.port_key == port.key() && f.path != target).map(|f| f.dir_name.clone());
        PortChoice { key: port.key().to_string(), name: port.name.clone(), installed: crate::core::installer::is_installed(port, &screen.ports_folder), found_in }
    }

    pub fn label(&self) -> String {
        match (&self.found_in, self.installed) {
            (_, true) => format!("{} (already in your ports folder)", self.name),
            (Some(folder), false) => format!("{} (already found in {folder})", self.name),
            (None, false) => self.name.clone(),
        }
    }
}

/// "Another port...": the ports a folder could be that are not in the ports folder yet, those sharing words
/// with the folder name first (most shared first), the rest by name. Android-only ones are left out.
pub struct PortListScreen {
    pub ports: Vec<PortChoice>,
    /// How many at the top share a word with the folder name.
    pub related: usize,
}

impl PortListScreen {
    pub fn new(screen: &ImportScreen, target: &ChoiceTarget) -> PortListScreen {
        let dir_name = target.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut scored: Vec<(usize, PortChoice)> = screen
            .scan
            .ports
            .iter()
            .filter(|p| !p.is_android_only() && !p.name.trim().is_empty())
            .map(|p| (shared_words(&dir_name, p), PortChoice::of(p, screen, &target.path)))
            .filter(|(_, c)| !c.installed)
            .collect();
        scored.sort_by(|(a, x), (b, y)| b.cmp(a).then_with(|| x.name.to_lowercase().cmp(&y.name.to_lowercase())));
        let related = scored.iter().filter(|(n, _)| *n > 0).count();
        PortListScreen { ports: scored.into_iter().map(|(_, c)| c).collect(), related }
    }

    /// The positions in `ports` whose label has `filter` in it (any case); all of them for an empty filter.
    pub fn matching(&self, filter: &str) -> Vec<usize> {
        let filter = filter.trim().to_lowercase();
        (0..self.ports.len()).filter(|i| filter.is_empty() || self.ports[*i].label().to_lowercase().contains(&filter)).collect()
    }
}

/// What is said when every ticked port is blocked: only the reasons.
pub fn blocked_text(plans: &[PortPlan], home: &Path) -> String {
    let lines: Vec<String> = plans.iter().map(|p| format!("  {}: {}", p.request.port_name, p.block.as_ref().map_or(String::new(), |b| b.text(home)))).collect();
    format!("Nothing can be imported now:\n{}", listed(&lines))
}

/// The requests of the ticked ports that are blocked only for lack of room: the ones that can be linked
/// where they stand instead.
pub fn space_blocked(plans: &[PortPlan]) -> Vec<super::plan::MoveRequest> {
    plans.iter().filter(|p| p.block.as_ref().is_some_and(Block::is_space)).map(|p| p.request.clone()).collect()
}

/// Rows of the "what will importing do" list: one per ticked port.
pub fn plan_labels(plans: &[PortPlan]) -> Vec<String> {
    plans
        .iter()
        .map(|p| match &p.block {
            Some(Block::NotEnoughSpace { .. }) => format!("{}: not enough free space", p.request.port_name),
            Some(_) => format!("{}: cannot be imported now", p.request.port_name),
            None => format!("{}: {} steps", p.request.port_name, p.steps(Path::new("")).len()),
        })
        .collect()
}

pub fn plan_text(plan: &PortPlan, home: &Path) -> String {
    let steps = plan.steps(home);
    let numbered: Vec<String> = if plan.block.is_some() { steps } else { steps.iter().enumerate().map(|(i, s)| format!("{}. {s}", i + 1)).collect() };
    format!("{}:\n\n{}", plan.request.port_name, numbered.join("\n"))
}

// ---------------------------------------------------------------------------------------------
// Scan folders
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderLine {
    /// What happened to the last change ("already in the list", "could not save").
    Note,
    Add,
    /// A well-known folder, by index into `WELL_KNOWN`.
    Default(usize),
    /// A removable drive that is plugged in, by index into `ScanFolders::drives`.
    Drive(usize),
    /// One the user added, by index into `ScanFolders::user`.
    User(usize),
    Deeper,
    Back,
}

pub struct FoldersScreen {
    pub home: PathBuf,
    pub folders: ScanFolders,
    pub note: Option<String>,
}

impl FoldersScreen {
    pub fn new(home: &Path, folders: ScanFolders) -> FoldersScreen {
        FoldersScreen { home: home.to_path_buf(), folders, note: None }
    }

    /// The row the cursor starts on: the first real row, not the note.
    pub fn first_row(&self) -> usize {
        usize::from(self.note.is_some())
    }

    pub fn lines(&self) -> Vec<FolderLine> {
        let mut lines = Vec::new();
        if self.note.is_some() {
            lines.push(FolderLine::Note);
        }
        lines.push(FolderLine::Add);
        lines.extend((0..WELL_KNOWN.len()).map(FolderLine::Default));
        lines.extend((0..self.folders.drives.len()).map(FolderLine::Drive));
        lines.extend((0..self.folders.user.len()).map(FolderLine::User));
        lines.extend([FolderLine::Deeper, FolderLine::Back]);
        lines
    }

    pub fn labels(&self) -> Vec<String> {
        self.lines()
            .into_iter()
            .map(|line| match line {
                FolderLine::Note => self.note.clone().unwrap_or_default(),
                FolderLine::Add => "Add a folder to scan...".to_string(),
                FolderLine::Default(i) => {
                    let name = WELL_KNOWN[i];
                    let missing = if self.home.join(name).is_dir() { "" } else { ", not there" };
                    format!("{}~/{name} (standard{missing})", if self.folders.default_is_on(name) { "[x] " } else { "[ ] " })
                }
                FolderLine::Drive(i) => format!("{}{} (standard)", if self.folders.drive_is_on(i) { "[x] " } else { "[ ] " }, self.folders.drives[i].title),
                FolderLine::User(i) => {
                    let shown = ellipsize(&short_path(&crate::fork::adopt::expand_home(&self.folders.user[i], &self.home), &self.home), 56);
                    let missing = if crate::fork::adopt::expand_home(&self.folders.user[i], &self.home).is_dir() { "" } else { " (not there)" };
                    format!("{shown}{missing}: choose to remove")
                }
                FolderLine::Deeper => format!("{}Also look one folder deeper (slower)", if self.folders.look_deeper { "[x] " } else { "[ ] " }),
                FolderLine::Back => "Back to Import ports".to_string(),
            })
            .collect()
    }

    /// The text of the "remove this folder?" confirm.
    pub fn remove_text(&self, user_index: usize) -> String {
        let path = crate::fork::adopt::expand_home(&self.folders.user[user_index], &self.home);
        format!("Stop scanning {}?\n\nOnly the entry in this list goes. The folder and what is in it are not touched.", short_path(&path, &self.home))
    }
}


#[cfg(test)]
mod tests {
    use super::super::cards::Verdict;
    use super::super::plan::{build_requests, plan_all, SteamWrite};
    use super::super::scan::fixtures::*;
    use super::super::scan::{read_shortcuts, run_scan, ScanInput};
    use super::*;
    use crate::fork::settings::ForkSettings;
    use std::fs;

    fn screen(h: &Home) -> ImportScreen {
        let folders = ScanFolders::default();
        let input = ScanInput {
            roots: folders.roots(&h.home, &h.library),
            catalog: catalog(),
            library_dir: h.library.clone(),
            home: h.home.clone(),
            deeper: false,
            shortcuts: read_shortcuts(&h.home, &ForkSettings::load(&h.home.join("s.json"))),
            ..Default::default()
        };
        let scan = run_scan(&input, &|| false, &|_| {});
        let mut cards = Cards::from_scan(&scan, &[], &h.library);
        for i in 0..cards.len() {
            cards.set(i, Verdict::Yes);
        }
        ImportScreen::new(&h.home, &h.library, folders, scan, cards, false)
    }

    fn plans_of(s: &ImportScreen, free: Option<u64>) -> Vec<PortPlan> {
        plan_all(&build_requests(&s.selected_found(), &s.ports_folder), free, SteamWrite::Now)
    }

    fn tims_screen(dir: &Path, extra: impl FnOnce(&Path)) -> (PathBuf, ImportScreen) {
        let sd = crate::fork::adopt::test_fixtures::tims_sd_card(dir);
        extra(&sd);
        let input = super::super::scan::tests::tims_input(&sd, Default::default());
        let home = input.home.clone();
        let scan = run_scan(&input, &|| false, &|_| {});
        let cards = Cards::from_scan(&scan, &[], &sd.join("Ports"));
        (sd.clone(), ImportScreen::new(&home, &sd.join("Ports"), ScanFolders::default(), scan, cards, false))
    }

    #[test]
    fn only_the_finds_the_user_said_yes_to_are_imported() {
        let h = fake_home("screen_selected");
        let mut s = screen(&h);
        assert_eq!(s.selected_count(), 2);
        assert_eq!(s.selected_found().iter().map(|f| f.port_name.as_str()).collect::<Vec<_>>(), ["Dusklight", "Ship of Harkinian"]);
        s.cards.set(0, Verdict::NotAPort);
        assert_eq!(s.selected_count(), 1);
        assert_eq!(s.selected_found()[0].port_name, "Ship of Harkinian");
        s.cards.set(1, Verdict::Undecided);
        assert!(s.selected_found().is_empty());
    }

    #[test]
    fn when_every_port_is_blocked_only_the_reasons_are_said_and_room_blocked_ones_can_be_linked() {
        let h = fake_home("screen_confirm_block");
        let s = screen(&h);
        let free = crate::fork::import::plan::SPACE_MARGIN + 100;
        let plans = plans_of(&s, Some(free));
        let text = blocked_text(&plans, &h.home);
        assert!(text.starts_with("Nothing can be imported now:\n  Dusklight: not enough free space"), "{text}");
        assert_eq!(space_blocked(&plans).len(), 2, "both could be linked where they stand instead");
    }

    #[test]
    fn the_plan_list_and_text_show_the_numbered_steps() {
        let h = fake_home("screen_plan");
        let s = screen(&h);
        let plans = plans_of(&s, None);
        assert_eq!(plan_labels(&plans), ["Dusklight: 6 steps", "Ship of Harkinian: 6 steps"]);
        let text = plan_text(&plans[1], &h.home);
        assert!(text.starts_with("Ship of Harkinian:\n\n1. Check the game files"), "{text}");
        assert!(text.contains("\n5. Repoint Steam shortcut"), "{text}");
        assert!(text.contains("\n6. Keep the old folder"), "{text}");
        let blocked = plans_of(&s, Some(5));
        assert_eq!(plan_labels(&blocked)[0], "Dusklight: not enough free space");
        assert!(plan_text(&blocked[0], &h.home).contains("Nothing is done: not enough free space"));
    }

    #[test]
    fn the_folders_screen_lists_standard_and_added_folders_with_their_state() {
        let h = fake_home("screen_folders");
        let sd = h.home.join("sd/Ports");
        fs::create_dir_all(&sd).unwrap();
        let mut folders = ScanFolders::default();
        folders.user.push(sd.to_string_lossy().into_owned());
        folders.user.push("/no/such/dir".into());
        folders.toggle_default("Games");
        let mut fs_screen = FoldersScreen::new(&h.home, folders);
        let labels = fs_screen.labels();
        assert_eq!(labels[0], "Add a folder to scan...");
        assert_eq!(labels[1], "[x] ~/Applications (standard)");
        assert_eq!(labels[2], "[ ] ~/Games (standard)");
        assert_eq!(labels[3], "[x] ~/Ports (standard, not there)");
        assert_eq!(labels[4], "~/sd/Ports: choose to remove");
        assert_eq!(labels[5], "/no/such/dir (not there): choose to remove");
        assert_eq!(labels[6], "[ ] Also look one folder deeper (slower)");
        assert_eq!(labels[7], "Back to Import ports");
        assert_eq!(fs_screen.lines()[4], FolderLine::User(0));
        assert!(fs_screen.remove_text(0).starts_with("Stop scanning ~/sd/Ports?"));
        assert!(fs_screen.remove_text(0).contains("not touched"));
        assert!(!labels.join("\n").contains("json"), "no JSON anywhere in the UI");
        fs_screen.note = Some("Already in the list".into());
        assert_eq!(fs_screen.labels()[0], "Already in the list");
        assert_eq!(fs_screen.lines()[0], FolderLine::Note);
        assert_eq!(fs_screen.first_row(), 1, "the cursor skips the note");
    }

    #[test]
    fn a_new_entry_port_blocked_for_room_is_in_the_plan_and_can_be_linked_instead() {
        let dir = temp_dir("screen_tims_blocked");
        let (_, mut s) = tims_screen(&dir, |_| {});
        for i in 0..s.cards.len() {
            s.cards.set(i, Verdict::Yes);
        }
        let plans = plans_of(&s, Some(5));
        assert!(plans.iter().any(|p| p.request.new_entry.is_some()), "the new-to-list port is in the plan");
        assert!(space_blocked(&plans).iter().any(|r| r.new_entry.is_some()), "and can be linked instead");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_port_list_leaves_out_ports_already_in_the_ports_folder_and_puts_related_names_first() {
        let dir = temp_dir("screen_port_list");
        let (sd, s) = tims_screen(&dir, |sd| {
            crate::fork::adopt::test_fixtures::touch_exe(&sd.join("Ports/dusklight/dusklight.sh"));
            crate::fork::adopt::test_fixtures::touch_exe(&sd.join("The Legend of Zelda Majoras Mask/mm.appimage"));
        });
        let target = ChoiceTarget { path: sd.join("The Legend of Zelda Majoras Mask"), could_be: vec![] };
        let list = PortListScreen::new(&s, &target);
        assert!(!list.ports.iter().any(|p| p.installed), "a port already in the ports folder is not offered");
        assert!(!list.ports.iter().any(|p| p.name.contains("dusklight")), "{:?}", list.ports.iter().map(|p| &p.name).collect::<Vec<_>>());
        assert!(list.ports[..2].iter().all(|p| p.name.starts_with("The Legend of Zelda: Majora")), "{:?}", list.ports.iter().map(|p| &p.name).collect::<Vec<_>>());
        assert!(list.related >= 2);
        let found_in = list.ports.iter().find(|p| p.name.contains("2ship2harkinian")).unwrap();
        assert!(found_in.label().contains("already found in 2Ship-Keiichi-Charlie-Linux"), "{}", found_in.label());
        assert_eq!(list.matching("zelda64").len(), 1);
        assert_eq!(list.matching("  ").len(), list.ports.len(), "a blank filter shows everything");
        assert_eq!(list.matching("no such port").len(), 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_chosen_folder_gets_the_same_safety_checks_as_any_find() {
        use super::super::plan::{plan_all_with, Block, PlanGuards};
        let dir = temp_dir("screen_choice_guards");
        let home = dir.join("home");
        put_exe(&home.join("game.x86_64"), b"x");
        let library = dir.join("Ports");
        fs::create_dir_all(&library).unwrap();
        let ports = catalog();
        let input = ScanInput { roots: vec![dir.clone()], catalog: ports.clone(), library_dir: library.clone(), home: home.clone(), ..Default::default() };
        let scan = run_scan(&input, &|| false, &|_| {});
        let mut s = ImportScreen::new(&home, &library, ScanFolders::default(), scan, Cards::new(vec![]), false);
        let port = s.port(ports[2].key()).unwrap().clone();
        let found = super::super::scan::measure_choice(&home, &port, &[], None, &|| false);
        let at = s.add_found(found);
        assert_eq!(s.scan.found[at].path, home);
        s.cards = Cards::from_scan(&s.scan, &[], &library);
        for i in 0..s.cards.len() {
            s.cards.set(i, Verdict::Yes);
        }
        let guards = PlanGuards { home: home.clone(), config_dir: dir.join("config"), scan_roots: s.scan.roots_scanned.clone() };
        let plans = plan_all_with(&build_requests(&s.selected_found(), &library), None, SteamWrite::Now, &guards);
        let plan = plans.iter().find(|p| p.request.from == home).unwrap();
        assert_eq!(plan.block, Some(Block::Protected(home.clone(), "the home folder")));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn leaving_a_folder_alone_takes_it_out_of_the_to_choose_lists() {
        let dir = temp_dir("screen_leave_alone");
        let (sd, mut s) = tims_screen(&dir, |sd| crate::fork::adopt::test_fixtures::touch_exe(&sd.join("Mystery Game/game.x86_64")));
        assert_eq!(s.scan.unrecognised, [sd.join("Mystery Game")]);
        s.leave_alone(&sd.join("Mystery Game"));
        s.leave_alone(&sd.join("Mystery Game"));
        assert!(s.scan.unrecognised.is_empty());
        assert_eq!(s.scan.left_alone, [sd.join("Mystery Game")], "noted once");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_plugged_in_drive_has_a_row_that_can_be_switched_off() {
        let mut folders = ScanFolders::default().with_drives(vec![crate::fork::import::folders::DriveRoot { mount: "/run/media/deck/SD512".into(), title: "SD card (SD512)".into() }]);
        let s = FoldersScreen::new(Path::new("/home/deck"), folders.clone());
        assert!(s.labels().contains(&"[x] SD card (SD512) (standard)".to_string()), "{:?}", s.labels());
        assert!(s.lines().contains(&FolderLine::Drive(0)));
        folders.toggle_drive(0);
        let s = FoldersScreen::new(Path::new("/home/deck"), folders);
        assert!(s.labels().contains(&"[ ] SD card (SD512) (standard)".to_string()));
    }
}
