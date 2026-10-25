//! What the "Add to Steam" screen shows and does, without any UI code.
//!
//! The screens in `src/app/steam_screens.rs` are thin: they print `labels()` as a list, call
//! `toggle`/`run` when a row is chosen and show the text these models return. Everything that decides
//! something lives here so it can be tested with fixtures.

use super::art;
use super::artwork;
use super::backups;
use super::settings::ForkSettings;
use super::steam_queue::SteamQueue;
use super::steam_sync::{self, Action, PlanItem, WantedShortcut};
use super::steam_user::{self, SteamAccount};
use super::store::{backup_stamp, check_steam, ShortcutsFile, SteamProcessCheck, StoreError};
use crate::core::models::Port;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Shown whenever a write is refused because Steam is up. Game Mode always has Steam up, so say that.
pub const CLOSE_STEAM_HELP: &str = "Steam is running, so nothing was changed.\n\
    Steam overwrites its shortcuts when it closes.\n\
    In Desktop Mode, close Steam (Steam menu, Exit) and try again.\n\
    Game Mode cannot close Steam.";

pub const RESTART_STEAM_HELP: &str = "Start Steam again to see them under Non-Steam games.";

/// `CLOSE_STEAM_HELP` plus the processes still found, since Steam's helper can outlive "Exit" by a few seconds.
pub fn close_steam_message(processes: &[String]) -> String {
    if processes.is_empty() {
        return CLOSE_STEAM_HELP.to_string();
    }
    format!("{CLOSE_STEAM_HELP}\nStill running: {} (it can take a few seconds to stop).", processes.join(", "))
}

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
    /// Why the queue file could not be read (it was kept aside).
    QueueNote,
    /// "N Steam changes queued from Game Mode: apply now..." (or, with Steam up, that they wait).
    ApplyQueue,
    /// View the queue, remove items or clear it.
    ManageQueue,
    Account,
    Status,
    Port(usize),
    Review,
}

#[derive(Debug)]
pub struct PortRow {
    pub name: String,
    pub key: String,
    /// The catalog folder, which names the port's cached artwork.
    pub folder: String,
    /// The catalog's setup text, which says which game files the port needs (the art sources use it to
    /// find the user's ROM of the original game).
    pub instructions: String,
    pub wanted: Result<WantedShortcut, String>,
    pub plan: Option<PlanItem>,
    pub selected: bool,
}

impl PortRow {
    pub(crate) fn can_change(&self) -> bool {
        self.plan.as_ref().is_some_and(PlanItem::changes_the_file)
    }

    /// An existing shortcut found by name only: it points at some other program today.
    pub(crate) fn by_name(&self) -> bool {
        matches!(self.plan.as_ref().map(|p| &p.action), Some(Action::Update { by_name: true, .. }))
    }
}

pub struct SteamScreen {
    pub steam_root: Option<PathBuf>,
    pub accounts: Vec<SteamAccount>,
    pub account: Option<usize>,
    pub rows: Vec<PortRow>,
    /// Why the shortcuts file could not be read, if so.
    pub load_error: Option<String>,
    /// Where the app caches catalog images (`<folder>.png`, `<folder>_icon.png`). Artwork is only
    /// placed from files found here.
    pub cache_dir: Option<PathBuf>,
    /// Changes queued from Game Mode (`steam-queue.json`); the app fills this after `load`.
    pub queue: SteamQueue,
    /// Set when the queue file could not be read and was moved aside.
    pub queue_note: Option<String>,
    /// Running under gamescope (Game Mode), where Steam cannot be closed; only changes the wording.
    pub game_mode: bool,
}

impl SteamScreen {
    /// `favorite_exe` answers, per port key, which program the user picked in the port's info.
    pub fn load(home: &Path, library_dir: &Path, catalog: &[Port], settings: &ForkSettings, favorite_exe: &dyn Fn(&str) -> Option<String>) -> SteamScreen {
        let mut screen = SteamScreen { steam_root: None, accounts: Vec::new(), account: None, rows: Vec::new(), load_error: None, cache_dir: None, queue: SteamQueue::default(), queue_note: None, game_mode: false };
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
        let wanted: Vec<(&Port, Result<WantedShortcut, String>)> =
            eligible.iter().map(|p| (*p, steam_sync::wanted_from_port(p, library_dir, favorite_exe(p.key()).as_deref()))).collect();
        let ok: Vec<WantedShortcut> = wanted.iter().filter_map(|(_, w)| w.as_ref().ok().cloned()).collect();
        let mut plans = steam_sync::plan(&file.doc, &ok).into_iter();
        for (port, wanted) in wanted {
            let plan = wanted.is_ok().then(|| plans.next()).flatten();
            let mut row = PortRow {
                name: port.display_name().to_string(),
                key: port.key().to_string(),
                folder: port.folder.clone(),
                instructions: port.instructions.clone(),
                wanted,
                plan,
                selected: false,
            };
            // A shortcut found by name only is left off until the user switches it on.
            row.selected = row.can_change() && !row.by_name();
            self.rows.push(row);
        }
    }

    pub fn lines(&self) -> Vec<SteamLine> {
        let mut lines = Vec::new();
        if self.queue_note.is_some() {
            lines.push(SteamLine::QueueNote);
        }
        if !self.queue.is_empty() {
            lines.extend([SteamLine::ApplyQueue, SteamLine::ManageQueue]);
        }
        lines.extend([SteamLine::Account, SteamLine::Status]);
        lines.extend((0..self.rows.len()).map(SteamLine::Port));
        lines.push(SteamLine::Review);
        lines
    }

    pub fn selected_count(&self) -> usize {
        self.rows.iter().filter(|r| r.selected && r.can_change()).count()
    }

    pub fn label(&self, line: SteamLine, steam: &dyn SteamProcessCheck) -> String {
        match line {
            SteamLine::QueueNote => "The saved queue could not be read and was kept aside (choose for details)".to_string(),
            SteamLine::ApplyQueue => {
                let n = self.queue.len();
                let changes = if n == 1 { "1 Steam change".to_string() } else { format!("{n} Steam changes") };
                if self.game_mode {
                    return format!("{changes} queued: will apply next time in Desktop Mode with Steam closed");
                }
                match steam.running_steam_processes() {
                    Ok(p) if p.is_empty() => format!("{changes} queued {}: apply now...", self.queue.origin()),
                    Ok(_) => format!("{changes} waiting: close Steam and update {} now...", if n == 1 { "it" } else { "them" }),
                    Err(_) => format!("{changes} queued: could not tell if Steam is running"),
                }
            }
            SteamLine::ManageQueue => "View or remove queued changes...".to_string(),
            SteamLine::Account => match self.active_account() {
                Some(a) if self.accounts.len() > 1 => format!("Steam account: {} (choose another)", a.label()),
                Some(a) => format!("Steam account: {}", a.label()),
                None if self.accounts.is_empty() => "No Steam account found".to_string(),
                None => "Choose your Steam account".to_string(),
            },
            SteamLine::Status if self.game_mode => "Game Mode: Steam can't be closed here, so changes are queued for Desktop Mode".to_string(),
            SteamLine::Status => match steam.running_steam_processes() {
                Ok(p) if p.is_empty() => "Steam is closed: ready to write".to_string(),
                Ok(_) => "Steam is running: close it to write, or queue the changes".to_string(),
                Err(_) => "Could not tell if Steam is running: nothing will be written".to_string(),
            },
            SteamLine::Port(i) => {
                let row = &self.rows[i];
                let mark = if row.can_change() { if row.selected { "[x] " } else { "[ ] " } } else { "" };
                let what = match (&row.wanted, row.plan.as_ref().map(|p| &p.action)) {
                    (Err(reason), _) => format!("skipped, {reason}"),
                    (_, Some(Action::Add)) => "will add".to_string(),
                    (_, Some(Action::Update { changes, by_name: true, .. })) => {
                        let now = changes.iter().find(|c| c.field == "program").map_or("another program", |c| program_name(&c.from));
                        format!("same name as a shortcut for {now}, will repoint it")
                    }
                    (_, Some(Action::Update { changes, .. })) => {
                        format!("will update ({})", changes.iter().map(|c| c.field).collect::<Vec<_>>().join(", "))
                    }
                    (_, Some(Action::UpToDate { .. })) => "already in Steam".to_string(),
                    (_, Some(Action::Blocked(reason))) => format!("skipped, {reason}"),
                    (_, None) => "skipped".to_string(),
                };
                let queued = match self.active_account() {
                    Some(a) if self.queue.contains(a.account_id, &row.key) => " (queued)",
                    _ => "",
                };
                format!("{mark}{}: {what}{queued}", row.name)
            }
            SteamLine::Review => match (&self.load_error, self.selected_count()) {
                (Some(_), _) => "Could not read shortcuts.vdf: see the message".to_string(),
                (None, _) if self.account.is_none() => "Choose a Steam account first".to_string(),
                (None, 0) if self.rows.is_empty() => "No installed ports to add".to_string(),
                (None, 0) => "Nothing selected to add or update".to_string(),
                (None, n) if self.can_queue(steam) => {
                    let until = if self.game_mode { "for Desktop Mode" } else { "until Steam is closed" };
                    format!("Queue {n} change{} {until}...", if n == 1 { "" } else { "s" })
                }
                (None, 1) => "Review 1 change...".to_string(),
                (None, n) => format!("Review {n} changes..."),
            },
        }
    }

    pub fn labels(&self, steam: &dyn SteamProcessCheck) -> Vec<String> {
        self.lines().into_iter().map(|l| self.label(l, steam)).collect()
    }

    /// Ports that are not in Steam yet (or whose shortcut is out of date): the rows that would change the file,
    /// except a shortcut found by name only, which points at some other program and is left to the user.
    pub fn missing_count(&self) -> usize {
        self.rows.iter().filter(|r| r.can_change() && !r.by_name()).count()
    }

    /// Selects exactly those ports (the setup wizard's "Add all").
    pub fn select_all_missing(&mut self) {
        for row in &mut self.rows {
            row.selected = row.can_change() && !row.by_name();
        }
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

    /// Why "Review" cannot go ahead, as the message to show; `None` when it can.
    pub fn review_problem(&self, steam: &dyn SteamProcessCheck) -> Option<String> {
        if let Some(e) = &self.load_error {
            return Some(e.clone());
        }
        if self.account.is_none() {
            return Some("Choose your Steam account first (the first row).".to_string());
        }
        if self.selected_count() == 0 {
            return Some("Nothing is selected to add or update.".to_string());
        }
        match steam.running_steam_processes() {
            Ok(p) if p.is_empty() => None,
            Ok(p) => Some(close_steam_message(&p)),
            Err(_) => Some("Could not tell whether Steam is running, so nothing will be written.".to_string()),
        }
    }

    /// Steam is up (as in Game Mode), so the selected changes can be queued for later instead of written.
    /// Not when the check itself fails: there is no telling when Steam would be closed.
    /// In Game Mode always: nothing is written there, whatever the process check says.
    pub fn can_queue(&self, steam: &dyn SteamProcessCheck) -> bool {
        self.load_error.is_none()
            && self.account.is_some()
            && self.selected_count() > 0
            && (self.game_mode || steam.running_steam_processes().is_ok_and(|p| !p.is_empty()))
    }

    /// The text of the confirmation: what will be added, what will change, where the backup goes.
    pub fn review_text(&self) -> String {
        let mut added = Vec::new();
        let mut updated = Vec::new();
        for item in self.selected_items() {
            match &item.action {
                Action::Add => added.push(format!("  {}", item.wanted.app_name)),
                Action::Update { changes, by_name, .. } => {
                    let fields: Vec<&str> = changes.iter().map(|c| c.field).collect();
                    let detail = match (by_name, changes.iter().find(|c| c.field == "program")) {
                        (true, Some(c)) => format!("now {}: {}", program_name(&c.from), fields.join(", ")),
                        _ => fields.join(", "),
                    };
                    updated.push(format!("  {} ({detail})", item.wanted.app_name));
                }
                _ => {}
            }
        }
        let mut out = String::new();
        if !added.is_empty() {
            out.push_str(&format!("Add {}:\n{}\n\n", added.len(), listed(&added)));
        }
        if !updated.is_empty() {
            out.push_str(&format!("Update {} (play time stays):\n{}\n\n", updated.len(), listed(&updated)));
        }
        out.push_str("shortcuts.vdf is backed up first.");
        out
    }
}

/// The file name of a program path, for short labels.
fn program_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().filter(|n| !n.is_empty()).unwrap_or(path)
}

/// At most `MAX_LISTED` lines, so a long list cannot outgrow its dialog.
const MAX_LISTED: usize = 6;

pub(crate) fn listed(lines: &[String]) -> String {
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

/// Places the artwork for each wanted shortcut and returns what happened, one line per port that needs
/// mentioning. Sets `WantedShortcut::icon` when an icon of ours is in place. Never overwrites art the user
/// chose (the writer decides), never touches `shortcuts.vdf`.
///
/// The pictures come from the art resolver (`art::resolve`) in `Mode::CacheOnly`: the catalog's cached
/// portrait and icon, plus, when an art source is set up under Settings > Artwork sources, whatever its
/// Fetch step saved (hero, logo, wide, better portrait and icon). This runs on the UI thread, so it never
/// touches the network; a port nothing was fetched for just gets the catalog art, exactly as before. With
/// no source set up the resolver returns the catalog art unchanged. One `Session` serves the whole batch.
/// A port with nothing cached at all gets a note, not an error: the catalog images are fetched in the
/// background when the screen opens. Only a shortcut with no icon of its own gets ours; an existing one is kept.
fn place_catalog_art(screen: &SteamScreen, grid_dir: &Path, items: &[PlanItem], taken_ids: &[u32], wanted: &mut [WantedShortcut], no_art: &[String]) -> Vec<String> {
    let Some(cache_dir) = &screen.cache_dir else { return Vec::new() };
    if !grid_dir.is_absolute() {
        return vec!["  skipped: the Steam folder path is not absolute".to_string()];
    }
    // `cache/` sits in the app folder, next to `secrets.json` and `ports.local.json`.
    let config_dir = cache_dir.parent().unwrap_or(cache_dir);
    let session = art::resolve::Session::open(art::resolve::ArtDirs { config_dir, cache_dir }, art::resolve::Mode::CacheOnly);
    let mut notes = Vec::new();
    for item in items {
        if !item.changes_the_file() || no_art.contains(&item.wanted.port_key) {
            continue;
        }
        let Some(app_id) = steam_sync::expected_app_id(item) else { continue };
        // A new entry whose ID another shortcut already stores will be refused; its art would land on that one.
        if matches!(item.action, Action::Add) && taken_ids.contains(&app_id) {
            continue;
        }
        let Some(row) = screen.rows.iter().find(|r| r.key == item.wanted.port_key) else { continue };
        let portrait = art::catalog::cached_portrait_path(cache_dir, &row.folder);
        let icon = crate::core::image_cache::cached_image_path(cache_dir, &format!("{}_icon", row.folder)).ok().filter(|p| p.is_file());
        let reference = art::resolve::PortRef { key: &row.key, folder: &row.folder, name: &row.name, instructions: Some(&row.instructions) };
        // Passing the grid folder and app ID limits the lookup to the slots that want art. Art the user
        // chose for a slot is never touched, and a hero they chose means no hero is resolved and so no
        // wide capsule is cropped from one either (a wide image from a source of its own is still placed).
        let found = session.resolve_port(config_dir, reference, portrait.as_deref(), icon.as_deref(), Some((grid_dir, app_id)));
        if found.set.is_empty() && found.problems.is_empty() && portrait.is_none() && icon.is_none() {
            notes.push(format!("  {}: no cached artwork yet", row.name));
            continue;
        }
        let mut problems: Vec<String> = found.problems.iter().map(|(slot, e)| format!("{}: {e}", slot.name())).collect();
        // `refresh_own`: a picture this app composed earlier may be replaced by a better one; anything
        // the user put there is never replaced, whatever the option says.
        match art::grid::write_art(grid_dir, app_id, &found.set, &art::grid::WriteOptions { refresh_own: true }) {
            Ok(report) => {
                if let Some(slot) = wanted.iter_mut().find(|w| w.port_key == item.wanted.port_key) {
                    if let Some(icon_path) = report.icon_path.as_ref().and_then(|p| p.to_str()).filter(|p| !p.contains('"')) {
                        slot.icon = Some(icon_path.to_string());
                    }
                }
                problems.extend(report.failures().into_iter().map(|(slot, why)| format!("{}: {why}", slot.name())));
                if let Some(e) = &report.manifest_error {
                    problems.push(format!("could not record it: {e}"));
                }
            }
            Err(e) => problems.push(e.to_string()),
        }
        if !problems.is_empty() {
            notes.push(format!("  {}: {}", row.name, problems.join("; ")));
        }
    }
    notes
}

/// Writes the selected rows. Loads the file again, so edits Steam or another tool made since the
/// screen opened are not lost; the plan is rebuilt from that fresh copy.
pub fn apply_selected(screen: &SteamScreen, steam: &dyn SteamProcessCheck, settings: &ForkSettings, now: SystemTime) -> Result<ApplyOutcome, String> {
    apply_selected_with(screen, steam, settings, now, &[])
}

/// `apply_selected`, placing no artwork for the port keys in `no_art` (a queued change can ask for none).
pub fn apply_selected_with(screen: &SteamScreen, steam: &dyn SteamProcessCheck, settings: &ForkSettings, now: SystemTime, no_art: &[String]) -> Result<ApplyOutcome, String> {
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
    // Before any artwork is placed: with Steam up nothing at all should change.
    match check_steam(steam) {
        Ok(()) => {}
        Err(StoreError::SteamRunning(procs)) => return Err(close_steam_message(&procs)),
        Err(e) => return Err(format!("{e}\n\nNothing was changed.")),
    }
    let mut file = ShortcutsFile::load(&path).map_err(|e| e.to_string())?;

    // Art goes in before the write so the icon path can ride along with the shortcut.
    let mut art_notes = Vec::new();
    if let Some(grid) = screen.grid_dir() {
        let first = steam_sync::plan(&file.doc, &wanted);
        let taken: Vec<u32> = super::shortcuts::list(&file.doc).iter().filter_map(|s| s.app_id).collect();
        art_notes = place_catalog_art(screen, &grid, &first, &taken, &mut wanted, no_art);
    }
    let items = steam_sync::plan(&file.doc, &wanted);

    let report = match steam_sync::apply(&mut file, &items, steam, &backup_stamp()) {
        Ok(report) => report,
        Err(steam_sync::SyncError::Store(StoreError::SteamRunning(procs))) => return Err(close_steam_message(&procs)),
        Err(e) => return Err(format!("{e}\n\nNothing was changed in shortcuts.vdf.")),
    };

    let pruned = if report.save.wrote { backups::prune_backups(&path, settings.backups_keep(), now).map(|r| r.deleted.len()).unwrap_or(0) } else { 0 };
    let mut text = format!("Added {}, updated {}, already fine {}.", report.added.len(), report.updated.len(), report.unchanged.len());
    if !report.skipped.is_empty() {
        let lines: Vec<String> = report
            .skipped
            .iter()
            .map(|(key, why)| {
                let name = screen.rows.iter().find(|r| &r.key == key).map_or(key.as_str(), |r| r.name.as_str());
                format!("  {name}: {why}")
            })
            .collect();
        text.push_str(&format!("\nLeft out {}:\n{}", lines.len(), listed(&lines)));
    }
    if !art_notes.is_empty() {
        text.push_str(&format!("\nArtwork:\n{}", listed(&art_notes)));
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

    fn put_exe(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// A fake home with a Steam folder (account 1234, signed in last), a library with three ports and
    /// a shortcuts file that already has a hand-made "Ship of Harkinian" pointing at an old path.
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
        put_exe(&library.join("twoprog").join("a.exe"));
        put_exe(&library.join("twoprog").join("b.exe"));
        (home, library, path)
    }

    #[test]
    fn the_screen_lists_installed_ports_with_what_would_happen_to_each() {
        let (home, library, _) = fake_home("screen");
        let screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        let labels = screen.labels(&no_steam);
        assert_eq!(labels[0], "Steam account: Account 1234");
        assert_eq!(labels[1], "Steam is closed: ready to write");
        assert_eq!(labels[2], "[x] Dusklight: will add");
        assert_eq!(labels[3], "[ ] Ship of Harkinian: same name as a shortcut for soh.appimage, will repoint it");
        assert!(labels[4].starts_with("Two Programs: skipped, several programs found"));
        assert_eq!(labels.last().unwrap(), "Review 1 change...");
        assert_eq!(labels.len(), 6, "the port that is not installed is not listed");
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn toggling_changes_the_count_and_unchangeable_rows_ignore_it() {
        let (home, library, _) = fake_home("toggle");
        let mut screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        assert_eq!(screen.selected_count(), 1);
        assert_eq!(screen.label(SteamLine::Review, &no_steam), "Review 1 change...");
        screen.toggle(2);
        assert!(screen.rows[2].plan.is_none() && !screen.rows[2].selected);
        screen.toggle(1);
        assert_eq!(screen.label(SteamLine::Review, &no_steam), "Review 2 changes...");
        screen.toggle(0);
        screen.toggle(1);
        assert_eq!(screen.label(SteamLine::Review, &no_steam), "Nothing selected to add or update");
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn the_review_text_names_what_is_added_and_updated() {
        let (home, library, _) = fake_home("review");
        let mut screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        let text = screen.review_text();
        assert!(text.contains("Add 1:\n  Dusklight"));
        assert!(!text.contains("Update"), "the name-only match is off until switched on");
        screen.toggle(1);
        let text = screen.review_text();
        assert!(text.contains("Update 1 (play time stays):\n  Ship of Harkinian (now soh.appimage: program, start folder)"), "{text}");
        assert!(text.contains("backed up first"));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn applying_writes_a_backup_keeps_the_id_and_says_to_restart_steam() {
        let (home, library, path) = fake_home("apply");
        let before = fs::read(&path).unwrap();
        let mut screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        screen.toggle(1);
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

    #[test]
    fn applying_while_steam_runs_gives_the_close_steam_message_and_touches_nothing() {
        let (home, library, path) = fake_home("running");
        let before = fs::read(&path).unwrap();
        let screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        let running = || -> std::io::Result<Vec<String>> { Ok(vec!["99 steam".into()]) };
        assert_eq!(screen.label(SteamLine::Status, &running), "Steam is running: close it to write, or queue the changes");
        let err = apply_selected(&screen, &running, &ForkSettings::load(&home.join("s.json")), SystemTime::now()).unwrap_err();
        assert!(err.starts_with(CLOSE_STEAM_HELP));
        assert!(err.contains("Still running: 99 steam"), "{err}");
        assert!(err.contains("Desktop Mode") && err.contains("Game Mode"));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(backups::list_backups(&path).unwrap().is_empty());
        let _ = fs::remove_dir_all(home);
    }

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

    #[test]
    fn an_unreadable_shortcuts_file_is_reported_and_blocks_review() {
        let (home, library, path) = fake_home("garbled");
        fs::write(&path, b"\x00shortcuts\x00\x09bad\x00").unwrap();
        let screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        assert!(screen.load_error.is_some() && screen.rows.is_empty());
        assert!(screen.label(SteamLine::Review, &no_steam).starts_with("Could not read shortcuts.vdf"));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn two_accounts_without_a_clear_choice_ask_which_and_the_choice_sticks() {
        let (home, library, _) = fake_home("two_accounts");
        let steam = home.join(".local/share/Steam");
        fs::create_dir_all(steam.join("userdata").join("5678").join("config")).unwrap();
        let settings_path = home.join("s.json");
        let mut screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&settings_path), &|_| None);
        assert!(screen.account.is_none() && screen.rows.is_empty());
        assert_eq!(screen.label(SteamLine::Account, &no_steam), "Choose your Steam account");
        assert_eq!(screen.label(SteamLine::Review, &no_steam), "Choose a Steam account first");
        assert!(screen.review_problem(&no_steam).unwrap().contains("Choose your Steam account first"));
        let index = screen.accounts.iter().position(|a| a.account_id == 5678).unwrap();
        screen.set_account(index, &library, &catalog(), &|_| None);
        assert_eq!(screen.label(SteamLine::Account, &no_steam), "Steam account: Account 5678 (choose another)");
        // With no shortcuts file yet, every installed port is an add.
        assert_eq!(screen.rows.len(), 3);
        assert!(screen.rows.iter().filter(|r| r.plan.is_some()).all(|r| matches!(r.plan.as_ref().unwrap().action, Action::Add)));
        let mut settings = ForkSettings::load(&settings_path);
        settings.set_steam_account(5678).unwrap();
        let again = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&settings_path), &|_| None);
        assert_eq!(again.active_account().unwrap().account_id, 5678);
        let _ = fs::remove_dir_all(home);
    }


    #[test]
    fn a_shortcut_found_by_name_only_is_off_by_default_and_the_label_says_what_it_points_at() {
        use super::super::vdf::fixtures::*;
        let (home, library, path) = fake_home("by_name");
        // Another program's shortcut that happens to share the port's name.
        let entry = steam_entry(0, 0xAAAA_0001, "Dusklight", "\"/usr/bin/other-launcher\"", "\"/usr/bin/\"", &[]);
        fs::write(&path, file_of(&[entry])).unwrap();
        let mut screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        let labels = screen.labels(&no_steam);
        assert_eq!(labels[2], "[ ] Dusklight: same name as a shortcut for other-launcher, will repoint it");
        assert_eq!(screen.selected_count(), 1, "only the ordinary add is on");
        assert!(!screen.review_text().contains("Dusklight"));
        screen.toggle(0);
        assert!(screen.review_text().contains("Dusklight (now other-launcher: program, start folder)"));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn a_start_folder_that_differs_only_by_a_trailing_slash_is_not_a_change() {
        let (home, library, path) = fake_home("slash");
        // The port is already in Steam the way Steam writes it: quoted program, start folder with a slash.
        let dusk = library.join("dusklight").join("dusk");
        let dir = format!("{}/", library.join("dusklight").to_str().unwrap());
        let entry = super::super::vdf::fixtures::steam_entry(0, 0xAAAA_0002, "Dusklight", &format!("\"{}\"", dusk.to_str().unwrap()), &format!("\"{dir}\""), &[]);
        fs::write(&path, super::super::vdf::fixtures::file_of(&[entry])).unwrap();
        let screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        assert_eq!(screen.labels(&no_steam)[2], "Dusklight: already in Steam");
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn a_library_path_with_spaces_goes_all_the_way_through_and_reads_as_up_to_date_afterwards() {
        let (home, library, path) = fake_home("path with spaces");
        let screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        apply_selected(&screen, &no_steam, &ForkSettings::load(&home.join("s.json")), SystemTime::now()).unwrap();
        let saved = super::super::shortcuts::list(&ShortcutsFile::load(&path).unwrap().doc);
        let dusk = saved.iter().find(|s| s.app_name == "Dusklight").unwrap();
        assert!(dusk.exe_path().contains("path with spaces"));
        let again = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        assert_eq!(again.labels(&no_steam)[2], "Dusklight: already in Steam");
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn steam_starting_after_the_screen_opened_refuses_at_apply_and_names_the_process() {
        let (home, library, path) = fake_home("late");
        let before = fs::read(&path).unwrap();
        let cache = home.join("cache");
        fs::create_dir_all(&cache).unwrap();
        image::RgbaImage::from_pixel(600, 900, image::Rgba([200, 30, 30, 255])).save(cache.join("dusklight.png")).unwrap();
        let mut screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        screen.cache_dir = Some(cache);
        assert!(screen.review_problem(&no_steam).is_none());
        let started = || -> std::io::Result<Vec<String>> { Ok(vec!["31 steamwebhelper".into()]) };
        assert!(screen.review_problem(&started).unwrap().contains("Still running: 31 steamwebhelper"));
        let err = apply_selected(&screen, &started, &ForkSettings::load(&home.join("s.json")), SystemTime::now()).unwrap_err();
        assert!(err.contains("Still running: 31 steamwebhelper"));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!path.parent().unwrap().join("grid").exists(), "no artwork was placed either");
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn art_is_not_placed_for_a_new_entry_whose_id_another_shortcut_already_stores() {
        let (home, library, path) = fake_home("art_taken");
        let cache = home.join("cache");
        fs::create_dir_all(&cache).unwrap();
        image::RgbaImage::from_pixel(600, 900, image::Rgba([200, 30, 30, 255])).save(cache.join("dusklight.png")).unwrap();
        let mut screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        screen.cache_dir = Some(cache);
        // Another shortcut (renamed and repointed in Steam) already stores the ID Dusklight would get.
        let wanted = screen.rows[0].wanted.as_ref().unwrap();
        let id = super::super::app_id::shortcut_app_id(&format!("\"{}\"", wanted.exe), "Dusklight");
        let entry = super::super::vdf::fixtures::steam_entry(1, id, "Renamed", "\"/else/where\"", "\"/else/\"", &[]);
        fs::write(&path, super::super::vdf::fixtures::file_of(&[entry])).unwrap();
        let out = apply_selected(&screen, &no_steam, &ForkSettings::load(&home.join("s.json")), SystemTime::now()).unwrap();
        assert!(out.text.contains("Left out 1") && out.text.contains("Dusklight"), "{}", out.text);
        assert!(!path.parent().unwrap().join("grid").join(format!("{id}p.png")).exists());
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn review_cannot_start_when_the_steam_check_fails() {
        let (home, library, _) = fake_home("review_checkfail");
        let screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        let broken = || -> std::io::Result<Vec<String>> { Err(std::io::Error::other("no /proc")) };
        let message = screen.review_problem(&broken).unwrap();
        assert!(message.contains("Could not tell whether Steam is running"), "{message}");
        assert!(!message.contains("Steam is running, so nothing was changed"), "must not claim Steam is running");
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn cached_catalog_art_is_placed_before_the_write_and_its_icon_goes_into_the_shortcut() {
        let (home, library, path) = fake_home("art");
        let cache = home.join("cache");
        fs::create_dir_all(&cache).unwrap();
        image::RgbaImage::from_pixel(600, 900, image::Rgba([200, 30, 30, 255])).save(cache.join("dusklight.png")).unwrap();
        image::RgbaImage::from_pixel(128, 128, image::Rgba([30, 30, 200, 255])).save(cache.join("dusklight_icon.png")).unwrap();
        let mut screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        screen.cache_dir = Some(cache);
        screen.toggle(1);
        let out = apply_selected(&screen, &no_steam, &ForkSettings::load(&home.join("s.json")), SystemTime::now()).unwrap();
        let saved = super::super::shortcuts::list(&ShortcutsFile::load(&path).unwrap().doc);
        let dusk = saved.iter().find(|s| s.app_name == "Dusklight").unwrap();
        let id = dusk.app_id.unwrap();
        let grid = path.parent().unwrap().join("grid");
        assert!(grid.join(format!("{id}p.png")).is_file(), "portrait placed under the shortcut's app ID");
        assert!(grid.join(format!("{id}_icon.png")).is_file());
        assert_eq!(dusk.icon, grid.join(format!("{id}_icon.png")).to_str().unwrap());
        // The other port has nothing cached: told, not failed.
        assert!(out.text.contains("Artwork:") && out.text.contains("Ship of Harkinian: no cached artwork yet"), "{}", out.text);
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn art_the_user_chose_is_left_alone_when_adding_to_steam() {
        let (home, library, path) = fake_home("art_user");
        let cache = home.join("cache");
        fs::create_dir_all(&cache).unwrap();
        image::RgbaImage::from_pixel(600, 900, image::Rgba([200, 30, 30, 255])).save(cache.join("dusklight.png")).unwrap();
        let mut screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        screen.cache_dir = Some(cache);
        // The ID the new shortcut will get, and a cover the user already put there.
        let wanted = screen.rows[0].wanted.as_ref().unwrap();
        let id = super::super::app_id::shortcut_app_id(&format!("\"{}\"", wanted.exe), "Dusklight");
        let grid = path.parent().unwrap().join("grid");
        fs::create_dir_all(&grid).unwrap();
        fs::write(grid.join(format!("{id}p.png")), b"the user's own cover").unwrap();
        apply_selected(&screen, &no_steam, &ForkSettings::load(&home.join("s.json")), SystemTime::now()).unwrap();
        assert_eq!(fs::read(grid.join(format!("{id}p.png"))).unwrap(), b"the user's own cover");
        let _ = fs::remove_dir_all(home);
    }

    // -- Online art sources ----------------------------------------------------------------------

    fn png_bytes(w: u32, h: u32, c: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbaImage::from_pixel(w, h, image::Rgba(c)).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    /// What an earlier Fetch leaves behind for Dusklight: its SteamGridDB game id and a hero, a wide
    /// capsule and a logo in the image cache. A key is saved only when `with_key`.
    fn fetched_art(home: &Path, cache: &Path, portrait_key: &str, with_key: bool) {
        use super::super::art::cache::{ArtCache, IdEntry, IdOrigin, IdTable};
        use super::super::art::fit::Slot;
        use super::super::art::source::SourceId;
        use super::super::art::title::Confidence;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let art_cache = ArtCache::new(cache);
        let mut logo = image::RgbaImage::from_pixel(800, 300, image::Rgba([255, 255, 255, 255]));
        for x in 0..800 {
            logo.put_pixel(x, 0, image::Rgba([0, 0, 0, 0]));
        }
        let mut logo_png = Vec::new();
        logo.write_to(&mut std::io::Cursor::new(&mut logo_png), image::ImageFormat::Png).unwrap();
        art_cache.image_put(SourceId::SteamGridDb, 900001, Slot::Hero, &png_bytes(1920, 620, [9, 90, 9, 255]), 1, now).unwrap();
        art_cache.image_put(SourceId::SteamGridDb, 900001, Slot::Wide, &png_bytes(920, 430, [90, 9, 9, 255]), 2, now).unwrap();
        art_cache.image_put(SourceId::SteamGridDb, 900001, Slot::Logo, &logo_png, 3, now).unwrap();
        let mut table = IdTable::load(&art_cache.ids_path());
        table.set_found(portrait_key, SourceId::SteamGridDb, IdEntry { id: 900001, name: "Dusklight".into(), origin: IdOrigin::Search, confidence: Confidence::High }, now);
        table.save().unwrap();
        if with_key {
            let mut secrets = super::super::secrets::Secrets::load(&home.join("secrets.json"));
            secrets.set(super::super::secrets::Field::SteamGridDbKey, Some("sgdb-test-key-abcdef123456")).unwrap();
        }
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| !n.starts_with(".dpm-art-")).collect();
        names.sort();
        names
    }

    #[test]
    fn with_a_key_set_up_add_to_steam_places_the_fetched_hero_logo_and_wide_beside_the_catalog_art() {
        let (home, library, path) = fake_home("art_fetched");
        let cache = home.join("cache");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("dusklight.png"), png_bytes(600, 900, [200, 30, 30, 255])).unwrap();
        fs::write(cache.join("dusklight_icon.png"), png_bytes(128, 128, [30, 30, 200, 255])).unwrap();
        let mut screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        fetched_art(&home, &cache, &screen.rows[0].key.clone(), true);
        screen.cache_dir = Some(cache);
        apply_selected(&screen, &no_steam, &ForkSettings::load(&home.join("s.json")), SystemTime::now()).unwrap();
        let saved = super::super::shortcuts::list(&ShortcutsFile::load(&path).unwrap().doc);
        let dusk = saved.iter().find(|s| s.app_name == "Dusklight").unwrap();
        let id = dusk.app_id.unwrap();
        let grid = path.parent().unwrap().join("grid");
        assert_eq!(files_in(&grid), vec![format!("{id}.json"), format!("{id}.png"), format!("{id}_hero.png"), format!("{id}_icon.png"), format!("{id}_logo.png"), format!("{id}p.png")]);
        assert_eq!(image::image_dimensions(grid.join(format!("{id}_hero.png"))).unwrap(), (1920, 620));
        assert_eq!(image::image_dimensions(grid.join(format!("{id}.png"))).unwrap(), (920, 430));
        assert_eq!(dusk.icon, grid.join(format!("{id}_icon.png")).to_str().unwrap(), "the icon still goes into the shortcut");
        let manifest = super::super::art::grid::Manifest::load(&grid);
        assert_eq!(manifest.entries()[&format!("{id}_hero.png")].source, "steamgriddb");
        assert_eq!(manifest.entries()[&format!("{id}p.png")].source, "catalog");
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn with_no_key_the_same_cache_gives_exactly_the_old_catalog_art_and_nothing_else() {
        let (home, library, path) = fake_home("art_nokey");
        let cache = home.join("cache");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("dusklight.png"), png_bytes(600, 900, [200, 30, 30, 255])).unwrap();
        fs::write(cache.join("dusklight_icon.png"), png_bytes(128, 128, [30, 30, 200, 255])).unwrap();
        let mut screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        fetched_art(&home, &cache, &screen.rows[0].key.clone(), false);
        screen.cache_dir = Some(cache.clone());
        apply_selected(&screen, &no_steam, &ForkSettings::load(&home.join("s.json")), SystemTime::now()).unwrap();
        let saved = super::super::shortcuts::list(&ShortcutsFile::load(&path).unwrap().doc);
        let id = saved.iter().find(|s| s.app_name == "Dusklight").unwrap().app_id.unwrap();
        let grid = path.parent().unwrap().join("grid");
        assert_eq!(files_in(&grid), vec![format!("{id}_icon.png"), format!("{id}p.png")], "no hero, logo or wide without a key");
        let old = art::catalog::catalog_art_from_files(Some(&cache.join("dusklight.png")), Some(&cache.join("dusklight_icon.png")));
        assert_eq!(fs::read(grid.join(format!("{id}p.png"))).unwrap(), old.portrait().unwrap().to_png().unwrap(), "byte for byte what the catalog-only code placed");
        assert_eq!(fs::read(grid.join(format!("{id}_icon.png"))).unwrap(), old.icon().unwrap().to_png().unwrap());
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn a_hero_the_user_chose_is_kept_and_no_wide_capsule_is_cropped_from_the_fetched_one() {
        let (home, library, path) = fake_home("art_user_hero");
        let cache = home.join("cache");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("dusklight.png"), png_bytes(600, 900, [200, 30, 30, 255])).unwrap();
        let mut screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        fetched_art(&home, &cache, &screen.rows[0].key.clone(), true);
        // Take the fetched wide capsule away: the only way to get a wide would be to crop the hero.
        let _ = fs::remove_file(cache.join("art/images/steamgriddb/900001-wide.img"));
        let _ = fs::remove_file(cache.join("art/images/steamgriddb/900001-wide.json"));
        screen.cache_dir = Some(cache);
        let wanted = screen.rows[0].wanted.as_ref().unwrap();
        let id = super::super::app_id::shortcut_app_id(&format!("\"{}\"", wanted.exe), "Dusklight");
        let grid = path.parent().unwrap().join("grid");
        fs::create_dir_all(&grid).unwrap();
        fs::write(grid.join(format!("{id}_hero.jpg")), b"the user's own hero").unwrap();
        apply_selected(&screen, &no_steam, &ForkSettings::load(&home.join("s.json")), SystemTime::now()).unwrap();
        assert_eq!(fs::read(grid.join(format!("{id}_hero.jpg"))).unwrap(), b"the user's own hero");
        assert!(!grid.join(format!("{id}_hero.png")).exists());
        assert!(!grid.join(format!("{id}.png")).exists(), "no wide capsule cropped from a hero that is not the user's");
        assert!(grid.join(format!("{id}_logo.png")).is_file() && grid.join(format!("{id}p.png")).is_file());
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn a_composed_picture_of_ours_is_replaced_when_a_real_one_has_been_fetched() {
        let (home, library, path) = fake_home("art_refresh");
        let cache = home.join("cache");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("dusklight.png"), png_bytes(600, 900, [200, 30, 30, 255])).unwrap();
        let mut screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        let key = screen.rows[0].key.clone();
        let wanted = screen.rows[0].wanted.as_ref().unwrap();
        let id = super::super::app_id::shortcut_app_id(&format!("\"{}\"", wanted.exe), "Dusklight");
        let grid = path.parent().unwrap().join("grid");
        // An earlier run composed a wide capsule (a crop of wider art) and recorded it as ours.
        let composed = art::fit::fit(art::fit::Slot::Wide, &image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(1920, 1080, image::Rgba([1, 2, 3, 255])))).unwrap().with_source("steamgriddb");
        let mut set = art::grid::ArtSet::default();
        set.set(composed);
        art::grid::write_art(&grid, id, &set, &art::grid::WriteOptions::default()).unwrap();
        let before = fs::read(grid.join(format!("{id}.png"))).unwrap();
        fetched_art(&home, &cache, &key, true);
        screen.cache_dir = Some(cache);
        apply_selected(&screen, &no_steam, &ForkSettings::load(&home.join("s.json")), SystemTime::now()).unwrap();
        let after = fs::read(grid.join(format!("{id}.png"))).unwrap();
        assert_ne!(before, after, "the composed wide capsule was replaced by the real one");
        assert_eq!(image::load_from_memory(&after).unwrap().to_rgba8().get_pixel(3, 3)[0], 90);
        let _ = fs::remove_dir_all(home);
    }

    // -- Game Mode queue ------------------------------------------------------------------------

    #[test]
    fn with_steam_running_review_becomes_queue_and_queued_ports_are_marked() {
        use super::super::steam_queue::{ArtChoice, QueuedChange, QueuedKind};
        let (home, library, _) = fake_home("queue_lines");
        let mut screen = SteamScreen::load(&home, &library, &catalog(), &ForkSettings::load(&home.join("s.json")), &|_| None);
        let running = || -> std::io::Result<Vec<String>> { Ok(vec!["5 steam".into()]) };
        let broken = || -> std::io::Result<Vec<String>> { Err(std::io::Error::other("no /proc")) };
        assert!(screen.can_queue(&running) && !screen.can_queue(&no_steam) && !screen.can_queue(&broken));
        assert_eq!(screen.label(SteamLine::Review, &running), "Queue 1 change until Steam is closed...");
        assert_eq!(screen.label(SteamLine::Review, &no_steam), "Review 1 change...");
        assert_eq!(screen.label(SteamLine::Review, &broken), "Review 1 change...", "a failed check does not offer a queue");
        assert_eq!(screen.lines()[0], SteamLine::Account, "no queue rows while nothing is queued");

        screen.queue.upsert(QueuedChange {
            account_id: 1234,
            account_label: String::new(),
            port_key: "a/dusk".into(),
            name: "Dusklight".into(),
            kind: QueuedKind::Add,
            repoint_by_name: false,
            art: ArtChoice::Catalog,
            from_game_mode: false,
            queued_at: String::new(),
            repoint: None,
        });
        assert_eq!(screen.lines()[..3], [SteamLine::ApplyQueue, SteamLine::ManageQueue, SteamLine::Account]);
        assert_eq!(screen.label(SteamLine::ApplyQueue, &no_steam), "1 Steam change queued while Steam was running: apply now...");
        assert_eq!(screen.label(SteamLine::ApplyQueue, &running), "1 Steam change waiting: close Steam and update it now...");
        let labels = screen.labels(&running);
        assert!(labels.contains(&"[x] Dusklight: will add (queued)".to_string()), "{labels:?}");
        screen.queue_note = Some("kept aside".into());
        assert_eq!(screen.lines()[0], SteamLine::QueueNote);
        screen.game_mode = true;
        let game = "Game Mode: Steam can't be closed here, so changes are queued for Desktop Mode";
        assert_eq!((screen.label(SteamLine::Status, &running), screen.label(SteamLine::Status, &no_steam)), (game.to_string(), game.to_string()));
        assert!(screen.can_queue(&no_steam) && screen.can_queue(&broken), "Game Mode queues even if /proc shows no Steam");
        assert_eq!(screen.label(SteamLine::Review, &no_steam), "Queue 1 change for Desktop Mode...");
        assert_eq!(screen.label(SteamLine::ApplyQueue, &no_steam), "1 Steam change queued: will apply next time in Desktop Mode with Steam closed");
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
