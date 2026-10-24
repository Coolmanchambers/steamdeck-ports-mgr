//! The one page about the old copies moves left behind ("Remove the 2 old copies?"), instead of one question
//! per port, and the "Does it start?" cards. No UI code here; the screen is in
//! `src/app/ports_folder_screen/batch.rs`.
//!
//! Every port is listed with one status line (`old_copies::Readiness`): "Waiting for Steam to update", "Start
//! it once from its new place", "Ready to remove", or why it stays when its `removal_check` failed. Only the
//! Ready ones that passed the check are offered. "Keep them" is the recommended choice, has the focus, and is
//! what B does: nothing is removed unless the user chooses "Remove them", and each removal then runs the same
//! checks (readiness included) and the same saves backup as removing a single old copy. "It starts fine..."
//! asks about each port that waits for a first start, one card at a time.

use crate::fork::art::thumbs::ArtRef;
use crate::fork::page::{Card, Focus, Page, PageButton, PageOption};
use crate::fork::port_mover::old_copies::{OldCopy, Readiness};
use crate::fork::rom_screens::util::{ellipsize, human_bytes};
use crate::fork::screens::short_path;
use std::path::Path;

/// An old copy with what its check said (`Ok` with the bytes it would free, or why it must stay) and whether
/// it is ready to go.
#[derive(Debug, Clone)]
pub struct Checked {
    pub copy: OldCopy,
    pub check: Result<u64, String>,
    pub ready: Readiness,
    pub art: Option<ArtRef>,
}

impl Checked {
    pub fn removable(&self) -> bool {
        self.ready.is_ready() && self.check.is_ok()
    }

    /// The port's line on the page.
    fn status_line(&self) -> String {
        match (&self.check, self.ready) {
            (_, r) if !r.is_ready() => format!("{}: {}", self.copy.name, r.line()),
            (Err(why), _) => format!("{} stays: {}", self.copy.name, ellipsize(why, 90)),
            (Ok(_), r) => format!("{}: {}", self.copy.name, r.line()),
        }
    }

    /// Why it stays, whole, for the closing text.
    fn why_kept(&self) -> String {
        match (&self.check, self.ready) {
            (_, r) if !r.is_ready() => r.line().to_lowercase(),
            (Err(why), _) => why.clone(),
            (Ok(_), _) => String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchAction {
    Keep,
    Remove,
    /// Show the old folders (paths and sizes) and where the save backups go.
    Details,
    /// Nothing can be removed: just carry on.
    Ok,
    /// Say which ports start fine from their new place (the cards).
    Confirm,
    /// Close Steam and make its waiting changes (the shared flow), then look again.
    UpdateSteam,
}

/// What a "Does it start?" card's choices do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    Yes,
    NotYet,
}

pub const WINDOW_TITLE: &str = "Import ports: old copies";

/// Where the page was opened from, for its header and window title.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Origin {
    /// After an import (and the setup wizard, whose header replaces this one).
    #[default]
    Import,
    /// Settings > Ports folder: after a move, "Finish switching", or an entry chosen in Old copies.
    PortsFolder,
}

impl Origin {
    pub fn header(self) -> &'static str {
        match self {
            Origin::Import => crate::fork::import::cards::HEADER,
            Origin::PortsFolder => "Ports folder",
        }
    }

    pub fn window_title(self) -> &'static str {
        match self {
            Origin::Import => WINDOW_TITLE,
            Origin::PortsFolder => "Ports folder: old copies",
        }
    }
}

/// Said on the last page of the setup wizard when no old copy is ready yet.
pub const KEPT_UNTIL_CHECKED: &str = "Old copies are kept until each port has been checked.";

pub struct Batch {
    pub items: Vec<Checked>,
    pub origin: Origin,
    /// Steam changes waiting, so a "Waiting for Steam" port has a way on from this page.
    pub steam: crate::fork::import::cards::SteamWaiting,
}

impl Batch {
    pub fn new(items: Vec<Checked>) -> Batch {
        Batch { items, origin: Origin::Import, steam: crate::fork::import::cards::SteamWaiting::Nothing }
    }

    pub fn from(items: Vec<Checked>, origin: Origin) -> Batch {
        Batch { items, origin, steam: crate::fork::import::cards::SteamWaiting::Nothing }
    }

    pub fn with_steam(mut self, steam: crate::fork::import::cards::SteamWaiting) -> Batch {
        self.steam = steam;
        self
    }

    pub fn removable(&self) -> Vec<&Checked> {
        self.items.iter().filter(|c| c.removable()).collect()
    }

    pub fn kept(&self) -> Vec<&Checked> {
        self.items.iter().filter(|c| !c.removable()).collect()
    }

    /// The ports that wait for a first start from their new place (the cards ask about these).
    pub fn to_confirm(&self) -> Vec<&Checked> {
        self.items.iter().filter(|c| c.ready == Readiness::StartOnce).collect()
    }

    /// What removing them would free. A link's target stays, so a link frees nothing.
    pub fn freed_bytes(&self) -> u64 {
        self.removable().iter().filter(|c| !c.copy.link_only).filter_map(|c| c.check.as_ref().ok()).sum()
    }

    fn names(items: &[&Checked]) -> String {
        items.iter().map(|c| c.copy.name.as_str()).collect::<Vec<_>>().join(", ")
    }

    /// What is said when nothing is removed (the user kept them, or none could go).
    pub fn kept_text(&self) -> String {
        let removable = self.removable();
        let mut parts: Vec<String> = Vec::new();
        if !removable.is_empty() {
            parts.push(format!("Kept the old {} of {}. Remove {} later in Settings, Ports folder, Old copies.", copies(removable.len()), Batch::names(&removable), if removable.len() == 1 { "it" } else { "them" }));
        }
        parts.extend(self.kept().iter().map(|c| format!("The old copy of {} stays: {}", c.copy.name, c.why_kept())));
        parts.join("\n")
    }

    /// The old folders and their sizes, and where the saves of a removed copy are zipped first: the paths are
    /// behind Details, not on the page. `home` shortens the paths; `saves` is where the backups go.
    pub fn details_text(&self, home: &Path, saves: &Path) -> String {
        let mut lines: Vec<String> = Vec::new();
        for c in &self.items {
            let size = match &c.check {
                Ok(_) if c.copy.link_only => "a link, the folder it points to stays".to_string(),
                Ok(bytes) => human_bytes(*bytes),
                Err(_) => human_bytes(c.copy.bytes),
            };
            lines.push(format!("{}: {} ({size}). {}", c.copy.name, short_path(&c.copy.old_path, home), c.ready.line()));
            if let Err(why) = &c.check {
                if c.ready.is_ready() {
                    lines.push(format!("  Kept: {why}"));
                }
            }
        }
        lines.push(String::new());
        lines.push(format!("Before an old copy is removed, its saves are zipped into {}.", short_path(saves, home)));
        lines.push("An old copy can go once Steam starts the new copy and the port was started from its new place (or you said it starts and your saves are there).".to_string());
        lines.push("Whatever is kept stays listed under Settings, Ports folder, Old copies.".to_string());
        lines.join("\n")
    }

    fn confirm_option(&self) -> Option<PageOption<BatchAction>> {
        let n = self.to_confirm().len();
        (n > 0).then(|| PageOption {
            title: "It starts fine...".to_string(),
            detail: if self.items.len() == 1 { "Say it starts and has your saves".to_string() } else { format!("Say which of {} start and have your saves", if n == 1 { "1 port".to_string() } else { format!("{n} ports") }) },
            choice: None,
            enabled: true,
            recommended: false,
            action: BatchAction::Confirm,
        })
    }

    /// The page. Keep (or OK) is first, recommended and focused; Remove is never the default.
    pub fn page(&self) -> Page<BatchAction> {
        let removable = self.removable();
        let n = removable.len();
        let mut p = Page::<BatchAction> { window_title: self.origin.window_title().to_string(), large: true, header: self.origin.header().to_string(), ..Page::default() };
        p.facts = self.items.iter().map(Checked::status_line).collect();
        p.buttons[crate::fork::page::LEFT] = Some(PageButton { label: "Details".to_string(), action: BatchAction::Details });
        p.initial = Focus::Option(0);
        let one = self.items.len() == 1;
        let waiting_for_steam = self.items.iter().any(|c| c.ready == Readiness::WaitingForSteam);
        let start_note = (!self.to_confirm().is_empty()).then(|| "Played it from Steam already? It may only show after Steam restarts, or say so with It starts fine...".to_string());
        if n == 0 {
            p.title = if one { "This old copy stays for now".to_string() } else { "The old copies stay for now".to_string() };
            p.paragraphs = vec![if one { "The old copy is kept until its port has been checked.".to_string() } else { KEPT_UNTIL_CHECKED.to_string() }];
            p.options = vec![PageOption { title: "OK".to_string(), detail: String::new(), choice: None, enabled: true, recommended: true, action: BatchAction::Ok }];
            match self.steam {
                // Steam's changes are the next thing: they can be made from here (Desktop Mode).
                crate::fork::import::cards::SteamWaiting::Desktop(w) if waiting_for_steam && w > 0 => {
                    p.paragraphs.push("Steam still starts the old copies until its shortcuts are updated.".to_string());
                    p.options[0].recommended = false;
                    p.options.insert(0, PageOption { title: crate::fork::steam_close::CLOSE_LABEL.to_string(), detail: "Then start each port once from its new place".to_string(), choice: None, enabled: true, recommended: true, action: BatchAction::UpdateSteam });
                }
                crate::fork::import::cards::SteamWaiting::GameMode(w) if waiting_for_steam && w > 0 => p.paragraphs.push(crate::fork::steam_close::GAME_MODE_TEXT.to_string()),
                _ => p.paragraphs.push("An old copy can go once Steam starts the new copy and the port has been started from there.".to_string()),
            }
            p.paragraphs.extend(start_note);
            p.options.extend(self.confirm_option());
            return p;
        }
        p.title = if n == 1 { "Remove the old copy?".to_string() } else { format!("Remove the {n} old copies?") };
        let freed = self.freed_bytes();
        let size = if freed > 0 { format!(" The old {} {} {}.", copies(n), if n == 1 { "uses" } else { "use" }, human_bytes(freed)) } else { String::new() };
        p.paragraphs = vec![format!("{} from {} new place, so {} old {} can go.{size}", if n == 1 { "This port starts" } else { "These ports start" }, if n == 1 { "its" } else { "their" }, if n == 1 { "its" } else { "their" }, copies(n))];
        p.paragraphs.extend(start_note);
        // Every port is named on its status line, so no cover row: the page fits a 7 inch screen without scrolling.
        let (keep, remove) = if n == 1 { ("Keep it", "Remove it") } else { ("Keep them", "Remove them") };
        p.options = vec![
            PageOption { title: keep.to_string(), detail: format!("Nothing is deleted. You can remove {} later from Settings.", if n == 1 { "it" } else { "them" }), choice: None, enabled: true, recommended: true, action: BatchAction::Keep },
            PageOption { title: remove.to_string(), detail: if one { "Its saves are zipped to a backup first".to_string() } else { "Only the ready ones. Saves are zipped to a backup first".to_string() }, choice: None, enabled: true, recommended: false, action: BatchAction::Remove },
        ];
        p.options.extend(self.confirm_option());
        p
    }
}

/// "Does Majora's Mask start from its new place?": one card, `index` of `count`.
pub fn confirm_page(c: &Checked, index: usize, count: usize, origin: Origin) -> Page<ConfirmAction> {
    let mut p = Page::<ConfirmAction> {
        window_title: format!("{}: does it start", origin.window_title()),
        large: true,
        header: origin.header().to_string(),
        step_text: format!("Port {} of {count}", index + 1),
        title: "Does it start from its new place?".to_string(),
        ..Page::default()
    };
    p.card = Some(Card {
        art: c.art.clone(),
        qr: None,
        title: c.copy.name.clone(),
        lines: vec!["Start it from Steam or from this app, play a moment, and check that your saves are there.".to_string(), "Say yes only when both are true. The old copy is still only removed when you choose Remove.".to_string()],
    });
    p.options = vec![
        PageOption { title: "Yes, it starts and my saves are there".to_string(), detail: String::new(), choice: None, enabled: true, recommended: true, action: ConfirmAction::Yes },
        PageOption { title: "Not yet".to_string(), detail: "Its old copy stays".to_string(), choice: None, enabled: true, recommended: false, action: ConfirmAction::NotYet },
    ];
    p.initial = Focus::Option(0);
    p
}

fn copies(n: usize) -> &'static str {
    if n == 1 { "copy" } else { "copies" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::port_mover::old_copies::StartedBy;
    use std::path::PathBuf;

    fn copy(name: &str, bytes: u64, link_only: bool) -> OldCopy {
        OldCopy {
            port_key: name.to_lowercase(),
            name: name.to_string(),
            old_path: PathBuf::from(format!("/home/deck/Games/{name}")),
            new_path: PathBuf::from(format!("/run/media/sd/Ports/{name}")),
            bytes,
            moved_at: String::new(),
            link_only,
            switched: true,
            import_folder: None,
            started: None,
            steam_at: None,
            repoint_queued: None,
        }
    }

    fn ok(name: &str, bytes: u64) -> Checked {
        Checked { copy: copy(name, bytes, false), check: Ok(bytes), ready: Readiness::Ready(StartedBy::App), art: Some(ArtRef { folder: name.to_lowercase(), url: None }) }
    }

    fn waiting(name: &str, ready: Readiness) -> Checked {
        Checked { copy: copy(name, 7, false), check: Err("Steam shortcuts still point into it".to_string()), ready, art: None }
    }

    fn stays(name: &str, why: &str) -> Checked {
        Checked { copy: copy(name, 5, false), check: Err(why.to_string()), ready: Readiness::Ready(StartedBy::User), art: None }
    }

    fn all_text(p: &Page<BatchAction>) -> String {
        let mut parts = vec![p.title.clone()];
        parts.extend(p.paragraphs.iter().cloned());
        parts.extend(p.facts.iter().cloned());
        parts.extend(p.tiles.iter().map(|t| t.label.clone()));
        parts.extend(p.options.iter().flat_map(|o| [o.title.clone(), o.detail.clone()]));
        parts.join("\n")
    }

    #[test]
    fn the_page_asks_once_for_the_ready_old_copies_and_keeping_is_the_default() {
        let b = Batch::new(vec![ok("Dusklight", 3_000_000_000), ok("SoH", 200_000_000), ok("Abuse", 10_000_000), ok("Pinball", 1_000_000)]);
        let p = b.page();
        assert_eq!(p.title, "Remove the 4 old copies?");
        assert_eq!(p.window_title, "Import ports: old copies");
        assert_eq!(p.paragraphs, ["These ports start from their new place, so their old copies can go. The old copies use 3.2 GB."]);
        assert!(p.tiles.is_empty(), "the names are on the status lines");
        assert_eq!(p.options.iter().map(|o| (o.title.as_str(), o.action, o.recommended)).collect::<Vec<_>>(), [("Keep them", BatchAction::Keep, true), ("Remove them", BatchAction::Remove, false)]);
        assert_eq!(p.initial, Focus::Option(0), "Keep them has the focus");
        assert_eq!(p.facts, ["Dusklight: Ready to remove", "SoH: Ready to remove", "Abuse: Ready to remove", "Pinball: Ready to remove"]);
    }

    #[test]
    fn each_port_has_one_status_line_and_only_ready_ones_are_offered() {
        let b = Batch::new(vec![ok("Severed Chains", 100), waiting("Majora's Mask", Readiness::WaitingForSteam), waiting("Ocarina of Time", Readiness::StartOnce), stays("Twilight Princess", "a program is running from it")]);
        let p = b.page();
        assert_eq!(p.title, "Remove the old copy?", "only the ready one that passed its check counts");
        assert_eq!(
            p.facts,
            ["Severed Chains: Ready to remove", "Majora's Mask: Waiting for Steam to update", "Ocarina of Time: Start it once from its new place", "Twilight Princess stays: a program is running from it"]
        );
        assert_eq!(b.removable().iter().map(|c| c.copy.name.as_str()).collect::<Vec<_>>(), ["Severed Chains"]);
        assert_eq!(p.options.iter().map(|o| o.action).collect::<Vec<_>>(), [BatchAction::Keep, BatchAction::Remove, BatchAction::Confirm]);
        assert_eq!(b.to_confirm().iter().map(|c| c.copy.name.as_str()).collect::<Vec<_>>(), ["Ocarina of Time"]);
    }

    #[test]
    fn a_port_that_is_not_ready_is_never_removable_even_when_its_check_passed() {
        for r in [Readiness::WaitingForSteam, Readiness::StartOnce] {
            let c = Checked { ready: r, ..ok("SoH", 10) };
            assert!(!c.removable(), "{r:?}");
            assert!(Batch::new(vec![c]).removable().is_empty());
        }
    }

    #[test]
    fn tims_deck_now_every_copy_waits_and_the_page_says_they_stay_until_checked() {
        let b = Batch::new(vec![
            waiting("Severed Chains", Readiness::WaitingForSteam),
            waiting("Majora's Mask", Readiness::WaitingForSteam),
            waiting("Ocarina of Time", Readiness::WaitingForSteam),
            waiting("Twilight Princess", Readiness::WaitingForSteam),
        ]);
        let p = b.page();
        assert_eq!(p.title, "The old copies stay for now");
        assert_eq!(p.paragraphs[0], KEPT_UNTIL_CHECKED);
        assert_eq!(p.options.iter().map(|o| o.action).collect::<Vec<_>>(), [BatchAction::Ok], "nothing to confirm while Steam still starts the old copies");
        // With Steam's changes waiting, the page leads on to them (Desktop Mode) or says where to finish (Game Mode).
        use crate::fork::import::cards::SteamWaiting;
        let items = || vec![waiting("Severed Chains", Readiness::WaitingForSteam), waiting("Majora's Mask", Readiness::WaitingForSteam)];
        let d = Batch::new(items()).with_steam(SteamWaiting::Desktop(2)).page();
        assert_eq!(d.options.iter().map(|o| o.action).collect::<Vec<_>>(), [BatchAction::UpdateSteam, BatchAction::Ok]);
        assert!(d.options[0].recommended && d.initial == Focus::Option(0));
        let g = Batch::new(items()).with_steam(SteamWaiting::GameMode(2)).page();
        assert_eq!(g.options.iter().map(|o| o.action).collect::<Vec<_>>(), [BatchAction::Ok]);
        assert!(g.paragraphs.contains(&crate::fork::steam_close::GAME_MODE_TEXT.to_string()));
        assert!(p.facts.iter().all(|f| f.ends_with("Waiting for Steam to update")), "{:?}", p.facts);
        assert!(b.kept_text().contains("The old copy of Severed Chains stays: waiting for steam to update"), "{}", b.kept_text());
        // Steam updated, not started yet: the cards are offered.
        let b = Batch::new(vec![waiting("Severed Chains", Readiness::StartOnce), waiting("Majora's Mask", Readiness::StartOnce)]);
        let p = b.page();
        assert_eq!(p.options.iter().map(|o| o.action).collect::<Vec<_>>(), [BatchAction::Ok, BatchAction::Confirm]);
        assert_eq!(p.options[1].detail, "Say which of 2 ports start and have your saves");
        assert_eq!(p.initial, Focus::Option(0), "OK, never the confirm, has the focus");
        assert!(p.paragraphs.iter().any(|t| t.contains("may only show after Steam restarts")), "{:?}", p.paragraphs);
        // One port: singular words.
        let one = Batch::new(vec![waiting("Severed Chains", Readiness::StartOnce)]).page();
        assert_eq!(one.title, "This old copy stays for now");
        assert_eq!(one.options[1].detail, "Say it starts and has your saves");
    }

    #[test]
    fn the_confirm_card_shows_the_port_and_starts_on_yes() {
        let c = waiting("Ocarina of Time", Readiness::StartOnce);
        let p = confirm_page(&c, 1, 3, Origin::Import);
        assert_eq!(p.title, "Does it start from its new place?");
        assert_eq!(p.card.as_ref().unwrap().title, "Ocarina of Time", "the port is named on its card");
        assert_eq!((p.step_text.as_str(), p.window_title.as_str()), ("Port 2 of 3", "Import ports: old copies: does it start"));
        assert_eq!(p.options.iter().map(|o| o.action).collect::<Vec<_>>(), [ConfirmAction::Yes, ConfirmAction::NotYet]);
        assert_eq!(p.options[0].title, "Yes, it starts and my saves are there");
        assert!(p.card.as_ref().unwrap().lines[0].contains("check that your saves are there"));
    }

    #[test]
    fn from_settings_the_page_says_ports_folder_not_import_ports() {
        let c = waiting("SoH", Readiness::StartOnce);
        let p = Batch::from(vec![c.clone()], Origin::PortsFolder).page();
        assert_eq!((p.header.as_str(), p.window_title.as_str()), ("Ports folder", "Ports folder: old copies"));
        let card = confirm_page(&c, 0, 1, Origin::PortsFolder);
        assert_eq!((card.header.as_str(), card.window_title.as_str()), ("Ports folder", "Ports folder: old copies: does it start"));
        assert_eq!(Batch::new(vec![c]).page().header, "Import ports");
    }

    #[test]
    fn one_old_copy_reads_in_the_singular() {
        let p = Batch::new(vec![ok("Dusklight", 3_000_000_000)]).page();
        assert_eq!(p.options[1].detail, "Its saves are zipped to a backup first");
        assert_eq!(p.title, "Remove the old copy?");
        assert_eq!(p.paragraphs, ["This port starts from its new place, so its old copy can go. The old copy uses 3.0 GB."]);
        assert_eq!(p.options[0].title, "Keep it");
        assert_eq!(p.options[1].title, "Remove it");
    }

    #[test]
    fn when_nothing_can_go_the_page_says_so_and_has_only_ok() {
        let b = Batch::new(vec![stays("Dusklight", "a program is running from it"), stays("SoH", "saves newer than the new copy")]);
        let p = b.page();
        assert_eq!(p.title, "The old copies stay for now");
        assert_eq!(p.options.len(), 1);
        assert_eq!(p.options[0].action, BatchAction::Ok);
        assert!(p.tiles.is_empty());
        assert_eq!(p.facts.len(), 2);
        assert!(b.kept_text().contains("The old copy of Dusklight stays: a program is running from it"));
    }

    #[test]
    fn a_link_frees_nothing_so_the_size_sentence_leaves_it_out() {
        let link = Checked { copy: copy("Linked", 9_000_000_000, true), check: Ok(9_000_000_000), ready: Readiness::Ready(StartedBy::App), art: None };
        let b = Batch::new(vec![ok("Dusklight", 2_000_000_000), link.clone()]);
        assert_eq!(b.freed_bytes(), 2_000_000_000);
        assert!(b.page().paragraphs[0].contains("use 2.0 GB"));
        assert_eq!(Batch::new(vec![link]).page().paragraphs[0], "This port starts from its new place, so its old copy can go.");
    }

    #[test]
    fn keeping_them_says_which_stay_and_where_to_remove_them_later() {
        let b = Batch::new(vec![ok("Dusklight", 1), ok("SoH", 1), stays("Abuse", "no longer in the catalog")]);
        let t = b.kept_text();
        assert!(t.contains("Kept the old copies of Dusklight, SoH. Remove them later in Settings, Ports folder, Old copies."), "{t}");
        assert!(t.contains("The old copy of Abuse stays: no longer in the catalog"), "{t}");
        let one = Batch::new(vec![ok("Dusklight", 1)]).kept_text();
        assert!(one.starts_with("Kept the old copy of Dusklight. Remove it later"), "{one}");
    }

    #[test]
    fn a_long_reason_is_shortened_on_the_page_but_kept_whole_in_the_closing_text() {
        let long = "x".repeat(300);
        let b = Batch::new(vec![ok("A", 1), stays("B", &long)]);
        assert!(b.page().facts[1].len() < 140, "{}", b.page().facts[1]);
        assert!(b.kept_text().contains(&long));
    }

    #[test]
    fn no_page_shows_a_path_or_a_file_name() {
        let b = Batch::new(vec![ok("Dusklight", 100), stays("SoH", "Steam shortcuts still point into it"), waiting("X", Readiness::StartOnce)]);
        for p in [b.page(), Batch::new(vec![stays("SoH", "x")]).page()] {
            let t = all_text(&p);
            assert!(!t.contains("/home") && !t.contains("/run/") && !t.to_lowercase().contains("json"), "{t}");
        }
    }

    #[test]
    fn details_list_each_old_folder_with_its_size_and_status_and_say_where_the_save_backups_go() {
        let b = Batch::new(vec![ok("Dusklight", 3_000_000_000), stays("SoH", "Steam shortcuts still point into it")]);
        let text = b.details_text(Path::new("/home/deck"), Path::new("/home/deck/Apps/dpm/Saves Backup"));
        assert!(text.contains("Dusklight: ~/Games/Dusklight (3.0 GB). Ready to remove") || text.contains("Dusklight: /home/deck/Games/Dusklight (3.0 GB). Ready to remove"), "{text}");
        assert!(text.contains("SoH: ") && text.contains("(5 B)") && text.contains("Kept: Steam shortcuts still point into it"), "{text}");
        assert!(text.contains("saves are zipped into ~/Apps/dpm/Saves Backup"), "{text}");
        assert!(text.contains("Settings, Ports folder, Old copies"), "{text}");
        for page in [b.page(), Batch::new(vec![stays("SoH", "x")]).page()] {
            let details = page.buttons[crate::fork::page::LEFT].as_ref().expect("Details is in the footer");
            assert_eq!(details.action, BatchAction::Details);
        }
    }
}
