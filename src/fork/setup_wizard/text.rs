//! What each wizard page says and which parts of it can take focus. A [`Page`] is plain data: a title,
//! paragraphs (wrapped by the window, never selectable), an optional summary card, a few real options
//! and up to four footer buttons (Back on the left, Skip next to it, the rerun's "Close the wizard",
//! and the primary Next or Finish on the right). Only enabled options and buttons take the controller's
//! focus; [`move_focus`] decides where the D-pad goes. Plain words for someone who has never used the
//! app; English only, not part of upstream's translation tables.

use super::flow::{Action, Mode, Step, View, Wizard};
use super::places::{Kind, QuickChoice};
use crate::fork::rom_screens::util::ellipsize;
use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "Deck Ports Manager";

/// Footer slots, left to right.
pub const LEFT: usize = 0;
pub const MIDDLE: usize = 1;
pub const EXTRA: usize = 2;
pub const RIGHT: usize = 3;
pub const SLOTS: usize = 4;

/// What the screen reads from the app when it draws.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// The ports folder in use now.
    pub ports_folder: PathBuf,
    /// The ports folder when this wizard run started (the ports step's Next says "Keep this folder"
    /// until a different one is chosen).
    pub started_with: PathBuf,
    /// The ports folder in use is the default one next to the app (so an SD card is worth suggesting).
    pub current_is_default: bool,
    pub quick: Vec<QuickChoice>,
    pub rom_folder: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageOption {
    pub title: String,
    pub detail: String,
    /// `Some(chosen)` draws a radio mark (a choice); `None` is an action that opens something.
    pub choice: Option<bool>,
    /// False: shown dimmed, cannot be chosen or focused (the folder in use, a drive that cannot hold ports).
    pub enabled: bool,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageButton {
    pub label: String,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub step_number: usize,
    pub step_count: usize,
    pub title: String,
    pub paragraphs: Vec<String>,
    /// Lines of the summary card (what is set, what was found).
    pub facts: Vec<String>,
    pub options: Vec<PageOption>,
    pub footnote: String,
    /// Left, middle, extra, right.
    pub buttons: [Option<PageButton>; SLOTS],
    /// Which button is drawn as the primary one.
    pub primary: Option<usize>,
    /// Where the cursor starts.
    pub initial: Focus,
}

/// Something that can take focus: option `n`, or the button in slot `n`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Option(usize),
    Button(usize),
}

fn btn(label: impl Into<String>, action: Action) -> Option<PageButton> {
    Some(PageButton { label: label.into(), action })
}

fn shown(path: &Path) -> String {
    ellipsize(&path.to_string_lossy(), 64)
}

fn paras(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn page(w: &Wizard, title: &str) -> Page {
    Page {
        step_number: w.step.index() + 1,
        step_count: Step::ALL.len(),
        title: title.to_string(),
        paragraphs: Vec::new(),
        facts: Vec::new(),
        options: Vec::new(),
        footnote: String::new(),
        buttons: [None, None, None, None],
        primary: None,
        initial: Focus::Button(RIGHT),
    }
}

/// The window's title (also its name to the window system).
pub fn title(w: &Wizard) -> String {
    match &w.view {
        View::ConfirmSkipSetup => "Setup wizard: skip setup?".into(),
        View::Notice(_) | View::SaveFailed(_) => "Setup wizard".into(),
        View::Step => match w.step {
            Step::Welcome => "Setup wizard: welcome".into(),
            Step::Done => "Setup wizard: all done".into(),
            step => format!("Setup wizard: {}", step_name(step)),
        },
    }
}

fn step_name(step: Step) -> &'static str {
    match step {
        Step::Welcome => "Welcome",
        Step::PortsFolder => "Ports folder",
        Step::Import => "Ports you already have",
        Step::RomFolder => "Game files",
        Step::Steam => "Steam",
        Step::Done => "All done",
    }
}

/// "Step 2 of 6" for the header.
pub fn step_text(page: &Page) -> String {
    format!("Step {} of {}", page.step_number, page.step_count)
}

pub fn page_for(w: &Wizard, ctx: &Context) -> Page {
    match &w.view {
        View::ConfirmSkipSetup => {
            let mut p = page(w, "Skip setup?");
            p.paragraphs = paras(&["Nothing will be changed.", "You can open the setup wizard again any time from Settings > Setup wizard."]);
            p.buttons[LEFT] = btn("Keep going with setup", Action::No);
            p.buttons[RIGHT] = btn("Skip setup", Action::Yes);
            // A stray A must not skip setup.
            p.initial = Focus::Button(LEFT);
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
        View::Step => step_page(w, ctx),
    }
}

/// Back, Skip where allowed, "Close the wizard" in a rerun, and the primary button.
fn nav(p: &mut Page, w: &Wizard, next_label: &str) {
    p.buttons[LEFT] = btn("Back", Action::Back);
    if w.step.skippable() {
        p.buttons[MIDDLE] = btn("Skip this step", Action::Skip);
    }
    if w.mode == Mode::Rerun {
        p.buttons[EXTRA] = btn("Close the wizard", Action::SkipSetup);
    }
    p.buttons[RIGHT] = btn(next_label, Action::Next);
    p.primary = Some(RIGHT);
}

fn action_option(title: &str, detail: &str, action: Action) -> PageOption {
    PageOption { title: title.into(), detail: detail.into(), choice: None, enabled: true, action }
}

fn step_page(w: &Wizard, ctx: &Context) -> Page {
    match w.step {
        Step::Welcome => {
            let mut p = page(w, "Welcome");
            p.paragraphs = paras(&[
                "Welcome to Deck Ports Manager. It installs and updates PC versions of console games (ports), connects them to the game files you own, and adds them to Steam with artwork.",
                "Setup takes a few minutes. These are the steps:\n   Step 2: Choose where ports are kept\n   Step 3: Find ports you already have\n   Step 4: Say where your game files are\n   Step 5: Add your ports to Steam\n   Step 6: See what was set",
            ]);
            p.footnote = "You can skip steps 3 to 5. On step 2, Next keeps the current folder. Nothing changes until you choose it. To run setup again later, open Settings > Setup wizard.".into();
            p.buttons[LEFT] = btn(if w.mode == Mode::FirstRun { "Skip setup..." } else { "Close the wizard" }, Action::SkipSetup);
            p.buttons[RIGHT] = btn("Start setup", Action::Next);
            p.primary = Some(RIGHT);
            p
        }
        Step::PortsFolder => ports_page(w, ctx),
        Step::Import => {
            let mut p = page(w, "Ports you already have");
            p.paragraphs = paras(&["Do you already have ports installed outside this app? It can find them and add them to your list. Nothing is deleted, and you are asked before anything is moved."]);
            if let Some(n) = w.summary.imported {
                p.facts = vec![match n {
                    0 => "No new ports were added.".to_string(),
                    1 => "1 port was added to your list.".to_string(),
                    n => format!("{n} ports were added to your list."),
                }];
            }
            p.options.push(action_option("Find ports I already have...", "Look for ports installed elsewhere", Action::FindPorts));
            nav(&mut p, w, "Next");
            p
        }
        Step::RomFolder => {
            let mut p = page(w, "Your game files");
            p.paragraphs = paras(&[
                "Some ports need game files you own, such as a ROM or a disc image. Tell the app which folder holds them and it can find the right file for each port.",
                "Your game files are only read. They are never changed or moved.",
            ]);
            p.facts = vec![match &ctx.rom_folder {
                Some(dir) => format!("Game files folder: {}", shown(dir)),
                None => "Game files folder: not chosen yet".to_string(),
            }];
            let (label, detail) = if ctx.rom_folder.is_some() {
                ("Change the game files folder...", "Pick a different folder and scan it")
            } else {
                ("Choose the game files folder...", "Pick the folder and scan it")
            };
            p.options.push(action_option(label, detail, Action::ChooseRomFolder));
            nav(&mut p, w, "Next");
            p
        }
        Step::Steam => {
            let mut p = page(w, "Add your ports to Steam");
            p.paragraphs = paras(&[
                "Add your ports to Steam so they start from Game Mode, with cover art.",
                "Adding happens in Desktop Mode, with Steam closed. In Game Mode you can queue the change and it is applied the next time you are in Desktop Mode.",
                "Optional: a free SteamGridDB key (under Artwork sources) finds more artwork.",
            ]);
            p.options.push(action_option("Open Add to Steam...", "Add your installed ports as Steam shortcuts", Action::OpenSteam));
            p.options.push(action_option("Open Artwork sources (optional)...", "Add a SteamGridDB key for more artwork", Action::OpenArtSources));
            nav(&mut p, w, "Next");
            p
        }
        Step::Done => done_page(w, ctx),
    }
}

fn ports_page(w: &Wizard, ctx: &Context) -> Page {
    let mut p = page(w, "Where should ports be kept?");
    p.paragraphs = paras(&[
        "Ports you install are kept together in one folder. An SD card usually has the most room.",
        "If you pick a different folder, ports you already installed can be moved there. The old copies are kept until you choose to remove them.",
    ]);
    p.facts = vec![format!("Right now: {}", shown(&ctx.ports_folder))];
    if !ctx.quick.iter().any(|c| c.kind == Kind::Drive) {
        p.facts.push("No SD card found. Put one in and come back to this step (Settings > Setup wizard), or keep internal storage.".to_string());
    }
    for choice in &ctx.quick {
        let in_use = choice.path == ctx.ports_folder;
        p.options.push(PageOption {
            title: choice.title.clone(),
            detail: if in_use { format!("{} - in use now", choice.detail) } else { choice.detail.clone() },
            choice: Some(in_use),
            // The folder in use and a drive that cannot hold ports are shown but cannot be chosen.
            enabled: choice.usable && !in_use,
            action: Action::UseFolder(choice.path.clone()),
        });
    }
    p.options.push(action_option("Choose another folder...", "Browse for any folder", Action::ChooseFolder));
    nav(&mut p, w, if ctx.ports_folder == ctx.started_with { "Keep this folder" } else { "Next" });
    // With an SD card in and the default folder in use, start on the card: a reflex A offers the better
    // place, and the checks still run before anything changes.
    if ctx.current_is_default {
        if let Some(i) = p.options.iter().zip(&ctx.quick).position(|(o, c)| o.enabled && c.kind == Kind::Drive) {
            p.initial = Focus::Option(i);
        }
    }
    p
}

fn done_page(w: &Wizard, ctx: &Context) -> Page {
    let mut p = page(w, "All done");
    p.paragraphs = paras(&["Setup is finished. This is what was set:"]);
    let s = &w.summary;
    p.facts = vec![
        format!("Ports folder: {}", shown(&ctx.ports_folder)),
        match s.imported {
            Some(0) => "Ports you already had: none added".to_string(),
            Some(n) => format!("Ports you already had: {n} added"),
            None if s.skipped.contains(&Step::Import) => "Ports you already had: skipped".to_string(),
            None => "Ports you already had: not checked".to_string(),
        },
        match &ctx.rom_folder {
            Some(dir) => format!("Game files folder: {}", shown(dir)),
            None if s.skipped.contains(&Step::RomFolder) => "Game files folder: skipped".to_string(),
            None if s.rom_folder_opened => "Game files folder: none chosen".to_string(),
            None => "Game files folder: not chosen".to_string(),
        },
        if s.steam_opened {
            "Steam: opened Add to Steam (it says there what was added)".to_string()
        } else if s.skipped.contains(&Step::Steam) {
            "Steam: skipped".to_string()
        } else {
            "Steam: not set up yet".to_string()
        },
    ];
    if s.art_opened {
        p.facts.push("Artwork sources: opened".to_string());
    }
    p.footnote = "Anything you skipped can be done later from Settings. To see this wizard again, open Settings > Setup wizard.".into();
    p.buttons[LEFT] = btn("Back", Action::Back);
    if w.mode == Mode::Rerun {
        p.buttons[EXTRA] = btn("Close the wizard", Action::SkipSetup);
    }
    p.buttons[RIGHT] = btn("Finish", Action::Finish);
    p.primary = Some(RIGHT);
    p
}

/// Everything that can take focus, in order: the enabled options, then the buttons that exist.
pub fn focusables(page: &Page) -> Vec<Focus> {
    let mut out: Vec<Focus> = (0..page.options.len()).filter(|i| page.options[*i].enabled).map(Focus::Option).collect();
    out.extend((0..SLOTS).filter(|s| page.buttons[*s].is_some()).map(Focus::Button));
    out
}

/// Position of `focus` in [`focusables`] (the number the window draws as selected).
pub fn focus_number(page: &Page, focus: Focus) -> usize {
    focusables(page).iter().position(|f| *f == focus).unwrap_or(0)
}

/// The focus number of option `index`, or -1 when it cannot take focus.
pub fn option_number(page: &Page, index: usize) -> i32 {
    focusables(page).iter().position(|f| *f == Focus::Option(index)).map_or(-1, |n| n as i32)
}

/// The number of the button in `slot`, or -1 when it is not shown.
pub fn button_number(page: &Page, slot: usize) -> i32 {
    focusables(page).iter().position(|f| *f == Focus::Button(slot)).map_or(-1, |n| n as i32)
}

/// What a focus number stands for.
pub fn focus_at(page: &Page, number: usize) -> Option<Focus> {
    focusables(page).get(number).copied()
}

pub fn initial_focus(page: &Page) -> usize {
    let items = focusables(page);
    let wanted = if items.contains(&page.initial) { page.initial } else { page.primary.map_or(items[0], Focus::Button) };
    focus_number(page, wanted)
}

/// The D-pad: up and down walk the options and then drop to the buttons (the primary one first), left
/// and right move between buttons. It never lands on text or a disabled option, and stays put at an edge.
pub fn move_focus(page: &Page, current: usize, dx: i32, dy: i32) -> usize {
    let items = focusables(page);
    let Some(here) = items.get(current).copied() else { return initial_focus(page) };
    let index_of = |f: Focus| items.iter().position(|x| *x == f);
    let enabled: Vec<usize> = (0..page.options.len()).filter(|i| page.options[*i].enabled).collect();
    let present: Vec<usize> = (0..SLOTS).filter(|s| page.buttons[*s].is_some()).collect();
    let target = match here {
        Focus::Option(i) if dy > 0 => match enabled.iter().find(|o| **o > i) {
            Some(next) => Some(Focus::Option(*next)),
            None => page.primary.filter(|s| present.contains(s)).or_else(|| present.last().copied()).map(Focus::Button),
        },
        Focus::Option(i) if dy < 0 => enabled.iter().rev().find(|o| **o < i).map(|o| Focus::Option(*o)),
        Focus::Button(_) if dy < 0 => enabled.last().map(|o| Focus::Option(*o)),
        Focus::Button(slot) if dx != 0 => {
            let at = present.iter().position(|s| *s == slot).unwrap_or(0) as i32;
            let next = at + dx.signum();
            (next >= 0).then(|| present.get(next as usize)).flatten().map(|s| Focus::Button(*s))
        }
        _ => None,
    };
    target.and_then(index_of).unwrap_or(current)
}

/// What choosing the focused item does.
pub fn action_at(page: &Page, number: usize) -> Option<Action> {
    match focus_at(page, number)? {
        Focus::Option(i) => page.options.get(i).map(|o| o.action.clone()),
        Focus::Button(s) => page.buttons[s].as_ref().map(|b| b.action.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::setup_wizard::flow::Mode;

    fn quick(title: &str, detail: &str, path: &str, kind: Kind, usable: bool) -> QuickChoice {
        QuickChoice { title: title.into(), detail: detail.into(), path: path.into(), kind, usable }
    }

    fn ctx() -> Context {
        Context {
            ports_folder: "/home/deck/Apps/dpm/Library".into(),
            started_with: "/home/deck/Apps/dpm/Library".into(),
            current_is_default: true,
            quick: vec![
                quick("SD card (SD512)", "/run/media/deck/SD512/Ports - new folder", "/run/media/deck/SD512/Ports", Kind::Drive, true),
                quick("Internal storage", "/home/deck/Apps/dpm/Library - 94 GB free", "/home/deck/Apps/dpm/Library", Kind::Internal, true),
            ],
            rom_folder: None,
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

    fn focused(p: &Page, n: usize) -> Focus {
        focus_at(p, n).unwrap()
    }

    #[test]
    fn welcome_says_what_the_app_does_what_the_steps_are_and_where_to_rerun_it() {
        let p = pg(Step::Welcome);
        let t = all_text(&p);
        for must in [
            "installs and updates PC versions of console games",
            "connects them to the game files",
            "adds them to Steam with artwork",
            "Step 2: Choose where ports are kept",
            "Step 3: Find ports you already have",
            "Step 4: Say where your game files are",
            "Step 5: Add your ports to Steam",
            "Step 6: See what was set",
            "Settings > Setup wizard",
        ] {
            assert!(t.contains(must), "missing {must:?} in\n{t}");
        }
        assert_eq!(button_actions(&p), [Action::SkipSetup, Action::Next]);
        assert_eq!(p.buttons[RIGHT].as_ref().unwrap().label, "Start setup");
        assert_eq!(p.primary, Some(RIGHT));
        assert!(p.options.is_empty());
    }

    #[test]
    fn the_welcome_text_agrees_with_the_header_numbering() {
        // The header counts six steps, so the list does not say "four steps" or number from 1.
        let t = all_text(&pg(Step::Welcome));
        assert!(!t.contains("four steps") && !t.contains("1. Choose"), "{t}");
        for (n, step) in Step::ALL.iter().enumerate() {
            assert_eq!(step_text(&pg(*step)), format!("Step {} of 6", n + 1));
        }
    }

    #[test]
    fn the_skip_hint_is_true_the_ports_folder_cannot_be_skipped_and_next_keeps_the_folder() {
        let t = all_text(&pg(Step::Welcome));
        assert!(t.contains("You can skip steps 3 to 5") && t.contains("On step 2, Next keeps the current folder"), "{t}");
        assert!(!t.contains("skip any step"));
        for step in Step::ALL {
            let skippable = matches!(step, Step::Import | Step::RomFolder | Step::Steam);
            assert_eq!(step.skippable(), skippable, "{step:?}");
        }
    }

    #[test]
    fn the_welcome_exit_button_is_skip_setup_on_first_run_and_close_in_a_rerun() {
        assert_eq!(pg(Step::Welcome).buttons[LEFT].as_ref().unwrap().label, "Skip setup...");
        let mut w = at(Step::Welcome);
        w.mode = Mode::Rerun;
        let p = page_for(&w, &ctx());
        assert_eq!(p.buttons[LEFT].as_ref().unwrap().label, "Close the wizard");
        assert!(p.buttons[EXTRA].is_none(), "not twice on the first page");
    }

    #[test]
    fn a_rerun_offers_close_the_wizard_on_every_page_after_the_first() {
        for step in [Step::PortsFolder, Step::Import, Step::RomFolder, Step::Steam, Step::Done] {
            let mut w = at(step);
            w.mode = Mode::Rerun;
            let p = page_for(&w, &ctx());
            let extra = p.buttons[EXTRA].as_ref().unwrap_or_else(|| panic!("{step:?}"));
            assert_eq!((extra.label.as_str(), &extra.action), ("Close the wizard", &Action::SkipSetup));
            // It is reachable between Back or Skip and the primary button.
            assert!(focusables(&p).contains(&Focus::Button(EXTRA)));
            // And the first run has no such button.
            assert!(page_for(&at(step), &ctx()).buttons[EXTRA].is_none(), "{step:?}");
        }
    }

    #[test]
    fn the_ports_folder_step_shows_the_current_folder_real_options_and_the_picker() {
        let p = pg(Step::PortsFolder);
        assert_eq!(p.facts, ["Right now: /home/deck/Apps/dpm/Library"]);
        let t = all_text(&p);
        assert!(t.contains("ports you already installed can be moved there") && t.contains("old copies are kept until you choose to remove them"), "{t}");
        let uses: Vec<_> = p.options.iter().filter_map(|o| match &o.action { Action::UseFolder(path) => Some(path.clone()), _ => None }).collect();
        assert_eq!(uses, [PathBuf::from("/run/media/deck/SD512/Ports"), PathBuf::from("/home/deck/Apps/dpm/Library")], "SD card first");
        assert_eq!(p.options[0].choice, Some(false));
        assert_eq!(p.options[1].choice, Some(true), "the folder in use is the chosen one");
        let last = p.options.last().unwrap();
        assert_eq!((last.title.as_str(), &last.action, last.choice), ("Choose another folder...", &Action::ChooseFolder, None));
        assert_eq!(button_actions(&p), [Action::Back, Action::Next], "the ports folder always has an answer, so no Skip");
    }

    #[test]
    fn the_folder_in_use_is_shown_as_in_use_and_cannot_be_chosen() {
        let p = pg(Step::PortsFolder);
        let current = &p.options[1];
        assert!(!current.enabled);
        assert!(current.detail.contains("in use now"), "{}", current.detail);
        // Focus skips it, and so does the D-pad.
        assert!(!focusables(&p).contains(&Focus::Option(1)));
        let card = focus_number(&p, Focus::Option(0));
        let next = move_focus(&p, card, 0, 1);
        assert_eq!(focused(&p, next), Focus::Option(2), "down from the card goes past the folder in use");
        assert_eq!(focused(&p, move_focus(&p, next, 0, -1)), Focus::Option(0));
        assert_eq!(option_number(&p, 1), -1);
    }

    #[test]
    fn choosing_a_folder_moves_the_chosen_mark_and_the_in_use_row() {
        let mut c = ctx();
        c.ports_folder = "/run/media/deck/SD512/Ports".into();
        let p = page_for(&at(Step::PortsFolder), &c);
        assert_eq!((p.options[0].choice, p.options[1].choice), (Some(true), Some(false)));
        assert!(!p.options[0].enabled && p.options[1].enabled);
        assert_eq!(p.facts[0], "Right now: /run/media/deck/SD512/Ports");
    }

    #[test]
    fn a_drive_that_cannot_hold_ports_is_listed_but_not_selectable() {
        let mut c = ctx();
        c.quick.push(quick("SD card (EXFAT) (can't be used)", "/run/media/deck/EXFAT/Ports - exFAT cannot hold ports", "/run/media/deck/EXFAT/Ports", Kind::Drive, false));
        let p = page_for(&at(Step::PortsFolder), &c);
        let bad = p.options.iter().find(|o| o.title.contains("can't be used")).unwrap();
        assert!(!bad.enabled);
        let n = p.options.iter().position(|o| o.title.contains("can't be used")).unwrap();
        assert!(!focusables(&p).contains(&Focus::Option(n)));
        assert_eq!(option_number(&p, n), -1);
        // Pressing its action by hand (a stale row) is the screen's to refuse: it has no focus number.
        assert!(focusables(&p).iter().all(|f| *f != Focus::Option(n)));
    }

    #[test]
    fn with_an_sd_card_in_and_the_default_folder_in_use_the_cursor_starts_on_the_card() {
        let p = pg(Step::PortsFolder);
        assert_eq!(focused(&p, initial_focus(&p)), Focus::Option(0));
        assert_eq!(action_at(&p, initial_focus(&p)), Some(Action::UseFolder("/run/media/deck/SD512/Ports".into())));
        // Not when a card folder is already in use, when the default is not in use, or without a card.
        let mut c = ctx();
        c.ports_folder = "/run/media/deck/SD512/Ports".into();
        c.current_is_default = false;
        let p = page_for(&at(Step::PortsFolder), &c);
        assert_eq!(focused(&p, initial_focus(&p)), Focus::Button(RIGHT));
        let mut c = ctx();
        c.current_is_default = false;
        let p = page_for(&at(Step::PortsFolder), &c);
        assert_eq!(focused(&p, initial_focus(&p)), Focus::Button(RIGHT));
        let p = page_for(&at(Step::PortsFolder), &no_card_ctx());
        assert_eq!(focused(&p, initial_focus(&p)), Focus::Button(RIGHT));
    }

    #[test]
    fn an_unusable_card_does_not_get_the_initial_focus() {
        let mut c = no_card_ctx();
        c.quick.insert(0, quick("SD card (X) (can't be used)", "exFAT cannot hold ports", "/run/media/deck/X/Ports", Kind::Drive, false));
        let p = page_for(&at(Step::PortsFolder), &c);
        assert_eq!(focused(&p, initial_focus(&p)), Focus::Button(RIGHT));
    }

    #[test]
    fn the_ports_folder_button_says_keep_this_folder_until_a_different_one_is_chosen() {
        let p = pg(Step::PortsFolder);
        assert_eq!(p.buttons[RIGHT].as_ref().unwrap().label, "Keep this folder");
        let mut c = ctx();
        c.ports_folder = "/run/media/deck/SD512/Ports".into();
        let p = page_for(&at(Step::PortsFolder), &c);
        assert_eq!(p.buttons[RIGHT].as_ref().unwrap().label, "Next");
        assert_eq!(p.buttons[RIGHT].as_ref().unwrap().action, Action::Next, "either way it moves on");
    }

    #[test]
    fn without_an_sd_card_the_page_says_so_and_what_to_do() {
        let p = page_for(&at(Step::PortsFolder), &no_card_ctx());
        assert_eq!(p.facts.len(), 2);
        assert!(p.facts[1].contains("No SD card found") && p.facts[1].contains("Settings > Setup wizard") && p.facts[1].contains("keep internal storage"), "{}", p.facts[1]);
        assert_eq!(pg(Step::PortsFolder).facts.len(), 1, "no such line when a card is there");
        // A card that cannot be used still counts as a card, but is listed with its reason.
        let mut c = no_card_ctx();
        c.quick.push(quick("SD card (X) (can't be used)", "exFAT cannot hold ports", "/run/media/deck/X/Ports", Kind::Drive, false));
        assert_eq!(page_for(&at(Step::PortsFolder), &c).facts.len(), 1);
    }

    #[test]
    fn steps_that_can_be_skipped_offer_skip_and_the_others_do_not() {
        for step in Step::ALL {
            let has_skip = pg(step).buttons[MIDDLE].as_ref().is_some_and(|b| b.action == Action::Skip);
            assert_eq!(has_skip, step.skippable(), "{step:?}");
        }
    }

    #[test]
    fn every_middle_step_has_back_left_next_right_and_starts_on_next_unless_a_card_is_suggested() {
        for step in [Step::PortsFolder, Step::Import, Step::RomFolder, Step::Steam] {
            let p = page_for(&at(step), &no_card_ctx());
            assert_eq!(p.buttons[LEFT].as_ref().map(|b| &b.action), Some(&Action::Back), "{step:?}");
            assert_eq!(p.buttons[RIGHT].as_ref().map(|b| &b.action), Some(&Action::Next), "{step:?}");
            assert_eq!(p.primary, Some(RIGHT));
            assert_eq!(action_at(&p, initial_focus(&p)), Some(Action::Next), "{step:?}");
        }
    }

    #[test]
    fn the_import_step_explains_itself_and_reports_what_was_added() {
        let mut w = at(Step::Import);
        let p = page_for(&w, &ctx());
        let t = all_text(&p);
        assert!(t.contains("already have ports installed outside this app") && t.contains("Nothing is deleted"));
        assert!(p.facts.is_empty());
        assert_eq!(p.options[0].action, Action::FindPorts);
        for (n, text) in [(0, "No new ports were added."), (1, "1 port was added to your list."), (4, "4 ports were added to your list.")] {
            w.summary.imported = Some(n);
            assert_eq!(page_for(&w, &ctx()).facts, [text]);
        }
    }

    #[test]
    fn the_game_files_step_says_files_are_only_read_and_names_the_chosen_folder() {
        let mut c = ctx();
        let t = all_text(&page_for(&at(Step::RomFolder), &c));
        assert!(t.contains("only read") && t.contains("never changed or moved") && t.contains("not chosen yet") && t.contains("Choose the game files folder..."));
        c.rom_folder = Some("/run/media/deck/SD512/ROMs".into());
        let p = page_for(&at(Step::RomFolder), &c);
        assert_eq!(p.facts, ["Game files folder: /run/media/deck/SD512/ROMs"]);
        assert_eq!(p.options[0].title, "Change the game files folder...");
        assert_eq!(p.options[0].action, Action::ChooseRomFolder);
    }

    #[test]
    fn the_steam_step_explains_desktop_mode_steam_closed_and_the_game_mode_queue() {
        let p = pg(Step::Steam);
        let t = all_text(&p);
        for must in ["Desktop Mode", "with Steam closed", "queue the change", "SteamGridDB key"] {
            assert!(t.contains(must), "missing {must:?}");
        }
        let actions: Vec<_> = p.options.iter().map(|o| o.action.clone()).collect();
        assert_eq!(actions, [Action::OpenSteam, Action::OpenArtSources]);
    }

    #[test]
    fn the_last_page_summarises_what_was_set_and_what_happened() {
        let mut w = at(Step::Done);
        let mut c = ctx();
        c.rom_folder = Some("/roms".into());
        w.summary.imported = Some(3);
        w.summary.steam_opened = true;
        w.summary.art_opened = true;
        let p = page_for(&w, &c);
        assert_eq!(
            p.facts,
            [
                "Ports folder: /home/deck/Apps/dpm/Library",
                "Ports you already had: 3 added",
                "Game files folder: /roms",
                "Steam: opened Add to Steam (it says there what was added)",
                "Artwork sources: opened"
            ]
        );
        assert!(all_text(&p).contains("Settings > Setup wizard"));
        assert_eq!(action_at(&p, initial_focus(&p)), Some(Action::Finish));
        assert_eq!(p.buttons[RIGHT].as_ref().unwrap().label, "Finish");
    }

    #[test]
    fn the_last_page_says_so_when_things_were_skipped_or_opened_without_a_result() {
        let mut w = at(Step::Done);
        w.summary.skipped = vec![Step::Import, Step::RomFolder, Step::Steam];
        let t = all_text(&page_for(&w, &ctx()));
        assert!(t.contains("Ports you already had: skipped") && t.contains("Game files folder: skipped") && t.contains("Steam: skipped"));
        let t = all_text(&pg(Step::Done));
        assert!(t.contains("not checked") && t.contains("Game files folder: not chosen") && t.contains("Steam: not set up yet"));
        // The game files screen was opened but no folder came of it.
        let mut w = at(Step::Done);
        w.summary.rom_folder_opened = true;
        assert!(all_text(&page_for(&w, &ctx())).contains("Game files folder: none chosen"));
    }

    #[test]
    fn the_skip_question_starts_on_the_safe_button_and_the_notices_on_ok() {
        let mut w = at(Step::Welcome);
        w.view = View::ConfirmSkipSetup;
        let p = page_for(&w, &ctx());
        assert!(all_text(&p).contains("Nothing will be changed") && all_text(&p).contains("Settings > Setup wizard"));
        assert_eq!(button_actions(&p), [Action::No, Action::Yes]);
        assert_eq!(action_at(&p, initial_focus(&p)), Some(Action::No), "a stray A must not skip setup");
        assert_eq!(p.primary, None);

        w.view = View::Notice("Something went wrong.\n\nMore detail.".into());
        let p = page_for(&w, &ctx());
        assert_eq!(p.paragraphs, ["Something went wrong.", "More detail."]);
        assert_eq!(button_actions(&p), [Action::Ok]);
    }

    #[test]
    fn a_failed_save_page_offers_close_anyway_and_ok_and_starts_on_ok() {
        let mut w = at(Step::Done);
        w.view = View::SaveFailed("Setup could not be saved.\n\nfork-settings.json: disk full".into());
        let p = page_for(&w, &ctx());
        assert_eq!(button_actions(&p), [Action::CloseAnyway, Action::Ok]);
        assert_eq!(p.buttons[LEFT].as_ref().unwrap().label, "Close anyway");
        assert_eq!(action_at(&p, initial_focus(&p)), Some(Action::Ok));
        assert!(all_text(&p).contains("disk full"));
        assert!(title(&w).starts_with("Setup wizard"));
        // Left from OK reaches Close anyway: a controller can leave.
        let ok = initial_focus(&p);
        assert_eq!(action_at(&p, move_focus(&p, ok, -1, 0)), Some(Action::CloseAnyway));
    }

    #[test]
    fn titles_name_the_step_and_all_start_with_setup_wizard() {
        let mut w = at(Step::PortsFolder);
        assert_eq!(title(&w), "Setup wizard: Ports folder");
        w.step = Step::Steam;
        assert_eq!(title(&w), "Setup wizard: Steam");
        for step in Step::ALL {
            assert!(title(&at(step)).starts_with("Setup wizard"));
        }
        w.view = View::ConfirmSkipSetup;
        assert!(title(&w).starts_with("Setup wizard"));
    }

    #[test]
    fn questions_keep_the_step_they_interrupt_in_the_header() {
        let mut w = at(Step::Import);
        w.view = View::ConfirmSkipSetup;
        assert_eq!(step_text(&page_for(&w, &ctx())), "Step 3 of 6");
    }

    // -- focus ---------------------------------------------------------------------------------

    #[test]
    fn only_enabled_options_and_buttons_take_focus_never_text() {
        let p = pg(Step::PortsFolder); // options: card, folder in use (disabled), choose another
        assert_eq!(focusables(&p), [Focus::Option(0), Focus::Option(2), Focus::Button(LEFT), Focus::Button(RIGHT)]);
        let p = pg(Step::Welcome);
        assert_eq!(focusables(&p), [Focus::Button(LEFT), Focus::Button(RIGHT)], "a page with no options has only its buttons");
        assert_eq!(button_number(&p, LEFT), 0);
        assert_eq!(button_number(&p, MIDDLE), -1);
        assert_eq!(button_number(&p, RIGHT), 1);
    }

    #[test]
    fn the_primary_button_has_the_initial_focus_except_where_a_card_is_suggested() {
        for step in Step::ALL {
            let p = page_for(&at(step), &no_card_ctx());
            assert_eq!(focused(&p, initial_focus(&p)), Focus::Button(RIGHT), "{step:?}");
        }
    }

    #[test]
    fn down_walks_the_options_then_lands_on_the_primary_button_and_up_comes_back() {
        let p = pg(Step::Steam); // options 0, 1; buttons Back, Skip, Next
        assert_eq!(focusables(&p).len(), 5);
        let start = initial_focus(&p);
        assert_eq!(focused(&p, start), Focus::Button(RIGHT));
        let up = move_focus(&p, start, 0, -1);
        assert_eq!(focused(&p, up), Focus::Option(1));
        let up = move_focus(&p, up, 0, -1);
        assert_eq!(focused(&p, up), Focus::Option(0));
        assert_eq!(move_focus(&p, up, 0, -1), up, "stays at the top");
        let down = move_focus(&p, up, 0, 1);
        assert_eq!(focused(&p, down), Focus::Option(1));
        let down = move_focus(&p, down, 0, 1);
        assert_eq!(focused(&p, down), Focus::Button(RIGHT), "the primary button first");
        assert_eq!(move_focus(&p, down, 0, 1), down, "stays on the buttons");
    }

    #[test]
    fn left_and_right_move_between_buttons_only() {
        let p = pg(Step::Import); // Back, Skip, Next
        let next = initial_focus(&p);
        let skip = move_focus(&p, next, -1, 0);
        assert_eq!(focused(&p, skip), Focus::Button(MIDDLE));
        let back = move_focus(&p, skip, -1, 0);
        assert_eq!(focused(&p, back), Focus::Button(LEFT));
        assert_eq!(move_focus(&p, back, -1, 0), back);
        assert_eq!(move_focus(&p, back, 1, 0), skip);
        assert_eq!(move_focus(&p, next, 1, 0), next);
        // Left and right on an option do nothing.
        assert_eq!(move_focus(&p, 0, 1, 0), 0);
        assert_eq!(move_focus(&p, 0, -1, 0), 0);
    }

    #[test]
    fn the_rerun_close_button_sits_between_skip_and_the_primary_button() {
        let mut w = at(Step::Import);
        w.mode = Mode::Rerun;
        let p = page_for(&w, &ctx());
        let order: Vec<Focus> = focusables(&p).into_iter().filter(|f| matches!(f, Focus::Button(_))).collect();
        assert_eq!(order, [Focus::Button(LEFT), Focus::Button(MIDDLE), Focus::Button(EXTRA), Focus::Button(RIGHT)]);
        let next = initial_focus(&p);
        let close = move_focus(&p, next, -1, 0);
        assert_eq!(action_at(&p, close), Some(Action::SkipSetup));
        assert_eq!(action_at(&p, move_focus(&p, close, -1, 0)), Some(Action::Skip));
    }

    #[test]
    fn with_a_missing_middle_button_left_and_right_skip_over_the_gap() {
        let p = page_for(&at(Step::PortsFolder), &no_card_ctx()); // Back, Next
        let next = initial_focus(&p);
        let back = move_focus(&p, next, -1, 0);
        assert_eq!(focused(&p, back), Focus::Button(LEFT));
        assert_eq!(move_focus(&p, back, 1, 0), next);
    }

    #[test]
    fn a_page_with_only_buttons_ignores_up_and_down() {
        let p = pg(Step::Welcome);
        let start = initial_focus(&p);
        assert_eq!(move_focus(&p, start, 0, -1), start);
        assert_eq!(move_focus(&p, start, 0, 1), start);
    }

    #[test]
    fn choosing_the_focused_item_gives_its_action() {
        let p = pg(Step::PortsFolder); // card, (folder in use), choose another, Back, Keep this folder
        assert_eq!(action_at(&p, 0), Some(Action::UseFolder("/run/media/deck/SD512/Ports".into())));
        assert_eq!(action_at(&p, 1), Some(Action::ChooseFolder));
        assert_eq!(action_at(&p, 2), Some(Action::Back));
        assert_eq!(action_at(&p, 3), Some(Action::Next));
        assert_eq!(action_at(&p, 9), None);
    }

    #[test]
    fn a_stale_focus_number_falls_back_to_the_initial_one() {
        let p = pg(Step::Welcome);
        assert_eq!(move_focus(&p, 99, 0, 1), initial_focus(&p));
    }
}
