//! What the Import ports lists say and do, without any UI code. The screens in
//! `src/app/import_screen.rs` print `labels()`, call `toggle` or open the next list when a row is chosen
//! and show the texts these models return.

use super::folders::ScanFolders;
use super::plan::{Block, PortPlan};
use super::scan::{Confidence, Found, ImportScan};
use crate::core::models::Port;
use crate::fork::adopt::{shared_words, WELL_KNOWN};
use crate::fork::rom_screens::util::{ellipsize, human_bytes};
use crate::fork::screens::{listed, short_path};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------------------------
// The main list
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportLine {
    Intro,
    PortsFolder,
    Folders,
    Found(usize),
    /// A folder that fits two or three ports (index into `ImportScan::ambiguous`): the user chooses.
    Choose(usize),
    /// A folder with a program that fits no port (index into `ImportScan::unrecognised`): the user may choose.
    Unrecognised(usize),
    Info(usize),
    Rescan,
    /// Opens the step-by-step plan of the ticked ports.
    Preview,
    Import,
    /// Only when the wizard opened this screen: carry on with the setup.
    Continue,
}

pub struct ImportScreen {
    pub home: PathBuf,
    pub ports_folder: PathBuf,
    pub folders: ScanFolders,
    pub scan: ImportScan,
    pub selected: Vec<bool>,
    pub from_wizard: bool,
    /// The row last chosen, so the list comes back on it after a tick or a dialog.
    pub last_row: Option<usize>,
}

impl ImportScreen {
    pub fn new(home: &Path, ports_folder: &Path, folders: ScanFolders, scan: ImportScan, from_wizard: bool) -> ImportScreen {
        let selected = scan.found.iter().map(|f| f.confidence.preselected()).collect();
        ImportScreen { home: home.to_path_buf(), ports_folder: ports_folder.to_path_buf(), folders, scan, selected, from_wizard, last_row: None }
    }

    /// Where the cursor goes when the list is shown: the row last chosen (the nearest one left when rows
    /// went), else the first row that does something.
    pub fn start_row(&self) -> usize {
        let count = self.lines().len();
        match self.last_row {
            Some(row) if count > 0 => row.min(count - 1),
            _ => self.first_row(),
        }
    }

    /// Rows under the finds: what was left out and why, then what was looked at.
    fn info_lines(&self) -> Vec<String> {
        let scan = &self.scan;
        let mut out: Vec<String> = Vec::new();
        let choosable = !scan.ambiguous.is_empty() || !scan.unrecognised.is_empty();
        if scan.found.is_empty() && !choosable {
            out.push("No ports found outside your ports folder".to_string());
        } else if scan.found.is_empty() {
            out.push("Nothing was recognised for sure: choose the port for a folder above".to_string());
        }
        if scan.found.iter().any(|f| f.confidence == Confidence::CheckThis) {
            out.push("\"Check this\" rows matched by one word of the folder name or a program inside it, so they start unticked: look before you tick them".to_string());
        }
        if scan.found.iter().any(|f| f.new_entry.is_some()) {
            out.push("\"New to your port list\": on the Deck-ready list but not in the catalog; importing adds it to your port list first".to_string());
        }
        if let Some(note) = &scan.deck_ready_note {
            out.push(note.clone());
        }
        if !scan.collections.is_empty() {
            let names: Vec<String> = scan.collections.iter().map(|p| self.short_name(p)).collect();
            out.push(format!("Skipped, looks like a collection of many games: {}", names.join(", ")));
        }
        if !scan.left_alone.is_empty() {
            let names: Vec<String> = scan.left_alone.iter().map(|p| self.short_name(p)).collect();
            out.push(format!("Left alone (you said not a port): {}", names.join(", ")));
        }
        out.extend(scan.duplicates.iter().map(|(name, path)| format!("Also found {name} in {}: not imported", short_path(path, &self.home))));
        if !scan.already_in_library.is_empty() {
            out.push(format!("Already in your ports folder: {}", scan.already_in_library.join(", ")));
        }
        if let Some(note) = &scan.steam_note {
            out.push(note.clone());
        }
        if scan.cancelled {
            out.push("The scan was stopped, so this list may be incomplete".to_string());
        }
        if scan.deep_limit_hit {
            out.push(format!("Looking deeper stopped after {} folders", crate::fork::adopt::MAX_DEEP_FOLDERS));
        }
        let roots: Vec<String> = scan.roots_scanned.iter().map(|r| short_path(r, &self.home)).collect();
        out.push(if roots.is_empty() { "None of your scan folders exist yet".to_string() } else { format!("Looked in: {}", roots.join(", ")) });
        out
    }

    pub fn lines(&self) -> Vec<ImportLine> {
        let mut lines = vec![ImportLine::Intro, ImportLine::PortsFolder, ImportLine::Folders];
        lines.extend((0..self.scan.found.len()).map(ImportLine::Found));
        lines.extend((0..self.scan.ambiguous.len()).map(ImportLine::Choose));
        lines.extend((0..self.scan.unrecognised.len()).map(ImportLine::Unrecognised));
        lines.extend((0..self.info_lines().len()).map(ImportLine::Info));
        lines.push(ImportLine::Rescan);
        if !self.scan.found.is_empty() {
            lines.push(ImportLine::Preview);
        }
        lines.push(ImportLine::Import);
        if self.from_wizard {
            lines.push(ImportLine::Continue);
        }
        lines
    }

    /// Where the cursor starts: the first row that does something (Scan folders, a find, Import, Continue), not
    /// the intro, the ports folder line or an information line.
    pub fn first_row(&self) -> usize {
        self.lines().iter().position(|l| !matches!(l, ImportLine::Intro | ImportLine::PortsFolder | ImportLine::Info(_))).unwrap_or(0)
    }

    pub fn selected_count(&self) -> usize {
        self.selected.iter().filter(|&&s| s).count()
    }

    fn found_label(&self, i: usize) -> String {
        let f = &self.scan.found[i];
        let at_least = if f.size.complete { "" } else { "at least " };
        let steam = match f.steam.len() {
            0 => String::new(),
            1 => ", Steam shortcut".to_string(),
            n => format!(", {n} Steam shortcuts"),
        };
        let new = if f.new_entry.is_some() { ", new to your port list" } else { "" };
        format!(
            "{}{} ({}{new}) {at_least}{}, {}{steam}",
            if self.selected[i] { "[x] " } else { "[ ] " },
            f.port_name,
            f.confidence.label(),
            human_bytes(f.size.bytes),
            ellipsize(&short_path(&f.path, &self.home), 44)
        )
    }

    /// A folder for an information row: `~/...` under the home folder, else just its name.
    pub fn short_name(&self, path: &Path) -> String {
        let short = short_path(path, &self.home);
        if short.starts_with('~') {
            short
        } else {
            path.file_name().map_or(short, |n| n.to_string_lossy().into_owned())
        }
    }

    fn folder_label(&self, path: &Path) -> String {
        ellipsize(&short_path(path, &self.home), 44)
    }

    /// The folder a Choose or Unrecognised row is about, and the ports it could be (empty when unrecognised).
    pub fn choice_target(&self, line: ImportLine) -> Option<ChoiceTarget> {
        match line {
            ImportLine::Choose(i) => self.scan.ambiguous.get(i).map(|a| ChoiceTarget { path: a.path.clone(), could_be: a.keys.clone() }),
            ImportLine::Unrecognised(i) => self.scan.unrecognised.get(i).map(|p| ChoiceTarget { path: p.clone(), could_be: Vec::new() }),
            _ => None,
        }
    }

    /// The user chose a port for a folder: the folder becomes a ticked find ("you chose") and goes through the
    /// same plan and checks as any other. Its Choose or Unrecognised row goes.
    pub fn apply_choice(&mut self, found: Found) {
        self.drop_target(&found.path);
        let at = self.scan.found.iter().position(|f| f.port_name.to_lowercase() > found.port_name.to_lowercase()).unwrap_or(self.scan.found.len());
        self.scan.found.insert(at, found);
        self.selected.insert(at, true);
    }

    /// The user said a folder is not a port: its row goes, an information row says so.
    pub fn leave_alone(&mut self, path: &Path) {
        self.drop_target(path);
        if !self.scan.left_alone.iter().any(|p| p == path) {
            self.scan.left_alone.push(path.to_path_buf());
        }
    }

    fn drop_target(&mut self, path: &Path) {
        self.scan.ambiguous.retain(|a| a.path != path);
        self.scan.unrecognised.retain(|p| p != path);
    }

    /// The port with this key, from everything the scan could match.
    pub fn port(&self, key: &str) -> Option<&Port> {
        self.scan.ports.iter().find(|p| p.key() == key)
    }

    pub fn labels(&self) -> Vec<String> {
        let info = self.info_lines();
        self.lines()
            .into_iter()
            .map(|line| match line {
                ImportLine::Intro => "Finds ports installed outside your ports folder (once called Adopt) and moves them in. Old copies stay until you remove them.".to_string(),
                ImportLine::PortsFolder => format!("Moves them into: {}", ellipsize(&short_path(&self.ports_folder, &self.home), 60)),
                ImportLine::Folders => format!("Scan folders: {} on (choose to add or change)...", self.folders.on_count()),
                ImportLine::Found(i) => self.found_label(i),
                ImportLine::Choose(i) => {
                    let a = &self.scan.ambiguous[i];
                    format!("Choose which port this is: {} (could be {})...", self.folder_label(&a.path), ellipsize(&could_be(&a.matches), 120))
                }
                ImportLine::Unrecognised(i) => format!("Not recognised: {}. Choose the port...", self.folder_label(&self.scan.unrecognised[i])),
                ImportLine::Info(i) => info[i].clone(),
                ImportLine::Rescan => "Scan again".to_string(),
                ImportLine::Preview => "Show what importing will do...".to_string(),
                ImportLine::Import => match (self.scan.found.is_empty(), self.selected_count()) {
                    (true, _) => "Nothing to import".to_string(),
                    (false, 0) => "Nothing selected".to_string(),
                    (false, 1) => "Import 1 port...".to_string(),
                    (false, n) => format!("Import {n} ports..."),
                },
                ImportLine::Continue => "Continue setup".to_string(),
            })
            .collect()
    }

    pub fn toggle(&mut self, index: usize) {
        if let Some(s) = self.selected.get_mut(index) {
            *s = !*s;
        }
    }

    pub fn selected_found(&self) -> Vec<&Found> {
        self.scan.found.iter().zip(&self.selected).filter(|(_, &s)| s).map(|(f, _)| f).collect()
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
        let found_in = screen.scan.found.iter().find(|f| f.port_key == port.key() && f.path != target).map(|f| screen.short_name(&f.path));
        PortChoice { key: port.key().to_string(), name: port.name.clone(), installed: crate::core::installer::is_installed(port, &screen.ports_folder), found_in }
    }

    fn label(&self) -> String {
        match (&self.found_in, self.installed) {
            (_, true) => format!("{} (already in your ports folder)", self.name),
            (Some(folder), false) => format!("{} (already found in {folder})", self.name),
            (None, false) => self.name.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChooseLine {
    Header,
    Match(usize),
    Another,
    NotAPort,
    Back,
}

/// "Which port is this folder?": the ports it could be, "Another port...", "Not a port (leave it)".
pub struct ChooserScreen {
    pub target: ChoiceTarget,
    pub home: PathBuf,
    pub matches: Vec<PortChoice>,
}

impl ChooserScreen {
    pub fn new(screen: &ImportScreen, target: ChoiceTarget) -> ChooserScreen {
        let matches = target.could_be.iter().filter_map(|k| screen.port(k)).map(|p| PortChoice::of(p, screen, &target.path)).collect();
        ChooserScreen { target, home: screen.home.clone(), matches }
    }

    pub fn lines(&self) -> Vec<ChooseLine> {
        let mut lines = vec![ChooseLine::Header];
        lines.extend((0..self.matches.len()).map(ChooseLine::Match));
        lines.extend([ChooseLine::Another, ChooseLine::NotAPort, ChooseLine::Back]);
        lines
    }

    /// The cursor starts on the first port it could be, or on "Another port...".
    pub fn first_row(&self) -> usize {
        1
    }

    pub fn labels(&self) -> Vec<String> {
        let folder = ellipsize(&short_path(&self.target.path, &self.home), 60);
        self.lines()
            .into_iter()
            .map(|line| match line {
                ChooseLine::Header if self.matches.is_empty() => format!("Which port is in {folder}?"),
                ChooseLine::Header => format!("Which port is in {folder}? It could be one of these:"),
                ChooseLine::Match(i) => self.matches[i].label(),
                ChooseLine::Another => "Another port...".to_string(),
                ChooseLine::NotAPort => "Not a port (leave it)".to_string(),
                ChooseLine::Back => "Back to Import ports".to_string(),
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortListLine {
    Header,
    Port(usize),
    Back,
}

/// "Another port...": every port, those sharing words with the folder name first (most shared first), the
/// rest by name.
/// Android-only ones are left out.
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
            .collect();
        scored.sort_by(|(a, x), (b, y)| b.cmp(a).then_with(|| x.name.to_lowercase().cmp(&y.name.to_lowercase())));
        let related = scored.iter().filter(|(n, _)| *n > 0).count();
        PortListScreen { ports: scored.into_iter().map(|(_, c)| c).collect(), related }
    }

    pub fn lines(&self) -> Vec<PortListLine> {
        let mut lines = vec![PortListLine::Header];
        lines.extend((0..self.ports.len()).map(PortListLine::Port));
        lines.push(PortListLine::Back);
        lines
    }

    pub fn first_row(&self) -> usize {
        1
    }

    pub fn labels(&self) -> Vec<String> {
        self.lines()
            .into_iter()
            .map(|line| match line {
                PortListLine::Header if self.related > 0 => format!("Choose the port ({} named like the folder come first):", self.related),
                PortListLine::Header => "Choose the port:".to_string(),
                PortListLine::Port(i) => self.ports[i].label(),
                PortListLine::Back => "Back".to_string(),
            })
            .collect()
    }
}

/// The confirm dialog before an import. `free` is the free space in the ports folder's drive when known.
pub fn confirm_text(plans: &[PortPlan], ports_folder: &Path, home: &Path, free: Option<u64>, dry_run: bool) -> String {
    let (runnable, blocked): (Vec<&PortPlan>, Vec<&PortPlan>) = plans.iter().partition(|p| p.can_run());
    if runnable.is_empty() {
        return blocked_text(plans, home);
    }
    let names: Vec<String> = runnable
        .iter()
        .map(|p| {
            let new = if p.request.new_entry.is_some() { ", added to your port list first" } else { "" };
            format!("  {} ({}){new}", p.request.port_name, human_bytes(p.request.size.bytes))
        })
        .collect();
    let total: u64 = runnable.iter().map(|p| p.request.size.bytes).sum();
    let mut text = String::new();
    if dry_run {
        text.push_str("Preview only: this build shows what would happen and does not copy anything yet.\n\n");
    }
    text.push_str(&format!("Import {} into {}:\n{}", if runnable.len() == 1 { "1 port".to_string() } else { format!("{} ports", runnable.len()) }, short_path(ports_folder, home), listed(&names)));
    text.push_str(&format!("\n\nTotal {}", human_bytes(total)));
    if let Some(free) = free {
        text.push_str(&format!(", {} free", human_bytes(free)));
    }
    text.push_str(".\nEach is copied and checked (its game files too) before the app switches over. Steam shortcuts pointing at it keep their app ID and follow it, or are queued while Steam runs. The old folders stay: you are asked afterwards whether to remove each one.");
    if !blocked.is_empty() {
        let lines: Vec<String> = blocked.iter().map(|p| format!("  {}: {}", p.request.port_name, p.block.as_ref().map_or(String::new(), |b| b.text(home)))).collect();
        text.push_str(&format!("\n\nNot imported now:\n{}", listed(&lines)));
    }
    text
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
    use super::super::plan::{build_requests, plan_all, SteamWrite};
    use super::super::scan::fixtures::*;
    use super::super::scan::{measure_choice, read_shortcuts, run_scan, Confidence, ScanInput};
    use super::*;
    use crate::fork::settings::ForkSettings;
    use std::fs;

    fn screen(h: &Home, from_wizard: bool) -> ImportScreen {
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
        ImportScreen::new(&h.home, &h.library, folders, scan, from_wizard)
    }

    fn plans_of(s: &ImportScreen, free: Option<u64>) -> Vec<PortPlan> {
        plan_all(&build_requests(&s.selected_found(), &s.ports_folder), free, SteamWrite::Now)
    }

    #[test]
    fn the_list_shows_each_find_with_confidence_size_location_and_steam() {
        let h = fake_home("screen_list");
        let s = screen(&h, false);
        let labels = s.labels();
        assert!(labels[0].starts_with("Finds ports installed outside your ports folder (once called Adopt)"), "{labels:#?}");
        assert_eq!(labels[1], "Moves them into: ~/DeckPorts/Library");
        assert_eq!(labels[2], "Scan folders: 3 on (choose to add or change)...");
        assert_eq!(labels[3], "[x] Dusklight (likely match) 700 B, ~/Games/dusk-old");
        assert_eq!(labels[4], "[x] Ship of Harkinian (exact match) 8 KB, ~/Applications/SoH, Steam shortcut");
        assert_eq!(labels[5], "Looked in: ~/Applications, ~/Games");
        assert_eq!(&labels[6..], ["Scan again", "Show what importing will do...", "Import 2 ports..."]);
    }

    #[test]
    fn the_cursor_starts_on_the_first_row_that_does_something() {
        let h = fake_home("screen_first_row");
        for from_wizard in [false, true] {
            let s = screen(&h, from_wizard);
            let first = s.first_row();
            assert_eq!(s.lines()[first], ImportLine::Folders, "not the intro, the ports folder line or an information line");
            assert!(first > 1);
        }
    }

    #[test]
    fn selection_toggles_and_the_import_row_counts_it() {
        let h = fake_home("screen_toggle");
        let mut s = screen(&h, false);
        assert_eq!(s.selected_count(), 2);
        s.toggle(0);
        assert_eq!(s.selected_count(), 1);
        assert!(s.labels()[3].starts_with("[ ] Dusklight"));
        assert_eq!(s.labels().last().unwrap(), "Import 1 port...");
        assert_eq!(s.selected_found().len(), 1);
        assert_eq!(s.selected_found()[0].port_name, "Ship of Harkinian");
        s.toggle(1);
        assert_eq!(s.labels().last().unwrap(), "Nothing selected");
        s.toggle(99);
        assert_eq!(s.selected_count(), 0);
    }

    #[test]
    fn a_check_this_find_starts_unticked() {
        let h = fake_home("screen_check");
        put_exe(&h.home.join("Games/misc/severedchains.x86_64"), b"x");
        let s = screen(&h, false);
        let i = s.scan.found.iter().position(|f| f.confidence == Confidence::CheckThis).unwrap();
        assert!(!s.selected[i]);
        assert!(s.labels()[3 + i].starts_with("[ ] Severed Chains (check this)"));
        assert!(s.labels().iter().any(|l| l.starts_with("\"Check this\" rows matched by one word of the folder name or a program inside it")), "{:?}", s.labels());
    }

    #[test]
    fn nothing_found_says_so_and_offers_no_import() {
        let h = fake_home("screen_empty");
        fs::remove_dir_all(h.home.join("Applications")).unwrap();
        fs::remove_dir_all(h.home.join("Games")).unwrap();
        let s = screen(&h, false);
        let labels = s.labels();
        assert_eq!(labels[3], "No ports found outside your ports folder");
        assert_eq!(labels[4], "None of your scan folders exist yet");
        assert_eq!(&labels[5..], ["Scan again", "Nothing to import"]);
        assert!(!s.lines().contains(&ImportLine::Preview));
    }

    #[test]
    fn left_out_folders_are_explained() {
        let h = fake_home("screen_info");
        put_file(&h.home.join("Games/Fan Remake/x"), b"1");
        put_file(&h.home.join("Games/soh-copy/shipofharkinian.appimage"), b"1");
        fs::create_dir_all(h.library.join("severed-chains")).unwrap();
        put_file(&h.home.join("Games/severed-chains/x"), b"1");
        let info = screen(&h, false).labels().join("\n");
        assert!(info.contains("Choose which port this is: ~/Games/Fan Remake (could be Fan Remake or Fan Remake Deluxe)..."), "{info}");
        assert!(info.contains("Also found Ship of Harkinian in ~/Games/soh-copy: not imported"), "{info}");
        assert!(info.contains("Already in your ports folder: Severed Chains"), "{info}");
    }

    #[test]
    fn the_wizard_gets_a_continue_row_and_the_settings_entry_does_not() {
        let h = fake_home("screen_wizard");
        assert_eq!(screen(&h, true).labels().last().unwrap(), "Continue setup");
        assert_eq!(screen(&h, true).lines().last(), Some(&ImportLine::Continue));
        assert_eq!(screen(&h, false).labels().last().unwrap(), "Import 2 ports...");
    }

    #[test]
    fn the_confirm_names_the_ports_the_size_the_free_space_and_what_stays() {
        let h = fake_home("screen_confirm");
        let s = screen(&h, false);
        let plans = plans_of(&s, Some(120_000_000_000));
        let text = confirm_text(&plans, &h.library, &h.home, Some(120_000_000_000), false);
        assert!(text.starts_with("Import 2 ports into ~/DeckPorts/Library:\n  Dusklight (700 B)\n  Ship of Harkinian (8 KB)"), "{text}");
        assert!(text.contains("Total 8 KB, 120.0 GB free."), "{text}");
        assert!(text.contains("The old folders stay: you are asked afterwards whether to remove each one."), "{text}");
        assert!(!text.contains("Preview only"));
        let preview = confirm_text(&plans, &h.library, &h.home, None, true);
        assert!(preview.starts_with("Preview only: this build shows what would happen and does not copy anything yet."), "{preview}");
        assert!(preview.contains("Total 8 KB.\n"), "{preview}");
    }

    #[test]
    fn the_confirm_lists_ports_that_cannot_go_ahead_with_the_reason() {
        let h = fake_home("screen_confirm_block");
        let s = screen(&h, false);
        let free = crate::fork::import::plan::SPACE_MARGIN + 100;
        let plans = plans_of(&s, Some(free));
        // The margin plus 100 bytes free: Dusklight (700 B) cannot keep the margin, nor can the other.
        let text = confirm_text(&plans, &h.library, &h.home, Some(free), false);
        assert!(text.starts_with("Nothing can be imported now:\n  Dusklight: not enough free space"), "{text}");
        assert!(!text.contains("Import 0 ports"), "{text}");
        assert_eq!(space_blocked(&plans).len(), 2, "both could be linked where they stand instead");
    }

    #[test]
    fn the_plan_list_and_text_show_the_numbered_steps() {
        let h = fake_home("screen_plan");
        let s = screen(&h, false);
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
    fn no_row_of_the_import_screen_mentions_json() {
        let h = fake_home("screen_nojson");
        let all = screen(&h, true).labels().join("\n");
        assert!(!all.to_lowercase().contains("json"), "{all}");
    }

    /// The import screen over Tim's SD card (his catalog, the Deck-ready list) plus extra folders.
    fn tims_screen(dir: &Path, extra: impl FnOnce(&Path)) -> (PathBuf, ImportScreen) {
        let sd = crate::fork::adopt::test_fixtures::tims_sd_card(dir);
        extra(&sd);
        let input = super::super::scan::tests::tims_input(&sd, Default::default());
        let home = input.home.clone();
        let scan = run_scan(&input, &|| false, &|_| {});
        (sd.clone(), ImportScreen::new(&home, &sd.join("Ports"), ScanFolders::default(), scan, false))
    }

    #[test]
    fn tims_card_lists_four_ticked_ports_and_severed_chains_says_it_is_added_first() {
        let dir = temp_dir("screen_tims");
        let (_, s) = tims_screen(&dir, |_| {});
        let labels = s.labels();
        assert_eq!(s.selected_count(), 4);
        assert!(labels[3].starts_with("[x] The Legend of Dragoon (Severed-Chains) (likely match, new to your port list) "), "{labels:#?}");
        assert!(labels[3].ends_with("~/sd/Severed_Chains_Steam_Deck") || labels[3].contains("Severed_Chains_Steam_Deck"), "{}", labels[3]);
        assert!(labels.contains(&"\"New to your port list\": on the Deck-ready list but not in the catalog; importing adds it to your port list first".to_string()), "{labels:#?}");
        assert!(labels[4..7].iter().all(|l| l.starts_with("[x] The Legend of Zelda") && !l.contains("not in the catalog")), "{labels:#?}");
        assert!(!labels.iter().any(|l| l.contains("Emulation") || l.contains("steamapps") || l.contains("lost+found") || l.contains("Trash")), "{labels:#?}");
        assert_eq!(labels.last().unwrap(), "Import 4 ports...");
        assert_eq!(s.lines()[s.first_row()], ImportLine::Folders);
        let plans = plans_of(&s, None);
        assert!(plans.iter().all(PortPlan::can_run), "{:?}", plans.iter().map(|p| &p.block).collect::<Vec<_>>());
        let text = confirm_text(&plans, &s.ports_folder, &s.home, None, false);
        assert!(text.contains("The Legend of Dragoon (Severed-Chains) (") && text.contains("), added to your port list first"), "{text}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn folders_to_choose_get_their_own_rows_and_collections_one_line() {
        let dir = temp_dir("screen_choose_rows");
        let (_, s) = tims_screen(&dir, |sd| {
            put_exe(&sd.join("The Legend of Zelda Majoras Mask/mm.appimage"), b"x");
            put_exe(&sd.join("Mystery Game/game.x86_64"), b"x");
            put_file(&sd.join("Retro/roms/a.z64"), b"x");
            for exe in ["dusklight.sh", "2ship2harkinian.sh", "Shipwright.sh", "Zelda64Recomp.sh"] {
                put_exe(&sd.join("Bundle").join(exe), b"x");
            }
        });
        let lines = s.lines();
        let labels = s.labels();
        let choose = lines.iter().position(|l| *l == ImportLine::Choose(0)).unwrap();
        assert_eq!(lines[choose - 1], ImportLine::Found(3), "after the finds");
        assert!(
            labels[choose].starts_with("Choose which port this is: ")
                && labels[choose].ends_with(" (could be 2ship2harkinian or Zelda64Recomp)..."),
            "{}",
            labels[choose]
        );
        assert_eq!(lines[choose + 1], ImportLine::Unrecognised(0));
        assert!(labels[choose + 1].starts_with("Not recognised: ") && labels[choose + 1].ends_with("Mystery Game. Choose the port..."), "{}", labels[choose + 1]);
        assert!(labels.contains(&"Skipped, looks like a collection of many games: Bundle, Retro".to_string()), "{labels:#?}");
        assert_eq!(lines[s.first_row()], ImportLine::Folders, "the cursor still starts on Scan folders");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_port_already_found_elsewhere_says_so_in_the_choosers() {
        let dir = temp_dir("screen_found_elsewhere");
        let (sd, s) = tims_screen(&dir, |sd| put_exe(&sd.join("The Legend of Zelda Majoras Mask/mm.appimage"), b"x"));
        let i = s.lines().iter().position(|l| matches!(l, ImportLine::Choose(_))).unwrap();
        let target = s.choice_target(s.lines()[i]).unwrap();
        let chooser = ChooserScreen::new(&s, target.clone());
        assert_eq!(chooser.labels()[1], "The Legend of Zelda: Majora's Mask (2ship2harkinian) (already found in 2Ship-Keiichi-Charlie-Linux)");
        assert_eq!(chooser.labels()[2], "The Legend of Zelda: Majora's Mask (Zelda64Recomp)");
        let list = PortListScreen::new(&s, &target);
        assert!(list.labels().contains(&"The Legend of Zelda: Ocarina of Time (Shipwright) (already found in SoH-Ackbar-Delta-Linux)".to_string()), "{:#?}", list.labels());
        let _ = sd;
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_cursor_comes_back_to_the_row_last_chosen() {
        let h = fake_home("screen_last_row");
        let mut s = screen(&h, false);
        assert_eq!(s.start_row(), s.first_row());
        s.last_row = Some(4);
        assert_eq!(s.start_row(), 4);
        s.last_row = Some(99);
        assert_eq!(s.start_row(), s.lines().len() - 1, "rows that went: the nearest one left");
    }

    #[test]
    fn a_missing_deck_ready_list_gets_an_information_row() {
        let h = fake_home("screen_no_list");
        let mut s = screen(&h, false);
        s.scan.deck_ready_note = Some("The Deck-ready list couldn't be loaded.".into());
        assert!(s.labels().contains(&"The Deck-ready list couldn't be loaded.".to_string()));
    }

    #[test]
    fn the_could_be_list_names_one_game_once() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(could_be(&names(&["Mask (a)", "Mask (b)"])), "a or b");
        assert_eq!(could_be(&names(&["Mask (a)", "Mask (b)", "Mask (c)"])), "a, b or c");
        assert_eq!(could_be(&names(&["Fan Remake", "Fan Remake Deluxe"])), "Fan Remake or Fan Remake Deluxe");
        assert_eq!(could_be(&names(&["Mask (a)", "Other (b)"])), "Mask (a) or Other (b)");
    }

    #[test]
    fn only_folders_to_choose_say_nothing_was_recognised_for_sure() {
        let h = fake_home("screen_only_choose");
        fs::remove_dir_all(h.home.join("Applications")).unwrap();
        fs::remove_dir_all(h.home.join("Games")).unwrap();
        put_exe(&h.home.join("Games/Mystery/run.x86_64"), b"x");
        let s = screen(&h, false);
        let labels = s.labels();
        assert!(labels[3].starts_with("Not recognised: ~/Games/Mystery"), "{labels:#?}");
        assert!(labels.contains(&"Nothing was recognised for sure: choose the port for a folder above".to_string()), "{labels:#?}");
        assert!(!labels.contains(&"No ports found outside your ports folder".to_string()));
        assert_eq!(labels.last().unwrap(), "Nothing to import");
    }

    #[test]
    fn the_chooser_offers_the_matches_another_port_and_not_a_port() {
        let dir = temp_dir("screen_chooser");
        let (sd, s) = tims_screen(&dir, |sd| put_exe(&sd.join("The Legend of Zelda Majoras Mask/mm.appimage"), b"x"));
        let i = s.lines().iter().position(|l| matches!(l, ImportLine::Choose(_))).unwrap();
        let target = s.choice_target(s.lines()[i]).unwrap();
        assert_eq!(target.path, sd.join("The Legend of Zelda Majoras Mask"));
        let chooser = ChooserScreen::new(&s, target.clone());
        let labels = chooser.labels();
        assert!(labels[0].starts_with("Which port is in ") && labels[0].ends_with("It could be one of these:"), "{}", labels[0]);
        assert_eq!(
            &labels[1..],
            [
                "The Legend of Zelda: Majora's Mask (2ship2harkinian) (already found in 2Ship-Keiichi-Charlie-Linux)",
                "The Legend of Zelda: Majora's Mask (Zelda64Recomp)",
                "Another port...",
                "Not a port (leave it)",
                "Back to Import ports"
            ]
        );
        assert_eq!(chooser.lines()[chooser.first_row()], ChooseLine::Match(0));
        assert_eq!(chooser.matches[1].key, "Zelda64Recomp/Zelda64Recomp");

        let list = PortListScreen::new(&s, &target);
        let labels = list.labels();
        assert!(labels[0].starts_with("Choose the port ("), "{}", labels[0]);
        assert_eq!(list.lines()[list.first_row()], PortListLine::Port(0));
        assert_eq!(labels.last().unwrap(), "Back");
        // Every port of the scan is there once, the Zelda ones (sharing "zelda", "majoras", "mask") first.
        assert_eq!(list.ports.len(), s.scan.ports.len());
        assert!(list.related >= 2, "{:?}", list.ports);
        // The two Majora's Mask entries share the most words ("legend", "zelda", "majoras", "mask"), so they lead.
        assert!(list.ports[..2].iter().all(|p| p.name.starts_with("The Legend of Zelda: Majora's Mask")), "{:?}", list.ports);
        assert!(list.ports[list.related..].iter().all(|p| !p.name.contains("Zelda")), "{:?}", list.ports);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unrecognised_folder_chooser_has_no_could_be_list() {
        let dir = temp_dir("screen_chooser_unrec");
        let (_, s) = tims_screen(&dir, |sd| put_exe(&sd.join("Mystery Game/game.x86_64"), b"x"));
        let i = s.lines().iter().position(|l| matches!(l, ImportLine::Unrecognised(_))).unwrap();
        let chooser = ChooserScreen::new(&s, s.choice_target(s.lines()[i]).unwrap());
        let labels = chooser.labels();
        assert!(labels[0].starts_with("Which port is in ") && labels[0].ends_with("Mystery Game?"), "{}", labels[0]);
        assert_eq!(&labels[1..], ["Another port...", "Not a port (leave it)", "Back to Import ports"]);
        assert_eq!(chooser.lines()[chooser.first_row()], ChooseLine::Another);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn choosing_a_port_makes_a_ticked_find_and_not_a_port_leaves_the_folder_alone() {
        let dir = temp_dir("screen_apply_choice");
        let (sd, mut s) = tims_screen(&dir, |sd| {
            put_exe(&sd.join("Mystery Game/game.x86_64"), b"x");
            put_exe(&sd.join("Other Thing/tool.sh"), b"x");
        });
        let port = s.port("Zelda64Recomp/Zelda64Recomp").unwrap().clone();
        let found = measure_choice(&sd.join("Mystery Game"), &port, &[], None, &|| false);
        s.apply_choice(found);
        assert_eq!(s.scan.unrecognised, [sd.join("Other Thing")]);
        let at = s.scan.found.iter().position(|f| f.path == sd.join("Mystery Game")).unwrap();
        assert!(s.selected[at]);
        let label = s.labels()[3 + at].clone();
        assert!(label.starts_with("[x] The Legend of Zelda: Majora's Mask (you chose) "), "{label}");
        assert_eq!(s.selected.len(), s.scan.found.len());
        assert_eq!(s.labels().last().unwrap(), "Import 5 ports...");
        s.leave_alone(&sd.join("Other Thing"));
        assert!(s.scan.unrecognised.is_empty());
        assert!(s.labels().contains(&"Left alone (you said not a port): Other Thing".to_string()), "{:#?}", s.labels());
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_chosen_folder_gets_the_same_safety_checks_as_any_find() {
        use super::super::plan::{plan_all_with, Block, PlanGuards};
        let dir = temp_dir("screen_choice_guards");
        let home = dir.join("home");
        put_exe(&home.join("game.x86_64"), b"x");
        let elsewhere = dir.join("elsewhere");
        put_exe(&elsewhere.join("run.sh"), b"x");
        std::os::unix::fs::symlink(&elsewhere, dir.join("linked")).unwrap();
        let library = dir.join("Ports");
        fs::create_dir_all(&library).unwrap();
        let ports = catalog();
        let input = ScanInput { roots: vec![dir.clone()], catalog: ports.clone(), library_dir: library.clone(), home: home.clone(), ..Default::default() };
        let scan = run_scan(&input, &|| false, &|_| {});
        let mut s = ImportScreen::new(&home, &library, ScanFolders::default(), scan, false);
        assert!(s.scan.unrecognised.contains(&home) && s.scan.unrecognised.contains(&dir.join("linked")), "{:?}", s.scan.unrecognised);
        for (folder, key) in [(home.clone(), ports[2].key()), (dir.join("linked"), ports[1].key())] {
            let port = s.port(key).unwrap().clone();
            let found = measure_choice(&folder, &port, &[], None, &|| false);
            s.apply_choice(found);
        }
        let guards = PlanGuards { home: home.clone(), config_dir: dir.join("config"), scan_roots: s.scan.roots_scanned.clone() };
        let plans = plan_all_with(&build_requests(&s.selected_found(), &library), None, SteamWrite::Now, &guards);
        let block_of = |from: &Path| plans.iter().find(|p| p.request.from == from).unwrap().block.clone();
        assert_eq!(block_of(&home), Some(Block::Protected(home.clone(), "the home folder")));
        assert!(matches!(block_of(&dir.join("linked")), Some(Block::Link(_))), "{:?}", block_of(&dir.join("linked")));
        let _ = fs::remove_dir_all(dir);
    }
}
