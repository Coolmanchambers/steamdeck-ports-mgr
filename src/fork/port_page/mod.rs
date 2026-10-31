//! The Port page: everything about one installed port, one card per area, each card opening its own page. Pure data for
//! `fork::page` (what the pages say, the one-line summaries, where a card leads); the screen that gathers the facts and
//! runs the actions is `src/app/port_page_screen.rs`.
//!
//! **Looking changes nothing.** The pages only show what the app already keeps or what a read of the disk tells (the
//! worker gathers [`DiskFacts`]); every change is the existing flow behind a card (the save backup, the restore screen,
//! the save finder, Add to Steam, the icon repair, the uninstall with its guards...).
//!
//! The install wizard (part 3 of `docs/design/port-page-and-one-window.md`) reuses the card pages: see
//! `port_page_screen::open_card`.

pub mod paths;
pub mod settings_files;

#[cfg(test)]
mod tests;

use crate::fork::art::thumbs::ArtRef;
use crate::fork::page::{Card as PageCard, Focus, Icon, Mark, Page, PageButton, PageOption, LEFT};
use crate::fork::setup_wizard::files_state::FilesState;
use paths::{human_path, size_text, Places};
use settings_files::SettingsFile;
use std::path::{Path, PathBuf};

/// The first page's window title starts with this ("Port: Dusklight"); a card's page adds its title ("Port: Dusklight:
/// Saves"). The smoke tests look for them.
pub const TITLE: &str = "Port";

/// One card of the first page, in the order shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Card {
    Play,
    Fix,
    Program,
    Saves,
    Settings,
    GameFiles,
    Steam,
    Updates,
    Files,
    Uninstall,
    More,
}

impl Card {
    pub const ALL: [Card; 11] = [Card::Play, Card::Fix, Card::Program, Card::Saves, Card::Settings, Card::GameFiles, Card::Steam, Card::Updates, Card::Files, Card::Uninstall, Card::More];

    pub fn title(self) -> &'static str {
        match self {
            Card::Play => "Play",
            Card::Fix => "Fix problems",
            Card::Program => "Program to start",
            Card::Saves => "Saves",
            Card::Settings => "Port settings",
            Card::GameFiles => "Game files",
            Card::Steam => "Steam",
            Card::Updates => "Updates",
            Card::Files => "Files",
            Card::Uninstall => "Uninstall",
            Card::More => "More",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Card::Play => Icon::Play,
            Card::Fix => Icon::Fix,
            Card::Program => Icon::Program,
            Card::Saves => Icon::Saves,
            Card::Settings => Icon::TextFile,
            Card::GameFiles => Icon::GameFiles,
            Card::Steam => Icon::Steam,
            Card::Updates => Icon::Update,
            Card::Files => Icon::Folder,
            Card::Uninstall => Icon::Remove,
            Card::More => Icon::More,
        }
    }

    /// Where choosing the card leads.
    pub fn route(self) -> Route {
        match self {
            Card::Play => Route::Start,
            Card::Fix => Route::Checkup,
            Card::GameFiles => Route::GameFiles,
            _ => Route::Page,
        }
    }

    /// Needs the worker's facts before its page can be drawn.
    pub fn needs_disk(self) -> bool {
        matches!(self, Card::Fix | Card::Program | Card::Saves | Card::Settings | Card::Steam | Card::Files | Card::Uninstall)
    }
}

/// What a card does when chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Starts the port (the same as A on Home).
    Start,
    /// The Fix problems checkup for this port only.
    Checkup,
    /// The game files search for this port, its answer shown as the card's page.
    GameFiles,
    /// The card's own page.
    Page,
}

/// The window title of a card's page.
pub fn window_title(name: &str, card: Option<Card>) -> String {
    match card {
        None => format!("{TITLE}: {name}"),
        Some(c) => format!("{TITLE}: {name}: {}", c.title()),
    }
}

// -- Facts ----------------------------------------------------------------------------------------------------

/// What "Check for an update now" found this time the page was open.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateCheck {
    #[default]
    NotAsked,
    Checking,
    /// Up to date: the newest version's name.
    UpToDate(String),
    /// A newer version: its name.
    Ready(String),
    Failed(String),
}

/// What the app holds in memory about the port: read at once, no disk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortFacts {
    pub key: String,
    pub name: String,
    pub art: Option<ArtRef>,
    /// Home's badge for it ("Ready", "Update ready") and its mark.
    pub badge: String,
    pub badge_mark: Mark,
    /// "today", "yesterday", "3 days ago", "on 2026-10-01"; `None` when never played.
    pub last_played: Option<String>,
    pub playtime: Option<String>,
    pub running: bool,
    /// Installing, updating or having its saves restored now.
    pub busy: bool,
    /// The user's own entry (upstream shows "Open folder" for it and never uninstalls it).
    pub user_managed: bool,
    pub has_playtime: bool,
    /// The port's own update switch (upstream's Info > Update).
    pub update_on: bool,
    /// Follows GitHub or GitLab releases (so it can be checked, and a version picked).
    pub tracks_releases: bool,
    pub has_extra: bool,
    /// Home's check found a newer version.
    pub update_ready: bool,
    pub version: Option<String>,
    pub game_mode: bool,
    pub update_check: UpdateCheck,
    /// The port's project page (its GitHub or GitLab page, or its website); never a download link.
    pub project_url: Option<String>,
}

/// Where a save folder the app knows came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveSource {
    Catalog,
    Finder,
}

impl SaveSource {
    pub fn words(self) -> &'static str {
        match self {
            SaveSource::Catalog => "from the catalog",
            SaveSource::Finder => "chosen by you",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveFolder {
    pub path: PathBuf,
    pub source: SaveSource,
    pub exists: bool,
    /// Files in it (counted up to a limit).
    pub files: usize,
    pub capped: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SavesFacts {
    pub folders: Vec<SaveFolder>,
    /// The user told the save finder this game has no saves.
    pub no_saves: bool,
    /// Whole days since the newest backup that holds this port's saves; `None` when there is none.
    pub last_backup_days: Option<u64>,
    pub backups: usize,
    /// A save field has no known Linux location (the uninstall does not keep those).
    pub unknown_on_linux: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProgramFacts {
    pub game_dir: PathBuf,
    /// Every program found, shallowest folder first.
    pub candidates: Vec<PathBuf>,
    /// The remembered pick (relative to the port folder), and whether it is still there.
    pub favorite: Option<String>,
    pub favorite_found: bool,
    /// The catalog names the program itself.
    pub catalog_exe: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameFilesFacts {
    /// `None`: the port names no game files.
    pub state: Option<FilesState>,
    /// The connected files' names.
    pub connected: Vec<String>,
}

/// The port's shortcut in Steam (the active account), as Add to Steam would treat it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SteamState {
    NotInSteam,
    InSteam,
    /// In Steam but out of date (the fields that differ).
    NeedsUpdate(String),
    /// A shortcut with its name starts another program.
    OtherProgram(String),
    /// Add to Steam leaves it out (why).
    CannotAdd(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamFacts {
    /// `Err`: Steam (or its shortcuts) could not be read, and why.
    pub state: Result<SteamState, String>,
    /// What is wrong with the icon of a shortcut of this port (`None`: fine, or none).
    pub icon_problem: Option<String>,
    /// A shortcut still starts an old copy of the port.
    pub at_old_copy: bool,
    /// A change for it waits for Steam to close.
    pub queued: bool,
}

impl Default for SteamFacts {
    fn default() -> SteamFacts {
        SteamFacts { state: Err("not looked at yet".into()), icon_problem: None, at_old_copy: false, queued: false }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilesFacts {
    pub dir: PathBuf,
    pub size: Option<u64>,
    /// The size count stopped at its limit (the folder is at least this big).
    pub size_capped: bool,
    /// Older versions kept in `.previous`: their names and sizes.
    pub previous: Vec<(String, u64)>,
    /// What an uninstall moves to Kept files (`fork::uninstall_keep`): game files, keys and dumps, older versions.
    pub keep: crate::fork::uninstall_keep::KeepSummary,
    /// Where they go: `<Saves Backup>/Kept files/<port folder>`.
    pub kept_dir: PathBuf,
}

/// What the worker read from the disk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiskFacts {
    /// The port's Fix problems line ("Everything looks fine", "2 things to fix") and how many there are.
    pub checkup: Option<(String, usize)>,
    pub program: ProgramFacts,
    pub saves: SavesFacts,
    pub settings: Vec<SettingsFile>,
    pub game_files: GameFilesFacts,
    pub steam: SteamFacts,
    pub files: FilesFacts,
}

// -- Words ----------------------------------------------------------------------------------------------------

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// "backed up today", "backed up 3 days ago", "no backup yet".
pub fn backup_words(days: Option<u64>) -> String {
    match days {
        None => "no backup yet".into(),
        Some(0) => "backed up today".into(),
        Some(1) => "backed up yesterday".into(),
        Some(n) => format!("backed up {n} days ago"),
    }
}

/// "Last played yesterday", "Never played", "Running now".
pub fn play_summary(p: &PortFacts) -> String {
    if p.running {
        return "Running now".into();
    }
    if p.busy {
        return "Busy: installing, updating or restoring".into();
    }
    match &p.last_played {
        Some(when) => format!("Last played {when}"),
        None => "Never played".into(),
    }
}

/// The program Play starts, in a few words.
pub fn program_summary(f: &ProgramFacts) -> String {
    if let Some(fav) = &f.favorite {
        let name = file_name(Path::new(fav));
        return if f.favorite_found { name } else { format!("{name} is gone; asks every time") };
    }
    if let Some(exe) = &f.catalog_exe {
        return file_name(Path::new(exe));
    }
    let top = crate::fork::program_detect::shallowest(&f.game_dir, f.candidates.clone());
    match top.as_slice() {
        [] => "No program found".into(),
        [one] => file_name(one),
        _ => "Asks every time".into(),
    }
}

pub fn saves_summary(f: &SavesFacts, places: &Places) -> String {
    if f.no_saves {
        return "No saves (you said so)".into();
    }
    let Some(folder) = f.folders.iter().find(|s| s.exists && s.files > 0).or_else(|| f.folders.iter().find(|s| s.exists)) else {
        return if f.unknown_on_linux { "Not found yet (no Linux folder is known)".into() } else { "Not found yet".into() };
    };
    let place = human_path(&folder.path, places);
    if folder.files == 0 {
        return format!("{place}, no saves yet");
    }
    let files = if folder.capped { format!("{}+ files", folder.files) } else { count(folder.files, "file", "files") };
    format!("{place}, {files}, {}", backup_words(f.last_backup_days))
}

pub fn settings_summary(files: &[SettingsFile], game_dir: &Path) -> String {
    match files {
        [] => "None found".into(),
        [one] => {
            let where_ = if one.path.parent() == Some(game_dir) { "in the port folder" } else if one.path.starts_with(game_dir) { "in a folder of the port" } else { "with the saves" };
            format!("{} {where_}", file_name(&one.path))
        }
        many => format!("{} and {} more", file_name(&many[0].path), many.len() - 1),
    }
}

pub fn game_files_summary(f: &GameFilesFacts) -> String {
    match f.state {
        None => "Needs no game files".into(),
        Some(FilesState::Have) => match f.connected.first() {
            Some(name) => format!("Ready: {name}"),
            None => "Ready (in the port folder)".into(),
        },
        Some(FilesState::Missing) => "Add game files (the game will not start without them)".into(),
        Some(FilesState::Unverifiable) => "Add game files".into(),
    }
}

pub fn steam_summary(f: &SteamFacts) -> String {
    if f.queued {
        return "A change waits for Steam to close".into();
    }
    if f.at_old_copy {
        return "Steam still starts the old copy".into();
    }
    let state = match &f.state {
        Err(why) if why.contains("No Steam") => return "Steam was not found".into(),
        Err(_) => return "Steam could not be read".into(),
        Ok(s) => s,
    };
    match state {
        SteamState::NotInSteam => "Not in Steam".into(),
        SteamState::OtherProgram(_) => "Not in Steam (a shortcut with its name starts something else)".into(),
        SteamState::CannotAdd(why) => format!("Not in Steam: {why}"),
        SteamState::NeedsUpdate(_) => "In Steam, shortcut out of date".into(),
        SteamState::InSteam if f.icon_problem.is_some() => "In Steam, icon needs fixing".into(),
        SteamState::InSteam => "In Steam, icon fine".into(),
    }
}

pub fn files_summary(f: &FilesFacts, version: Option<&str>, places: &Places) -> String {
    let mut parts = vec![human_path(&f.dir, places)];
    if let Some(size) = f.size {
        parts.push(if f.size_capped { format!("over {}", size_text(size)) } else { size_text(size) });
    }
    if let Some(v) = version {
        parts.push(v.to_string());
    }
    parts.join(", ")
}

pub fn updates_summary(p: &PortFacts) -> String {
    if !p.tracks_releases {
        return "Not tracked: this port has no releases to check".into();
    }
    let checks = if p.update_on { "checks on" } else { "checks off" };
    match &p.update_check {
        UpdateCheck::Checking => "Checking...".into(),
        UpdateCheck::Ready(tag) => format!("Update ready: {tag}"),
        UpdateCheck::UpToDate(tag) => format!("Up to date ({tag}), {checks}"),
        _ if p.update_ready => "Update ready".into(),
        _ => match &p.version {
            Some(v) => format!("{v}, {checks}"),
            None => format!("Version not known, {checks}"),
        },
    }
}

pub fn uninstall_summary(p: &PortFacts) -> String {
    if p.user_managed {
        "Your own port: its folder is left alone".into()
    } else if p.running {
        "Close the game first".into()
    } else {
        "Remove the port from this Deck".into()
    }
}

/// The one line under a card. `disk` is `None` while the worker is still reading.
pub fn summary(card: Card, p: &PortFacts, disk: Option<&DiskFacts>, places: &Places) -> String {
    let looking = || "Looking...".to_string();
    match card {
        Card::Play => play_summary(p),
        Card::Fix => disk.and_then(|d| d.checkup.as_ref()).map_or_else(looking, |(s, _)| s.clone()),
        Card::Program => disk.map_or_else(looking, |d| program_summary(&d.program)),
        Card::Saves => disk.map_or_else(looking, |d| saves_summary(&d.saves, places)),
        Card::Settings => disk.map_or_else(looking, |d| settings_summary(&d.settings, &d.files.dir)),
        Card::GameFiles => disk.map_or_else(looking, |d| game_files_summary(&d.game_files)),
        Card::Steam => disk.map_or_else(looking, |d| steam_summary(&d.steam)),
        Card::Updates => updates_summary(p),
        Card::Files => disk.map_or_else(|| p.version.clone().unwrap_or_else(looking), |d| files_summary(&d.files, p.version.as_deref(), places)),
        Card::Uninstall => uninstall_summary(p),
        Card::More => "Project page, all details, reset game time".into(),
    }
}

// -- Pages ----------------------------------------------------------------------------------------------------

/// Everything a Port page can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// A card of the first page.
    Open(Card),
    /// B or the Back button: up one level.
    Back,
    /// Program to start: "Ask every time", or program `n` of the list.
    AskEveryTime,
    UseProgram(usize),
    BackUpNow,
    Restore,
    FindSaves,
    OpenSaveFolder,
    /// Port settings: file `n`'s View page; its folder; it in a text editor.
    View(usize),
    OpenFileFolder(usize),
    OpenInEditor(usize),
    SteamAdd,
    SteamRepoint,
    SteamFixIcon,
    SteamFinish,
    OpenInstallFolder,
    CheckNow,
    UpdateNow,
    ToggleUpdates,
    SelectVersion,
    InstallExtras,
    UninstallYes,
    ResetPlaytime,
    AllDetails,
    /// The project page: the link page (`link_page`).
    ProjectPage,
}

fn option(title: &str, detail: impl Into<String>, action: Act) -> PageOption<Act> {
    PageOption { title: title.to_string(), detail: detail.into(), choice: None, enabled: true, recommended: false, action }
}

fn disabled(mut o: PageOption<Act>) -> PageOption<Act> {
    o.enabled = false;
    o
}

fn back_buttons() -> [Option<PageButton<Act>>; 4] {
    [Some(PageButton { label: "Back".to_string(), action: Act::Back }), None, None, None]
}

/// A card's page: its window title, the port's name as the header, the card's title, a Back button.
fn card_page(p: &PortFacts, card: Card) -> Page<Act> {
    Page { window_title: window_title(&p.name, Some(card)), large: true, header: p.name.clone(), title: card.title().to_string(), buttons: back_buttons(), initial: Focus::Option(0), ..Page::default() }
}

/// "Only in Desktop Mode" for the cards that open a file manager or an editor (Game Mode shows one window at a time).
fn desktop_only(o: PageOption<Act>, game_mode: bool) -> PageOption<Act> {
    if game_mode {
        disabled(PageOption { detail: "Only in Desktop Mode".into(), ..o })
    } else {
        o
    }
}

/// The first page: the cover, the name and its badge, then one card per area with its live line.
pub fn first_page(p: &PortFacts, disk: Option<&DiskFacts>, places: &Places) -> Page<Act> {
    // The name is on the card under the title; the title says what the page is.
    let mut page = Page::<Act> { window_title: window_title(&p.name, None), large: true, header: String::new(), title: "Port options".to_string(), buttons: back_buttons(), initial: Focus::Option(0), ..Page::default() };
    let mut lines = vec![p.badge.clone()];
    if let Some(t) = &p.playtime {
        lines.push(format!("Played {t}"));
    }
    page.card = Some(PageCard { art: p.art.clone(), qr: None, title: p.name.clone(), lines });
    for (i, card) in Card::ALL.into_iter().enumerate() {
        let mut o = option(card.title(), summary(card, p, disk, places), Act::Open(card));
        if card == Card::Play && (p.busy || p.running) {
            o.enabled = false;
        }
        if card == Card::Fix && disk.and_then(|d| d.checkup.as_ref()).is_some_and(|(_, n)| *n > 0) {
            o.recommended = false;
        }
        page.options.push(o);
        page = page.icon(i, card.icon());
    }
    page
}

/// Program to start: "Ask every time" and every program found (shallowest first), the one in use marked; each with its
/// whole place, the human way.
pub fn program_page(p: &PortFacts, f: &ProgramFacts, places: &Places) -> Page<Act> {
    let mut page = card_page(p, Card::Program);
    page.paragraphs.push("Which program starts when you choose Play. Your choice is remembered.".into());
    if let Some(exe) = &f.catalog_exe {
        page.paragraphs.push(format!("The catalog names {} as the program; a choice here wins over it.", file_name(Path::new(exe))));
    }
    let current = f.favorite.as_deref().filter(|_| f.favorite_found);
    let mut ask = option("Ask every time", "Play asks which program to start when there is more than one", Act::AskEveryTime);
    ask.choice = Some(current.is_none());
    page.options.push(ask);
    for (i, c) in f.candidates.iter().take(MAX_PROGRAMS).enumerate() {
        let rel = crate::fork::program_detect::relative_pick(&f.game_dir, c);
        let mut o = option(&file_name(c), human_path(c, places), Act::UseProgram(i));
        o.choice = Some(rel.is_some() && rel.as_deref() == current);
        page.options.push(o);
    }
    if f.candidates.is_empty() {
        page.paragraphs.push("No program was found in the port's folder. Reinstall the port, or look at its files in Desktop Mode.".into());
    } else if f.candidates.len() > MAX_PROGRAMS {
        page.footnote = format!("{} more programs are not shown (the ones deepest in its folders).", f.candidates.len() - MAX_PROGRAMS);
    }
    if f.favorite.is_some() && !f.favorite_found {
        page.fact(Mark::Problem, format!("The program you chose ({}) is not there any more", f.favorite.as_deref().unwrap_or_default()));
    }
    let chosen = page.options.iter().position(|o| o.choice == Some(true)).unwrap_or(0);
    page.initial = Focus::Option(chosen);
    page
}

/// At most this many programs are listed on Program to start.
pub const MAX_PROGRAMS: usize = 30;

pub fn saves_page(p: &PortFacts, f: &SavesFacts, places: &Places) -> Page<Act> {
    let mut page = card_page(p, Card::Saves);
    if f.no_saves {
        page.fact(Mark::Done, "You said this game has no saves");
    }
    for s in &f.folders {
        let files = if !s.exists {
            "not there yet".to_string()
        } else if s.capped {
            format!("{}+ files", s.files)
        } else {
            count(s.files, "file", "files")
        };
        let mark = if s.exists && s.files > 0 { Mark::Done } else { Mark::Waiting };
        page.fact(mark, format!("{} ({}, {files})", human_path(&s.path, places), s.source.words()));
    }
    if f.folders.is_empty() && !f.no_saves {
        page.fact(Mark::ToDo, if f.unknown_on_linux { "Not found yet: the catalog knows no Linux folder for them" } else { "Not found yet" });
    }
    page.fact(if f.last_backup_days.is_some() { Mark::Done } else { Mark::ToDo }, {
        let w = backup_words(f.last_backup_days);
        let mut c = w.chars();
        c.next().map(|x| x.to_uppercase().collect::<String>() + c.as_str()).unwrap_or(w)
    });
    let has_files = f.folders.iter().any(|s| s.exists && s.files > 0);
    let back_up = option("Back up this port now", "Copies its saves into a zip in Saves Backup; nothing is changed", Act::BackUpNow);
    page.options.push(if has_files { back_up } else { disabled(PageOption { detail: "No saves to copy yet".into(), ..back_up }) });
    let restore = option("Restore", if f.backups == 0 { "No backups of this port yet".to_string() } else { format!("Put an earlier backup back ({})", count(f.backups, "backup", "backups")) }, Act::Restore);
    page.options.push(if f.backups == 0 { disabled(restore) } else { restore });
    let found = f.folders.iter().any(|s| s.exists);
    page.options.push(option(if found { "Find saves or choose the folder" } else { "Find saves" }, "Look for the game's save folder, or pick it yourself", Act::FindSaves));
    let open = option("Open the folder", "Show the save folder in the file manager", Act::OpenSaveFolder);
    page.options.push(if found { desktop_only(open, p.game_mode) } else { disabled(open) });
    page.icons.clear();
    page
}

pub fn settings_page(p: &PortFacts, files: &[SettingsFile], places: &Places, now: std::time::SystemTime) -> Page<Act> {
    let mut page = card_page(p, Card::Settings);
    if files.is_empty() {
        page.paragraphs.push(format!("No settings files were found for {}: nothing in its folder or save folders looks like one.", p.name));
        page.paragraphs.push("Some ports keep their settings somewhere else. A port's entry in ports.local.json can name them with a \"settings\" field.".into());
        return page;
    }
    page.paragraphs.push("The port's own settings files. Looking at them changes nothing.".into());
    for (i, f) in files.iter().enumerate() {
        let folder = f.path.parent().map(|d| human_path(d, places)).unwrap_or_default();
        let changed = f.modified.map(|m| crate::fork::save_finder::search::ago(now, m)).unwrap_or_default();
        let detail = [folder, size_text(f.size), changed].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
        page.options.push(option(&file_name(&f.path), detail, Act::View(i)));
        page = page.icon(i, Icon::TextFile);
    }
    page
}

/// The read-only View page of a settings file: where it is first (so a photo of it carries the facts), then its text.
pub fn view_page(p: &PortFacts, index: usize, f: &SettingsFile, text: &Result<(String, bool), String>, places: &Places) -> Page<Act> {
    let mut page = card_page(p, Card::Settings);
    page.window_title = format!("{}: {}", window_title(&p.name, Some(Card::Settings)), file_name(&f.path));
    page.title = file_name(&f.path);
    page.paragraphs.push(human_path(&f.path, places));
    match text {
        Ok((body, cut)) => {
            page.paragraphs.push(if body.trim().is_empty() { "(The file is empty.)".to_string() } else { body.clone() });
            if *cut {
                page.footnote = "Only the start of the file is shown. Open it in a text editor in Desktop Mode to see it all.".into();
            }
        }
        Err(why) => page.paragraphs.push(why.clone()),
    }
    page.options.push(desktop_only(option("Open in a text editor", "Desktop Mode's editor; the app never edits it", Act::OpenInEditor(index)), p.game_mode));
    page.options.push(desktop_only(option("Open the folder", "Show it in the file manager", Act::OpenFileFolder(index)), p.game_mode));
    page.initial = Focus::Button(LEFT);
    page
}

pub fn steam_page(p: &PortFacts, f: &SteamFacts) -> Page<Act> {
    let mut page = card_page(p, Card::Steam);
    page.paragraphs.push(format!("{} in Steam, this port only. Steam must be closed to change shortcuts; the app backs them up first.", p.name));
    match &f.state {
        Err(why) => page.fact(Mark::Problem, why.clone()),
        Ok(SteamState::NotInSteam) => page.fact(Mark::ToDo, "Not in Steam yet"),
        Ok(SteamState::InSteam) => page.fact(Mark::Done, "In Steam"),
        Ok(SteamState::NeedsUpdate(fields)) => page.fact(Mark::ToDo, format!("In Steam, out of date ({fields})")),
        Ok(SteamState::OtherProgram(what)) => page.fact(Mark::ToDo, format!("A shortcut with its name starts {what}")),
        Ok(SteamState::CannotAdd(why)) => page.fact(Mark::Problem, format!("Cannot be added: {why}")),
    }
    if f.at_old_copy {
        page.fact(Mark::ToDo, "Steam still starts the old copy of it");
    }
    match &f.icon_problem {
        Some(what) => page.fact(Mark::ToDo, format!("Icon: {what}")),
        None if matches!(f.state, Ok(SteamState::InSteam | SteamState::NeedsUpdate(_))) => page.fact(Mark::Done, "Icon is fine"),
        None => {}
    }
    if f.queued {
        page.fact(Mark::Waiting, "A change waits for Steam to close");
    }
    if f.queued {
        let mut o = option("Close Steam and finish", "Applies the waiting change, then starts Steam again", Act::SteamFinish);
        o.recommended = true;
        page.options.push(o);
    }
    if f.at_old_copy {
        let mut o = option("Point Steam at the new copy", "Its name, art, play time and launch options stay", Act::SteamRepoint);
        o.recommended = !f.queued;
        page.options.push(o);
    }
    match &f.state {
        Ok(SteamState::NotInSteam) => page.options.push(PageOption { recommended: !f.queued && !f.at_old_copy, ..option("Add to Steam", "With its artwork, so it starts from Game Mode", Act::SteamAdd) }),
        Ok(SteamState::NeedsUpdate(_)) => page.options.push(option("Update the shortcut", "Points it at this copy; play time stays", Act::SteamAdd)),
        Ok(SteamState::OtherProgram(_)) => page.options.push(option("Use that shortcut for this port", "Repoints it at this port; you see the change first", Act::SteamAdd)),
        _ => {}
    }
    if f.icon_problem.is_some() {
        page.options.push(option("Fix the icon", "Puts the port's icon back on its shortcut", Act::SteamFixIcon));
    }
    if page.options.is_empty() {
        page.paragraphs.push("Nothing to do here.".into());
    }
    page
}

pub fn files_page(p: &PortFacts, f: &FilesFacts, places: &Places) -> Page<Act> {
    let mut page = card_page(p, Card::Files);
    page.paragraphs.push(human_path(&f.dir, places));
    match f.size {
        Some(size) if f.size_capped => page.fact(Mark::None, format!("Size: over {}", size_text(size))),
        Some(size) => page.fact(Mark::None, format!("Size: {}", size_text(size))),
        None => page.fact(Mark::None, "Size: counting..."),
    }
    page.fact(Mark::None, format!("Version: {}", p.version.clone().unwrap_or_else(|| "not known".into())));
    if f.previous.is_empty() {
        page.fact(Mark::None, "Older versions kept: none");
    } else {
        for (name, size) in f.previous.iter().take(6) {
            page.fact(Mark::None, format!("Older version kept: {name} ({})", size_text(*size)));
        }
        page.footnote = "Older versions are kept in a hidden .previous folder inside the port's folder; Updates > Select version goes back to one.".into();
    }
    page.options.push(desktop_only(option("Open the folder", "Show the port's folder in the file manager", Act::OpenInstallFolder), p.game_mode));
    page
}

pub fn updates_page(p: &PortFacts) -> Page<Act> {
    let mut page = card_page(p, Card::Updates);
    page.fact(Mark::None, format!("Installed: {}", p.version.clone().unwrap_or_else(|| "version not known".into())));
    match &p.update_check {
        UpdateCheck::Checking => page.fact(Mark::Waiting, "Checking for an update..."),
        UpdateCheck::UpToDate(tag) => page.fact(Mark::Done, format!("Up to date ({tag})")),
        UpdateCheck::Ready(tag) => page.fact(Mark::ToDo, format!("Update ready: {tag}")),
        UpdateCheck::Failed(why) => page.fact(Mark::Problem, format!("Could not check: {why}")),
        UpdateCheck::NotAsked if p.update_ready => page.fact(Mark::ToDo, "Update ready"),
        UpdateCheck::NotAsked => {}
    }
    if !p.tracks_releases {
        page.paragraphs.push("This port does not follow GitHub or GitLab releases, so there is nothing to check or pick.".into());
    }
    let ready = matches!(p.update_check, UpdateCheck::Ready(_)) || (p.update_ready && !matches!(p.update_check, UpdateCheck::UpToDate(_)));
    if ready {
        let mut o = option("Update now", "Installs the newest version; saves are backed up first and the old version is kept", Act::UpdateNow);
        o.recommended = true;
        if p.busy || p.running {
            o = disabled(PageOption { detail: "Not while it runs, installs or restores".into(), ..o });
        }
        page.options.push(o);
    }
    let check = option("Check for an update now", "Asks GitHub or GitLab once", Act::CheckNow);
    page.options.push(if p.tracks_releases && p.update_check != UpdateCheck::Checking { check } else { disabled(check) });
    let toggle = option("Check for updates", "Look for new versions of this port when you start it", Act::ToggleUpdates);
    let switch_at = page.options.len();
    page.options.push(if p.tracks_releases { toggle } else { disabled(toggle) });
    page = page.toggle(switch_at, p.update_on);
    let select = option("Select version", "Install another version (the newest few)", Act::SelectVersion);
    page.options.push(if p.tracks_releases { select } else { disabled(select) });
    if p.has_extra {
        page.options.push(option("Install extras", "The extra files the catalog lists for this port", Act::InstallExtras));
    }
    page
}

/// The plain confirm page of Uninstall: what goes, what stays, "Keep it" first.
pub fn uninstall_page(p: &PortFacts, files: Option<&FilesFacts>, saves: Option<&SavesFacts>, places: &Places) -> Page<Act> {
    let mut page = card_page(p, Card::Uninstall);
    page.title = format!("Uninstall {}?", p.name);
    if p.user_managed {
        page.paragraphs.push(format!("{} is a port you added yourself: the app never deletes its folder. Remove it in Desktop Mode if you want it gone.", p.name));
        return page;
    }
    let saves_kept = !saves.is_some_and(|s| s.unknown_on_linux);
    let keeps = files.and_then(|f| f.keep.what());
    match files {
        Some(f) => {
            let size = f.size.map(|s| format!(" ({}{})", if f.size_capped { "over " } else { "" }, size_text(s))).unwrap_or_default();
            if let Some(what) = &keeps {
                let mut line = format!("{what} will be moved to Kept files");
                if saves_kept {
                    line.push_str(", saves are backed up");
                }
                line.push_str(&format!(". Kept files: {}.", human_path(&f.kept_dir, places)));
                page.fact(Mark::Done, line);
                if !f.keep.names.is_empty() {
                    page.paragraphs.push(format!("Kept: {}.", f.keep.names.join(", ")));
                }
                page.paragraphs.push(format!("The rest of its folder is deleted: {}{size}.", human_path(&f.dir, places)));
            } else {
                page.paragraphs.push(format!("Its folder is deleted: {}{size}.", human_path(&f.dir, places)));
            }
        }
        None => page.paragraphs.push("Its folder is deleted. Game files, keys and older versions in it are moved to Kept files first.".into()),
    }
    if !saves_kept {
        page.fact(Mark::Problem, "Saves are not kept: no Linux save folder is known for this port. Copy any save inside its folder yourself first.");
    } else {
        page.paragraphs.push("Saves inside its folder are copied to Saves Backup first and come back if you install it again. Saves kept elsewhere are not touched.".into());
    }
    page.paragraphs.push("Steam shortcuts are left as they are.".into());
    let mut keep = option("Keep it", "Nothing is changed", Act::Back);
    keep.recommended = true;
    page.options.push(keep);
    let go = option("Uninstall", if keeps.is_some() { "Keep the game files, delete the rest now" } else { "Delete the port's folder now" }, Act::UninstallYes);
    page.options.push(if p.running || p.busy { disabled(PageOption { detail: "Not while it runs, installs or restores".into(), ..go }) } else { go });
    page
}

pub fn more_page(p: &PortFacts) -> Page<Act> {
    let mut page = card_page(p, Card::More);
    let project = option("Open the project page", p.project_url.clone().unwrap_or_else(|| "No project page is known for this port".into()), Act::ProjectPage);
    page.options.push(if p.project_url.is_some() { project } else { disabled(project) });
    page.options.push(option("All details", "Instructions and every option, as Info shows them", Act::AllDetails));
    let reset = option("Reset game time", p.playtime.clone().map_or_else(|| "Not played yet".to_string(), |t| format!("Played {t}; asks first")), Act::ResetPlaytime);
    page.options.push(if p.has_playtime { reset } else { disabled(reset) });
    page
}

/// A short message page under a card (a backup made, a check that failed): its one button goes back to the card.
pub fn notice_page(p: &PortFacts, card: Card, title: &str, paragraphs: Vec<String>) -> Page<Act> {
    let mut page = card_page(p, card);
    page.window_title = format!("{}: {title}", window_title(&p.name, Some(card)));
    page.title = title.to_string();
    page.paragraphs = paragraphs;
    page.initial = Focus::Button(LEFT);
    page
}

// -- The project page link ------------------------------------------------------------------------------------

/// The link a port's "Open the project page" shows: its website, or its GitHub or GitLab page; only http(s), and never a
/// direct download (a port installed from a file link has none unless the catalog gives a website).
pub fn project_url(port: &crate::core::models::Port) -> Option<String> {
    port.website_url().map(str::trim).filter(|u| crate::core::launch::is_web_url(u) && (u.starts_with("https://") || u.starts_with("http://"))).map(str::to_string)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkAct {
    OpenInBrowser,
    Back,
}

/// The window title of the link page.
pub fn link_window_title(name: &str) -> String {
    format!("Project page: {name}")
}

/// The link page: the address as text (so it is seen before anything opens) and a QR code to scan with a phone. In
/// Desktop Mode one card opens it in the browser; Game Mode has no browser, so the code and the text are the way.
pub fn link_page(name: &str, url: &str, game_mode: bool) -> Page<LinkAct> {
    let mut page = Page::<LinkAct> { window_title: link_window_title(name), large: true, header: name.to_string(), title: "Project page".to_string(), ..Page::default() };
    page.card = Some(PageCard { art: None, qr: crate::fork::cloud::qr::QrMatrix::encode(url), title: name.to_string(), lines: vec!["Scan with your phone to open it there".to_string()] });
    page.paragraphs.push(url.to_string());
    page.paragraphs.push("This is where the port is made: its source code, its releases and what people say about it.".to_string());
    page.buttons = [Some(PageButton { label: "Back".to_string(), action: LinkAct::Back }), None, None, None];
    if game_mode {
        page.paragraphs.push("Game Mode has no web browser: scan the code with your phone, or open it from the port's page in Desktop Mode.".to_string());
        page.initial = Focus::Button(LEFT);
    } else {
        page.options.push(PageOption { title: "Open in the browser".to_string(), detail: "Opens the page above in your web browser".to_string(), choice: None, enabled: true, recommended: true, action: LinkAct::OpenInBrowser });
        page.initial = Focus::Option(0);
    }
    page
}
