//! What each wizard page says. The wizard's pages are [`Page`]s of `crate::fork::page` (plain data: a title,
//! one or two short sentences, a summary box, a few big options and up to four footer buttons: Back on the
//! left, the rerun's "Close the wizard"); the focus rules (only enabled options and buttons take the
//! controller's focus, where the D-pad goes) live there too and are re-exported here. Each page asks one
//! question and choosing an answer moves on, so there is no Next button. Plain words for someone who has
//! never used the app, no file paths, English only, not part of upstream's translation tables.

use super::flow::{Action, Mode, SteamOutcome, Step, View, Wizard};
use super::places::{Kind, QuickChoice};
pub use crate::fork::page::{Focus, EXTRA, LEFT, RIGHT};
#[cfg(test)]
pub use crate::fork::page::MIDDLE;
#[cfg(test)]
pub use crate::fork::page::{action_at, button_number, focus_at, focus_number, focusables, initial_focus, move_focus, option_number};
use std::path::PathBuf;

pub const APP_NAME: &str = "Deck Ports Manager";

pub type Page = crate::fork::page::Page<Action>;
pub type PageOption = crate::fork::page::PageOption<Action>;
pub type PageButton = crate::fork::page::PageButton<Action>;

/// What the screen reads from the app when it draws.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// The ports folder in use now.
    pub ports_folder: PathBuf,
    /// The ports folder in use is the default one next to the app (so an SD card is worth suggesting).
    pub current_is_default: bool,
    pub quick: Vec<QuickChoice>,
    /// A game files folder is set.
    pub rom_folder_set: bool,
    /// Where Steam stands; read only while the Steam page is up (`None` before that).
    pub steam: Option<super::steam::Info>,
}

fn btn(label: impl Into<String>, action: Action) -> Option<PageButton> {
    Some(PageButton { label: label.into(), action })
}

fn paras(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn page(w: &Wizard, title: &str) -> Page {
    Page {
        large: true,
        header: APP_NAME.to_string(),
        step_number: w.step.number(),
        step_count: Step::COUNTED,
        step_text: format!("Step {} of {}", w.step.number(), Step::COUNTED),
        title: title.to_string(),
        ..Page::default()
    }
}

/// The window's title (also its name to the window system).
pub fn title(w: &Wizard) -> String {
    match &w.view {
        View::ConfirmSkipSetup => "Setup wizard: skip setup?".into(),
        View::Notice(_) | View::SaveFailed(_) => "Setup wizard".into(),
        View::PickAccount(_) => "Setup wizard: Steam account".into(),
        View::Step => match w.step {
            Step::Done => "Setup wizard: all done".into(),
            step => format!("Setup wizard: {}", step_name(step)),
        },
    }
}

fn step_name(step: Step) -> &'static str {
    match step {
        Step::PortsFolder => "Ports folder",
        Step::YourPorts => "Your ports",
        Step::GameFiles => "Game files",
        Step::Steam => "Steam",
        Step::OldCopies => "Old copies",
        Step::Done => "All done",
    }
}

/// "Step 2 of 5" for the header.
pub fn step_text(page: &Page) -> String {
    page.step_text.clone()
}

pub fn page_for(w: &Wizard, ctx: &Context) -> Page {
    let mut page = page_inside(w, ctx);
    page.window_title = title(w);
    page
}

fn page_inside(w: &Wizard, ctx: &Context) -> Page {
    match &w.view {
        View::ConfirmSkipSetup => {
            let mut p = page(w, "Skip setup?");
            p.paragraphs = paras(&["Nothing will be changed. You can run setup again from Settings > Setup wizard."]);
            let mut keep = option("Keep going with setup", "", Action::No);
            keep.recommended = true;
            p.options = vec![keep, option("Skip setup", "Leave without setting anything up", Action::Yes)];
            // A stray A must not skip setup.
            p.initial = Focus::Option(0);
            p
        }
        View::Notice(message) => {
            let mut p = page(w, "Please note");
            p.paragraphs = message.split("\n\n").map(str::to_string).collect();
            p.buttons[RIGHT] = btn("OK", Action::Ok);
            p.primary = Some(RIGHT);
            p
        }
        View::SaveFailed(message) => {
            let mut p = page(w, "Setup could not be saved");
            p.paragraphs = message.split("\n\n").map(str::to_string).collect();
            p.buttons[LEFT] = btn("Close anyway", Action::CloseAnyway);
            p.buttons[RIGHT] = btn("OK", Action::Ok);
            p.primary = Some(RIGHT);
            p
        }
        View::PickAccount(names) => {
            let mut p = page(w, "Which Steam account?");
            p.paragraphs = paras(&["Your ports are added to one account."]);
            p.options = names.iter().enumerate().map(|(i, name)| option(name, "", Action::ChooseAccount(i))).collect();
            p.buttons[LEFT] = btn("Back", Action::Back);
            p.initial = Focus::Option(0);
            p
        }
        View::Step => step_page(w, ctx),
    }
}

fn option(title: &str, detail: &str, action: Action) -> PageOption {
    PageOption { title: title.into(), detail: detail.into(), choice: None, enabled: true, recommended: false, action }
}

fn recommended(title: &str, detail: &str, action: Action) -> PageOption {
    PageOption { recommended: true, ..option(title, detail, action) }
}

/// Back (and, in a rerun, "Close the wizard" beside it). The first page has no Back: its B asks about skipping.
fn nav(p: &mut Page, w: &Wizard) {
    if w.step == Step::PortsFolder {
        p.buttons[LEFT] = btn(if w.mode == Mode::FirstRun { "Skip setup..." } else { "Close the wizard" }, Action::SkipSetup);
        return;
    }
    p.buttons[LEFT] = btn("Back", Action::Back);
    if w.mode == Mode::Rerun {
        p.buttons[EXTRA] = btn("Close the wizard", Action::SkipSetup);
    }
}

/// Where the cursor starts: the recommended option, else the first one that can be chosen.
fn start_on_the_recommended(p: &mut Page) {
    let first = p.options.iter().position(|o| o.recommended && o.enabled).or_else(|| p.options.iter().position(|o| o.enabled));
    if let Some(i) = first {
        p.initial = Focus::Option(i);
    }
}

fn step_page(w: &Wizard, ctx: &Context) -> Page {
    match w.step {
        Step::PortsFolder => ports_page(w, ctx),
        Step::YourPorts => {
            let mut p = page(w, "Ports you already have");
            p.paragraphs = paras(&["Look for ports on this device and move them into your ports folder.", "Each is copied and checked first. Old copies stay until you say."]);
            if let Some(n) = w.summary.imported {
                p.facts = vec![moved_line(n)];
            }
            p.options = vec![recommended("Look for my ports", "", Action::FindPorts), option("Skip", "Do this later from Settings", Action::Skip)];
            nav(&mut p, w);
            start_on_the_recommended(&mut p);
            p
        }
        Step::GameFiles => {
            let mut p = page(w, "Where are your game files?");
            p.paragraphs = paras(&["Some ports need a game file you own, such as a ROM.", "Your files are only read, never changed or moved."]);
            if ctx.rom_folder_set {
                p.facts = vec!["A game files folder is set.".to_string()];
                p.options = vec![recommended("Keep it", "Go on with the folder that is set", Action::Next), option("Pick another folder...", "Choose a different folder", Action::ChooseRomFolder)];
            } else {
                p.options = vec![recommended("Pick a folder...", "The folder that holds your ROMs", Action::ChooseRomFolder), option("Skip for now", "Do this later from Settings", Action::Skip)];
            }
            nav(&mut p, w);
            start_on_the_recommended(&mut p);
            p
        }
        Step::Steam => steam_page(w, ctx),
        // Normally this step opens the "Remove the old copies?" page and has none of its own.
        Step::OldCopies => {
            let mut p = page(w, "Old copies");
            p.paragraphs = paras(&["Some old copies are kept. They are listed under Settings > Ports folder > Old copies."]);
            p.options = vec![recommended("Continue", "", Action::Next)];
            nav(&mut p, w);
            start_on_the_recommended(&mut p);
            p
        }
        Step::Done => done_page(w, ctx),
    }
}

fn steam_page(w: &Wizard, ctx: &Context) -> Page {
    let info = ctx.steam.clone().unwrap_or_default();
    let mut p = page(w, "Add your ports to Steam?");
    if let Some(problem) = &info.problem {
        p.paragraphs = vec![problem.clone()];
        p.options = vec![recommended("Continue", "Do this later from Settings", Action::Next)];
    } else if info.to_add == 0 {
        p.paragraphs = paras(&["Nothing to add: no ports are installed yet, or they are already in Steam."]);
        p.options = vec![recommended("Continue", "", Action::Next)];
    } else {
        let n = info.to_add;
        p.paragraphs = vec![
            format!("{} can be added. They start from Game Mode, with cover art.", if n == 1 { "1 port".to_string() } else { format!("{n} ports") }),
            if info.immediate {
                "Steam is closed, so they are added now. Your Steam shortcut list is backed up first.".to_string()
            } else {
                "Steam is running, so they are queued and added next time you are in Desktop Mode with Steam closed.".to_string()
            },
        ];
        p.options = vec![
            recommended(&format!("Add all {n}"), "", Action::AddToSteam),
            option("Artwork sources (optional)", "A free SteamGridDB key finds more art", Action::OpenArtSources),
            option("Skip", "Do this later from Settings", Action::Skip),
        ];
    }
    nav(&mut p, w);
    start_on_the_recommended(&mut p);
    p
}

fn steam_line(w: &Wizard, ctx: &Context) -> String {
    let s = &w.summary;
    match &s.steam {
        Some(SteamOutcome::Added(n)) => format!("{} added to Steam.", ports_word(*n)),
        Some(SteamOutcome::Queued { count, game_mode: true }) => format!("{} queued for Desktop Mode.", ports_word(*count)),
        Some(SteamOutcome::Queued { count, game_mode: false }) => format!("{} queued until Steam is closed.", ports_word(*count)),
        _ if s.skipped.contains(&Step::Steam) => "Steam: skipped.".to_string(),
        _ if ctx.steam.as_ref().is_some_and(|i| i.to_add == 0) => "Steam: nothing to add.".to_string(),
        _ => "Steam: not set up yet.".to_string(),
    }
}

fn ports_word(n: usize) -> String {
    if n == 1 { "1 port".to_string() } else { format!("{n} ports") }
}

fn moved_line(n: usize) -> String {
    match n {
        0 => "No ports were moved in.".to_string(),
        1 => "1 port was moved into your ports folder.".to_string(),
        n => format!("{n} ports were moved into your ports folder."),
    }
}

fn ports_page(w: &Wizard, ctx: &Context) -> Page {
    let mut p = page(w, "Where should ports go?");
    p.paragraphs = paras(&["Pick a place with room for your games."]);
    if !ctx.quick.iter().any(|c| c.kind == Kind::SdCard) {
        p.paragraphs.push("No SD card found.".to_string());
    }
    let mut suggested = false;
    for choice in &ctx.quick {
        let in_use = choice.path == ctx.ports_folder;
        let mut o = if in_use {
            // The place in use is kept by choosing it: it goes on without changing anything.
            let detail = if choice.note.is_empty() { "In use now".to_string() } else { format!("In use now. {}", choice.note) };
            option(&choice.title, &detail, Action::Next)
        } else {
            option(&choice.title, &choice.note, Action::UseFolder(choice.path.clone()))
        };
        o.enabled = choice.usable;
        // With an SD card in and the default folder in use, the card is suggested: a reflex A offers the better
        // place, and the checks still run before anything changes.
        if choice.usable && !in_use && !suggested && ctx.current_is_default && choice.kind == Kind::SdCard {
            o.recommended = true;
            o.detail = if choice.note.is_empty() { "Recommended".to_string() } else { format!("Recommended. {}", choice.note) };
            suggested = true;
        }
        p.options.push(o);
    }
    p.options.push(option("Pick a folder...", "Browse for any folder", Action::ChooseFolder));
    if w.mode == Mode::FirstRun {
        p.footnote = "Setup takes a few minutes and you can skip steps. You can run it again from Settings > Setup wizard.".to_string();
    }
    nav(&mut p, w);
    // The suggestion, else the place in use (A keeps it), else the first choice that works.
    let in_use = ctx.quick.iter().position(|c| c.path == ctx.ports_folder && c.usable);
    match p.options.iter().position(|o| o.recommended) {
        Some(i) => p.initial = Focus::Option(i),
        None => match in_use.or_else(|| p.options.iter().position(|o| o.enabled)) {
            Some(i) => p.initial = Focus::Option(i),
            None => {}
        },
    }
    p
}

fn ports_where(ctx: &Context) -> String {
    match ctx.quick.iter().find(|c| c.path == ctx.ports_folder).map(|c| c.kind) {
        Some(Kind::SdCard) => "Ports go to your SD card.".to_string(),
        Some(Kind::Internal) => "Ports go to internal storage.".to_string(),
        Some(Kind::OtherDrive) => "Ports go to a USB drive.".to_string(),
        Some(Kind::Current | Kind::Earlier) | None => "Ports go to a folder you picked.".to_string(),
    }
}

fn done_page(w: &Wizard, ctx: &Context) -> Page {
    let mut p = page(w, "You're set up");
    let s = &w.summary;
    p.facts = vec![
        ports_where(ctx),
        match s.imported {
            Some(n) => moved_line(n),
            None if s.skipped.contains(&Step::YourPorts) => "Ports you already have: skipped.".to_string(),
            None => "Ports you already have: not looked for.".to_string(),
        },
        if ctx.rom_folder_set {
            "A game files folder is set.".to_string()
        } else if s.skipped.contains(&Step::GameFiles) {
            "Game files folder: skipped.".to_string()
        } else {
            "No game files folder yet.".to_string()
        },
        steam_line(w, ctx),
    ];
    if let Some(r) = s.old_copies {
        p.facts.push(match (r.removed, r.kept) {
            (0, 0) => "No old copies are kept.".to_string(),
            (0, 1) => "1 old copy is kept. Remove it later in Settings.".to_string(),
            (0, n) => format!("{n} old copies are kept. Remove them later in Settings."),
            (removed, 0) => format!("{} removed.", copies(removed)),
            (removed, kept) => format!("{} removed, {kept} kept.", copies(removed)),
        });
    }
    p.footnote = "Anything you skipped can be done later from Settings. Run setup again from Settings > Setup wizard.".to_string();
    p.options = vec![recommended("Finish", "", Action::Finish)];
    nav(&mut p, w);
    start_on_the_recommended(&mut p);
    p
}

fn copies(n: usize) -> String {
    if n == 1 { "1 old copy".to_string() } else { format!("{n} old copies") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::setup_wizard::flow::{Mode, OldCopiesResult};

    fn quick(title: &str, note: &str, path: &str, kind: Kind, usable: bool) -> QuickChoice {
        QuickChoice { title: title.into(), detail: format!("{path} - {note}"), note: note.into(), path: path.into(), kind, usable }
    }

    fn ctx() -> Context {
        Context {
            ports_folder: "/home/deck/Apps/dpm/Library".into(),
            current_is_default: true,
            quick: vec![
                quick("SD card (SD512)", "a new folder will be made", "/run/media/deck/SD512/Ports", Kind::SdCard, true),
                quick("Internal storage", "94 GB free", "/home/deck/Apps/dpm/Library", Kind::Internal, true),
            ],
            rom_folder_set: false,
            steam: Some(crate::fork::setup_wizard::steam::Info { to_add: 2, immediate: true, ..Default::default() }),
        }
    }

    fn no_card_ctx() -> Context {
        let mut c = ctx();
        c.quick.remove(0);
        c
    }

    fn at(step: Step) -> Wizard {
        let mut w = Wizard::new(Mode::FirstRun);
        w.step = step;
        w
    }

    fn pg(step: Step) -> Page {
        page_for(&at(step), &ctx())
    }

    /// All the text of a page as one line, so a phrase counts even where it wraps.
    fn all_text(p: &Page) -> String {
        let mut parts: Vec<String> = vec![p.title.clone()];
        parts.extend(p.paragraphs.iter().cloned());
        parts.extend(p.facts.iter().cloned());
        parts.push(p.footnote.clone());
        parts.extend(p.options.iter().flat_map(|o| [o.title.clone(), o.detail.clone()]));
        parts.extend(p.buttons.iter().flatten().map(|b| b.label.clone()));
        parts.join(" ").split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn button_actions(p: &Page) -> Vec<Action> {
        p.buttons.iter().flatten().map(|b| b.action.clone()).collect()
    }

    fn option_actions(p: &Page) -> Vec<Action> {
        p.options.iter().map(|o| o.action.clone()).collect()
    }

    fn focused(p: &Page, n: usize) -> Focus {
        focus_at(p, n).unwrap()
    }

    fn start(p: &Page) -> Focus {
        focused(p, initial_focus(p))
    }

    #[test]
    fn there_is_no_welcome_page_and_the_first_page_is_the_ports_folder_question() {
        let p = pg(Step::PortsFolder);
        assert_eq!((p.title.as_str(), p.window_title.as_str()), ("Where should ports go?", "Setup wizard: Ports folder"));
        assert!(p.paragraphs[0].starts_with("Pick a place with room for your games."));
        assert!(p.footnote.contains("Setup takes a few minutes") && p.footnote.contains("Settings > Setup wizard"), "{}", p.footnote);
        assert_eq!(step_text(&p), "Step 1 of 5");
    }

    #[test]
    fn the_header_counts_five_steps_and_the_old_copy_question_is_not_one() {
        let texts: Vec<String> = [Step::PortsFolder, Step::YourPorts, Step::GameFiles, Step::Steam, Step::Done].iter().map(|s| step_text(&pg(*s))).collect();
        assert_eq!(texts, ["Step 1 of 5", "Step 2 of 5", "Step 3 of 5", "Step 4 of 5", "Step 5 of 5"]);
        for step in Step::ALL {
            let p = pg(step);
            assert_eq!(p.step_count, 5);
            assert!(p.large, "the pages are read from further away");
        }
    }

    #[test]
    fn every_page_is_short_and_has_at_most_three_big_choices() {
        let mut c = ctx();
        for kept in [false, true] {
            c.rom_folder_set = kept;
            for step in Step::ALL {
                let p = page_for(&at(step), &c);
                assert!(p.paragraphs.len() <= 2, "{step:?}: {:?}", p.paragraphs);
                assert!(p.options.len() <= if step == Step::PortsFolder { 4 } else { 3 }, "{step:?}");
                assert!(p.paragraphs.iter().all(|t| t.len() <= 110), "{step:?}: {:?}", p.paragraphs);
                assert!(p.facts.len() <= 5, "{step:?}");
            }
        }
    }

    #[test]
    fn no_page_shows_a_file_path_or_a_file_name() {
        let mut c = ctx();
        c.rom_folder_set = true;
        let mut w = at(Step::Done);
        w.summary.imported = Some(2);
        w.summary.steam = Some(SteamOutcome::Added(2));
        w.summary.old_copies = Some(OldCopiesResult { removed: 1, kept: 2 });
        for step in Step::ALL {
            w.step = step;
            for view in [View::Step, View::ConfirmSkipSetup] {
                w.view = view;
                let t = all_text(&page_for(&w, &c));
                // "Settings > Setup wizard" is a menu path, not a file path.
                assert!(!t.contains('/') && !t.contains(".json") && !t.contains("\\"), "{step:?}: {t}");
            }
        }
    }

    #[test]
    fn the_ports_folder_page_offers_the_card_the_internal_storage_and_a_picker() {
        let p = pg(Step::PortsFolder);
        assert_eq!(p.options.iter().map(|o| o.title.as_str()).collect::<Vec<_>>(), ["SD card (SD512)", "Internal storage", "Pick a folder..."]);
        assert_eq!(option_actions(&p), [Action::UseFolder("/run/media/deck/SD512/Ports".into()), Action::Next, Action::ChooseFolder]);
        assert!(p.options[0].recommended && !p.options[1].recommended);
        assert!(p.options[0].detail.starts_with("Recommended") && p.options[0].detail.contains("a new folder will be made"), "{}", p.options[0].detail);
        assert_eq!(p.options[1].detail, "In use now. 94 GB free");
        assert!(p.options.iter().all(|o| o.choice.is_none()), "choices, not radio buttons");
        assert_eq!(start(&p), Focus::Option(0), "the cursor starts on the suggestion");
        assert_eq!(button_actions(&p), [Action::SkipSetup]);
    }

    #[test]
    fn the_place_in_use_is_chosen_to_keep_it_and_goes_on_without_changing_anything() {
        let mut c = ctx();
        c.current_is_default = false;
        let p = page_for(&at(Step::PortsFolder), &c);
        assert!(p.options.iter().all(|o| !o.recommended), "no suggestion unless the default is in use");
        assert_eq!(start(&p), Focus::Option(1), "A keeps the place in use");
        assert_eq!(action_at(&p, initial_focus(&p)), Some(Action::Next));
        // With no card at all it is the same.
        let p = page_for(&at(Step::PortsFolder), &no_card_ctx());
        assert_eq!(start(&p), Focus::Option(0));
        assert_eq!(p.options[0].action, Action::Next);
        assert!(all_text(&p).contains("No SD card found."));
        assert!(!all_text(&pg(Step::PortsFolder)).contains("No SD card found."));
    }

    #[test]
    fn a_card_in_use_is_kept_by_choosing_it_and_the_internal_storage_becomes_a_change() {
        let mut c = ctx();
        c.ports_folder = "/run/media/deck/SD512/Ports".into();
        c.current_is_default = false;
        let p = page_for(&at(Step::PortsFolder), &c);
        assert_eq!(p.options[0].action, Action::Next);
        assert_eq!(p.options[1].action, Action::UseFolder("/home/deck/Apps/dpm/Library".into()));
        assert_eq!(start(&p), Focus::Option(0));
    }

    #[test]
    fn a_drive_that_cannot_hold_ports_is_listed_but_cannot_be_chosen() {
        let mut c = ctx();
        c.quick.push(quick("SD card (EXFAT) (can't be used)", "exFAT cannot hold ports", "/run/media/deck/EXFAT/Ports", Kind::SdCard, false));
        let p = page_for(&at(Step::PortsFolder), &c);
        let n = p.options.iter().position(|o| o.title.contains("can't be used")).unwrap();
        assert!(!p.options[n].enabled && !p.options[n].recommended);
        assert!(!focusables(&p).contains(&Focus::Option(n)));
        assert_eq!(option_number(&p, n), -1);
        // An unusable card is never what the cursor starts on or what is suggested.
        let mut c = no_card_ctx();
        c.quick.insert(0, quick("SD card (X) (can't be used)", "exFAT cannot hold ports", "/run/media/deck/X/Ports", Kind::SdCard, false));
        let p = page_for(&at(Step::PortsFolder), &c);
        assert!(p.options.iter().all(|o| !o.recommended));
        assert_eq!(start(&p), Focus::Option(1));
    }

    #[test]
    fn a_usb_drive_is_offered_but_the_suggestion_is_for_an_sd_card_only() {
        let mut c = no_card_ctx();
        c.quick.insert(0, quick("USB drive (STICK)", "30 GB free", "/run/media/deck/STICK/Ports", Kind::OtherDrive, true));
        let p = page_for(&at(Step::PortsFolder), &c);
        assert!(p.options[0].enabled && !p.options[0].recommended);
        assert!(all_text(&p).contains("No SD card found."));
        assert_eq!(start(&p), Focus::Option(1), "the place in use");
        c.quick.insert(0, quick("SD card (SD512)", "a new folder will be made", "/run/media/deck/SD512/Ports", Kind::SdCard, true));
        let p = page_for(&at(Step::PortsFolder), &c);
        assert!(p.options[0].recommended);
        assert!(!all_text(&p).contains("No SD card found."));
    }

    #[test]
    fn your_ports_has_a_look_choice_and_a_skip_and_reports_what_was_moved() {
        let mut w = at(Step::YourPorts);
        let p = page_for(&w, &ctx());
        assert_eq!((p.title.as_str(), p.window_title.as_str()), ("Ports you already have", "Setup wizard: Your ports"));
        assert_eq!(option_actions(&p), [Action::FindPorts, Action::Skip]);
        assert!(p.options[0].recommended && p.facts.is_empty());
        assert_eq!(start(&p), Focus::Option(0));
        assert!(all_text(&p).contains("Old copies stay until you say"));
        for (n, text) in [(0, "No ports were moved in."), (1, "1 port was moved into your ports folder."), (4, "4 ports were moved into your ports folder.")] {
            w.summary.imported = Some(n);
            assert_eq!(page_for(&w, &ctx()).facts, [text]);
        }
    }

    #[test]
    fn the_game_files_page_asks_for_a_folder_or_keeps_the_one_set_and_says_files_are_only_read() {
        let p = pg(Step::GameFiles);
        assert_eq!(p.title, "Where are your game files?");
        assert!(all_text(&p).contains("only read, never changed or moved") && all_text(&p).contains("ROM"));
        assert_eq!(option_actions(&p), [Action::ChooseRomFolder, Action::Skip]);
        assert!(p.facts.is_empty());
        let mut c = ctx();
        c.rom_folder_set = true;
        let p = page_for(&at(Step::GameFiles), &c);
        assert_eq!(option_actions(&p), [Action::Next, Action::ChooseRomFolder]);
        assert_eq!(p.facts, ["A game files folder is set."]);
        assert_eq!(start(&p), Focus::Option(0));
        assert!(!all_text(&p).contains('/'), "no path");
    }

    fn info(to_add: usize, immediate: bool) -> crate::fork::setup_wizard::steam::Info {
        crate::fork::setup_wizard::steam::Info { to_add, immediate, ..Default::default() }
    }

    fn steam_ctx(i: crate::fork::setup_wizard::steam::Info) -> Context {
        Context { steam: Some(i), ..ctx() }
    }

    #[test]
    fn the_steam_page_offers_add_all_with_the_count_artwork_and_skip() {
        let p = pg(Step::Steam);
        assert_eq!(p.title, "Add your ports to Steam?");
        assert_eq!(option_actions(&p), [Action::AddToSteam, Action::OpenArtSources, Action::Skip]);
        assert_eq!(p.options[0].title, "Add all 2");
        assert!(p.options[0].recommended);
        let t = all_text(&p);
        for must in ["2 ports can be added", "Game Mode", "cover art", "Steam is closed, so they are added now", "backed up first", "SteamGridDB"] {
            assert!(t.contains(must), "missing {must:?} in {t}");
        }
        assert!(!t.contains("shortcuts.vdf"), "no file name");
        assert_eq!(start(&p), Focus::Option(0));
        let one = page_for(&at(Step::Steam), &steam_ctx(info(1, true)));
        assert_eq!(one.options[0].title, "Add all 1");
        assert!(all_text(&one).contains("1 port can be added"));
    }

    #[test]
    fn with_steam_running_the_page_says_the_ports_are_queued() {
        let p = page_for(&at(Step::Steam), &steam_ctx(info(3, false)));
        let t = all_text(&p);
        assert!(t.contains("Steam is running, so they are queued") && t.contains("Desktop Mode with Steam closed"), "{t}");
        assert!(!t.contains("added now"));
    }

    #[test]
    fn nothing_to_add_or_no_steam_account_is_one_continue_and_no_dead_end() {
        let p = page_for(&at(Step::Steam), &steam_ctx(info(0, true)));
        assert_eq!(option_actions(&p), [Action::Next]);
        assert!(all_text(&p).contains("Nothing to add"));
        let problem = crate::fork::setup_wizard::steam::Info { problem: Some("No Steam account folder was found. Start Steam once and sign in, then try again.".into()), ..Default::default() };
        let p = page_for(&at(Step::Steam), &steam_ctx(problem));
        assert_eq!(option_actions(&p), [Action::Next]);
        assert!(all_text(&p).contains("Start Steam once"));
        assert_eq!(start(&p), Focus::Option(0));
    }

    #[test]
    fn the_account_page_lists_the_accounts_and_back_returns() {
        let mut w = at(Step::Steam);
        w.view = View::PickAccount(vec!["Tim".into(), "Kid (kid42)".into()]);
        let p = page_for(&w, &ctx());
        assert_eq!((p.title.as_str(), p.window_title.as_str()), ("Which Steam account?", "Setup wizard: Steam account"));
        assert_eq!(option_actions(&p), [Action::ChooseAccount(0), Action::ChooseAccount(1)]);
        assert_eq!(button_actions(&p), [Action::Back]);
        assert_eq!(step_text(&p), "Step 4 of 5");
    }

    #[test]
    fn the_last_page_says_what_adding_to_steam_did() {
        let say = |steam: Option<SteamOutcome>, skipped: bool, to_add: usize| {
            let mut w = at(Step::Done);
            w.summary.steam = steam;
            if skipped {
                w.summary.skipped.push(Step::Steam);
            }
            page_for(&w, &steam_ctx(info(to_add, true))).facts[3].clone()
        };
        assert_eq!(say(Some(SteamOutcome::Added(4)), false, 4), "4 ports added to Steam.");
        assert_eq!(say(Some(SteamOutcome::Added(1)), false, 1), "1 port added to Steam.");
        assert_eq!(say(Some(SteamOutcome::Queued { count: 3, game_mode: true }), false, 3), "3 ports queued for Desktop Mode.");
        assert_eq!(say(Some(SteamOutcome::Queued { count: 2, game_mode: false }), false, 2), "2 ports queued until Steam is closed.");
        assert_eq!(say(None, true, 2), "Steam: skipped.");
        assert_eq!(say(None, false, 0), "Steam: nothing to add.");
        assert_eq!(say(None, false, 2), "Steam: not set up yet.");
    }

    #[test]
    fn the_last_page_is_plain_short_and_ends_in_finish() {
        let mut w = at(Step::Done);
        let mut c = ctx();
        c.rom_folder_set = true;
        w.summary.imported = Some(3);
        w.summary.steam = Some(SteamOutcome::Added(2));
        w.summary.old_copies = Some(OldCopiesResult { removed: 2, kept: 1 });
        let p = page_for(&w, &c);
        assert_eq!((p.title.as_str(), p.window_title.as_str()), ("You're set up", "Setup wizard: all done"));
        assert_eq!(
            p.facts,
            [
                "Ports go to internal storage.",
                "3 ports were moved into your ports folder.",
                "A game files folder is set.",
                "2 ports added to Steam.",
                "2 old copies removed, 1 kept."
            ]
        );
        assert_eq!(option_actions(&p), [Action::Finish]);
        assert_eq!(start(&p), Focus::Option(0));
        assert_eq!(button_actions(&p), [Action::Back]);
        assert!(all_text(&p).contains("Settings > Setup wizard"));
    }

    #[test]
    fn the_last_page_says_so_when_things_were_skipped_and_where_the_ports_go() {
        let mut w = at(Step::Done);
        w.summary.skipped = vec![Step::YourPorts, Step::GameFiles, Step::Steam];
        let p = page_for(&w, &ctx());
        assert_eq!(p.facts, ["Ports go to internal storage.", "Ports you already have: skipped.", "Game files folder: skipped.", "Steam: skipped."]);
        let p = pg(Step::Done);
        assert_eq!(p.facts[1], "Ports you already have: not looked for.");
        assert_eq!(p.facts[2], "No game files folder yet.");
        assert_eq!(p.facts[3], "Steam: not set up yet.");
        // Where the ports go is told by kind, never by path.
        let mut c = ctx();
        c.ports_folder = "/run/media/deck/SD512/Ports".into();
        assert_eq!(page_for(&at(Step::Done), &c).facts[0], "Ports go to your SD card.");
        c.ports_folder = "/somewhere/else".into();
        assert_eq!(page_for(&at(Step::Done), &c).facts[0], "Ports go to a folder you picked.");
    }

    #[test]
    fn the_old_copy_line_reads_for_each_outcome() {
        let line = |removed, kept| {
            let mut w = at(Step::Done);
            w.summary.old_copies = Some(OldCopiesResult { removed, kept });
            page_for(&w, &ctx()).facts.last().cloned().unwrap()
        };
        assert_eq!(line(1, 0), "1 old copy removed.");
        assert_eq!(line(3, 0), "3 old copies removed.");
        assert_eq!(line(0, 1), "1 old copy is kept. Remove it later in Settings.");
        assert_eq!(line(0, 4), "4 old copies are kept. Remove them later in Settings.");
        assert_eq!(line(0, 0), "No old copies are kept.");
        assert_eq!(pg(Step::Done).facts.len(), 4, "no line when the question was not asked");
    }

    #[test]
    fn the_first_page_has_skip_setup_and_every_later_page_has_back() {
        assert_eq!(pg(Step::PortsFolder).buttons[LEFT].as_ref().unwrap().label, "Skip setup...");
        for step in [Step::YourPorts, Step::GameFiles, Step::Steam, Step::OldCopies, Step::Done] {
            let p = pg(step);
            let left = p.buttons[LEFT].as_ref().unwrap_or_else(|| panic!("{step:?}"));
            assert_eq!((left.label.as_str(), &left.action), ("Back", &Action::Back), "{step:?}");
            assert!(p.buttons[EXTRA].is_none() && p.buttons[RIGHT].is_none() && p.buttons[MIDDLE].is_none(), "{step:?}: no Next button, choosing moves on");
        }
    }

    #[test]
    fn a_rerun_closes_the_wizard_from_the_first_page_and_has_close_beside_back_on_the_others() {
        let mut w = at(Step::PortsFolder);
        w.mode = Mode::Rerun;
        let p = page_for(&w, &ctx());
        assert_eq!(p.buttons[LEFT].as_ref().unwrap().label, "Close the wizard");
        assert!(p.buttons[EXTRA].is_none(), "not twice on the first page");
        assert!(!p.footnote.contains("Setup takes"), "a rerun needs no introduction");
        for step in [Step::YourPorts, Step::GameFiles, Step::Steam, Step::Done] {
            let mut w = at(step);
            w.mode = Mode::Rerun;
            let p = page_for(&w, &ctx());
            let extra = p.buttons[EXTRA].as_ref().unwrap_or_else(|| panic!("{step:?}"));
            assert_eq!((extra.label.as_str(), &extra.action), ("Close the wizard", &Action::SkipSetup));
            assert!(focusables(&p).contains(&Focus::Button(EXTRA)));
            assert!(page_for(&at(step), &ctx()).buttons[EXTRA].is_none(), "{step:?}");
        }
    }

    #[test]
    fn the_skip_question_starts_on_keep_going_and_a_stray_a_does_not_skip() {
        let mut w = at(Step::PortsFolder);
        w.view = View::ConfirmSkipSetup;
        let p = page_for(&w, &ctx());
        assert!(all_text(&p).contains("Nothing will be changed") && all_text(&p).contains("Settings > Setup wizard"));
        assert_eq!(option_actions(&p), [Action::No, Action::Yes]);
        assert_eq!(action_at(&p, initial_focus(&p)), Some(Action::No));
        assert!(p.options[0].recommended);
        assert_eq!(p.title, "Skip setup?");
        assert_eq!(p.window_title, "Setup wizard: skip setup?");
    }

    #[test]
    fn notices_start_on_ok_and_a_failed_save_page_can_be_left_by_the_controller() {
        let mut w = at(Step::Done);
        w.view = View::Notice("Something went wrong.\n\nMore detail.".into());
        let p = page_for(&w, &ctx());
        assert_eq!(p.paragraphs, ["Something went wrong.", "More detail."]);
        assert_eq!(button_actions(&p), [Action::Ok]);
        w.view = View::SaveFailed("Setup could not be saved.\n\nfork-settings.json: disk full".into());
        let p = page_for(&w, &ctx());
        assert_eq!(button_actions(&p), [Action::CloseAnyway, Action::Ok]);
        assert_eq!(action_at(&p, initial_focus(&p)), Some(Action::Ok));
        assert!(all_text(&p).contains("disk full") && title(&w).starts_with("Setup wizard"));
        let ok = initial_focus(&p);
        assert_eq!(action_at(&p, move_focus(&p, ok, -1, 0)), Some(Action::CloseAnyway));
    }

    #[test]
    fn every_title_starts_with_setup_wizard_and_names_the_page() {
        let expected = [
            (Step::PortsFolder, "Setup wizard: Ports folder"),
            (Step::YourPorts, "Setup wizard: Your ports"),
            (Step::GameFiles, "Setup wizard: Game files"),
            (Step::Steam, "Setup wizard: Steam"),
            (Step::OldCopies, "Setup wizard: Old copies"),
            (Step::Done, "Setup wizard: all done"),
        ];
        for (step, title) in expected {
            assert_eq!(super::title(&at(step)), title);
        }
    }

    #[test]
    fn questions_keep_the_step_they_interrupt_in_the_header() {
        let mut w = at(Step::GameFiles);
        w.view = View::ConfirmSkipSetup;
        assert_eq!(step_text(&page_for(&w, &ctx())), "Step 3 of 5");
    }

    // -- focus ---------------------------------------------------------------------------------

    #[test]
    fn only_enabled_options_and_buttons_take_focus_never_text() {
        let mut c = ctx();
        c.quick.push(quick("SD card (X) (can't be used)", "exFAT cannot hold ports", "/run/media/deck/X/Ports", Kind::SdCard, false));
        let p = page_for(&at(Step::PortsFolder), &c);
        assert_eq!(focusables(&p), [Focus::Option(0), Focus::Option(1), Focus::Option(3), Focus::Button(LEFT)]);
        assert_eq!(button_number(&p, LEFT), 3);
        assert_eq!(button_number(&p, RIGHT), -1);
    }

    #[test]
    fn every_page_starts_on_a_choice_that_can_be_chosen_and_never_on_the_riskier_one() {
        let mut c = ctx();
        for set in [false, true] {
            c.rom_folder_set = set;
            for step in Step::ALL {
                let p = page_for(&at(step), &c);
                let Focus::Option(i) = start(&p) else { panic!("{step:?} starts on a button") };
                assert!(p.options[i].enabled, "{step:?}");
                // Nothing starts on a Skip.
                assert_ne!(p.options[i].action, Action::Skip, "{step:?}");
                assert_ne!(p.options[i].action, Action::Yes, "{step:?}");
            }
        }
    }

    #[test]
    fn down_walks_the_choices_then_the_buttons_and_up_comes_back() {
        let p = pg(Step::Steam); // options 0..3, then Back
        assert_eq!(focusables(&p), [Focus::Option(0), Focus::Option(1), Focus::Option(2), Focus::Button(LEFT)]);
        let mut at_ = initial_focus(&p);
        let mut seen = vec![focused(&p, at_)];
        for _ in 0..5 {
            at_ = move_focus(&p, at_, 0, 1);
            seen.push(focused(&p, at_));
        }
        assert_eq!(seen.last(), Some(&Focus::Button(LEFT)), "down stops on the buttons");
        let up = move_focus(&p, at_, 0, -1);
        assert_eq!(focused(&p, up), Focus::Option(2));
    }

    #[test]
    fn left_and_right_move_between_the_footer_buttons_only() {
        let mut w = at(Step::Steam);
        w.mode = Mode::Rerun;
        let p = page_for(&w, &ctx());
        let back = focus_number(&p, Focus::Button(LEFT));
        let close = move_focus(&p, back, 1, 0);
        assert_eq!(action_at(&p, close), Some(Action::SkipSetup));
        assert_eq!(move_focus(&p, close, 1, 0), close);
        assert_eq!(move_focus(&p, close, -1, 0), back);
        // Left and right on a choice do nothing.
        assert_eq!(move_focus(&p, 0, 1, 0), 0);
        assert_eq!(move_focus(&p, 0, -1, 0), 0);
    }

    #[test]
    fn choosing_the_focused_item_gives_its_action() {
        let p = pg(Step::PortsFolder); // card, internal, picker, Skip setup
        assert_eq!(action_at(&p, 0), Some(Action::UseFolder("/run/media/deck/SD512/Ports".into())));
        assert_eq!(action_at(&p, 1), Some(Action::Next));
        assert_eq!(action_at(&p, 2), Some(Action::ChooseFolder));
        assert_eq!(action_at(&p, 3), Some(Action::SkipSetup));
        assert_eq!(action_at(&p, 9), None);
    }

    #[test]
    fn a_stale_focus_number_falls_back_to_the_initial_one() {
        let p = pg(Step::Steam);
        assert_eq!(move_focus(&p, 99, 0, 1), initial_focus(&p));
    }
}
