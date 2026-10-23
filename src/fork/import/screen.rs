//! What the Import ports lists say and do, without any UI code. The screens in
//! `src/app/import_screen.rs` print `labels()`, call `toggle` or open the next list when a row is chosen
//! and show the texts these models return.

use super::folders::ScanFolders;
use super::plan::{Block, PortPlan};
use super::scan::{Confidence, Found, ImportScan};
use crate::fork::adopt::WELL_KNOWN;
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
}

impl ImportScreen {
    pub fn new(home: &Path, ports_folder: &Path, folders: ScanFolders, scan: ImportScan, from_wizard: bool) -> ImportScreen {
        let selected = scan.found.iter().map(|f| f.confidence.preselected()).collect();
        ImportScreen { home: home.to_path_buf(), ports_folder: ports_folder.to_path_buf(), folders, scan, selected, from_wizard }
    }

    /// Rows under the finds: what was left out and why, then what was looked at.
    fn info_lines(&self) -> Vec<String> {
        let scan = &self.scan;
        let mut out: Vec<String> = Vec::new();
        if scan.found.is_empty() {
            out.push("No ports found outside your ports folder".to_string());
        }
        if scan.found.iter().any(|f| f.confidence == Confidence::CheckThis) {
            out.push("\"Check this\" rows matched only by a program inside the folder, so they start unticked: look before you tick them".to_string());
        }
        out.extend(scan.ambiguous.iter().map(|a| format!("Not imported, fits {} ports: {}", a.matches.len(), short_path(&a.path, &self.home))));
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
        format!(
            "{}{} ({}) {at_least}{}, {}{steam}",
            if self.selected[i] { "[x] " } else { "[ ] " },
            f.port_name,
            f.confidence.label(),
            human_bytes(f.size.bytes),
            ellipsize(&short_path(&f.path, &self.home), 44)
        )
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

/// The confirm dialog before an import. `free` is the free space in the ports folder's drive when known.
pub fn confirm_text(plans: &[PortPlan], ports_folder: &Path, home: &Path, free: Option<u64>, dry_run: bool) -> String {
    let (runnable, blocked): (Vec<&PortPlan>, Vec<&PortPlan>) = plans.iter().partition(|p| p.can_run());
    if runnable.is_empty() {
        return blocked_text(plans, home);
    }
    let names: Vec<String> = runnable.iter().map(|p| format!("  {} ({})", p.request.port_name, human_bytes(p.request.size.bytes))).collect();
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
    use super::super::scan::{read_shortcuts, run_scan, Confidence, ScanInput};
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
        assert!(s.labels().iter().any(|l| l.starts_with("\"Check this\" rows matched only by a program inside the folder")), "{:?}", s.labels());
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
        assert!(info.contains("Not imported, fits 2 ports: ~/Games/Fan Remake"), "{info}");
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
}
