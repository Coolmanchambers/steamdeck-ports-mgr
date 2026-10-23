//! What each wizard page says and which parts of it can take focus. A [`Page`] is plain data: a title,
//! paragraphs (wrapped by the window, never selectable), an optional summary card, a few real options
//! and up to three footer buttons (Back on the left, Skip next to it, the primary Next or Finish on
//! the right). Only options and buttons take the controller's focus; [`move_focus`] decides where the
//! D-pad goes. Plain words for someone who has never used the app; English only, not part of
//! upstream's translation tables.

use super::flow::{Action, Mode, Step, View, Wizard};
use super::ports_folder::QuickChoice;
use crate::fork::rom_screens::util::ellipsize;
use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "Deck Ports Manager";

/// What the screen reads from the app when it draws.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// The ports folder in use now (or the one chosen in this run).
    pub ports_folder: PathBuf,
    pub quick: Vec<QuickChoice>,
    pub rom_folder: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageOption {
    pub title: String,
    pub detail: String,
    /// `Some(chosen)` draws a radio mark (a choice); `None` is an action that opens something.
    pub choice: Option<bool>,
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
    /// Left, middle, right.
    pub buttons: [Option<PageButton>; 3],
    /// Which button is drawn as the primary one.
    pub primary: Option<usize>,
    /// Where the cursor starts.
    pub initial: Focus,
}

/// Something that can take focus: option `n`, or the button in slot `n` (0 left, 1 middle, 2 right).
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
        buttons: [None, None, None],
        primary: None,
        initial: Focus::Button(2),
    }
}

/// The window's title (also its name to the window system).
pub fn title(w: &Wizard) -> String {
    match &w.view {
        View::ConfirmSkipSetup => "Setup wizard: skip setup?".into(),
        View::ConfirmCreate(_) => "Setup wizard: create this folder?".into(),
        View::Notice(_) => "Setup wizard".into(),
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
            p.buttons = [btn("Keep going with setup", Action::No), None, btn("Skip setup", Action::Yes)];
            // A stray A must not skip setup.
            p.initial = Focus::Button(0);
            p
        }
        View::ConfirmCreate(path) => {
            let mut p = page(w, "Create this folder?");
            p.paragraphs = paras(&["This folder does not exist yet:"]);
            p.facts = vec![shown(path)];
            p.buttons = [btn("Choose a different folder", Action::No), None, btn("Create it and use it", Action::Yes)];
            p.primary = Some(2);
            p
        }
        View::Notice(message) => {
            let mut p = page(w, "Please note");
            p.paragraphs = message.split("\n\n").map(str::to_string).collect();
            p.buttons = [None, None, btn("OK", Action::Ok)];
            p.primary = Some(2);
            p
        }
        View::Step => step_page(w, ctx),
    }
}

fn nav(p: &mut Page, w: &Wizard, next_label: &str) {
    p.buttons[0] = btn("Back", Action::Back);
    if w.step.skippable() {
        p.buttons[1] = btn("Skip this step", Action::Skip);
    }
    p.buttons[2] = btn(next_label, Action::Next);
    p.primary = Some(2);
}

fn action_option(title: &str, detail: &str, action: Action) -> PageOption {
    PageOption { title: title.into(), detail: detail.into(), choice: None, action }
}

fn step_page(w: &Wizard, ctx: &Context) -> Page {
    match w.step {
        Step::Welcome => {
            let mut p = page(w, "Welcome");
            p.paragraphs = paras(&[
                "Welcome to Deck Ports Manager. It installs and updates PC versions of console games (ports), connects them to the game files you own, and adds them to Steam with artwork.",
                "Setup takes a few minutes, in four steps:\n   1. Choose where ports are kept\n   2. Find ports you already have\n   3. Say where your game files are\n   4. Add your ports to Steam",
            ]);
            p.footnote = "You can skip any step. Nothing changes until you choose it. To run setup again later, open Settings > Setup wizard.".into();
            p.buttons = [btn(if w.mode == Mode::FirstRun { "Skip setup..." } else { "Close the wizard" }, Action::SkipSetup), None, btn("Start setup", Action::Next)];
            p.primary = Some(2);
            p
        }
        Step::PortsFolder => {
            let mut p = page(w, "Where should ports be kept?");
            p.paragraphs = paras(&["Ports you install are kept together in one folder. An SD card usually has the most room."]);
            p.facts = vec![format!("Right now: {}", shown(&ctx.ports_folder))];
            for choice in &ctx.quick {
                p.options.push(PageOption {
                    title: choice.title.clone(),
                    detail: choice.detail.clone(),
                    choice: Some(choice.path == ctx.ports_folder),
                    action: Action::UseFolder(choice.path.clone()),
                });
            }
            p.options.push(action_option("Choose another folder...", "Browse for any folder", Action::ChooseFolder));
            nav(&mut p, w, "Next");
            p
        }
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
        Step::Done => {
            let mut p = page(w, "All done");
            p.paragraphs = paras(&["Setup is finished. This is what was set:"]);
            p.facts = vec![
                format!("Ports folder: {}", shown(&ctx.ports_folder)),
                match w.summary.imported {
                    Some(0) => "Ports you already had: none added".to_string(),
                    Some(n) => format!("Ports you already had: {n} added"),
                    None if w.summary.skipped.contains(&Step::Import) => "Ports you already had: skipped".to_string(),
                    None => "Ports you already had: not checked".to_string(),
                },
                match &ctx.rom_folder {
                    Some(dir) => format!("Game files folder: {}", shown(dir)),
                    None => "Game files folder: not chosen".to_string(),
                },
                if w.summary.steam_opened {
                    "Steam: Add to Steam was opened"
                } else if w.summary.skipped.contains(&Step::Steam) {
                    "Steam: skipped"
                } else {
                    "Steam: not set up yet"
                }
                .to_string(),
            ];
            p.footnote = "Anything you skipped can be done later from Settings. To see this wizard again, open Settings > Setup wizard.".into();
            p.buttons = [btn("Back", Action::Back), None, btn("Finish", Action::Finish)];
            p.primary = Some(2);
            p
        }
    }
}

/// Everything that can take focus, in order: the options, then the buttons that exist.
pub fn focusables(page: &Page) -> Vec<Focus> {
    let mut out: Vec<Focus> = (0..page.options.len()).map(Focus::Option).collect();
    out.extend((0..3).filter(|s| page.buttons[*s].is_some()).map(Focus::Button));
    out
}

/// Position of `focus` in [`focusables`] (the number the window draws as selected).
pub fn focus_number(page: &Page, focus: Focus) -> usize {
    focusables(page).iter().position(|f| *f == focus).unwrap_or(0)
}

/// The number of the button in `slot`, or -1 when it is not shown.
pub fn button_number(page: &Page, slot: usize) -> i32 {
    if page.buttons[slot].is_none() {
        return -1;
    }
    focusables(page).iter().position(|f| *f == Focus::Button(slot)).map_or(-1, |n| n as i32)
}

pub fn initial_focus(page: &Page) -> usize {
    let wanted = match page.initial {
        Focus::Button(slot) if page.buttons[slot].is_none() => page.primary.map_or(Focus::Option(0), Focus::Button),
        other => other,
    };
    focus_number(page, wanted)
}

/// The D-pad: up and down walk the options and then drop to the buttons (the primary one first), left
/// and right move between buttons. It never lands on text, and stays put at an edge.
pub fn move_focus(page: &Page, current: usize, dx: i32, dy: i32) -> usize {
    let items = focusables(page);
    let Some(here) = items.get(current).copied() else { return initial_focus(page) };
    let index_of = |f: Focus| items.iter().position(|x| *x == f);
    let present: Vec<usize> = (0..3).filter(|s| page.buttons[*s].is_some()).collect();
    let target = match here {
        Focus::Option(i) if dy > 0 => {
            if i + 1 < page.options.len() {
                Some(Focus::Option(i + 1))
            } else {
                let slot = page.primary.filter(|s| present.contains(s)).or_else(|| present.last().copied());
                slot.map(Focus::Button)
            }
        }
        Focus::Option(i) if dy < 0 => i.checked_sub(1).map(Focus::Option),
        Focus::Button(_) if dy < 0 => page.options.len().checked_sub(1).map(Focus::Option),
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
    match focusables(page).get(number)? {
        Focus::Option(i) => page.options.get(*i).map(|o| o.action.clone()),
        Focus::Button(s) => page.buttons[*s].as_ref().map(|b| b.action.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::setup_wizard::flow::Mode;

    fn ctx() -> Context {
        Context {
            ports_folder: "/home/deck/Apps/dpm/Library".into(),
            quick: vec![
                QuickChoice { title: "SD card or drive \"SD512\"".into(), detail: "/run/media/deck/SD512/Ports - new folder".into(), path: "/run/media/deck/SD512/Ports".into() },
                QuickChoice { title: "Internal storage".into(), detail: "/home/deck/Apps/dpm/Library - 94 GB free".into(), path: "/home/deck/Apps/dpm/Library".into() },
            ],
            rom_folder: None,
        }
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

    #[test]
    fn welcome_says_what_the_app_does_what_the_steps_are_and_where_to_rerun_it() {
        let p = pg(Step::Welcome);
        let t = all_text(&p);
        for must in [
            "installs and updates PC versions of console games",
            "connects them to the game files",
            "adds them to Steam with artwork",
            "1. Choose where ports are kept",
            "2. Find ports you already have",
            "3. Say where your game files are",
            "4. Add your ports to Steam",
            "Settings > Setup wizard",
        ] {
            assert!(t.contains(must), "missing {must:?} in\n{t}");
        }
        assert_eq!(button_actions(&p), [Action::SkipSetup, Action::Next]);
        assert_eq!(p.buttons[2].as_ref().unwrap().label, "Start setup");
        assert_eq!(p.primary, Some(2));
        assert!(p.options.is_empty());
    }

    #[test]
    fn the_welcome_exit_button_is_skip_setup_on_first_run_and_close_in_a_rerun() {
        assert_eq!(pg(Step::Welcome).buttons[0].as_ref().unwrap().label, "Skip setup...");
        let mut w = at(Step::Welcome);
        w.mode = Mode::Rerun;
        assert_eq!(page_for(&w, &ctx()).buttons[0].as_ref().unwrap().label, "Close the wizard");
    }

    #[test]
    fn the_ports_folder_step_shows_the_current_folder_real_options_and_the_picker() {
        let p = pg(Step::PortsFolder);
        assert_eq!(p.facts, ["Right now: /home/deck/Apps/dpm/Library"]);
        let uses: Vec<_> = p.options.iter().filter_map(|o| match &o.action { Action::UseFolder(path) => Some(path.clone()), _ => None }).collect();
        assert_eq!(uses, [PathBuf::from("/run/media/deck/SD512/Ports"), PathBuf::from("/home/deck/Apps/dpm/Library")], "SD card first");
        assert_eq!(p.options[0].choice, Some(false));
        assert_eq!(p.options[1].choice, Some(true), "the folder in use is the chosen one");
        let last = p.options.last().unwrap();
        assert_eq!((last.title.as_str(), &last.action, last.choice), ("Choose another folder...", &Action::ChooseFolder, None));
        assert_eq!(button_actions(&p), [Action::Back, Action::Next], "the ports folder always has an answer, so no Skip");
    }

    #[test]
    fn choosing_a_folder_moves_the_chosen_mark() {
        let mut c = ctx();
        c.ports_folder = "/run/media/deck/SD512/Ports".into();
        let p = page_for(&at(Step::PortsFolder), &c);
        assert_eq!((p.options[0].choice, p.options[1].choice), (Some(true), Some(false)));
        assert_eq!(p.facts, ["Right now: /run/media/deck/SD512/Ports"]);
    }

    #[test]
    fn steps_that_can_be_skipped_offer_skip_and_the_others_do_not() {
        for step in Step::ALL {
            let has_skip = pg(step).buttons[1].as_ref().is_some_and(|b| b.action == Action::Skip);
            assert_eq!(has_skip, step.skippable(), "{step:?}");
        }
    }

    #[test]
    fn every_middle_step_has_back_left_next_right_and_starts_on_next() {
        for step in [Step::PortsFolder, Step::Import, Step::RomFolder, Step::Steam] {
            let p = pg(step);
            assert_eq!(p.buttons[0].as_ref().map(|b| &b.action), Some(&Action::Back), "{step:?}");
            assert_eq!(p.buttons[2].as_ref().map(|b| &b.action), Some(&Action::Next), "{step:?}");
            assert_eq!(p.primary, Some(2));
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
    fn the_last_page_summarises_what_was_set() {
        let mut w = at(Step::Done);
        let mut c = ctx();
        c.rom_folder = Some("/roms".into());
        w.summary.imported = Some(3);
        w.summary.steam_opened = true;
        let p = page_for(&w, &c);
        assert_eq!(
            p.facts,
            [
                "Ports folder: /home/deck/Apps/dpm/Library",
                "Ports you already had: 3 added",
                "Game files folder: /roms",
                "Steam: Add to Steam was opened"
            ]
        );
        assert!(all_text(&p).contains("Settings > Setup wizard"));
        assert_eq!(action_at(&p, initial_focus(&p)), Some(Action::Finish));
        assert_eq!(p.buttons[2].as_ref().unwrap().label, "Finish");
    }

    #[test]
    fn the_last_page_says_so_when_things_were_skipped() {
        let mut w = at(Step::Done);
        w.summary.skipped = vec![Step::Import, Step::RomFolder, Step::Steam];
        let t = all_text(&page_for(&w, &ctx()));
        assert!(t.contains("Ports you already had: skipped") && t.contains("Game files folder: not chosen") && t.contains("Steam: skipped"));
        let t = all_text(&pg(Step::Done));
        assert!(t.contains("not checked") && t.contains("Steam: not set up yet"));
    }

    #[test]
    fn the_skip_question_starts_on_the_safe_button_and_the_others_on_their_primary() {
        let mut w = at(Step::Welcome);
        w.view = View::ConfirmSkipSetup;
        let p = page_for(&w, &ctx());
        assert!(all_text(&p).contains("Nothing will be changed") && all_text(&p).contains("Settings > Setup wizard"));
        assert_eq!(button_actions(&p), [Action::No, Action::Yes]);
        assert_eq!(action_at(&p, initial_focus(&p)), Some(Action::No), "a stray A must not skip setup");
        assert_eq!(p.primary, None);

        w.view = View::ConfirmCreate("/sd/Ports".into());
        let p = page_for(&w, &ctx());
        assert!(all_text(&p).contains("/sd/Ports") && all_text(&p).contains("does not exist yet"));
        assert_eq!(button_actions(&p), [Action::No, Action::Yes]);
        assert_eq!(action_at(&p, initial_focus(&p)), Some(Action::Yes));

        w.view = View::Notice("Something went wrong.\n\nMore detail.".into());
        let p = page_for(&w, &ctx());
        assert_eq!(p.paragraphs, ["Something went wrong.", "More detail."]);
        assert_eq!(button_actions(&p), [Action::Ok]);
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
    fn the_header_counts_steps_out_of_six() {
        for (step, text) in Step::ALL.iter().zip(["Step 1 of 6", "Step 2 of 6", "Step 3 of 6", "Step 4 of 6", "Step 5 of 6", "Step 6 of 6"]) {
            assert_eq!(step_text(&pg(*step)), text);
        }
        let mut w = at(Step::Import);
        w.view = View::ConfirmSkipSetup;
        assert_eq!(step_text(&page_for(&w, &ctx())), "Step 3 of 6", "questions keep the step they interrupt");
    }

    // -- focus ---------------------------------------------------------------------------------

    #[test]
    fn only_options_and_buttons_take_focus_never_text() {
        let p = pg(Step::PortsFolder);
        assert_eq!(focusables(&p), [Focus::Option(0), Focus::Option(1), Focus::Option(2), Focus::Button(0), Focus::Button(2)]);
        let p = pg(Step::Welcome);
        assert_eq!(focusables(&p), [Focus::Button(0), Focus::Button(2)], "a page with no options has only its buttons");
        assert_eq!(button_number(&p, 0), 0);
        assert_eq!(button_number(&p, 1), -1);
        assert_eq!(button_number(&p, 2), 1);
    }

    #[test]
    fn the_primary_button_has_the_initial_focus() {
        for step in Step::ALL {
            let p = pg(step);
            let f = focusables(&p)[initial_focus(&p)];
            assert_eq!(f, Focus::Button(2), "{step:?}");
        }
    }

    #[test]
    fn down_walks_the_options_then_lands_on_the_primary_button_and_up_comes_back() {
        let p = pg(Step::Steam); // options 0, 1; buttons Back, Skip, Next
        assert_eq!(focusables(&p).len(), 5);
        let start = initial_focus(&p);
        assert_eq!(focusables(&p)[start], Focus::Button(2));
        let up = move_focus(&p, start, 0, -1);
        assert_eq!(focusables(&p)[up], Focus::Option(1));
        let up = move_focus(&p, up, 0, -1);
        assert_eq!(focusables(&p)[up], Focus::Option(0));
        assert_eq!(move_focus(&p, up, 0, -1), up, "stays at the top");
        let down = move_focus(&p, up, 0, 1);
        assert_eq!(focusables(&p)[down], Focus::Option(1));
        let down = move_focus(&p, down, 0, 1);
        assert_eq!(focusables(&p)[down], Focus::Button(2), "the primary button first");
        assert_eq!(move_focus(&p, down, 0, 1), down, "stays on the buttons");
    }

    #[test]
    fn left_and_right_move_between_buttons_only() {
        let p = pg(Step::Import); // Back, Skip, Next
        let next = initial_focus(&p);
        let skip = move_focus(&p, next, -1, 0);
        assert_eq!(focusables(&p)[skip], Focus::Button(1));
        let back = move_focus(&p, skip, -1, 0);
        assert_eq!(focusables(&p)[back], Focus::Button(0));
        assert_eq!(move_focus(&p, back, -1, 0), back);
        assert_eq!(move_focus(&p, back, 1, 0), skip);
        assert_eq!(move_focus(&p, next, 1, 0), next);
        // Left and right on an option do nothing.
        assert_eq!(move_focus(&p, 0, 1, 0), 0);
        assert_eq!(move_focus(&p, 0, -1, 0), 0);
    }

    #[test]
    fn with_a_missing_middle_button_left_and_right_skip_over_the_gap() {
        let p = pg(Step::PortsFolder); // Back, Next
        let next = initial_focus(&p);
        let back = move_focus(&p, next, -1, 0);
        assert_eq!(focusables(&p)[back], Focus::Button(0));
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
        let p = pg(Step::PortsFolder);
        assert_eq!(action_at(&p, 0), Some(Action::UseFolder("/run/media/deck/SD512/Ports".into())));
        assert_eq!(action_at(&p, 2), Some(Action::ChooseFolder));
        assert_eq!(action_at(&p, 3), Some(Action::Back));
        assert_eq!(action_at(&p, 4), Some(Action::Next));
        assert_eq!(action_at(&p, 9), None);
    }

    #[test]
    fn a_stale_focus_number_falls_back_to_the_initial_one() {
        let p = pg(Step::Welcome);
        assert_eq!(move_focus(&p, 99, 0, 1), initial_focus(&p));
    }
}
