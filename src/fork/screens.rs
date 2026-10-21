//! What the "Add to Steam" and "Adopt installed ports" screens show and do, without any UI code.
//!
//! The screens in `src/app/steam_screens.rs` are thin: they print `labels()` as a list, call
//! `toggle`/`run` when a row is chosen and show the text these models return. Everything that decides
//! something lives here so it can be tested with fixtures.

use super::adopt::{self, Candidate, ScanResult};
use super::artwork;
use super::backups;
use super::settings::ForkSettings;
use super::steam_sync::{self, Action, PlanItem, WantedShortcut};
use super::steam_user::{self, SteamAccount};
use super::store::{backup_stamp, ShortcutsFile, SteamProcessCheck, StoreError};
use crate::core::models::Port;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Shown whenever a write is refused because Steam is up. Game Mode always has Steam up, so say that.
pub const CLOSE_STEAM_HELP: &str = "Steam is running, so nothing was changed.\n\
    Steam overwrites its shortcuts when it closes.\n\
    In Desktop Mode, close Steam (Steam menu, Exit) and try again.\n\
    Game Mode cannot close Steam.";

pub const RESTART_STEAM_HELP: &str = "Start Steam again to see them under Non-Steam games.";

pub fn short_path(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => {
            let parts: Vec<_> = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            format!("~/{}", parts.join("/"))
        }
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

// ---------------------------------------------------------------------------------------------
// Add to Steam
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteamLine {
    Account,
    Status,
    Port(usize),
    Review,
}

#[derive(Debug)]
pub struct PortRow {
    pub name: String,
    pub key: String,
    pub wanted: Result<WantedShortcut, String>,
    pub plan: Option<PlanItem>,
    pub selected: bool,
}

impl PortRow {
    fn can_change(&self) -> bool {
        self.plan.as_ref().is_some_and(PlanItem::changes_the_file)
    }
}

pub struct SteamScreen {
    pub steam_root: Option<PathBuf>,
    pub accounts: Vec<SteamAccount>,
    pub account: Option<usize>,
    pub rows: Vec<PortRow>,
    /// Why the shortcuts file could not be read, if so.
    pub load_error: Option<String>,
}

impl SteamScreen {
    /// `favorite_exe` answers, per port key, which program the user picked in the port's info.
    pub fn load(home: &Path, library_dir: &Path, catalog: &[Port], settings: &ForkSettings, favorite_exe: &dyn Fn(&str) -> Option<String>) -> SteamScreen {
        let mut screen = SteamScreen { steam_root: None, accounts: Vec::new(), account: None, rows: Vec::new(), load_error: None };
        for root in artwork::existing_steam_roots(home) {
            if let Ok(accounts) = steam_user::discover_accounts(&root) {
                if !accounts.is_empty() {
                    screen.steam_root = Some(root);
                    screen.accounts = accounts;
                    break;
                }
            }
        }
        if screen.steam_root.is_none() {
            screen.load_error = Some("No Steam account folder was found. Start Steam once and sign in, then try again.".into());
            return screen;
        }
        screen.account = steam_user::pick_active(&screen.accounts, settings.steam_account())
            .and_then(|a| screen.accounts.iter().position(|x| x.account_id == a.account_id));
        screen.reload_rows(library_dir, catalog, favorite_exe);
        screen
    }

    pub fn active_account(&self) -> Option<&SteamAccount> {
        self.account.and_then(|i| self.accounts.get(i))
    }

    pub fn shortcuts_path(&self) -> Option<PathBuf> {
        self.active_account().map(|a| artwork::shortcuts_path(&a.dir))
    }

    pub fn grid_dir(&self) -> Option<PathBuf> {
        self.active_account().map(|a| artwork::grid_dir(&a.dir))
    }

    pub fn set_account(&mut self, index: usize, library_dir: &Path, catalog: &[Port], favorite_exe: &dyn Fn(&str) -> Option<String>) {
        if index < self.accounts.len() {
            self.account = Some(index);
            self.reload_rows(library_dir, catalog, favorite_exe);
        }
    }

    fn reload_rows(&mut self, library_dir: &Path, catalog: &[Port], favorite_exe: &dyn Fn(&str) -> Option<String>) {
        self.rows.clear();
        self.load_error = None;
        let Some(path) = self.shortcuts_path() else { return };
        let file = match ShortcutsFile::load(&path) {
            Ok(file) => file,
            Err(e) => {
                self.load_error = Some(e.to_string());
                return;
            }
        };
        let mut eligible: Vec<&Port> = catalog
            .iter()
            .filter(|p| !p.is_android_only() && crate::core::installer::is_installed(p, library_dir))
            .collect();
        eligible.sort_by_key(|p| p.display_name().to_lowercase());
        let wanted: Vec<(String, String, Result<WantedShortcut, String>)> = eligible
            .iter()
            .map(|p| (p.display_name().to_string(), p.key().to_string(), steam_sync::wanted_from_port(p, library_dir, favorite_exe(p.key()).as_deref())))
            .collect();
        let ok: Vec<WantedShortcut> = wanted.iter().filter_map(|(_, _, w)| w.as_ref().ok().cloned()).collect();
        let mut plans = steam_sync::plan(&file.doc, &ok).into_iter();
        for (name, key, wanted) in wanted {
            let plan = wanted.is_ok().then(|| plans.next()).flatten();
            let selected = plan.as_ref().is_some_and(PlanItem::changes_the_file);
            self.rows.push(PortRow { name, key, wanted, plan, selected });
        }
    }

    pub fn lines(&self) -> Vec<SteamLine> {
        let mut lines = vec![SteamLine::Account, SteamLine::Status];
        lines.extend((0..self.rows.len()).map(SteamLine::Port));
        lines.push(SteamLine::Review);
        lines
    }

    pub fn selected_count(&self) -> usize {
        self.rows.iter().filter(|r| r.selected && r.can_change()).count()
    }

    pub fn label(&self, line: SteamLine, steam: &dyn SteamProcessCheck) -> String {
        match line {
            SteamLine::Account => match self.active_account() {
                Some(a) if self.accounts.len() > 1 => format!("Steam account: {} (choose another)", a.label()),
                Some(a) => format!("Steam account: {}", a.label()),
                None if self.accounts.is_empty() => "No Steam account found".to_string(),
                None => "Choose your Steam account".to_string(),
            },
            SteamLine::Status => match steam.running_steam_processes() {
                Ok(p) if p.is_empty() => "Steam is closed: ready to write".to_string(),
                Ok(_) => "Steam is running: close it before applying".to_string(),
                Err(_) => "Could not tell if Steam is running: nothing will be written".to_string(),
            },
            SteamLine::Port(i) => {
                let row = &self.rows[i];
                let mark = if row.can_change() { if row.selected { "[x] " } else { "[ ] " } } else { "" };
                let what = match (&row.wanted, row.plan.as_ref().map(|p| &p.action)) {
                    (Err(reason), _) => format!("skipped, {reason}"),
                    (_, Some(Action::Add)) => "will add".to_string(),
                    (_, Some(Action::Update { changes, .. })) => {
                        format!("will update ({})", changes.iter().map(|c| c.field).collect::<Vec<_>>().join(", "))
                    }
                    (_, Some(Action::UpToDate { .. })) => "already in Steam".to_string(),
                    (_, Some(Action::Blocked(reason))) => format!("skipped, {reason}"),
                    (_, None) => "skipped".to_string(),
                };
                format!("{mark}{}: {what}", row.name)
            }
            SteamLine::Review => match (&self.load_error, self.selected_count()) {
                (Some(_), _) => "Could not read shortcuts.vdf: see the message".to_string(),
                (None, 0) if self.rows.is_empty() => "No installed ports to add".to_string(),
                (None, 0) => "Nothing selected to add or update".to_string(),
                (None, 1) => "Review 1 change...".to_string(),
                (None, n) => format!("Review {n} changes..."),
            },
        }
    }

    pub fn labels(&self, steam: &dyn SteamProcessCheck) -> Vec<String> {
        self.lines().into_iter().map(|l| self.label(l, steam)).collect()
    }

    /// Switches a row on or off. Only rows that would change the file can be selected.
    pub fn toggle(&mut self, index: usize) {
        if let Some(row) = self.rows.get_mut(index) {
            if row.can_change() {
                row.selected = !row.selected;
            }
        }
    }

    /// The plan for the selected rows, in screen order.
    pub fn selected_items(&self) -> Vec<PlanItem> {
        self.rows.iter().filter(|r| r.selected).filter_map(|r| r.plan.clone().filter(PlanItem::changes_the_file)).collect()
    }

    /// The text of the confirmation: what will be added, what will change, where the backup goes.
    pub fn review_text(&self) -> String {
        let mut added = Vec::new();
        let mut updated = Vec::new();
        for item in self.selected_items() {
            match &item.action {
                Action::Add => added.push(format!("  {}", item.wanted.app_name)),
                Action::Update { changes, .. } => {
                    let fields: Vec<&str> = changes.iter().map(|c| c.field).collect();
                    updated.push(format!("  {} ({})", item.wanted.app_name, fields.join(", ")));
                }
                _ => {}
            }
        }
        let mut out = String::new();
        if !added.is_empty() {
            out.push_str(&format!("Add {}:\n{}\n\n", added.len(), listed(&added)));
        }
        if !updated.is_empty() {
            out.push_str(&format!("Update {} (play time and art stay):\n{}\n\n", updated.len(), listed(&updated)));
        }
        out.push_str("shortcuts.vdf is backed up first.");
        out
    }
}

/// At most `MAX_LISTED` lines, so a long list cannot outgrow its dialog.
const MAX_LISTED: usize = 6;

fn listed(lines: &[String]) -> String {
    if lines.len() <= MAX_LISTED {
        return lines.join("\n");
    }
    let mut shown: Vec<String> = lines[..MAX_LISTED - 1].to_vec();
    shown.push(format!("  and {} more", lines.len() - (MAX_LISTED - 1)));
    shown.join("\n")
}

#[derive(Debug)]
pub struct ApplyOutcome {
    pub text: String,
    pub wrote: bool,
}

/// ART HOOK: the one place the artwork pipeline plugs in. It is called after the plan is made and
/// before anything is written, with the app ID each wanted shortcut will have, and may fill in
/// `WantedShortcut::icon` (the placed icon's path). It must not write `shortcuts.vdf` and must never
/// overwrite art the user chose. No artwork is placed yet: this does nothing until the
/// `src/fork/art/` writer lands (`art::grid::write_art(grid_dir, app_id, set, options)`, which returns
/// the icon path), then call it here per `(wanted, app_id)` and re-plan.
fn art_hook(_grid_dir: &Path, _wanted: &mut [(WantedShortcut, u32)]) {}

/// Writes the selected rows. Loads the file again, so edits Steam or another tool made since the
/// screen opened are not lost; the plan is rebuilt from that fresh copy.
pub fn apply_selected(screen: &SteamScreen, steam: &dyn SteamProcessCheck, settings: &ForkSettings, now: SystemTime) -> Result<ApplyOutcome, String> {
    let path = screen.shortcuts_path().ok_or("No Steam account is selected.")?;
    let mut wanted: Vec<WantedShortcut> = screen
        .rows
        .iter()
        .filter(|r| r.selected && r.can_change())
        .filter_map(|r| r.wanted.as_ref().ok().cloned())
        .collect();
    if wanted.is_empty() {
        return Err("Nothing is selected.".into());
    }
    let mut file = ShortcutsFile::load(&path).map_err(|e| e.to_string())?;

    // Art goes in before the write so the icon path can ride along with the shortcut.
    if let Some(grid) = screen.grid_dir() {
        let first = steam_sync::plan(&file.doc, &wanted);
        let mut with_ids: Vec<(WantedShortcut, u32)> =
            first.iter().filter_map(|p| steam_sync::expected_app_id(p).map(|id| (p.wanted.clone(), id))).collect();
        art_hook(&grid, &mut with_ids);
        for (w, _) in with_ids {
            if let Some(slot) = wanted.iter_mut().find(|x| x.port_key == w.port_key) {
                *slot = w;
            }
        }
    }
    let items = steam_sync::plan(&file.doc, &wanted);

    let report = match steam_sync::apply(&mut file, &items, steam, &backup_stamp()) {
        Ok(report) => report,
        Err(steam_sync::SyncError::Store(StoreError::SteamRunning(_))) => return Err(CLOSE_STEAM_HELP.to_string()),
        Err(e) => return Err(format!("{e}\n\nNothing was changed.")),
    };

    let pruned = if report.save.wrote { backups::prune_backups(&path, settings.backups_keep(), now).map(|r| r.deleted.len()).unwrap_or(0) } else { 0 };
    let mut text = format!("Added {}, updated {}, already fine {}.", report.added.len(), report.updated.len(), report.unchanged.len());
    if !report.skipped.is_empty() {
        text.push_str(&format!("\nLeft out {}:", report.skipped.len()));
        for (key, why) in &report.skipped {
            let name = screen.rows.iter().find(|r| &r.key == key).map_or(key.as_str(), |r| r.name.as_str());
            text.push_str(&format!("\n  {name}: {why}"));
        }
    }
    if let Some(backup) = &report.backup {
        let file_name = backup.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        text.push_str(&format!("\n\nBackup of the old list: {file_name}"));
        if pruned > 0 {
            text.push_str(&format!(" ({pruned} older backup{} removed)", if pruned == 1 { "" } else { "s" }));
        }
    }
    text.push('\n');
    text.push_str(RESTART_STEAM_HELP);
    Ok(ApplyOutcome { text, wrote: report.save.wrote })
}

// ---------------------------------------------------------------------------------------------
// Adopt installed ports
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdoptLine {
    Candidate(usize),
    Info(usize),
    Adopt,
    EditFolders,
}

pub struct AdoptScreen {
    pub scan: ScanResult,
    pub selected: Vec<bool>,
    pub home: PathBuf,
}

impl AdoptScreen {
    pub fn load(home: &Path, library_dir: &Path, catalog: &[Port], settings: &ForkSettings) -> AdoptScreen {
        let roots = adopt::scan_roots(home, &settings.adopt_folders(), library_dir);
        let scan = adopt::scan(&roots, catalog, library_dir);
        let selected = vec![true; scan.candidates.len()];
        AdoptScreen { scan, selected, home: home.to_path_buf() }
    }

    /// Informational rows under the candidates: ambiguous folders, then what was scanned.
    fn info_lines(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .scan
            .ambiguous
            .iter()
            .map(|a| format!("Not adopted, fits {} ports: {}", a.matches.len(), short_path(&a.path, &self.home)))
            .collect();
        if !self.scan.already_in_library.is_empty() {
            out.push(format!("Already in the library: {}", self.scan.already_in_library.join(", ")));
        }
        let roots: Vec<String> = self.scan.roots_scanned.iter().map(|r| short_path(r, &self.home)).collect();
        out.push(if roots.is_empty() { "No scan folders exist yet".to_string() } else { format!("Looked in: {}", roots.join(", ")) });
        out
    }

    pub fn lines(&self) -> Vec<AdoptLine> {
        let mut lines: Vec<AdoptLine> = (0..self.scan.candidates.len()).map(AdoptLine::Candidate).collect();
        lines.extend((0..self.info_lines().len()).map(AdoptLine::Info));
        lines.push(AdoptLine::Adopt);
        lines.push(AdoptLine::EditFolders);
        lines
    }

    pub fn selected_count(&self) -> usize {
        self.selected.iter().filter(|&&s| s).count()
    }

    pub fn labels(&self) -> Vec<String> {
        let info = self.info_lines();
        self.lines()
            .into_iter()
            .map(|line| match line {
                AdoptLine::Candidate(i) => {
                    let c = &self.scan.candidates[i];
                    let how = match c.how {
                        adopt::MatchKind::FolderName => "by folder name",
                        adopt::MatchKind::ExecutableName => "by program name",
                    };
                    format!("{}{}: {} ({how})", if self.selected[i] { "[x] " } else { "[ ] " }, c.port_name, short_path(&c.path, &self.home))
                }
                AdoptLine::Info(i) => info[i].clone(),
                AdoptLine::Adopt => match (self.scan.candidates.is_empty(), self.selected_count()) {
                    (true, _) => "Nothing to adopt".to_string(),
                    (false, 0) => "Nothing selected".to_string(),
                    (false, 1) => "Adopt 1 port...".to_string(),
                    (false, n) => format!("Adopt {n} ports..."),
                },
                AdoptLine::EditFolders => "Add scan folders (opens fork-settings.json)".to_string(),
            })
            .collect()
    }

    pub fn toggle(&mut self, index: usize) {
        if let Some(s) = self.selected.get_mut(index) {
            *s = !*s;
        }
    }

    pub fn selected_candidates(&self) -> Vec<&Candidate> {
        self.scan.candidates.iter().zip(&self.selected).filter(|(_, &s)| s).map(|(c, _)| c).collect()
    }

    pub fn confirm_text(&self) -> String {
        let names: Vec<String> = self.selected_candidates().iter().map(|c| format!("  {}", c.port_name)).collect();
        format!(
            "Adopt {}:\n{}\n\nEach is linked into your library and added to ports.local.json (backed up first). \
             Your folders are not moved, copied or changed.",
            names.len(),
            names.join("\n")
        )
    }
}

#[derive(Debug)]
pub struct AdoptDone {
    /// New catalog entries (the registered `ports.local.json` entries), already parsed.
    pub adopted: Vec<Port>,
    pub text: String,
}

/// Adopts the selected candidates one by one; a failure is reported and the others still go ahead.
pub fn adopt_selected(screen: &AdoptScreen, catalog: &[Port], library_dir: &Path, local_json: &Path) -> AdoptDone {
    let mut adopted = Vec::new();
    let mut lines = Vec::new();
    for candidate in screen.selected_candidates() {
        let Some(port) = catalog.iter().find(|p| p.key() == candidate.port_key) else { continue };
        match adopt::adopt(&candidate.path, port, library_dir, local_json, &backup_stamp()) {
            Ok(_) => {
                let entry = adopt::local_entry(port);
                if let Ok(mut registered) = crate::core::models::port_from_value(&entry) {
                    registered.user_managed = true;
                    adopted.push(registered);
                }
                lines.push(format!("Adopted {}", candidate.port_name));
            }
            Err(e) => lines.push(format!("{}: {e}", candidate.port_name)),
        }
    }
    let mut text = lines.join("\n");
    if !adopted.is_empty() {
        text.push_str("\n\nThey now show as installed.\nAdopted ports are not checked for updates.");
    }
    AdoptDone { adopted, text }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::port_from_value;
    use serde_json::json;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("dpm_screens_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn no_steam() -> std::io::Result<Vec<String>> {
        Ok(Vec::new())
    }

    fn catalog() -> Vec<Port> {
        [
            json!({"name": "Ship of Harkinian", "folder": "SoH", "source": "https://github.com/a/soh", "executable": "soh.appimage"}),
            json!({"name": "Dusklight", "folder": "dusklight", "source": "https://github.com/a/dusk", "executable": "dusk"}),
            json!({"name": "Not Installed", "folder": "nope", "source": "https://github.com/a/nope"}),
            json!({"name": "Two Programs", "folder": "twoprog", "source": "https://github.com/a/two"}),
        ]
        .iter()
        .map(|v| port_from_value(v).unwrap())
        .collect()
    }

    #[cfg(unix)]
    fn put_exe(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"x").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A fake home with a Steam folder (account 1234, signed in last), a library with three ports and
    /// a shortcuts file that already has a hand-made "Ship of Harkinian" pointing at an old path.
    #[cfg(unix)]
    fn fake_home(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        use super::super::vdf::fixtures::*;
        let home = temp_dir(name);
        let steam = home.join(".local/share/Steam");
        let user = steam.join("userdata").join("1234");
        fs::create_dir_all(user.join("config")).unwrap();
        fs::create_dir_all(steam.join("config")).unwrap();
        let entry = steam_entry(0, 0xAAAA_0001, "Ship of Harkinian", "\"/old/soh.appimage\"", "\"/old/\"", &[]);
        let path = user.join("config").join("shortcuts.vdf");
        fs::write(&path, file_of(&[entry])).unwrap();
        let library = home.join("DeckPorts").join("Library");
        put_exe(&library.join("SoH").join("soh.appimage"));
        put_exe(&library.join("dusklight").join("dusk"));
        put_exe(&library.join("twoprog").join("a"));
        put_exe(&library.join("twoprog").join("b"));
        (home, library, path)
    }

    #[cfg(unix)]
    #[test]
    fn the_screen_lists_installed_ports_with_what_would_happen_to_each() {
        let (home, library, _) = fake_home("screen");
        let screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        let labels = screen.labels(&no_steam);
        assert_eq!(labels[0], "Steam account: Account 1234");
        assert_eq!(labels[1], "Steam is closed: ready to write");
        assert_eq!(labels[2], "[x] Dusklight: will add");
        assert_eq!(labels[3], "[x] Ship of Harkinian: will update (program, start folder)");
        assert!(labels[4].starts_with("Two Programs: skipped, several programs found"));
        assert_eq!(labels.last().unwrap(), "Review 2 changes...");
        assert_eq!(labels.len(), 6, "the port that is not installed is not listed");
        let _ = fs::remove_dir_all(home);
    }

    #[cfg(unix)]
    #[test]
    fn toggling_changes_the_count_and_unchangeable_rows_ignore_it() {
        let (home, library, _) = fake_home("toggle");
        let mut screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        screen.toggle(0);
        assert_eq!(screen.selected_count(), 1);
        assert_eq!(screen.label(SteamLine::Review, &no_steam), "Review 1 change...");
        screen.toggle(2);
        assert!(screen.rows[2].plan.is_none() && !screen.rows[2].selected);
        screen.toggle(1);
        assert_eq!(screen.label(SteamLine::Review, &no_steam), "Nothing selected to add or update");
        let _ = fs::remove_dir_all(home);
    }

    #[cfg(unix)]
    #[test]
    fn the_review_text_names_what_is_added_and_updated() {
        let (home, library, _) = fake_home("review");
        let screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        let text = screen.review_text();
        assert!(text.contains("Add 1:\n  Dusklight"));
        assert!(text.contains("Update 1 (play time and art stay):\n  Ship of Harkinian (program, start folder)"));
        assert!(text.contains("backed up first"));
        let _ = fs::remove_dir_all(home);
    }

    #[cfg(unix)]
    #[test]
    fn applying_writes_a_backup_keeps_the_id_and_says_to_restart_steam() {
        let (home, library, path) = fake_home("apply");
        let before = fs::read(&path).unwrap();
        let screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        let out = apply_selected(&screen, &no_steam, &ForkSettings::load(&home.join("s.json")), SystemTime::now()).unwrap();
        assert!(out.wrote);
        assert!(out.text.starts_with("Added 1, updated 1, already fine 0."), "{}", out.text);
        assert!(out.text.contains("Backup of the old list: shortcuts.vdf.bak-"));
        assert!(out.text.contains("Start Steam again"));
        let backups: Vec<_> = backups::list_backups(&path).unwrap();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(&backups[0].path).unwrap(), before);
        let saved = super::super::shortcuts::list(&ShortcutsFile::load(&path).unwrap().doc);
        assert_eq!(saved.len(), 2);
        let soh = saved.iter().find(|s| s.app_name == "Ship of Harkinian").unwrap();
        assert_eq!(soh.app_id, Some(0xAAAA_0001));
        assert_eq!(soh.exe_path(), library.join("SoH").join("soh.appimage").to_str().unwrap());
        // Run again: everything is now in place, so nothing is offered and nothing is written.
        let again = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        assert_eq!(again.selected_count(), 0);
        assert!(again.labels(&no_steam)[2].ends_with("already in Steam"));
        assert!(apply_selected(&again, &no_steam, &ForkSettings::load(&home.join("s.json")), SystemTime::now()).is_err());
        let _ = fs::remove_dir_all(home);
    }

    #[cfg(unix)]
    #[test]
    fn applying_while_steam_runs_gives_the_close_steam_message_and_touches_nothing() {
        let (home, library, path) = fake_home("running");
        let before = fs::read(&path).unwrap();
        let screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        let running = || -> std::io::Result<Vec<String>> { Ok(vec!["99 steam".into()]) };
        assert_eq!(screen.label(SteamLine::Status, &running), "Steam is running: close it before applying");
        let err = apply_selected(&screen, &running, &ForkSettings::load(&home.join("s.json")), SystemTime::now()).unwrap_err();
        assert_eq!(err, CLOSE_STEAM_HELP);
        assert!(err.contains("Desktop Mode") && err.contains("Game Mode"));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(backups::list_backups(&path).unwrap().is_empty());
        let _ = fs::remove_dir_all(home);
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_steam_check_also_refuses_and_says_nothing_was_changed() {
        let (home, library, path) = fake_home("checkfail");
        let before = fs::read(&path).unwrap();
        let screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        let broken = || -> std::io::Result<Vec<String>> { Err(std::io::Error::other("no /proc")) };
        assert!(screen.label(SteamLine::Status, &broken).contains("nothing will be written"));
        let err = apply_selected(&screen, &broken, &ForkSettings::load(&home.join("s.json")), SystemTime::now()).unwrap_err();
        assert!(err.contains("Nothing was changed"), "{err}");
        assert_eq!(fs::read(&path).unwrap(), before);
        let _ = fs::remove_dir_all(home);
    }

    #[cfg(unix)]
    #[test]
    fn edits_made_by_someone_else_since_the_screen_opened_are_kept_not_overwritten() {
        use super::super::vdf::fixtures::*;
        let (home, library, path) = fake_home("fresh");
        let screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        let entry = steam_entry(0, 0xAAAA_0001, "Ship of Harkinian", "\"/old/soh.appimage\"", "\"/old/\"", &[]);
        let other = steam_entry(1, 0xBBBB_0002, "Added meanwhile", "\"/x\"", "\"/\"", &[]);
        fs::write(&path, file_of(&[entry, other])).unwrap();
        apply_selected(&screen, &no_steam, &ForkSettings::load(&home.join("s.json")), SystemTime::now()).unwrap();
        let saved = super::super::shortcuts::list(&ShortcutsFile::load(&path).unwrap().doc);
        assert!(saved.iter().any(|s| s.app_name == "Added meanwhile"));
        assert_eq!(saved.len(), 3);
        let _ = fs::remove_dir_all(home);
    }

    #[cfg(unix)]
    #[test]
    fn old_backups_are_pruned_after_a_write_but_recent_ones_stay() {
        let (home, library, path) = fake_home("prune");
        let dir = path.parent().unwrap();
        for i in 1..=4 {
            let p = dir.join(format!("shortcuts.vdf.bak-2026010{i}-100000"));
            fs::write(&p, "old").unwrap();
            let f = fs::OpenOptions::new().write(true).open(&p).unwrap();
            f.set_times(fs::FileTimes::new().set_modified(SystemTime::now() - std::time::Duration::from_secs(86_400 * (10 + i)))).unwrap();
        }
        let settings_path = home.join("s.json");
        fs::write(&settings_path, r#"{"steam_backups_keep": 2}"#).unwrap();
        let screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&settings_path), &|_| None);
        let out = apply_selected(&screen, &no_steam, &ForkSettings::load(&settings_path), SystemTime::now()).unwrap();
        assert!(out.text.contains("older backups removed"), "{}", out.text);
        // The fresh backup plus the newest old one.
        assert_eq!(backups::list_backups(&path).unwrap().len(), 2);
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn no_steam_folder_gives_a_plain_message() {
        let home = temp_dir("nosteam");
        let screen = SteamScreen::load(&home, &home.join("Library"), &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        assert!(screen.load_error.as_deref().unwrap().contains("No Steam account folder"));
        assert!(apply_selected(&screen, &no_steam, &ForkSettings::load(&home.join("s.json")), SystemTime::now()).is_err());
        let _ = fs::remove_dir_all(home);
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_shortcuts_file_is_reported_and_blocks_review() {
        let (home, library, path) = fake_home("garbled");
        fs::write(&path, b"\x00shortcuts\x00\x09bad\x00").unwrap();
        let screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        assert!(screen.load_error.is_some() && screen.rows.is_empty());
        assert!(screen.label(SteamLine::Review, &no_steam).starts_with("Could not read shortcuts.vdf"));
        let _ = fs::remove_dir_all(home);
    }

    #[cfg(unix)]
    #[test]
    fn two_accounts_without_a_clear_choice_ask_which_and_the_choice_sticks() {
        let (home, library, _) = fake_home("two_accounts");
        let steam = home.join(".local/share/Steam");
        fs::create_dir_all(steam.join("userdata").join("5678").join("config")).unwrap();
        let settings_path = home.join("s.json");
        let mut screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&settings_path), &|_| None);
        assert!(screen.account.is_none() && screen.rows.is_empty());
        assert_eq!(screen.label(SteamLine::Account, &no_steam), "Choose your Steam account");
        let index = screen.accounts.iter().position(|a| a.account_id == 5678).unwrap();
        screen.set_account(index, &library, &catalog(), &|_| None);
        assert_eq!(screen.label(SteamLine::Account, &no_steam), "Steam account: Account 5678 (choose another)");
        // With no shortcuts file yet, every installed port is an add.
        assert!(screen.rows.iter().filter(|r| r.plan.is_some()).all(|r| matches!(r.plan.as_ref().unwrap().action, Action::Add)));
        let mut settings = ForkSettings::load(&settings_path);
        settings.set_steam_account(5678).unwrap();
        let again = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&settings_path), &|_| None);
        assert_eq!(again.active_account().unwrap().account_id, 5678);
        let _ = fs::remove_dir_all(home);
    }

    // -- adopt ----------------------------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn the_adopt_screen_offers_matches_and_adopting_registers_them_without_touching_the_install() {
        let home = temp_dir("adopt_screen");
        let library = home.join("DeckPorts").join("Library");
        fs::create_dir_all(&library).unwrap();
        put_exe(&home.join("Applications").join("SoH").join("soh.appimage"));
        fs::create_dir_all(home.join("Games").join("junk")).unwrap();
        let settings = ForkSettings::load(&home.join("s.json"));
        let mut screen = AdoptScreen::load(&home, &library, &catalog(), &settings);
        let labels = screen.labels();
        assert_eq!(labels[0], "[x] Ship of Harkinian: ~/Applications/SoH (by folder name)");
        assert!(labels.iter().any(|l| l == "Looked in: ~/Applications, ~/Games"));
        assert_eq!(labels.iter().filter(|l| l.starts_with("Adopt")).collect::<Vec<_>>(), ["Adopt 1 port..."]);
        assert!(screen.confirm_text().contains("not moved, copied or changed"));
        screen.toggle(0);
        assert_eq!(screen.labels().iter().find(|l| l.starts_with("Nothing")).unwrap(), "Nothing selected");
        screen.toggle(0);

        let local = home.join("ports.local.json");
        let done = adopt_selected(&screen, &catalog(), &library, &local);
        assert_eq!(done.adopted.len(), 1);
        assert!(done.adopted[0].user_managed);
        assert!(done.text.contains("Adopted Ship of Harkinian"));
        assert!(library.join("SoH").join("soh.appimage").exists());
        assert!(home.join("Applications").join("SoH").join("soh.appimage").exists());

        // The adopted port now shows up on the Add to Steam screen with its Library path.
        let mut merged = catalog();
        merged = crate::core::config::merge_local_catalog(merged, done.adopted);
        let steam_screen = SteamScreen::load(&home, &library, &merged, &settings, &|_| None);
        assert!(steam_screen.load_error.is_some(), "no Steam folder in this fixture");
        let _ = fs::remove_dir_all(home);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_adoption_is_reported_by_name_and_does_not_stop_the_others() {
        let home = temp_dir("adopt_partial");
        let library = home.join("Library");
        put_exe(&home.join("Applications").join("SoH").join("soh.appimage"));
        put_exe(&home.join("Applications").join("dusklight").join("dusk"));
        let screen = AdoptScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")));
        assert_eq!(screen.scan.candidates.len(), 2);
        // Something already sits where the first port's link would go.
        fs::create_dir_all(library.join("dusklight")).unwrap();
        let screen_after = AdoptScreen { scan: ScanResult { candidates: screen.scan.candidates.clone(), ..Default::default() }, selected: vec![true; 2], home: home.clone() };
        let done = adopt_selected(&screen_after, &catalog(), &library, &home.join("ports.local.json"));
        assert_eq!(done.adopted.len(), 1);
        assert!(done.text.contains("Dusklight: ") && done.text.contains("already exists"), "{}", done.text);
        assert!(done.text.contains("Adopted Ship of Harkinian"));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn long_lists_are_cut_so_the_dialog_stays_readable() {
        let lines: Vec<String> = (1..=9).map(|i| format!("  Port {i}")).collect();
        let out = listed(&lines);
        assert_eq!(out.lines().count(), MAX_LISTED);
        assert!(out.ends_with("  and 4 more"));
        assert_eq!(listed(&lines[..3]), "  Port 1\n  Port 2\n  Port 3");
    }

    #[test]
    fn short_paths_hide_the_home_folder() {
        let home = Path::new("/home/deck");
        assert_eq!(short_path(Path::new("/home/deck/Games/x"), home), "~/Games/x");
        assert_eq!(short_path(Path::new("/home/deck"), home), "~");
        assert_eq!(short_path(Path::new("/run/media/card/x"), home), "/run/media/card/x");
    }
}
