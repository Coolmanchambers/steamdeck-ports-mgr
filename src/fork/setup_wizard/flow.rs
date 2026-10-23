//! The setup wizard's state machine. No UI here: [`press`] takes the item the user chose, decides what
//! happens next and calls the [`Adapters`] for everything outside the wizard (the ports folder, the
//! import screen, the game files folder, Add to Steam, saving `setup_done`). The screen in
//! `src/app/setup_wizard_screen.rs` implements `Adapters` with the real dialogs; the tests below use a
//! recording fake.
//!
//! Views: a step, or a small question (skip the whole setup?) or a notice. All of them are one page,
//! with B (or Escape) mapped by [`back_action`], so the controller always has a way back.
//!
//! Hand-offs: the ports folder, import, game files, Steam and artwork screens are the app's own screens
//! and do not know about the wizard. [`press`] records a [`HandOff`], the screen opens the other
//! screen, and once it is closed the screen calls [`resume`], which reads what changed and shows the
//! same step again.

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Welcome,
    PortsFolder,
    Import,
    RomFolder,
    Steam,
    Done,
}

impl Step {
    pub const ALL: [Step; 6] = [Step::Welcome, Step::PortsFolder, Step::Import, Step::RomFolder, Step::Steam, Step::Done];

    pub fn next(self) -> Option<Step> {
        Step::ALL.get(self.index() + 1).copied()
    }

    pub fn prev(self) -> Option<Step> {
        self.index().checked_sub(1).map(|i| Step::ALL[i])
    }

    pub fn index(self) -> usize {
        Step::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    /// Steps that can be left out; the ports folder always has an answer (the current one).
    pub fn skippable(self) -> bool {
        matches!(self, Step::Import | Step::RomFolder | Step::Steam)
    }
}

/// First run (setup was never finished) or a rerun from Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    FirstRun,
    Rerun,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    Step,
    ConfirmSkipSetup,
    Notice(String),
    /// `setup_done` could not be saved: the page offers "Close anyway" as well as OK.
    SaveFailed(String),
}

/// What an option or button does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Next,
    Back,
    Skip,
    SkipSetup,
    /// Use this folder as the ports folder (the real flow checks it, creates it after a confirm, offers
    /// to move installed ports and asks about the old copies).
    UseFolder(PathBuf),
    /// Pick the ports folder with the folder picker.
    ChooseFolder,
    FindPorts,
    ChooseRomFolder,
    OpenSteam,
    OpenArtSources,
    Finish,
    Yes,
    No,
    Ok,
    /// Leave the wizard without marking setup done (after a failed save).
    CloseAnyway,
}

/// An existing screen the wizard sent the user to; [`resume`] runs when it has been closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandOff {
    PortsFolder,
    Import { installed_before: usize },
    RomFolder,
    Steam,
    Art,
}

/// What the wizard remembers for the last page. Values the app already holds (the ports folder, the
/// game files folder) are read again by the screen when it draws; this holds what only the wizard knows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    /// How many ports the import screen moved in, summed over the times it was opened; `None` = not opened.
    pub imported: Option<usize>,
    pub rom_folder_opened: bool,
    pub steam_opened: bool,
    pub art_opened: bool,
    pub skipped: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wizard {
    pub mode: Mode,
    pub step: Step,
    pub view: View,
    pub summary: Summary,
    pub handoff: Option<HandOff>,
}

impl Wizard {
    pub fn new(mode: Mode) -> Wizard {
        Wizard { mode, step: Step::Welcome, view: View::Step, summary: Summary::default(), handoff: None }
    }
}

/// Everything the wizard needs from the rest of the app. Each method is one integration point.
pub trait Adapters {
    /// Draw the wizard's current view.
    fn show(&mut self, wizard: &Wizard);
    /// The ports folder: `Some(folder)` for a quick choice, `None` to open the folder picker. The app's own
    /// flow takes over (check, create after a confirm, offer to move installed ports, old-copy prompts);
    /// the wizard comes back when it is closed.
    fn ports_folder_step(&mut self, chosen: Option<PathBuf>);
    /// Open the import screen (finding ports installed elsewhere and moving them into the ports folder). It
    /// tells the wizard when the user leaves it (`resume`), whichever way; quitting the app is the one exit that
    /// does not.
    fn import_step(&mut self);
    /// Open the game files folder choice and scan.
    fn rom_folder_step(&mut self);
    /// Open Add to Steam.
    fn steam_step(&mut self);
    /// Open Artwork sources.
    fn art_sources_step(&mut self);
    /// Ports installed right now (the import's result is the difference).
    fn installed_count(&mut self) -> usize;
    /// Remember that setup is finished (`setup_done`).
    fn mark_done(&mut self) -> Result<(), String>;
    /// Close the wizard and land on the main list.
    fn close(&mut self);
}

/// What B (or Escape) does in the current view.
pub fn back_action(wizard: &Wizard) -> Action {
    match &wizard.view {
        View::Notice(_) | View::SaveFailed(_) => Action::Ok,
        View::ConfirmSkipSetup => Action::No,
        View::Step if wizard.step == Step::Welcome => Action::SkipSetup,
        View::Step => Action::Back,
    }
}

fn show_step(w: &mut Wizard, a: &mut dyn Adapters) {
    w.view = View::Step;
    a.show(w);
}

fn go_to(w: &mut Wizard, step: Step, a: &mut dyn Adapters) {
    w.step = step;
    show_step(w, a);
}

fn finish(w: &mut Wizard, a: &mut dyn Adapters) {
    match a.mark_done() {
        Ok(()) => a.close(),
        Err(e) => {
            w.view = View::SaveFailed(format!("Setup could not be saved as finished, so it will open again next time.\n\n{e}"));
            a.show(w);
        }
    }
}

/// The user chose an item (or pressed B, which the screen turns into [`back_action`]).
pub fn press(w: &mut Wizard, action: Action, a: &mut dyn Adapters) {
    match (w.view.clone(), action) {
        (View::Notice(_) | View::SaveFailed(_), Action::Ok) => show_step(w, a),
        (View::SaveFailed(_), Action::CloseAnyway) => a.close(),
        (View::ConfirmSkipSetup, Action::Yes) => finish(w, a),
        (View::ConfirmSkipSetup, Action::No) => show_step(w, a),
        (View::Step, action) => press_in_step(w, action, a),
        // A press that does not belong to this view (a stale item) changes nothing.
        _ => {}
    }
}

fn press_in_step(w: &mut Wizard, action: Action, a: &mut dyn Adapters) {
    match action {
        Action::Next => {
            if let Some(next) = w.step.next() {
                go_to(w, next, a);
            }
        }
        Action::Back => {
            if let Some(prev) = w.step.prev() {
                go_to(w, prev, a);
            }
        }
        Action::Skip if w.step.skippable() => {
            if !w.summary.skipped.contains(&w.step) {
                w.summary.skipped.push(w.step);
            }
            if let Some(next) = w.step.next() {
                go_to(w, next, a);
            }
        }
        Action::SkipSetup => match w.mode {
            Mode::FirstRun => {
                w.view = View::ConfirmSkipSetup;
                a.show(w);
            }
            // Setup was finished before: leaving a rerun changes nothing, so there is nothing to confirm.
            Mode::Rerun => a.close(),
        },
        Action::Finish if w.step == Step::Done => finish(w, a),
        Action::UseFolder(path) if w.step == Step::PortsFolder => {
            w.handoff = Some(HandOff::PortsFolder);
            a.ports_folder_step(Some(path));
        }
        Action::ChooseFolder if w.step == Step::PortsFolder => {
            w.handoff = Some(HandOff::PortsFolder);
            a.ports_folder_step(None);
        }
        Action::FindPorts if w.step == Step::Import => {
            // Recorded first: the import screen can tell the wizard it is done before `import_step` returns.
            w.handoff = Some(HandOff::Import { installed_before: a.installed_count() });
            a.import_step();
        }
        Action::ChooseRomFolder if w.step == Step::RomFolder => {
            w.handoff = Some(HandOff::RomFolder);
            a.rom_folder_step();
        }
        Action::OpenSteam if w.step == Step::Steam => {
            w.handoff = Some(HandOff::Steam);
            a.steam_step();
        }
        Action::OpenArtSources if w.step == Step::Steam => {
            w.handoff = Some(HandOff::Art);
            a.art_sources_step();
        }
        _ => {}
    }
}

/// A menu an existing screen is about to open. The screens the wizard opens end in these menus when the user
/// backs out; the wizard comes back instead, but only when the menu is the one that hand-off leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    /// The Settings list.
    Settings,
    /// Settings > Ports folder.
    PortsFolder,
    /// Settings > Game files.
    GameFiles,
}

/// Whether opening `menu` while `handoff` is pending means "back to the wizard". The ports-folder flow
/// itself opens Game files ("Fix in Game files") and the game files flow never leads to the ports menu, so
/// those two only match their own hand-off; Settings matches whichever screen was left through its Back row.
pub fn menu_returns_to_wizard(handoff: HandOff, menu: Menu) -> bool {
    match menu {
        Menu::Settings => true,
        Menu::PortsFolder => handoff == HandOff::PortsFolder,
        Menu::GameFiles => handoff == HandOff::RomFolder,
    }
}

/// The screen opened by a hand-off was closed: note what it did and show the step again. The ports
/// folder is read again by the screen when it draws, so it needs no note here. With no hand-off pending
/// (an unrelated dialog came and went) the current view is drawn again as it was, so a question or a
/// failed-save page is not turned back into the step.
pub fn resume(w: &mut Wizard, a: &mut dyn Adapters) {
    let Some(handoff) = w.handoff.take() else { return a.show(w) };
    match handoff {
        HandOff::Import { installed_before } => {
            let added = a.installed_count().saturating_sub(installed_before);
            w.summary.imported = Some(w.summary.imported.unwrap_or(0) + added);
        }
        HandOff::PortsFolder => {}
        HandOff::RomFolder => w.summary.rom_folder_opened = true,
        HandOff::Steam => w.summary.steam_opened = true,
        HandOff::Art => w.summary.art_opened = true,
    }
    show_step(w, a);
}

/// The import screen is done (its `finish`): resume only when the hand-off still pending is the import. A late
/// finish (the wizard already came back, or moved on to another hand-off) changes nothing.
pub fn resume_import(w: &mut Wizard, a: &mut dyn Adapters) {
    if matches!(w.handoff, Some(HandOff::Import { .. })) {
        resume(w, a);
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;

    /// Records every call as text so a test can assert the exact sequence and arguments.
    #[derive(Default)]
    pub struct Fake {
        pub calls: Vec<String>,
        pub done_error: Option<String>,
        pub installed: usize,
    }

    impl Adapters for Fake {
        fn show(&mut self, w: &Wizard) {
            self.calls.push(format!("show {:?} {:?}", w.step, w.view));
        }
        fn ports_folder_step(&mut self, chosen: Option<PathBuf>) {
            self.calls.push(match chosen {
                Some(p) => format!("ports_folder {}", p.display()),
                None => "ports_folder picker".into(),
            });
        }
        fn import_step(&mut self) {
            self.calls.push("import".into());
        }
        fn rom_folder_step(&mut self) {
            self.calls.push("rom".into());
        }
        fn steam_step(&mut self) {
            self.calls.push("steam".into());
        }
        fn art_sources_step(&mut self) {
            self.calls.push("art".into());
        }
        fn installed_count(&mut self) -> usize {
            self.installed
        }
        fn mark_done(&mut self) -> Result<(), String> {
            self.calls.push("mark_done".into());
            self.done_error.clone().map_or(Ok(()), Err)
        }
        fn close(&mut self) {
            self.calls.push("close".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::Fake;
    use super::*;

    fn at(step: Step, mode: Mode) -> Wizard {
        let mut w = Wizard::new(mode);
        w.step = step;
        w
    }

    #[test]
    fn next_walks_the_steps_in_order_and_back_returns_through_them() {
        let (mut w, mut a) = (Wizard::new(Mode::FirstRun), Fake::default());
        let mut seen = vec![w.step];
        for _ in 0..5 {
            press(&mut w, Action::Next, &mut a);
            seen.push(w.step);
        }
        assert_eq!(seen, Step::ALL);
        press(&mut w, Action::Next, &mut a); // nothing after Done
        assert_eq!(w.step, Step::Done);
        for expected in [Step::Steam, Step::RomFolder, Step::Import, Step::PortsFolder, Step::Welcome] {
            press(&mut w, Action::Back, &mut a);
            assert_eq!(w.step, expected);
        }
        press(&mut w, Action::Back, &mut a); // nothing before Welcome
        assert_eq!(w.step, Step::Welcome);
    }

    #[test]
    fn only_import_game_files_and_steam_can_be_skipped_and_skipping_moves_on() {
        for step in Step::ALL {
            assert_eq!(step.skippable(), matches!(step, Step::Import | Step::RomFolder | Step::Steam), "{step:?}");
        }
        let (mut w, mut a) = (at(Step::Import, Mode::FirstRun), Fake::default());
        press(&mut w, Action::Skip, &mut a);
        assert_eq!(w.step, Step::RomFolder);
        press(&mut w, Action::Skip, &mut a);
        press(&mut w, Action::Skip, &mut a);
        assert_eq!(w.step, Step::Done);
        assert_eq!(w.summary.skipped, [Step::Import, Step::RomFolder, Step::Steam]);
        // A skip on a step that cannot be skipped does nothing.
        let mut w = at(Step::PortsFolder, Mode::FirstRun);
        press(&mut w, Action::Skip, &mut a);
        assert_eq!(w.step, Step::PortsFolder);
        assert!(w.summary.skipped.is_empty());
    }

    #[test]
    fn finishing_marks_setup_done_once_and_closes() {
        let (mut w, mut a) = (at(Step::Done, Mode::FirstRun), Fake::default());
        press(&mut w, Action::Finish, &mut a);
        assert_eq!(a.calls, ["mark_done", "close"]);
    }

    #[test]
    fn a_step_other_than_the_last_cannot_finish() {
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake::default());
        press(&mut w, Action::Finish, &mut a);
        assert!(a.calls.is_empty());
    }

    #[test]
    fn skip_setup_on_first_run_asks_first_and_marks_done_only_after_yes() {
        let (mut w, mut a) = (Wizard::new(Mode::FirstRun), Fake::default());
        press(&mut w, Action::SkipSetup, &mut a);
        assert_eq!(w.view, View::ConfirmSkipSetup);
        assert!(!a.calls.iter().any(|c| c == "mark_done" || c == "close"));
        // No keeps the wizard where it was.
        press(&mut w, Action::No, &mut a);
        assert_eq!((w.view.clone(), w.step), (View::Step, Step::Welcome));
        press(&mut w, Action::SkipSetup, &mut a);
        press(&mut w, Action::Yes, &mut a);
        assert_eq!(&a.calls[a.calls.len() - 2..], ["mark_done", "close"]);
    }

    #[test]
    fn skip_setup_in_a_rerun_just_closes_without_touching_setup_done() {
        let (mut w, mut a) = (Wizard::new(Mode::Rerun), Fake::default());
        press(&mut w, Action::SkipSetup, &mut a);
        assert_eq!(a.calls, ["close"]);
    }

    #[test]
    fn b_means_back_on_a_step_skip_setup_on_welcome_and_no_or_ok_on_questions() {
        let mut w = Wizard::new(Mode::FirstRun);
        assert_eq!(back_action(&w), Action::SkipSetup);
        w.step = Step::Steam;
        assert_eq!(back_action(&w), Action::Back);
        w.step = Step::Done;
        assert_eq!(back_action(&w), Action::Back);
        w.view = View::ConfirmSkipSetup;
        assert_eq!(back_action(&w), Action::No);
        w.view = View::Notice("n".into());
        assert_eq!(back_action(&w), Action::Ok);
    }

    #[test]
    fn setup_done_is_only_ever_marked_by_finish_on_the_last_page_or_a_confirmed_skip() {
        // Every action in every view and step, from a fresh wizard each time: the only presses that mark
        // setup done are Finish on the last page and Yes on the skip question. (Quitting half-way is the
        // screen closing the page, which is not an action at all, so it can never reach mark_done.)
        let actions = [
            Action::Next,
            Action::Back,
            Action::Skip,
            Action::SkipSetup,
            Action::UseFolder("/x".into()),
            Action::ChooseFolder,
            Action::FindPorts,
            Action::ChooseRomFolder,
            Action::OpenSteam,
            Action::OpenArtSources,
            Action::Finish,
            Action::Yes,
            Action::No,
            Action::Ok,
            Action::CloseAnyway,
        ];
        let views = [View::Step, View::ConfirmSkipSetup, View::Notice("n".into()), View::SaveFailed("e".into())];
        for mode in [Mode::FirstRun, Mode::Rerun] {
            for step in Step::ALL {
                for view in &views {
                    for action in &actions {
                        let (mut w, mut a) = (at(step, mode), Fake::default());
                        w.view = view.clone();
                        press(&mut w, action.clone(), &mut a);
                        let marks = a.calls.iter().any(|c| c == "mark_done");
                        let allowed = (*view == View::Step && step == Step::Done && *action == Action::Finish) || (*view == View::ConfirmSkipSetup && *action == Action::Yes);
                        assert_eq!(marks, allowed, "{mode:?} {step:?} {view:?} {action:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_failed_save_of_setup_done_is_shown_and_can_be_left_or_dismissed() {
        let (mut w, mut a) = (at(Step::Done, Mode::FirstRun), Fake { done_error: Some("disk full".into()), ..Fake::default() });
        press(&mut w, Action::Finish, &mut a);
        assert!(matches!(&w.view, View::SaveFailed(t) if t.contains("disk full") && t.contains("open again next time")));
        assert!(!a.calls.iter().any(|c| c == "close"));
        assert_eq!(back_action(&w), Action::Ok);
        // OK goes back to the page.
        press(&mut w, Action::Ok, &mut a);
        assert_eq!(w.view, View::Step);
        // Close anyway leaves without trying to mark setup done again.
        press(&mut w, Action::Finish, &mut a);
        let marks_before = a.calls.iter().filter(|c| *c == "mark_done").count();
        press(&mut w, Action::CloseAnyway, &mut a);
        assert_eq!(a.calls.last().unwrap(), "close");
        assert_eq!(a.calls.iter().filter(|c| *c == "mark_done").count(), marks_before);
    }

    #[test]
    fn close_anyway_belongs_to_the_failed_save_notice_only() {
        let (mut w, mut a) = (at(Step::Done, Mode::FirstRun), Fake::default());
        press(&mut w, Action::CloseAnyway, &mut a);
        assert!(a.calls.is_empty());
    }

    #[test]
    fn the_import_hand_off_is_recorded_before_the_screen_opens_and_a_second_resume_counts_nothing_twice() {
        let (mut w, mut a) = (at(Step::Import, Mode::FirstRun), Fake { installed: 3, ..Fake::default() });
        press(&mut w, Action::FindPorts, &mut a);
        assert_eq!(w.handoff, Some(HandOff::Import { installed_before: 3 }));
        // The screen's finish and the wizard's own watcher can both come back; only the first reads the result.
        a.installed = 5;
        resume(&mut w, &mut a);
        assert_eq!(w.summary.imported, Some(2));
        a.installed = 9;
        resume(&mut w, &mut a);
        assert_eq!(w.summary.imported, Some(2));
        assert_eq!((w.step, w.view.clone()), (Step::Import, View::Step));
    }

    #[test]
    fn a_late_import_finish_resumes_only_the_import_hand_off() {
        // Another hand-off is pending (the game files chooser): the import's late finish must not take it.
        let (mut w, mut a) = (at(Step::RomFolder, Mode::FirstRun), Fake::default());
        press(&mut w, Action::ChooseRomFolder, &mut a);
        let calls = a.calls.len();
        resume_import(&mut w, &mut a);
        assert_eq!(w.handoff, Some(HandOff::RomFolder), "the game files hand-off is still pending");
        assert!(!w.summary.rom_folder_opened);
        assert_eq!(a.calls.len(), calls, "and nothing was drawn");
        // Nothing pending at all (the wizard already came back): also nothing.
        let (mut w, mut a) = (at(Step::Import, Mode::FirstRun), Fake::default());
        resume_import(&mut w, &mut a);
        assert!(a.calls.is_empty());
        // The import hand-off itself resumes as usual, counting what the import moved.
        let (mut w, mut a) = (at(Step::Import, Mode::FirstRun), Fake { installed: 4, ..Fake::default() });
        press(&mut w, Action::FindPorts, &mut a);
        a.installed = 6;
        resume_import(&mut w, &mut a);
        assert_eq!((w.summary.imported, w.handoff), (Some(2), None));
        assert_eq!(a.calls.last().unwrap(), "show Import Step");
    }

    #[test]
    fn an_import_that_moved_nothing_is_reported_as_zero() {
        let (mut w, mut a) = (at(Step::Import, Mode::FirstRun), Fake { installed: 3, ..Fake::default() });
        press(&mut w, Action::FindPorts, &mut a);
        resume(&mut w, &mut a);
        assert_eq!(w.summary.imported, Some(0));
    }

    #[test]
    fn a_quick_choice_hands_its_folder_to_the_ports_folder_flow() {
        let (mut w, mut a) = (at(Step::PortsFolder, Mode::FirstRun), Fake::default());
        press(&mut w, Action::UseFolder("/run/media/deck/SD/Ports".into()), &mut a);
        assert_eq!(a.calls, ["ports_folder /run/media/deck/SD/Ports"]);
        assert_eq!(w.handoff, Some(HandOff::PortsFolder));
    }

    #[test]
    fn choose_another_folder_opens_the_picker_flow() {
        let (mut w, mut a) = (at(Step::PortsFolder, Mode::FirstRun), Fake::default());
        press(&mut w, Action::ChooseFolder, &mut a);
        assert_eq!(a.calls, ["ports_folder picker"]);
        assert_eq!(w.handoff, Some(HandOff::PortsFolder));
    }

    #[test]
    fn coming_back_from_the_ports_folder_flow_shows_the_same_step_and_changes_nothing_else() {
        let (mut w, mut a) = (at(Step::PortsFolder, Mode::FirstRun), Fake::default());
        press(&mut w, Action::ChooseFolder, &mut a);
        resume(&mut w, &mut a);
        assert_eq!((w.step, w.handoff), (Step::PortsFolder, None));
        assert_eq!(a.calls.last().unwrap(), "show PortsFolder Step");
        assert_eq!(w.summary, Summary::default());
    }

    #[test]
    fn folder_actions_belong_to_the_ports_folder_step_only() {
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake::default());
        press(&mut w, Action::UseFolder("/x".into()), &mut a);
        press(&mut w, Action::ChooseFolder, &mut a);
        assert!(a.calls.is_empty());
    }

    #[test]
    fn each_hand_off_calls_its_adapter_once_and_records_it() {
        let cases = [
            (Step::Import, Action::FindPorts, "import", HandOff::Import { installed_before: 7 }),
            (Step::RomFolder, Action::ChooseRomFolder, "rom", HandOff::RomFolder),
            (Step::Steam, Action::OpenSteam, "steam", HandOff::Steam),
            (Step::Steam, Action::OpenArtSources, "art", HandOff::Art),
        ];
        for (step, action, call, hand_off) in cases {
            let (mut w, mut a) = (at(step, Mode::FirstRun), Fake { installed: 7, ..Fake::default() });
            press(&mut w, action, &mut a);
            assert_eq!(a.calls, [call]);
            assert_eq!(w.handoff, Some(hand_off));
        }
    }

    #[test]
    fn a_hand_off_action_from_the_wrong_step_does_nothing() {
        let (mut w, mut a) = (at(Step::Welcome, Mode::FirstRun), Fake::default());
        for action in [Action::FindPorts, Action::ChooseRomFolder, Action::OpenSteam, Action::OpenArtSources] {
            press(&mut w, action, &mut a);
        }
        assert!(a.calls.is_empty() && w.handoff.is_none());
    }

    #[test]
    fn resuming_after_the_import_counts_the_ports_it_added() {
        let (mut w, mut a) = (at(Step::Import, Mode::FirstRun), Fake { installed: 7, ..Fake::default() });
        press(&mut w, Action::FindPorts, &mut a);
        a.installed = 10;
        resume(&mut w, &mut a);
        assert_eq!(w.summary.imported, Some(3));
        assert!(w.handoff.is_none());
        assert_eq!(w.step, Step::Import);
        // A second visit adds to the total; ports removed meanwhile never make it negative.
        press(&mut w, Action::FindPorts, &mut a);
        a.installed = 11;
        resume(&mut w, &mut a);
        assert_eq!(w.summary.imported, Some(4));
        press(&mut w, Action::FindPorts, &mut a);
        a.installed = 5;
        resume(&mut w, &mut a);
        assert_eq!(w.summary.imported, Some(4));
    }

    #[test]
    fn resuming_with_no_hand_off_draws_the_same_view_again_as_it_was() {
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake::default());
        resume(&mut w, &mut a);
        assert_eq!(a.calls, ["show Steam Step"]);
        // A question or a failed-save page that an unrelated dialog covered is shown again, not turned into the step.
        for view in [View::ConfirmSkipSetup, View::SaveFailed("disk full".into()), View::Notice("n".into())] {
            let (mut w, mut a) = (at(Step::Done, Mode::FirstRun), Fake::default());
            w.view = view.clone();
            resume(&mut w, &mut a);
            assert_eq!(w.view, view);
            assert_eq!(a.calls, [format!("show Done {view:?}")]);
            assert!(!a.calls.iter().any(|c| c == "mark_done"));
        }
    }

    #[test]
    fn resuming_after_a_hand_off_shows_the_step() {
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake::default());
        press(&mut w, Action::OpenSteam, &mut a);
        w.view = View::Notice("stale".into());
        resume(&mut w, &mut a);
        assert_eq!(w.view, View::Step);
    }

    #[test]
    fn a_menu_only_returns_to_the_wizard_for_the_hand_off_that_leads_to_it() {
        let all = [
            HandOff::PortsFolder,
            HandOff::Import { installed_before: 0 },
            HandOff::RomFolder,
            HandOff::Steam,
            HandOff::Art,
        ];
        for h in all {
            // The ports-folder flow opens Game files itself ("Fix in Game files"): that must not resume the wizard.
            assert_eq!(menu_returns_to_wizard(h, Menu::GameFiles), h == HandOff::RomFolder, "{h:?}");
            assert_eq!(menu_returns_to_wizard(h, Menu::PortsFolder), h == HandOff::PortsFolder, "{h:?}");
            assert!(menu_returns_to_wizard(h, Menu::Settings), "{h:?}");
        }
    }


    #[test]
    fn resuming_after_the_other_screens_notes_that_they_were_opened() {
        for (action, step) in [(Action::ChooseRomFolder, Step::RomFolder), (Action::OpenSteam, Step::Steam), (Action::OpenArtSources, Step::Steam)] {
            let (mut w, mut a) = (at(step, Mode::FirstRun), Fake::default());
            press(&mut w, action.clone(), &mut a);
            resume(&mut w, &mut a);
            assert!(w.summary.rom_folder_opened || w.summary.steam_opened || w.summary.art_opened, "{action:?}");
            assert_eq!(w.step, step);
        }
    }

    #[test]
    fn a_stale_press_in_a_question_view_changes_nothing() {
        let (mut w, mut a) = (Wizard::new(Mode::FirstRun), Fake::default());
        press(&mut w, Action::SkipSetup, &mut a);
        let before = a.calls.len();
        press(&mut w, Action::Next, &mut a);
        press(&mut w, Action::Finish, &mut a);
        assert_eq!(a.calls.len(), before);
        assert_eq!(w.view, View::ConfirmSkipSetup);
    }
}
