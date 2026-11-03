//! What the Import ports screens say and do, without any UI code. The cards (found, one per port, which
//! port is this, move) are in `cards.rs`; this file holds the state they share with the flow
//! (`ImportScreen`), the ports a folder could be (`PortChoice`, `PortListScreen`), the texts of the plan and
//! blocked lists, and the scan folders list. The screens in `src/app/import_cards_screen.rs` and
//! `src/app/import_screen.rs` draw them.

use super::cards::Cards;
use super::folders::{AddOutcome, ScanFolders};
use super::plan::{Block, PortPlan};
use super::scan::{Found, ImportScan};
use crate::core::models::Port;
use crate::fork::adopt::{shared_words, WELL_KNOWN};
use crate::fork::page::{Focus, Mark, Page, PageButton, PageOption};
use crate::fork::port_page::paths::{human_path, Places};
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
    /// What the list shows: the name without its trailing note, unless another port has the same name.
    pub name: String,
    /// The name without its trailing note.
    pub display: String,
    pub installed: bool,
    /// Another folder of this scan already found as this port (shortened for the label).
    pub found_in: Option<String>,
}

impl PortChoice {
    fn of(port: &Port, screen: &ImportScreen, target: &Path) -> PortChoice {
        let found_in = screen.scan.found.iter().find(|f| f.port_key == port.key() && f.path != target).map(|f| f.dir_name.clone());
        PortChoice { key: port.key().to_string(), name: port.name.clone(), display: port.display_name().to_string(), installed: crate::core::installer::is_installed(port, &screen.ports_folder), found_in }
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
        // The trailing note (a project name, a fork) is shown only to tell apart ports that share a name.
        let shown: Vec<String> = scored.iter().map(|(_, c)| c.display.to_lowercase()).collect();
        for (_, c) in &mut scored {
            if shown.iter().filter(|n| **n == c.display.to_lowercase()).count() == 1 {
                c.name = c.display.clone();
            }
        }
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

/// A card of the "Where to look for ports" page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderLine {
    Add,
    /// A well-known folder, by index into `WELL_KNOWN`: a switch.
    Default(usize),
    /// A removable drive that is plugged in, by index into `ScanFolders::drives`: a switch.
    Drive(usize),
    /// One the user added, by index into `ScanFolders::user`: choosing it asks whether to stop looking there.
    User(usize),
    /// Look one folder deeper: a switch.
    Deeper,
    Back,
}

/// The answer to "Stop looking in this folder?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopLooking {
    Yes,
    No,
}

pub struct FoldersScreen {
    pub home: PathBuf,
    pub folders: ScanFolders,
    /// What happened to the last change ("already in the list", "could not save").
    pub note: Option<String>,
}

/// Fork (game files settings): the note after "Add a folder to look in", places the human way ("Added Internal storage >
/// Emulation > ports", never "~/Emulation/ports"). `places` are told against `home`.
pub fn add_note(outcome: &AddOutcome, picked: &Path, home: &Path, places: &Places) -> String {
    let places = Places { home: Some(home.to_path_buf()), ..places.clone() };
    match outcome {
        AddOutcome::Added => format!("Added {}", human_path(picked, &places)),
        AddOutcome::DefaultTurnedOn(name) => format!("{} is back on", human_path(&home.join(name), &places)),
        AddOutcome::AlreadyListed => "That folder is already in the list".to_string(),
        AddOutcome::NotAFolder => "That is not a folder".to_string(),
    }
}

/// Fork (game files settings): the window title of the scan folders page.
pub const FOLDERS_WINDOW: &str = "Import ports: where to look";

impl FoldersScreen {
    pub fn new(home: &Path, folders: ScanFolders) -> FoldersScreen {
        FoldersScreen { home: home.to_path_buf(), folders, note: None }
    }

    /// `places` with this screen's home folder: "Internal storage > Games", never "~/Games".
    fn places(&self, places: &Places) -> Places {
        Places { home: Some(self.home.clone()), ..places.clone() }
    }

    fn user_place(&self, i: usize, places: &Places) -> (String, bool) {
        let path = crate::fork::adopt::expand_home(&self.folders.user[i], &self.home);
        (human_path(&path, &self.places(places)), path.is_dir())
    }

    /// Fork (game files settings): the scan folders as a page of cards (was a list of "[x] ~/Games (standard, not there)"
    /// rows). The standard places, the plugged-in drives and "Look one folder deeper" are On / Off switches; a folder the
    /// user added is a card that asks whether to stop looking there. Places are named the human way.
    pub fn folders_page(&self, places: &Places) -> Page<FolderLine> {
        let places = self.places(places);
        let option = |title: &str, detail: &str, action: FolderLine| PageOption { title: ellipsize(title, 40), detail: ellipsize(detail, 66), choice: None, enabled: true, recommended: false, action };
        let mut p = Page::<FolderLine> {
            window_title: FOLDERS_WINDOW.to_string(),
            large: true,
            header: "Import ports".to_string(),
            title: "Where to look for ports".to_string(),
            paragraphs: vec!["Switch a place off to stop looking there. Nothing in these folders is changed.".to_string()],
            buttons: [Some(PageButton { label: "Back".to_string(), action: FolderLine::Back }), None, None, None],
            initial: Focus::Option(0),
            ..Page::default()
        };
        if let Some(note) = &self.note {
            p.fact(Mark::Problem, note.clone());
        }
        p.options.push(option("Add a folder to look in", "Pick a folder that has ports in it", FolderLine::Add));
        let mut switches = Vec::new();
        for (i, name) in WELL_KNOWN.iter().enumerate() {
            let there = self.home.join(name).is_dir();
            let detail = if there { "A standard place" } else { "A standard place, not on this Deck yet" };
            switches.push((p.options.len(), self.folders.default_is_on(name)));
            p.options.push(option(&human_path(&self.home.join(name), &places), detail, FolderLine::Default(i)));
        }
        for (i, drive) in self.folders.drives.iter().enumerate() {
            switches.push((p.options.len(), self.folders.drive_is_on(i)));
            p.options.push(option(&drive.title, "The whole drive, while it is plugged in", FolderLine::Drive(i)));
        }
        for i in 0..self.folders.user.len() {
            let (place, there) = self.user_place(i, &places);
            let detail = if there { "Added by you. Choose it to stop looking there" } else { "Added by you, not there now. Choose it to stop looking there" };
            p.options.push(option(&place, detail, FolderLine::User(i)));
        }
        switches.push((p.options.len(), self.folders.look_deeper));
        p.options.push(option("Look one folder deeper", "Finds ports kept in folders inside folders; takes longer", FolderLine::Deeper));
        for (index, on) in switches {
            p = p.toggle(index, on);
        }
        p.b_means("Back")
    }

    /// "Stop looking in this folder?" for a folder the user added: only the list changes. No is where the cursor starts.
    pub fn stop_page(&self, user_index: usize, places: &Places) -> Page<StopLooking> {
        let (place, _) = self.user_place(user_index, places);
        let option = |title: &str, detail: &str, action: StopLooking| PageOption { title: title.to_string(), detail: detail.to_string(), choice: None, enabled: true, recommended: false, action };
        let mut p = Page::<StopLooking> {
            window_title: format!("{FOLDERS_WINDOW}: stop looking"),
            large: true,
            header: "Import ports".to_string(),
            title: "Stop looking in this folder?".to_string(),
            paragraphs: vec![format!("{place}."), "Only this list changes. The folder and what is in it are not touched.".to_string()],
            options: vec![option("Yes, stop looking there", "It comes off the list", StopLooking::Yes), option("No, keep it", "Nothing changes", StopLooking::No)],
            initial: Focus::Option(1),
            ..Page::default()
        };
        p.options[1].recommended = true;
        p.b_does(&StopLooking::No)
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
        let places = Places::default();
        let p = fs_screen.folders_page(&places);
        let cards: Vec<(&str, &str, Option<bool>)> = p.options.iter().enumerate().map(|(i, o)| (o.title.as_str(), o.detail.as_str(), p.toggle_of(i))).collect();
        assert_eq!(cards[0].0, "Add a folder to look in");
        assert_eq!(cards[1].2, Some(true));
        assert!(cards[1].0.ends_with("> Applications"), "{}", cards[1].0);
        assert_eq!(cards[2].2, Some(false), "Games is switched off");
        assert_eq!((cards[3].1, cards[3].2), ("A standard place, not on this Deck yet", Some(true)));
        assert_eq!((cards[4].0, cards[4].1, cards[4].2), ("Internal storage > sd > Ports", "Added by you. Choose it to stop looking there", None));
        assert_eq!((cards[5].0, cards[5].1), ("/no/such/dir", "Added by you, not there now. Choose it to stop looking there"));
        assert_eq!((cards[6].0, cards[6].2), ("Look one folder deeper", Some(false)));
        assert_eq!(p.options[4].action, FolderLine::User(0));
        let stop = fs_screen.stop_page(0, &places);
        assert_eq!(stop.paragraphs[0], "Internal storage > sd > Ports.");
        assert!(stop.paragraphs[1].contains("not touched"));
        assert!(p.options.iter().all(|o| !o.title.contains("json") && !o.detail.contains("json")), "no JSON anywhere in the UI");
        fs_screen.note = Some("Already in the list".into());
        let noted = fs_screen.folders_page(&places);
        assert_eq!(noted.facts, ["Already in the list"]);
        assert_eq!(noted.options[0].action, FolderLine::Add, "the note is text, never a card the cursor lands on");
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
        let switch_of = |s: &FoldersScreen| {
            let p = s.folders_page(&Places::default());
            let i = p.options.iter().position(|o| o.action == FolderLine::Drive(0)).expect("a card for the drive");
            assert_eq!(p.options[i].title, "SD card (SD512)");
            p.toggle_of(i)
        };
        assert_eq!(switch_of(&FoldersScreen::new(Path::new("/home/deck"), folders.clone())), Some(true));
        folders.toggle_drive(0);
        assert_eq!(switch_of(&FoldersScreen::new(Path::new("/home/deck"), folders)), Some(false));
    }

    #[test]
    fn the_port_list_names_show_no_trailing_note_unless_two_ports_share_a_name() {
        let dir = temp_dir("screen_list_names");
        let (sd, s) = tims_screen(&dir, |sd| {
            crate::fork::adopt::test_fixtures::touch_exe(&sd.join("The Legend of Zelda Majoras Mask/mm.appimage"));
        });
        let target = ChoiceTarget { path: sd.join("The Legend of Zelda Majoras Mask"), could_be: vec![] };
        let list = PortListScreen::new(&s, &target);
        let mut seen: Vec<String> = Vec::new();
        for p in &list.ports {
            let same = list.ports.iter().filter(|o| o.display.to_lowercase() == p.display.to_lowercase()).count();
            if same == 1 {
                assert_eq!(p.name, p.display, "{}", p.name);
            }
            seen.push(p.name.clone());
        }
        assert!(!seen.is_empty());
        let _ = fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod folders_page_tests {
    use super::super::folders::DriveRoot;
    use super::*;
    use crate::fork::page::{action_at, initial_focus};

    fn screen(home: &Path) -> FoldersScreen {
        let mut folders = ScanFolders { user: vec!["~/Emulation/ports".into(), "/run/media/deck/1234-ABCD/A Very Long Folder Name For Ports/And Another One Below It".into()], look_deeper: true, ..ScanFolders::default() };
        folders.drives = vec![DriveRoot { mount: "/run/media/deck/1234-ABCD".into(), title: "SD card (SD512)".into() }];
        folders.toggle_default("Applications");
        FoldersScreen::new(home, folders)
    }

    fn places() -> Places {
        Places { drives: vec![], home: None, media_roots: vec![PathBuf::from("/run/media/deck"), PathBuf::from("/run/media")] }
    }

    #[test]
    fn the_scan_folders_are_switches_with_places_the_human_way() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("Games")).unwrap();
        let p = screen(tmp.path()).folders_page(&places());
        assert_eq!((p.window_title.as_str(), p.title.as_str()), ("Import ports: where to look", "Where to look for ports"));
        let cards: Vec<(&str, FolderLine, Option<bool>)> = p.options.iter().enumerate().map(|(i, o)| (o.title.as_str(), o.action, p.toggle_of(i))).collect();
        assert_eq!(cards[0], ("Add a folder to look in", FolderLine::Add, None));
        assert_eq!(cards[1], ("Internal storage > Applications", FolderLine::Default(0), Some(false)), "switched off");
        assert_eq!(cards[2], ("Internal storage > Games", FolderLine::Default(1), Some(true)));
        assert_eq!(cards[4], ("SD card (SD512)", FolderLine::Drive(0), Some(true)));
        assert_eq!(cards[5], ("Internal storage > Emulation > ports", FolderLine::User(0), None), "a folder of yours is not a switch");
        assert_eq!(cards[6].1, FolderLine::User(1));
        assert!(cards[6].0.starts_with("SD card > "), "{}", cards[6].0);
        assert_eq!(cards[7], ("Look one folder deeper", FolderLine::Deeper, Some(true)));
        assert_eq!(p.options[2].detail, "A standard place");
        assert_eq!(p.options[3].detail, "A standard place, not on this Deck yet");
        assert_eq!(action_at(&p, initial_focus(&p)), Some(FolderLine::Add));
        assert_eq!(p.buttons[0].as_ref().map(|b| b.action), Some(FolderLine::Back));
        assert!(p.facts.is_empty());
    }

    #[test]
    fn the_add_folder_note_names_the_place_the_human_way() {
        let home = Path::new("/home/deck");
        assert_eq!(add_note(&AddOutcome::Added, &home.join("Emulation/ports"), home, &places()), "Added Internal storage > Emulation > ports");
        assert_eq!(add_note(&AddOutcome::DefaultTurnedOn("Games".into()), &home.join("Games"), home, &places()), "Internal storage > Games is back on");
        assert!(add_note(&AddOutcome::Added, Path::new("/run/media/deck/1234-ABCD/Ports"), home, &places()).starts_with("Added SD card > Ports"));
        assert_eq!(add_note(&AddOutcome::AlreadyListed, home, home, &places()), "That folder is already in the list");
        for o in [AddOutcome::Added, AddOutcome::DefaultTurnedOn("Games".into()), AddOutcome::AlreadyListed, AddOutcome::NotAFolder] {
            assert!(!add_note(&o, &home.join("Games"), home, &places()).contains('~'));
        }
    }

    #[test]
    fn the_scan_folder_pages_fit_and_never_say_tilde() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = screen(tmp.path());
        s.note = Some("That folder is already in the list.".into());
        let p = s.folders_page(&places());
        assert_eq!(p.facts, ["That folder is already in the list."]);
        let stop = s.stop_page(1, &places());
        assert_eq!(stop.title, "Stop looking in this folder?");
        assert_eq!(action_at(&stop, initial_focus(&stop)), Some(StopLooking::No));
        assert_eq!(stop.back_hint, "No, keep it");
        crate::fork::rom_screens::settings_pages::tests::assert_fits(&p);
        crate::fork::rom_screens::settings_pages::tests::assert_fits(&stop);
        crate::fork::rom_screens::settings_pages::tests::assert_plain(&p, &[]);
        crate::fork::rom_screens::settings_pages::tests::assert_plain(&stop, &[]);
        assert!(stop.paragraphs[0].starts_with("SD card > "), "{}", stop.paragraphs[0]);
    }
}
