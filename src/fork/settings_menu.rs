//! Settings as pages instead of one long list. The first page has a card for each group (Ports, Steam, Saves, App,
//! Updates), each with a one-line summary of where that stands; a group's page has its actions as cards, and a card
//! that is a switch shows On or Off and flips in place. Pure data for `fork::page`: the groups, what each entry
//! says, the summaries. What an entry does is `src/app/settings_menu_screen.rs::run_entry`; the screens are
//! `src/app/settings_menu_screen.rs`.
//!
//! Every entry of the old list is still here, in exactly one group, with the same behaviour.

use crate::fork::page::{Focus, Icon, Page, PageButton, PageOption};

/// The group pages' window titles start with this (the first page is just this).
pub const TITLE: &str = "Settings";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Ports,
    Steam,
    Saves,
    App,
    Updates,
}

impl Group {
    pub const ALL: [Group; 5] = [Group::Ports, Group::Steam, Group::Saves, Group::App, Group::Updates];

    pub fn title(self) -> &'static str {
        match self {
            Group::Ports => "Ports",
            Group::Steam => "Steam",
            Group::Saves => "Saves",
            Group::App => "App",
            Group::Updates => "Updates",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Group::Ports => Icon::Folder,
            Group::Steam => Icon::Steam,
            Group::Saves => Icon::Saves,
            Group::App => Icon::Gear,
            Group::Updates => Icon::Update,
        }
    }

    /// The window's name for the group's page (the smoke tests look for it).
    pub fn window_title(self) -> String {
        format!("{TITLE}: {}", self.title())
    }

    pub fn entries(self) -> &'static [Entry] {
        match self {
            Group::Ports => &[Entry::AddPort, Entry::ImportPorts, Entry::PortsFolder, Entry::GameFiles, Entry::OpenPortsFolder],
            Group::Steam => &[Entry::AddToSteam, Entry::ArtSources],
            Group::Saves => &[Entry::BackupSaves, Entry::RestoreSaves, Entry::CloudBackup],
            Group::App => &[Entry::SetupWizard, Entry::Themes, Entry::Language, Entry::Files, Entry::DiscordRpc],
            Group::Updates => &[Entry::CheckUpdates, Entry::ForceUpdate, Entry::Version],
        }
    }

    pub fn of(entry: Entry) -> Group {
        Group::ALL.into_iter().find(|g| g.entries().contains(&entry)).unwrap_or(Group::App)
    }
}

/// One thing Settings can do: the 17 entries of the old list, and Add a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    AddPort,
    SetupWizard,
    Themes,
    Language,
    Files,
    /// Upstream's "Library": opens the ports folder in the file manager.
    OpenPortsFolder,
    PortsFolder,
    AddToSteam,
    ImportPorts,
    GameFiles,
    ArtSources,
    BackupSaves,
    RestoreSaves,
    CloudBackup,
    CheckUpdates,
    ForceUpdate,
    DiscordRpc,
    Version,
}

impl Entry {
    pub const ALL: [Entry; 18] = [
        Entry::AddPort,
        Entry::SetupWizard,
        Entry::Themes,
        Entry::Language,
        Entry::Files,
        Entry::OpenPortsFolder,
        Entry::PortsFolder,
        Entry::AddToSteam,
        Entry::ImportPorts,
        Entry::GameFiles,
        Entry::ArtSources,
        Entry::BackupSaves,
        Entry::RestoreSaves,
        Entry::CloudBackup,
        Entry::CheckUpdates,
        Entry::ForceUpdate,
        Entry::DiscordRpc,
        Entry::Version,
    ];

    /// A switch: flips where it is, shows On or Off.
    pub fn is_switch(self) -> bool {
        matches!(self, Entry::CheckUpdates | Entry::DiscordRpc)
    }

    /// Reads or writes saves, so it waits while a save restore runs (the old list's guard).
    pub fn waits_for_restore(self) -> bool {
        matches!(self, Entry::BackupSaves | Entry::RestoreSaves | Entry::ForceUpdate)
    }
}

/// What the cloud backup is doing, for the Saves summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cloud {
    #[default]
    Off,
    On,
    /// Copying now, or signed in but nothing copied yet.
    Waiting,
    Problem,
}

impl Cloud {
    /// From the Home screen's status mark (`Mark::code`: 0 not signed in, 1 done, 3 waiting, 5 a problem).
    pub fn from_mark(mark: i32) -> Cloud {
        match mark {
            1 => Cloud::On,
            3 => Cloud::Waiting,
            5 => Cloud::Problem,
            _ => Cloud::Off,
        }
    }

    fn phrase(self) -> &'static str {
        match self {
            Cloud::Off => "cloud off",
            Cloud::On => "cloud on",
            Cloud::Waiting => "cloud backing up",
            Cloud::Problem => "cloud has a problem",
        }
    }
}

/// What the summaries and switches read, gathered cheaply by the screen (nothing here waits on slow input or output).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    /// Where the ports folder is ("SD card", "internal storage"); `None` when it could not be told.
    pub ports_place: Option<String>,
    pub installed: usize,
    /// Steam changes waiting for Steam to close.
    pub steam_waiting: usize,
    /// Whole days since the last "Back up saves now" zip; `None` when there is none.
    pub last_backup_days: Option<u64>,
    pub cloud: Cloud,
    pub check_updates: bool,
    pub discord: bool,
    pub version: String,
}

/// The translated upstream labels the screen reads with `tr`, for the entries that keep upstream's words. A switch's
/// title is its label without the state (the card shows On or Off itself): see [`without_state`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Labels {
    /// The title of the first page (upstream's "Settings"), also the header of a group's page.
    pub settings: String,
    pub themes: String,
    pub language: String,
    pub files: String,
    /// Upstream's "Library": the card that opens the ports folder.
    pub library: String,
    pub backup_saves: String,
    pub check_updates: String,
    pub discord: String,
    pub version: String,
}

impl Labels {
    pub fn english(version: &str) -> Labels {
        Labels {
            settings: "Settings".into(),
            themes: "Themes".into(),
            language: "Language".into(),
            files: "Files".into(),
            library: "Library".into(),
            backup_saves: "Save Backup".into(),
            check_updates: "Check for Updates".into(),
            discord: "Discord Rich Presence".into(),
            version: format!("Version: {version}"),
        }
    }
}

/// Upstream's switch labels carry the state ("Check for Updates: On"); the card has its own On/Off, so the title is
/// what comes before the last colon (an ASCII or full-width one, with a short word after it). Without one, the label as it is.
pub fn without_state(label: &str) -> String {
    let cut = label.rfind([':', '\u{ff1a}']);
    match cut {
        Some(i) if i > 0 => {
            let after = label[i..].trim_start_matches([':', '\u{ff1a}']).trim();
            if !after.is_empty() && after.chars().count() <= 12 { label[..i].trim_end().to_string() } else { label.to_string() }
        }
        _ => label.to_string(),
    }
}

/// What a ports folder's place is called on the Ports card; `None` for a folder that is neither on a drive nor the
/// internal default (the line then leaves the place out rather than guess).
pub fn place_name(kind: Option<crate::fork::setup_wizard::places::Kind>) -> Option<String> {
    use crate::fork::setup_wizard::places::Kind;
    match kind? {
        Kind::SdCard => Some("SD card".to_string()),
        Kind::OtherDrive => Some("Another drive".to_string()),
        Kind::Internal => Some("Internal storage".to_string()),
        Kind::Current | Kind::Earlier => None,
    }
}

/// Whole days from `then` to `now` (seconds since 1970); 0 when the clock reads earlier.
pub fn days_ago(then: i64, now: i64) -> u64 {
    (now.saturating_sub(then).max(0) / 86_400) as u64
}

/// When the newest zip in `dir` was made (the "Back up saves now" backups): one look at a folder of a few files, so it
/// is cheap enough to ask each time Settings opens. `None` when there is none or the folder cannot be read.
pub fn newest_zip(dir: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")))
        .filter_map(|e| e.metadata().ok().filter(std::fs::Metadata::is_file).and_then(|m| m.modified().ok()))
        .max()
}

fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// "Last backup 2 days ago" and the like.
fn backup_phrase(days: Option<u64>) -> String {
    match days {
        None => "no backup yet".into(),
        Some(0) => "backed up today".into(),
        Some(1) => "last backup yesterday".into(),
        Some(n) => format!("last backup {n} days ago"),
    }
}

fn capitalised(s: String) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => s,
    }
}

/// The one line under a group's card on the first page.
pub fn summary(group: Group, f: &Facts) -> String {
    let line = match group {
        Group::Ports => {
            let ports = if f.installed == 0 { "no ports yet".to_string() } else { count(f.installed, "port", "ports") };
            match &f.ports_place {
                Some(place) => format!("{place}, {ports}"),
                None => ports,
            }
        }
        Group::Steam if f.steam_waiting > 0 => format!("{} waiting for Steam", count(f.steam_waiting, "change", "changes")),
        Group::Steam => "Shortcuts and artwork".to_string(),
        Group::Saves => format!("{}, {}", backup_phrase(f.last_backup_days), f.cloud.phrase()),
        Group::App => "Setup, theme, language".to_string(),
        Group::Updates => {
            let checks = if f.check_updates { "update checks on" } else { "update checks off" };
            if f.version.is_empty() { checks.to_string() } else { format!("version {}, {checks}", f.version) }
        }
    };
    capitalised(line)
}

/// A card of a group's page.
struct Card {
    title: String,
    detail: String,
    /// `Some(on)` for a switch.
    switch: Option<bool>,
}

fn entry_card(entry: Entry, l: &Labels, f: &Facts) -> Card {
    let plain = |title: &str, detail: &str| Card { title: title.to_string(), detail: detail.to_string(), switch: None };
    match entry {
        Entry::AddPort => plain("Add a port", "Browse, bring in, or use a GitHub link"),
        Entry::ImportPorts => plain("Ports you already have", "Import ports: find them on this Deck and move them in"),
        Entry::PortsFolder => Card { title: "Ports folder".into(), detail: f.ports_place.clone().unwrap_or_else(|| "Where your ports are kept".to_string()), switch: None },
        Entry::GameFiles => plain("Game files", "Connect your game files to the ports that need them"),
        Entry::OpenPortsFolder => plain(&l.library, "Show the ports folder in the file manager"),
        Entry::AddToSteam if f.steam_waiting > 0 => Card { title: "Add to Steam".into(), detail: format!("{} waiting for Steam to close", count(f.steam_waiting, "change", "changes")), switch: None },
        Entry::AddToSteam => plain("Add to Steam", "Shortcuts and art for your ports"),
        Entry::ArtSources => plain("Artwork sources", "Where covers and pictures come from"),
        Entry::BackupSaves => plain(&l.backup_saves, "Back up every port's saves now"),
        Entry::RestoreSaves => plain("Restore saves", "Put an earlier backup back"),
        Entry::CloudBackup => plain(
            "Cloud backup",
            match f.cloud {
                Cloud::Off => "Keep a copy of your saves online",
                Cloud::On => "On: your saves are copied online",
                Cloud::Waiting => "On: backing up your saves",
                Cloud::Problem => "Needs a look",
            },
        ),
        Entry::SetupWizard => plain("Setup wizard", "Go through setup again"),
        Entry::Themes => plain(&l.themes, "Change how the app looks"),
        Entry::Language => plain(&l.language, "Choose the app's language"),
        Entry::Files => plain(&l.files, "Open the app's own data files"),
        Entry::DiscordRpc => Card { title: l.discord.clone(), detail: "Show what you are playing on Discord".into(), switch: Some(f.discord) },
        Entry::CheckUpdates => Card { title: l.check_updates.clone(), detail: "Look for new versions of your ports and this app".into(), switch: Some(f.check_updates) },
        Entry::ForceUpdate => plain("Update the app now", "Install the newest version of this app"),
        Entry::Version => plain(&l.version, "See what is new on GitHub"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// A card of the first page: that group's page.
    Open(Group),
    /// A card of a group's page.
    Do(Entry),
    /// B or the Back button: up one level.
    Back,
}

fn back_button() -> [Option<PageButton<Act>>; 4] {
    [Some(PageButton { label: "Back".to_string(), action: Act::Back }), None, None, None]
}

/// The first page: a card for each group, with where it stands.
pub fn top_page(l: &Labels, f: &Facts) -> Page<Act> {
    // No header line on the first page: the window already says "Settings" and so does the title under it.
    let mut p = Page::<Act> { window_title: TITLE.to_string(), large: true, header: String::new(), title: l.settings.clone(), initial: Focus::Option(0), buttons: back_button(), ..Page::default() };
    for (i, g) in Group::ALL.into_iter().enumerate() {
        p.options.push(PageOption { title: g.title().to_string(), detail: summary(g, f), choice: None, enabled: true, recommended: false, action: Act::Open(g) });
        p = p.icon(i, g.icon());
    }
    p
}

/// A group's page: its actions as cards; switches show On or Off.
pub fn group_page(group: Group, l: &Labels, f: &Facts) -> Page<Act> {
    let mut p = Page::<Act> { window_title: group.window_title(), large: true, header: l.settings.clone(), title: group.title().to_string(), initial: Focus::Option(0), buttons: back_button(), ..Page::default() };
    for (i, entry) in group.entries().iter().enumerate() {
        let c = entry_card(*entry, l, f);
        p.options.push(PageOption { title: c.title, detail: c.detail, choice: None, enabled: true, recommended: false, action: Act::Do(*entry) });
        if let Some(on) = c.switch {
            p = p.toggle(i, on);
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::page::{action_at, focus_number, hints, Focus, HINT_GAP, LEFT};

    fn facts() -> Facts {
        Facts { ports_place: Some("SD card".into()), installed: 5, steam_waiting: 0, last_backup_days: Some(2), cloud: Cloud::On, check_updates: true, discord: false, version: "1.2.3".into() }
    }

    #[test]
    fn every_entry_of_the_old_list_and_add_a_port_sits_in_exactly_one_group() {
        let in_groups: Vec<Entry> = Group::ALL.iter().flat_map(|g| g.entries().iter().copied()).collect();
        assert_eq!(in_groups.len(), Entry::ALL.len(), "no entry twice and none missing");
        for e in Entry::ALL {
            assert_eq!(in_groups.iter().filter(|x| **x == e).count(), 1, "{e:?}");
            assert!(Group::of(e).entries().contains(&e));
        }
        // The seventeen of the old Settings list (upstream's order), all still reachable.
        let old = [
            Entry::SetupWizard,
            Entry::Themes,
            Entry::Language,
            Entry::Files,
            Entry::OpenPortsFolder,
            Entry::PortsFolder,
            Entry::AddToSteam,
            Entry::ImportPorts,
            Entry::GameFiles,
            Entry::ArtSources,
            Entry::BackupSaves,
            Entry::RestoreSaves,
            Entry::CloudBackup,
            Entry::CheckUpdates,
            Entry::ForceUpdate,
            Entry::DiscordRpc,
            Entry::Version,
        ];
        assert!(old.iter().all(|e| in_groups.contains(e)));
    }

    #[test]
    fn the_first_page_has_a_card_for_each_group_with_a_picture_and_a_summary() {
        let p = top_page(&Labels::english("1.2.3"), &facts());
        assert_eq!(p.options.len(), 5);
        assert_eq!(p.title, "Settings");
        for (i, g) in Group::ALL.into_iter().enumerate() {
            assert_eq!(p.options[i].action, Act::Open(g));
            assert_eq!(p.options[i].title, g.title());
            assert!(!p.options[i].detail.is_empty());
            assert_ne!(p.icon_of(i), Icon::None);
        }
        assert_eq!(p.options[0].detail, "SD card, 5 ports");
        assert_eq!(p.options[2].detail, "Last backup 2 days ago, cloud on");
        assert_eq!(action_at(&p, crate::fork::page::initial_focus(&p)), Some(Act::Open(Group::Ports)));
    }

    #[test]
    fn summaries_say_what_is_known_and_stay_short_without_it() {
        let mut f = Facts::default();
        assert_eq!(summary(Group::Ports, &f), "No ports yet");
        f.installed = 1;
        assert_eq!(summary(Group::Ports, &f), "1 port");
        f.ports_place = Some("internal storage".into());
        assert_eq!(summary(Group::Ports, &f), "Internal storage, 1 port");
        assert_eq!(summary(Group::Steam, &f), "Shortcuts and artwork");
        f.steam_waiting = 1;
        assert_eq!(summary(Group::Steam, &f), "1 change waiting for Steam");
        f.steam_waiting = 3;
        assert_eq!(summary(Group::Steam, &f), "3 changes waiting for Steam");
        assert_eq!(summary(Group::Saves, &f), "No backup yet, cloud off");
        f.last_backup_days = Some(0);
        f.cloud = Cloud::Problem;
        assert_eq!(summary(Group::Saves, &f), "Backed up today, cloud has a problem");
        f.last_backup_days = Some(1);
        assert_eq!(summary(Group::Saves, &f), "Last backup yesterday, cloud has a problem");
        assert_eq!(summary(Group::Updates, &f), "Update checks off");
        f.version = "0.9".into();
        f.check_updates = true;
        assert_eq!(summary(Group::Updates, &f), "Version 0.9, update checks on");
        assert_eq!(summary(Group::App, &f), "Setup, theme, language");
    }

    #[test]
    fn a_group_page_lists_its_entries_and_only_the_switches_show_on_or_off() {
        let l = Labels::english("1.2.3");
        let f = facts();
        let p = group_page(Group::App, &l, &f);
        assert_eq!(p.window_title, "Settings: App");
        assert_eq!(p.title, "App");
        let acts: Vec<Act> = p.options.iter().map(|o| o.action).collect();
        assert_eq!(acts, [Act::Do(Entry::SetupWizard), Act::Do(Entry::Themes), Act::Do(Entry::Language), Act::Do(Entry::Files), Act::Do(Entry::DiscordRpc)]);
        let switches: Vec<Option<bool>> = (0..p.options.len()).map(|i| p.toggle_of(i)).collect();
        assert_eq!(switches, [None, None, None, None, Some(false)], "Discord is off in these facts");
        let updates = group_page(Group::Updates, &l, &f);
        assert_eq!((updates.toggle_of(0), updates.toggle_of(1), updates.toggle_of(2)), (Some(true), None, None));
        assert_eq!(updates.options[0].title, "Check for Updates", "the card's own On/Off says the state");
        assert_eq!(updates.options[2].title, "Version: 1.2.3");
        for g in Group::ALL {
            let page = group_page(g, &l, &f);
            assert!(page.options.iter().all(|o| !o.title.is_empty() && !o.detail.is_empty() && o.detail.len() < 64), "{g:?}: a plain line each");
            assert_eq!(page.options.iter().filter(|o| matches!(o.action, Act::Do(e) if e.is_switch())).count(), (0..page.options.len()).filter(|i| page.toggle_of(*i).is_some()).count());
        }
    }

    #[test]
    fn the_old_guards_and_switches_are_kept_as_data() {
        let guarded: Vec<Entry> = Entry::ALL.into_iter().filter(|e| e.waits_for_restore()).collect();
        assert_eq!(guarded, [Entry::BackupSaves, Entry::RestoreSaves, Entry::ForceUpdate], "the save-restore refusal guard of the old list");
        let switches: Vec<Entry> = Entry::ALL.into_iter().filter(|e| e.is_switch()).collect();
        assert_eq!(switches, [Entry::CheckUpdates, Entry::DiscordRpc]);
    }

    #[test]
    fn b_goes_up_one_level_and_the_pages_hint_at_it() {
        for p in [top_page(&Labels::english("1"), &facts()), group_page(Group::Saves, &Labels::english("1"), &facts())] {
            assert_eq!(p.buttons[LEFT].as_ref().map(|b| b.action), Some(Act::Back));
            assert_eq!(hints(&p), ["A Choose", "B Back", "D-pad Move"].join(HINT_GAP));
        }
        let saves = group_page(Group::Saves, &Labels::english("1"), &facts());
        assert_eq!(focus_number(&saves, Focus::Option(2)), 2);
    }

    #[test]
    fn a_translated_switch_label_loses_its_state_and_nothing_else() {
        assert_eq!(without_state("Check for Updates: On"), "Check for Updates");
        assert_eq!(without_state("Discord Rich Presence: Off"), "Discord Rich Presence");
        assert_eq!(without_state("Mises \u{e0} jour : activ\u{e9}es"), "Mises \u{e0} jour");
        assert_eq!(without_state("\u{66f4}\u{65b0}\u{3092}\u{78ba}\u{8a8d}\u{ff1a}\u{30aa}\u{30f3}"), "\u{66f4}\u{65b0}\u{3092}\u{78ba}\u{8a8d}");
        assert_eq!(without_state("Updates"), "Updates", "no state in it: as it is");
        assert_eq!(without_state("Note: this is a long sentence after a colon"), "Note: this is a long sentence after a colon");
        assert_eq!(without_state(": On"), ": On");
    }

    #[test]
    fn the_last_backup_is_the_newest_zip_in_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(newest_zip(dir.path()), None, "an empty folder");
        assert_eq!(newest_zip(&dir.path().join("missing")), None, "a folder that is not there");
        std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();
        assert_eq!(newest_zip(dir.path()), None, "only zips count");
        std::fs::write(dir.path().join("2026-10-01.zip"), b"x").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(5 * 86_400);
        std::fs::File::options().write(true).open(dir.path().join("2026-10-01.zip")).unwrap().set_modified(old).unwrap();
        std::fs::write(dir.path().join("2026-10-20.ZIP"), b"x").unwrap();
        let newest = newest_zip(dir.path()).unwrap();
        assert!(newest > old, "the newer zip wins, whatever its case");
        std::fs::create_dir(dir.path().join("folder.zip")).unwrap();
        assert_eq!(newest_zip(dir.path()), Some(newest), "a folder named like a zip is not a backup");
    }

    #[test]
    fn cards_say_what_is_waiting_and_what_the_cloud_is_doing() {
        let l = Labels::english("1");
        let mut f = facts();
        let card = |e: Entry, l: &Labels, f: &Facts| entry_card(e, l, f);
        assert_eq!((card(Entry::AddToSteam, &l, &f).title, card(Entry::AddToSteam, &l, &f).detail), ("Add to Steam".to_string(), "Shortcuts and art for your ports".to_string()), "the card has the screen's own name");
        f.steam_waiting = 2;
        let waiting = card(Entry::AddToSteam, &l, &f);
        assert_eq!((waiting.title.as_str(), waiting.detail.as_str()), ("Add to Steam", "2 changes waiting for Steam to close"));
        f.steam_waiting = 1;
        assert_eq!(card(Entry::AddToSteam, &l, &f).detail, "1 change waiting for Steam to close");
        for (cloud, detail, phrase) in [
            (Cloud::Off, "Keep a copy of your saves online", "cloud off"),
            (Cloud::On, "On: your saves are copied online", "cloud on"),
            (Cloud::Waiting, "On: backing up your saves", "cloud backing up"),
            (Cloud::Problem, "Needs a look", "cloud has a problem"),
        ] {
            f.cloud = cloud;
            assert_eq!(card(Entry::CloudBackup, &l, &f).detail, detail, "{cloud:?}");
            assert_eq!(cloud.phrase(), phrase);
        }
        assert_eq!(card(Entry::CheckUpdates, &l, &f).detail, "Look for new versions of your ports and this app", "the switch gates both checks");
    }

    #[test]
    fn one_name_for_import_and_the_translated_labels_where_upstream_has_them() {
        let mut l = Labels::english("1");
        l.settings = "Reglages".into();
        l.library = "Bibliotheque".into();
        l.backup_saves = "Sauvegarde".into();
        let f = facts();
        let top = top_page(&l, &f);
        assert_eq!((top.title.as_str(), top.header.as_str(), top.window_title.as_str()), ("Reglages", "", "Settings"), "translated title, no repeated header, the window name the tests look for");
        let ports = group_page(Group::Ports, &l, &f);
        assert_eq!(ports.header, "Reglages");
        assert_eq!(ports.options[1].title, "Ports you already have", "the Add a port card says the same");
        assert!(ports.options[1].detail.contains("Import ports"));
        assert_eq!(ports.options[4].title, "Bibliotheque");
        assert_eq!(group_page(Group::Saves, &l, &f).options[0].title, "Sauvegarde");
    }

    #[test]
    fn the_ports_card_names_the_place_or_says_what_the_folder_is_for() {
        use crate::fork::setup_wizard::places::Kind;
        assert_eq!(place_name(Some(Kind::SdCard)).as_deref(), Some("SD card"));
        assert_eq!(place_name(Some(Kind::OtherDrive)).as_deref(), Some("Another drive"));
        assert_eq!(place_name(Some(Kind::Internal)).as_deref(), Some("Internal storage"));
        assert_eq!(place_name(None), None, "a folder chosen elsewhere: no guess");
        let mut f = facts();
        f.ports_place = None;
        assert_eq!(entry_card(Entry::PortsFolder, &Labels::english("1"), &f).detail, "Where your ports are kept");
        assert_eq!(summary(Group::Ports, &f), "5 ports");
        f.ports_place = Some("SD card".into());
        assert_eq!(entry_card(Entry::PortsFolder, &Labels::english("1"), &f).detail, "SD card");
    }

    #[test]
    fn each_entry_knows_its_group() {
        assert_eq!(Group::of(Entry::AddPort), Group::Ports);
        assert_eq!(Group::of(Entry::OpenPortsFolder), Group::Ports);
        assert_eq!(Group::of(Entry::ArtSources), Group::Steam);
        assert_eq!(Group::of(Entry::CloudBackup), Group::Saves);
        assert_eq!(Group::of(Entry::DiscordRpc), Group::App);
        assert_eq!(Group::of(Entry::ForceUpdate), Group::Updates);
        for g in Group::ALL {
            assert!(g.entries().iter().all(|e| Group::of(*e) == g));
        }
    }

    #[test]
    fn days_since_the_last_backup_count_whole_days_and_never_go_negative() {
        assert_eq!(days_ago(0, 86_399), 0);
        assert_eq!(days_ago(0, 86_400), 1);
        assert_eq!(days_ago(0, 3 * 86_400 + 5), 3);
        assert_eq!(days_ago(500, 100), 0, "a file dated in the future");
        assert_eq!(Cloud::from_mark(0), Cloud::Off);
        assert_eq!((Cloud::from_mark(1), Cloud::from_mark(3), Cloud::from_mark(5)), (Cloud::On, Cloud::Waiting, Cloud::Problem));
    }
}
