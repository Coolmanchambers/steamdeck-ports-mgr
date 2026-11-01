//! Installing a port as one guided flow, one question per page, the same from every place that installs (A on a port in
//! All ports or Deck-ready, Add a port, Update on the Port page or in Fix problems):
//!
//! 1. "Install <name>?": the cover, what it is, where it goes, what game files it needs. A port that is the same game as
//!    one installed (or another version of it) says so here, and "Not now" is the recommended answer.
//! 2. Downloading: a real progress bar, "<MB> of <MB>" and the speed; "Keep going in the background" or "Stop and remove
//!    the download". Then "Installing..." while it unpacks.
//! 3. Put your kept files back? (a reinstall or update of a port whose uninstall kept files, `uninstall_keep`). Before
//!    the game files page, since kept files are mostly game files (`.o2r`, connected copies).
//! 4. Game files (only when the port names any it does not have yet): the card pages of `rom_screens::pages`.
//! 5. "Add it to Steam with its artwork?"
//! 6. Done: "<name> is ready", or installed but needing its game files. Play now / Back to Home.
//!
//! Everything here is plain data and words; `src/app/install_wizard_screen.rs` runs the pages and the work.

use crate::fork::art::thumbs::ArtRef;
use crate::fork::page::{Card, Focus, Mark, Page, PageButton, PageOption, LEFT};
use crate::fork::port_page::paths::size_text;
use crate::fork::rom_library::{RomSpec, SlotResult, SlotStatus};
use crate::fork::rom_screens::find::Need;
use crate::fork::rom_screens::pages::need_words;
use crate::fork::same_game::{Sys, Verdict};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;

/// What a choice on one of the wizard's pages means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizAct {
    Install,
    NotNow,
    ProjectPage,
    Background,
    Stop,
    PutBack,
    KeepLater,
    SteamYes,
    SteamNo,
    PlayNow,
    Home,
    FinishSetup,
}

/// The window title of every page of one install ("Install: Dusklight"; the main window's layer names the part before
/// the colon).
pub fn window_title(name: &str) -> String {
    format!("Install: {name}")
}

fn base(name: &str, title: &str) -> Page<WizAct> {
    Page { window_title: window_title(name), large: true, header: name.to_string(), title: title.to_string(), initial: Focus::Option(0), ..Page::default() }
}

fn option(title: &str, detail: &str, action: WizAct) -> PageOption<WizAct> {
    PageOption { title: title.to_string(), detail: detail.to_string(), choice: None, enabled: true, recommended: false, action }
}

fn recommended(mut o: PageOption<WizAct>) -> PageOption<WizAct> {
    o.recommended = true;
    o
}

fn card(name: &str, art: &Option<ArtRef>, lines: Vec<String>) -> Option<Card> {
    Some(Card { art: art.clone(), qr: None, title: name.to_string(), lines })
}

// -- 1. Install <name>? -----------------------------------------------------------------------------------------

/// The port being installed is the same game as one installed (or another version of it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SameNote {
    pub verdict: Verdict,
    /// The installed one's project name (`same_game::project_name`).
    pub owned_project: String,
}

impl SameNote {
    /// The one-line note on the first page.
    pub fn line(&self) -> String {
        match self.verdict {
            Verdict::SameGame => format!("You already have this game as {}. This is another project for the same game.", self.owned_project),
            Verdict::AnotherVersion => format!("You have another version of this game: {}.", self.owned_project),
        }
    }
}

/// What the first page says about the port, gathered by the screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallInfo {
    /// The name as the lists show it.
    pub name: String,
    pub art: Option<ArtRef>,
    /// "Wii U", "N64", "PC": from the catalog's tags or the Deck-ready label.
    pub system: Option<String>,
    /// The release that will be installed when known ("v1.4.1"), or, for an update, the one installed now.
    pub version: Option<String>,
    pub size: Option<u64>,
    /// Where it goes, the human way ("SD card > Ports").
    pub place: String,
    /// What game files it needs, in plain words (`rom_screens::pages::need_words`). Empty: none.
    pub needs: Vec<String>,
    pub same: Option<SameNote>,
    /// It is installed: this is an update (or a reinstall).
    pub update: bool,
    /// A Deck-ready project the catalog does not have: installing adds the user's own entry, and its game files are not
    /// known (so "No game files needed" is never claimed for it).
    pub own_entry: bool,
    pub has_project_page: bool,
}

/// "Wii U · v1.4.1 · 120 MB": what is known of the system, version and size.
pub fn what_line(system: Option<&str>, version: Option<&str>, size: Option<u64>, update: bool) -> Option<String> {
    let version = version.map(|v| if update { format!("Installed now: {v}") } else { v.to_string() });
    let parts: Vec<String> = [system.map(str::to_string), version, size.map(size_text)].into_iter().flatten().filter(|p| !p.trim().is_empty()).collect();
    (!parts.is_empty()).then(|| parts.join("  \u{b7}  "))
}

/// The system's short name for the "what it is" line.
pub fn sys_label(sys: &Sys) -> String {
    match sys {
        Sys::Known(s) => s.label().to_string(),
        Sys::Other("mac") => "Mac".to_string(),
        Sys::Other("flash") => "Flash".to_string(),
        Sys::Other("xone") => "Xbox One".to_string(),
        Sys::Other(other) => other.to_uppercase(),
    }
}

/// What the port needs, in plain words, from what its entry names (required files only).
pub fn needs_words(spec: &RomSpec, port_name: &str) -> Vec<String> {
    spec.requirement
        .slots
        .iter()
        .filter(|s| !s.optional)
        .map(|s| SlotResult { slot_id: s.id(), label: s.label.clone(), alternatives: s.alternatives.clone(), kind: s.kind, optional: false, vague: s.is_vague(), status: SlotStatus::Missing })
        .map(|r| need_words(&Need::from_slot(&r), port_name))
        .collect()
}

pub fn install_page(info: &InstallInfo) -> Page<WizAct> {
    let verb = if info.update { "Update" } else { "Install" };
    let mut p = base(&info.name, &format!("{verb} {}?", info.name));
    let lines = what_line(info.system.as_deref(), info.version.as_deref(), info.size, info.update).into_iter().collect();
    p.card = card(&info.name, &info.art, lines);
    p.fact(Mark::None, format!("Goes to: {}", info.place));
    match info.needs.as_slice() {
        [] if info.own_entry => p.fact(Mark::None, "Game files: its project page says what it needs".to_string()),
        [] => p.fact(Mark::Done, "No game files needed".to_string()),
        needs => {
            for n in needs.iter().take(3) {
                p.fact(Mark::ToDo, n.clone());
            }
            if needs.len() > 3 {
                p.fact(Mark::ToDo, format!("And {} more files", needs.len() - 3));
            }
        }
    }
    if let Some(same) = &info.same {
        p.paragraphs.push(same.line());
    }
    if info.update {
        p.paragraphs.push("Your saves are backed up before anything changes.".to_string());
    }
    if info.own_entry {
        p.paragraphs.push("This adds your own entry for it (in ports.local.json; the old file is backed up first).".to_string());
    }
    let not_now = option("Not now", "Nothing is downloaded", WizAct::NotNow);
    p.options = match (&info.same, info.update) {
        (Some(_), false) => vec![recommended(not_now), option("Install it as well", "Downloads it as a second copy, with its own saves and settings.", WizAct::Install)],
        (_, true) => vec![recommended(option("Update", "Downloads the newest version; saves are backed up first", WizAct::Install)), not_now],
        (None, false) => vec![recommended(option("Install", "Downloads it now; you can keep using the app meanwhile", WizAct::Install)), not_now],
    };
    if info.has_project_page {
        p.buttons[LEFT] = Some(PageButton { label: "Project page".to_string(), action: WizAct::ProjectPage });
    }
    p.b_does(&WizAct::NotNow)
}

// -- 2. Downloading -----------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Downloading,
    /// Backing up saves, unpacking, putting the files in place.
    Installing,
}

/// The installer's progress text says which part it is in: "Downloading <file>..." for the download, anything after
/// it (the save backup, "Extracting...", "Done.") is the install.
pub fn phase_of(message: &str) -> Phase {
    if message.starts_with("Downloading") { Phase::Downloading } else { Phase::Installing }
}

/// One install's progress, shared between the installer's thread (which writes it) and the pages and Home's tile
/// (which read it on a timer). The cancel flag is the installer's `DownloadCtl::cancel`.
pub struct Meter {
    downloaded: AtomicU64,
    /// 0 when the server gave no size.
    total: AtomicU64,
    installing: AtomicBool,
    stopping: AtomicU8,
    cancel: Arc<AtomicBool>,
}

impl Meter {
    pub fn new() -> Arc<Meter> {
        Arc::new(Meter { downloaded: AtomicU64::new(0), total: AtomicU64::new(0), installing: AtomicBool::new(false), stopping: AtomicU8::new(0), cancel: Arc::new(AtomicBool::new(false)) })
    }

    /// The installer's byte count.
    pub fn record(&self, downloaded: u64, total: Option<u64>) {
        self.downloaded.store(downloaded, Ordering::Relaxed);
        self.total.store(total.unwrap_or(0), Ordering::Relaxed);
    }

    /// The installer's progress text (see [`phase_of`]). Once installing, it stays installing.
    pub fn note(&self, message: &str) {
        if phase_of(message) == Phase::Installing {
            self.installing.store(true, Ordering::Relaxed);
        }
    }

    pub fn phase(&self) -> Phase {
        if self.installing.load(Ordering::Relaxed) { Phase::Installing } else { Phase::Downloading }
    }

    pub fn bytes(&self) -> (u64, Option<u64>) {
        let total = self.total.load(Ordering::Relaxed);
        (self.downloaded.load(Ordering::Relaxed), (total > 0).then_some(total))
    }

    /// "Stop and remove the download": the download stops at its next block, and the install never starts.
    pub fn stop(&self) {
        self.stopping.store(1, Ordering::Relaxed);
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn stopping(&self) -> bool {
        self.stopping.load(Ordering::Relaxed) != 0
    }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }
}

/// The download speed over the last few seconds, from the byte counts sampled by the page's timer.
#[derive(Debug, Clone, Default)]
pub struct Speed {
    samples: VecDeque<(f64, u64)>,
}

/// How far back the speed looks.
const SPEED_WINDOW: f64 = 4.0;

impl Speed {
    /// A sample: seconds since the download started, bytes so far.
    pub fn add(&mut self, at: f64, bytes: u64) {
        // A count that went down is a new download (a retry, another file): start over.
        if self.samples.back().is_some_and(|(_, b)| bytes < *b) {
            self.samples.clear();
        }
        self.samples.push_back((at, bytes));
        while self.samples.len() > 2 && self.samples.front().is_some_and(|(t, _)| at - t > SPEED_WINDOW) {
            self.samples.pop_front();
        }
    }

    /// Bytes a second, once half a second of samples is in.
    pub fn per_second(&self) -> Option<f64> {
        let (t0, b0) = *self.samples.front()?;
        let (t1, b1) = *self.samples.back()?;
        let span = t1 - t0;
        (span >= 0.5).then(|| (b1 - b0) as f64 / span)
    }
}

fn one_decimal_under_ten(v: f64) -> String {
    if v < 10.0 { format!("{v:.1}") } else { format!("{v:.0}") }
}

/// "12 MB of 120 MB", "0.4 GB of 1.2 GB" (one unit, chosen by the total), "12 MB so far" without a total.
pub fn mb_of_mb(done: u64, total: Option<u64>) -> String {
    match total {
        Some(t) if t >= 1_000_000_000 => format!("{} GB of {} GB", one_decimal_under_ten(done as f64 / 1e9), one_decimal_under_ten(t as f64 / 1e9)),
        Some(t) => format!("{} MB of {} MB", one_decimal_under_ten(done as f64 / 1e6), one_decimal_under_ten(t as f64 / 1e6)),
        None => format!("{} so far", size_text(done)),
    }
}

/// "4.2 MB/s", "850 KB/s".
pub fn speed_text(bytes_per_second: f64) -> String {
    if !bytes_per_second.is_finite() || bytes_per_second < 1000.0 {
        return "under 1 KB/s".to_string();
    }
    if bytes_per_second < 1e6 {
        return format!("{:.0} KB/s", bytes_per_second / 1e3);
    }
    format!("{} MB/s", one_decimal_under_ten(bytes_per_second / 1e6))
}

/// The line under Home's tile while it installs: "Downloading 42%", "Downloading 12 MB", "Installing...".
pub fn tile_note(phase: Phase, done: u64, total: Option<u64>) -> String {
    match (phase, total) {
        (Phase::Installing, _) => "Installing...".to_string(),
        (Phase::Downloading, Some(t)) => format!("Downloading {}%", (done.min(t) * 100 / t.max(1))),
        (Phase::Downloading, None) if done > 0 => format!("Downloading {}", size_text(done)),
        (Phase::Downloading, None) => "Downloading...".to_string(),
    }
}

/// What the downloading page shows now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapshot {
    pub phase: Phase,
    pub downloaded: u64,
    pub total: Option<u64>,
    /// Bytes a second, when known.
    pub speed: Option<f64>,
    pub stopping: bool,
}

pub fn downloading_page(name: &str, art: &Option<ArtRef>, s: &Snapshot) -> Page<WizAct> {
    let background = option("Keep going in the background", "Closes this page; Home shows progress and a note when done", WizAct::Background);
    if s.stopping {
        let mut p = base(name, "Stopping the download");
        p.card = card(name, art, Vec::new());
        p.paragraphs.push("Stopping and removing the download. Nothing is kept.".to_string());
        p.no_back = true;
        return p.with_progress(None);
    }
    let title = match s.phase {
        Phase::Downloading => format!("Downloading {name}"),
        Phase::Installing => format!("Installing {name}"),
    };
    let mut p = base(name, &title);
    p.card = card(name, art, Vec::new());
    match (s.phase, s.total) {
        (Phase::Installing, _) => {
            p.paragraphs.push("Installing... This takes a moment.".to_string());
            p.progress = Some(crate::fork::page::Progress::Busy);
        }
        (Phase::Downloading, Some(total)) => {
            p.fact(Mark::None, mb_of_mb(s.downloaded, Some(total)));
            p = p.with_progress(Some(s.downloaded as f32 / total.max(1) as f32));
        }
        (Phase::Downloading, None) => {
            p.paragraphs.push("Downloading...".to_string());
            if s.downloaded > 0 {
                p.fact(Mark::None, mb_of_mb(s.downloaded, None));
            }
            p.progress = Some(crate::fork::page::Progress::Busy);
        }
    }
    if let (Phase::Downloading, Some(speed)) = (s.phase, s.speed) {
        p.fact(Mark::None, speed_text(speed));
    }
    p.options.push(recommended(background));
    if s.phase == Phase::Downloading {
        p.options.push(option("Stop and remove the download", "Nothing is kept", WizAct::Stop));
    }
    p.b_does(&WizAct::Background)
}

// -- 3. Kept files ------------------------------------------------------------------------------------------------

/// The kept files a reinstall can put back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeptOffer {
    /// "2026-10-31 10:00".
    pub when: String,
    /// (file name, what it is: "game file", "key"...).
    pub files: Vec<(String, String)>,
    /// Of those, the ones whose place in the port folder has another file now (it is backed up first).
    pub replaces: usize,
    /// Where they are kept and where replaced files go, the human way.
    pub kept_place: String,
    pub replaced_place: String,
}

pub fn kept_page(name: &str, art: &Option<ArtRef>, offer: &KeptOffer) -> Page<WizAct> {
    let mut p = base(name, "Put your kept files back?");
    p.card = card(name, art, Vec::new());
    p.paragraphs.push(format!("When {name} was uninstalled ({}), these were kept in {}:", offer.when, offer.kept_place));
    for (file, kind) in offer.files.iter().take(5) {
        p.fact(Mark::None, format!("{file} ({kind})"));
    }
    if offer.files.len() > 5 {
        p.fact(Mark::None, format!("And {} more", offer.files.len() - 5));
    }
    if offer.replaces > 0 {
        let what = if offer.replaces == 1 { "1 of them is".to_string() } else { format!("{} of them are", offer.replaces) };
        p.paragraphs.push(format!("{what} in the port's folder already, in another version: that one is copied to {} first, never just overwritten.", offer.replaced_place));
    }
    p.options = vec![
        recommended(option("Put them back", "Copies them into the port's folder; Kept files keeps its copy", WizAct::PutBack)),
        option("Not now", "They stay in Kept files", WizAct::KeepLater),
    ];
    p.b_does(&WizAct::KeepLater)
}

// -- 5. Steam -----------------------------------------------------------------------------------------------------

pub fn steam_page(name: &str, art: &Option<ArtRef>, game_mode: bool, steam_running: bool) -> Page<WizAct> {
    let mut p = base(name, "Add it to Steam with its artwork?");
    p.card = card(name, art, Vec::new());
    p.paragraphs.push("Then it starts from your Steam library, with its cover, in Game Mode too.".to_string());
    if game_mode {
        p.paragraphs.push("Steam cannot be changed while it runs: it is added the next time you are in Desktop Mode.".to_string());
    } else if steam_running {
        p.paragraphs.push("Steam has to close for a moment to add it; you are asked first.".to_string());
    }
    p.options = vec![
        recommended(option("Yes", "Adds its shortcut and artwork; Steam's list is backed up first", WizAct::SteamYes)),
        option("Not now", "Its Port page can add it later", WizAct::SteamNo),
    ];
    p.b_does(&WizAct::SteamNo)
}

// -- 6. Done ------------------------------------------------------------------------------------------------------

/// A page that only waits (fetching artwork, copying kept files back): B does nothing.
pub fn working_page(name: &str, art: &Option<ArtRef>, title: &str, text: &str) -> Page<WizAct> {
    let mut p = base(name, title);
    p.card = card(name, art, Vec::new());
    p.paragraphs.push(text.to_string());
    p.no_back = true;
    p.with_progress(None)
}

pub fn done_page(name: &str, art: &Option<ArtRef>, ready: bool, notes: &[(Mark, String)]) -> Page<WizAct> {
    let title = if ready { format!("{name} is ready") } else { format!("{name} is installed; it needs its game files before it starts") };
    let mut p = base(name, &title);
    p.card = card(name, art, Vec::new());
    for (mark, text) in notes {
        p.fact(*mark, text.clone());
    }
    let play = option("Play now", if ready { "Starts it now" } else { "It will not start until its game files are added" }, WizAct::PlayNow);
    let home = option("Back to Home", "Your ports", WizAct::Home);
    p.options = if ready { vec![recommended(play), home] } else { vec![play, recommended(home)] };
    p.initial = Focus::Option(if ready { 0 } else { 1 });
    p.b_does(&WizAct::Home)
}

/// An install that went on in the background has finished: the note, and the rest of the questions if wanted.
pub fn finished_page(name: &str, art: &Option<ArtRef>, update: bool) -> Page<WizAct> {
    let mut p = base(name, &format!("{name} finished {}", if update { "updating" } else { "installing" }));
    p.card = card(name, art, Vec::new());
    p.paragraphs.push("It went on in the background while you did something else.".to_string());
    p.options = vec![recommended(option("Finish setting it up", "Game files and Steam, one question each", WizAct::FinishSetup)), option("Not now", "Its Port page has the rest", WizAct::Home)];
    p.b_does(&WizAct::Home)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::page::{action_at, initial_focus, Progress};

    fn info() -> InstallInfo {
        InstallInfo {
            name: "The Wind Waker HD".into(),
            art: Some(ArtRef { folder: "ww".into(), url: None }),
            system: Some("Wii U".into()),
            version: Some("v1.2".into()),
            size: None,
            place: "SD card > Ports".into(),
            needs: vec!["Your own copy of The Wind Waker HD: a Wii U copy, .wua or .wud".into()],
            same: None,
            update: false,
            own_entry: false,
            has_project_page: true,
        }
    }

    /// Every page of the flow, for the fit checks.
    fn all_pages() -> Vec<Page<WizAct>> {
        let art = None;
        let snap = |phase, total, speed, stopping| Snapshot { phase, downloaded: 12_300_000, total, speed, stopping };
        let mut same = info();
        same.same = Some(SameNote { verdict: Verdict::SameGame, owned_project: "ZeldaWWHDRecomp".into() });
        let mut update = info();
        update.update = true;
        let mut own = info();
        own.own_entry = true;
        own.needs.clear();
        let offer = KeptOffer {
            when: "2026-10-31 10:00".into(),
            files: (0..7).map(|i| (format!("file{i}.o2r"), "game file".to_string())).collect(),
            replaces: 2,
            kept_place: "Internal storage > Saves Backup > Kept files".into(),
            replaced_place: "Internal storage > Saves Backup > Replaced by restore".into(),
        };
        vec![
            install_page(&info()),
            install_page(&same),
            install_page(&update),
            install_page(&own),
            downloading_page("X", &art, &snap(Phase::Downloading, Some(120_000_000), Some(4.2e6), false)),
            downloading_page("X", &art, &snap(Phase::Downloading, None, None, false)),
            downloading_page("X", &art, &snap(Phase::Installing, None, None, false)),
            downloading_page("X", &art, &snap(Phase::Downloading, None, None, true)),
            kept_page("X", &art, &offer),
            steam_page("X", &art, false, true),
            steam_page("X", &art, true, true),
            working_page("X", &art, "Getting the artwork", "One moment."),
            done_page("X", &art, true, &[]),
            done_page("X", &art, false, &[]),
            finished_page("X", &art, false),
        ]
    }

    /// The page window is 800 px wide at the least, and an option's title and detail are one line each (elided, not
    /// wrapped): at the large page's text size about 66 characters of detail and 40 of title fit with the Recommended tag.
    #[test]
    fn every_option_fits_one_line_of_the_800_px_page() {
        for p in all_pages() {
            for o in &p.options {
                assert!(o.title.chars().count() <= 40, "{:?}: title too long: {}", p.title, o.title);
                assert!(o.detail.chars().count() <= 66, "{:?}: detail too long ({}): {}", p.title, o.detail.chars().count(), o.detail);
                assert!(!o.title.is_empty(), "no unlabeled choices");
            }
            for b in p.buttons.iter().flatten() {
                assert!(b.label.chars().count() <= 16, "{}", b.label);
            }
            assert!(p.window_title.starts_with("Install: "));
        }
    }

    #[test]
    fn the_first_page_says_what_where_and_what_it_needs() {
        let p = install_page(&info());
        assert_eq!(p.title, "Install The Wind Waker HD?");
        assert_eq!(p.card.as_ref().unwrap().lines, ["Wii U  \u{b7}  v1.2"]);
        assert_eq!(p.facts[0], "Goes to: SD card > Ports");
        assert_eq!(p.facts[1], "Your own copy of The Wind Waker HD: a Wii U copy, .wua or .wud");
        let titles: Vec<&str> = p.options.iter().map(|o| o.title.as_str()).collect();
        assert_eq!(titles, ["Install", "Not now"]);
        assert!(p.options[0].recommended);
        assert_eq!(action_at(&p, initial_focus(&p)), Some(WizAct::Install));
        assert_eq!(p.back_hint, "Not now");
        assert_eq!(p.buttons[LEFT].as_ref().unwrap().action, WizAct::ProjectPage);
        let mut none = info();
        none.needs.clear();
        none.has_project_page = false;
        let p = install_page(&none);
        assert_eq!((p.facts[1].as_str(), p.mark_of(1)), ("No game files needed", Mark::Done));
        assert!(p.buttons[LEFT].is_none());
    }

    #[test]
    fn a_deck_ready_project_never_claims_it_needs_no_game_files() {
        let mut own = info();
        own.own_entry = true;
        own.needs.clear();
        let p = install_page(&own);
        assert_eq!(p.facts[1], "Game files: its project page says what it needs");
        assert!(p.paragraphs.iter().any(|t| t.contains("ports.local.json")));
    }

    #[test]
    fn the_same_game_note_folds_into_the_first_page_and_not_now_is_recommended() {
        let mut same = info();
        same.same = Some(SameNote { verdict: Verdict::SameGame, owned_project: "2ship2harkinian".into() });
        let p = install_page(&same);
        assert_eq!(p.paragraphs[0], "You already have this game as 2ship2harkinian. This is another project for the same game.");
        let titles: Vec<&str> = p.options.iter().map(|o| o.title.as_str()).collect();
        assert_eq!(titles, ["Not now", "Install it as well"]);
        assert!(p.options[0].recommended && !p.options[1].recommended);
        assert_eq!(p.options[1].detail, "Downloads it as a second copy, with its own saves and settings.");
        assert_eq!(action_at(&p, initial_focus(&p)), Some(WizAct::NotNow));
        let mut other = info();
        other.same = Some(SameNote { verdict: Verdict::AnotherVersion, owned_project: "MM3D-PC".into() });
        let p = install_page(&other);
        assert_eq!(p.paragraphs[0], "You have another version of this game: MM3D-PC.");
        assert_eq!(p.options[1].title, "Install it as well");
    }

    #[test]
    fn an_update_says_update_and_that_saves_are_backed_up() {
        let mut u = info();
        u.update = true;
        let p = install_page(&u);
        assert_eq!(p.title, "Update The Wind Waker HD?");
        assert_eq!(p.options[0].title, "Update");
        assert_eq!(p.card.as_ref().unwrap().lines, ["Wii U  \u{b7}  Installed now: v1.2"]);
        assert!(p.paragraphs.iter().any(|t| t.contains("backed up")));
    }

    #[test]
    fn what_line_takes_what_is_known() {
        assert_eq!(what_line(None, None, None, false), None);
        assert_eq!(what_line(Some("N64"), None, Some(120_000_000), false).as_deref(), Some("N64  \u{b7}  120 MB"));
        assert_eq!(what_line(Some(" "), Some("v2"), None, false).as_deref(), Some("v2"));
    }

    #[test]
    fn system_labels_read_as_people_say_them() {
        use crate::fork::rom_library::system::System;
        assert_eq!(sys_label(&Sys::Known(System::WiiU)), "Wii U");
        assert_eq!(sys_label(&Sys::Other("pc")), "PC");
        assert_eq!(sys_label(&Sys::Other("mac")), "Mac");
        assert_eq!(sys_label(&Sys::Other("xone")), "Xbox One");
    }

    #[test]
    fn needs_come_from_the_entry_in_plain_words() {
        let port = crate::core::models::port_from_value(&serde_json::json!({"name": "Ship of Harkinian (Shipwright)", "folder": "soh", "source": "https://github.com/a/b", "instructions": "Required files: oot.z64"})).unwrap();
        let spec = RomSpec::for_port(&port, None);
        let words = needs_words(&spec, port.display_name());
        assert_eq!(words.len(), 1, "{words:?}");
        assert!(words[0].contains("oot.z64") && !words[0].contains("CRC"), "{words:?}");
        let bare = crate::core::models::port_from_value(&serde_json::json!({"name": "Fan game", "folder": "fan", "source": "https://github.com/a/c"})).unwrap();
        assert!(needs_words(&RomSpec::for_port(&bare, None), "Fan game").is_empty());
    }

    #[test]
    fn bytes_and_speed_read_plainly() {
        assert_eq!(mb_of_mb(12_300_000, Some(120_000_000)), "12 MB of 120 MB");
        assert_eq!(mb_of_mb(4_200_000, Some(9_500_000)), "4.2 MB of 9.5 MB");
        assert_eq!(mb_of_mb(400_000_000, Some(1_200_000_000)), "0.4 GB of 1.2 GB");
        assert_eq!(mb_of_mb(12_000_000, None), "12 MB so far");
        assert_eq!(speed_text(4.2e6), "4.2 MB/s");
        assert_eq!(speed_text(12.6e6), "13 MB/s");
        assert_eq!(speed_text(850e3), "850 KB/s");
        assert_eq!(speed_text(10.0), "under 1 KB/s");
        assert_eq!(speed_text(f64::NAN), "under 1 KB/s");
    }

    #[test]
    fn the_speed_is_over_the_last_seconds_and_a_new_download_starts_over() {
        let mut s = Speed::default();
        s.add(0.0, 0);
        assert_eq!(s.per_second(), None, "one sample says nothing");
        s.add(0.25, 1_000_000);
        assert_eq!(s.per_second(), None, "under half a second");
        s.add(1.0, 4_000_000);
        assert_eq!(s.per_second(), Some(4_000_000.0));
        for i in 2..=10 {
            s.add(i as f64, 4_000_000 + (i - 1) * 1_000_000);
        }
        assert_eq!(s.per_second(), Some(1_000_000.0), "the slow last seconds, not the fast start");
        s.add(11.0, 10);
        assert_eq!(s.per_second(), None, "a count that went down starts over");
    }

    #[test]
    fn home_tile_note_is_short() {
        assert_eq!(tile_note(Phase::Downloading, 42, Some(100)), "Downloading 42%");
        assert_eq!(tile_note(Phase::Downloading, 200, Some(100)), "Downloading 100%");
        assert_eq!(tile_note(Phase::Downloading, 12_000_000, None), "Downloading 12 MB");
        assert_eq!(tile_note(Phase::Downloading, 0, None), "Downloading...");
        assert_eq!(tile_note(Phase::Installing, 0, None), "Installing...");
    }

    #[test]
    fn the_meter_follows_the_installer() {
        let m = Meter::new();
        assert_eq!((m.phase(), m.bytes()), (Phase::Downloading, (0, None)));
        m.note("Downloading Dusklight.AppImage...");
        m.record(5, Some(10));
        assert_eq!((m.phase(), m.bytes()), (Phase::Downloading, (5, Some(10))));
        m.note("Extracting...");
        assert_eq!(m.phase(), Phase::Installing);
        m.note("Downloading again?");
        assert_eq!(m.phase(), Phase::Installing, "once installing, it stays");
        assert!(!m.stopping() && !m.cancel_flag().load(Ordering::Relaxed));
        m.stop();
        assert!(m.stopping() && m.cancel_flag().load(Ordering::Relaxed));
        assert_eq!(phase_of("Backing up saves..."), Phase::Installing);
    }

    #[test]
    fn the_downloading_page_has_a_real_bar_and_labelled_choices() {
        let s = Snapshot { phase: Phase::Downloading, downloaded: 30_000_000, total: Some(120_000_000), speed: Some(4.2e6), stopping: false };
        let p = downloading_page("Dusklight", &None, &s);
        assert_eq!(p.title, "Downloading Dusklight");
        assert_eq!(p.progress, Some(Progress::Part(250)));
        assert_eq!(p.facts, ["30 MB of 120 MB", "4.2 MB/s"]);
        let titles: Vec<&str> = p.options.iter().map(|o| o.title.as_str()).collect();
        assert_eq!(titles, ["Keep going in the background", "Stop and remove the download"]);
        assert_eq!(p.options[1].detail, "Nothing is kept");
        assert!(p.buttons.iter().all(Option::is_none), "no unlabeled buttons");
        assert_eq!(p.back_hint, "Keep going in the background");
        let unknown = downloading_page("Dusklight", &None, &Snapshot { total: None, speed: None, ..s });
        assert_eq!((unknown.progress, unknown.paragraphs[0].as_str()), (Some(Progress::Busy), "Downloading..."));
        assert_eq!(unknown.facts, ["30 MB so far"]);
        let installing = downloading_page("Dusklight", &None, &Snapshot { phase: Phase::Installing, ..s });
        assert_eq!((installing.title.as_str(), installing.progress), ("Installing Dusklight", Some(Progress::Busy)));
        assert_eq!(installing.options.len(), 1, "an install that has started unpacking is not stopped half way");
        assert!(installing.facts.is_empty(), "no speed while unpacking");
        let stopping = downloading_page("Dusklight", &None, &Snapshot { stopping: true, ..s });
        assert!(stopping.options.is_empty() && stopping.no_back);
    }

    #[test]
    fn the_kept_files_page_lists_them_and_says_what_is_backed_up() {
        let offer = KeptOffer {
            when: "2026-10-31 10:00".into(),
            files: vec![("oot.o2r".into(), "game file".into()), ("prod.keys".into(), "key".into())],
            replaces: 1,
            kept_place: "Internal storage > Saves Backup > Kept files".into(),
            replaced_place: "Internal storage > Saves Backup > Replaced by restore".into(),
        };
        let p = kept_page("Ship of Harkinian", &None, &offer);
        assert_eq!(p.facts, ["oot.o2r (game file)", "prod.keys (key)"]);
        assert!(p.paragraphs[0].contains("2026-10-31 10:00"));
        assert!(p.paragraphs[1].starts_with("1 of them is in the port's folder already"));
        assert_eq!(action_at(&p, initial_focus(&p)), Some(WizAct::PutBack));
        assert_eq!(p.back_hint, "Not now");
        let none = kept_page("X", &None, &KeptOffer { replaces: 0, ..offer });
        assert_eq!(none.paragraphs.len(), 1);
    }

    #[test]
    fn steam_and_done_pages() {
        let s = steam_page("X", &None, false, false);
        assert_eq!(s.title, "Add it to Steam with its artwork?");
        assert_eq!(action_at(&s, initial_focus(&s)), Some(WizAct::SteamYes));
        assert_eq!(s.paragraphs.len(), 1);
        assert!(steam_page("X", &None, false, true).paragraphs[1].contains("close"));
        assert!(steam_page("X", &None, true, true).paragraphs[1].contains("Desktop Mode"));
        let ready = done_page("Dusklight", &None, true, &[(Mark::Done, "Added to Steam".into())]);
        assert_eq!(ready.title, "Dusklight is ready");
        assert_eq!(action_at(&ready, initial_focus(&ready)), Some(WizAct::PlayNow));
        assert_eq!(ready.facts, ["Added to Steam"]);
        let missing = done_page("Dusklight", &None, false, &[]);
        assert_eq!(missing.title, "Dusklight is installed; it needs its game files before it starts");
        assert_eq!(action_at(&missing, initial_focus(&missing)), Some(WizAct::Home));
        let titles: Vec<&str> = missing.options.iter().map(|o| o.title.as_str()).collect();
        assert_eq!(titles, ["Play now", "Back to Home"]);
        assert_eq!(missing.back_hint, "Back to Home");
        let f = finished_page("Dusklight", &None, true);
        assert_eq!(f.title, "Dusklight finished updating");
    }
}
