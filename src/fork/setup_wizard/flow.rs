//! The setup wizard's state machine. No UI here: [`press`] takes the item the user chose, decides what
//! happens next and calls the [`Adapters`] for everything outside the wizard (the ports folder, the
//! import screen, the game files folder, Add to Steam, the old copies, saving `setup_done`). The screen in
//! `src/app/setup_wizard_screen.rs` implements `Adapters` with the real dialogs; the tests below use a
//! recording fake.
//!
//! Pages are one question each: choosing an answer is what moves on, so there is no Next button. B (or
//! Escape) goes back one page, mapped by [`back_action`]; on the first page it asks whether to skip setup.
//! A view is a page, a small question (skip the whole setup?) or a notice; all of them have a way back.
//!
//! Hand-offs: the ports folder, import, game files, Steam, artwork and old copy screens are the app's own
//! screens and do not know about the wizard. [`press`] records a [`HandOff`], the screen opens the other
//! screen, and once it is closed the screen calls [`resume`], which reads what changed and either goes
//! on to the next page (the question was answered) or shows the same page again (it was not). Two steps
//! have no page of their own: Your ports opens Import ports at once, and Old copies opens the one
//! "Remove the old copies?" page, but only when at least one old copy is ready to remove (Steam starts the new
//! copy and the port was started from there); otherwise the last page says they are kept until checked.

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    PortsFolder,
    YourPorts,
    GameFiles,
    Steam,
    OldCopies,
    Done,
}

impl Step {
    pub const ALL: [Step; 6] = [Step::PortsFolder, Step::YourPorts, Step::GameFiles, Step::Steam, Step::OldCopies, Step::Done];
    /// How many steps the header counts: the old copy question is only sometimes asked, so it is not counted.
    pub const COUNTED: usize = 5;

    pub fn next(self) -> Option<Step> {
        Step::ALL.get(self.index() + 1).copied()
    }

    /// The step B goes back to. The old copy question is never gone back to: it was answered.
    pub fn prev(self) -> Option<Step> {
        match self {
            Step::Done => Some(Step::Steam),
            step => step.index().checked_sub(1).map(|i| Step::ALL[i]),
        }
    }

    pub fn index(self) -> usize {
        Step::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    /// "Step 3" of the counted five.
    pub fn number(self) -> usize {
        match self {
            Step::PortsFolder => 1,
            Step::YourPorts => 2,
            Step::GameFiles => 3,
            Step::Steam | Step::OldCopies => 4,
            Step::Done => Step::COUNTED,
        }
    }

    /// Steps that can be left out; the ports folder always has an answer (the current one).
    pub fn skippable(self) -> bool {
        matches!(self, Step::YourPorts | Step::GameFiles | Step::Steam)
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
    /// Which Steam account the ports go to (only asked when there are several): their names.
    PickAccount(Vec<String>),
    /// "Connect them now" needs copies of game files in the ports' own folders: how many ports and how many bytes.
    ConfirmCopies { ports: usize, bytes: u64 },
}

/// What an option or button does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Go on to the next page with the answer as it is (keep the folder, keep the game files folder).
    Next,
    Back,
    Skip,
    SkipSetup,
    /// Use this folder as the ports folder (the real flow checks it, creates it after a confirm, offers
    /// to move installed ports and keeps the old copies).
    UseFolder(PathBuf),
    /// Pick the ports folder with the folder picker.
    ChooseFolder,
    /// Look for the ports already on the device (Import ports).
    FindPorts,
    ChooseRomFolder,
    /// Connect the game files found in the game files folder to their ports (links or in place where the port allows,
    /// the same rules as Settings > Game files).
    ConnectGameFiles,
    /// Copy them too (the size was shown).
    CopyGameFiles,
    /// Connect only the ones that need no copy.
    SkipGameFileCopies,
    /// Add every port that is not in Steam yet (or queue them while Steam runs).
    AddToSteam,
    /// Close Steam and make the queued Steam changes (Desktop Mode; the shared flow of `fork::steam_close`).
    CloseSteam,
    /// Look at Steam again (after signing in once).
    TryAgain,
    /// This account (by position in the names the page showed) gets the ports.
    ChooseAccount(usize),
    OpenArtSources,
    Finish,
    Yes,
    No,
    Ok,
    /// Leave the wizard without marking setup done (after a failed save).
    CloseAnyway,
}

/// An existing screen the wizard sent the user to; [`resume`] runs when it has been closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandOff {
    /// `before` is the ports folder when the screen opened: a different one afterwards means the question was answered.
    PortsFolder { before: PathBuf },
    Import { installed_before: usize },
    RomFolder,
    /// The game files are being connected (links, or copies after approval).
    GameFilesConnect,
    Art,
    /// The "Close Steam and update shortcuts" flow.
    SteamClose,
    /// `before` is how many old copies this run had kept when their question was opened.
    OldCopies { before: usize },
}

/// What the wizard remembers for the last page. Values the app already holds (the ports folder, the
/// game files folder) are read again by the screen when it draws; this holds what only the wizard knows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    /// How many ports the import screen moved in, summed over the times it was opened; `None` = not opened.
    pub imported: Option<usize>,
    pub rom_folder_opened: bool,
    /// How many installed ports need a game file, when the game files step was reached.
    pub game_files_needed: Option<usize>,
    /// What the last scan of the game files folder found for them.
    pub game_files: Option<super::game_files::Results>,
    /// How many ports "Connect them now" connected; `None` = not done.
    pub game_files_connected: Option<usize>,
    /// What adding to Steam did: `Added` or `Queued`; `None` = not done.
    pub steam: Option<SteamOutcome>,
    /// The account was asked about (several accounts), so it is not asked again this run.
    pub steam_account_asked: bool,
    /// The Steam close flow started Steam again at its end.
    pub steam_started: bool,
    pub art_opened: bool,
    /// What the old copy question did: how many copies were removed and how many stay; `None` = not asked.
    pub old_copies: Option<OldCopiesResult>,
    /// Old copies were kept but none was ready, so the question was not asked: how many.
    pub old_copies_waiting: Option<usize>,
    pub skipped: Vec<Step>,
}

/// What connecting the game files would do, before anything is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GamePlan {
    /// Ports that would get a file connected.
    pub ports: usize,
    /// Of those, the ones that need a copy of a game file in their own folder, and how big those copies are.
    pub copy_ports: usize,
    pub copy_bytes: u64,
}

/// What "Add all to Steam" did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SteamOutcome {
    /// Written to Steam's shortcut list (which was backed up first).
    Added(usize),
    /// Steam is running (always in Game Mode): queued for Desktop Mode, nothing written.
    Queued { count: usize, game_mode: bool },
    /// Nothing needed adding.
    Nothing,
    /// Nothing was done, and why, in plain words.
    Problem(String),
    /// Queued changes were made after closing Steam (adds, and shortcuts that now start a moved port).
    Applied(usize),
}

/// What the "Close Steam and update shortcuts" flow did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SteamCloseResult {
    pub applied: usize,
    /// Steam changes still waiting afterwards.
    pub waiting: usize,
    /// "Start Steam again" was chosen.
    pub steam_started: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OldCopiesResult {
    pub removed: usize,
    pub kept: usize,
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
        Wizard { mode, step: Step::PortsFolder, view: View::Step, summary: Summary::default(), handoff: None }
    }
}

/// Everything the wizard needs from the rest of the app. Each method is one integration point.
pub trait Adapters {
    /// Draw the wizard's current view.
    fn show(&mut self, wizard: &Wizard);
    /// The ports folder: `Some(folder)` for a quick choice, `None` to open the folder picker. The app's own
    /// flow takes over (check, create after a confirm, offer to move installed ports); the wizard comes back
    /// when it is closed. The old copies of ports it moves are kept, and asked about at the end.
    fn ports_folder_step(&mut self, chosen: Option<PathBuf>);
    /// The ports folder in use now.
    fn ports_folder(&mut self) -> PathBuf;
    /// Open the import screen (finding ports installed elsewhere and moving them into the ports folder). It
    /// tells the wizard when the user leaves it (`resume`), whichever way; quitting the app is the one exit
    /// that does not.
    fn import_step(&mut self);
    /// Whether the user left the import screen by going back from its first page (to the ports folder
    /// question) and not by skipping or finishing. Read once per visit.
    fn import_went_back(&mut self) -> bool;
    /// Open the game files folder choice and scan.
    fn rom_folder_step(&mut self);
    /// How many installed ports name a game file they need.
    fn ports_needing_game_files(&mut self) -> usize;
    /// What the scan the game files hand-off ran found for those ports; once per scan.
    fn game_files_result(&mut self) -> Option<super::game_files::Results>;
    /// What connecting the files found by the last scan would do (reads the disk, writes nothing).
    fn game_files_plan(&mut self) -> Result<GamePlan, String>;
    /// Connect them (`copies`: with the copies too, else only what needs none). Runs in the background;
    /// `resume` runs when it is done.
    fn game_files_connect_step(&mut self, copies: bool);
    /// How many ports that connected, or why it failed; once per run.
    fn game_files_connect_result(&mut self) -> Option<Result<usize, String>>;
    /// The names of the Steam accounts when there are several (else none).
    fn steam_accounts(&mut self) -> Vec<String>;
    /// Remember which account gets the ports (by position in `steam_accounts`).
    fn steam_choose_account(&mut self, index: usize) -> Result<(), String>;
    /// Add every port that is not in Steam yet, or queue them while Steam runs.
    fn steam_add_all(&mut self) -> SteamOutcome;
    /// Close Steam and make the queued changes (the shared flow); `resume` runs when it is left.
    fn steam_close_step(&mut self);
    /// What that flow did; once per run of it (`None` when it could not run).
    fn steam_close_result(&mut self) -> Option<SteamCloseResult>;
    /// Steam changes waiting in the queue now (adds and repoints).
    fn steam_waiting(&mut self) -> usize;
    /// Open Artwork sources.
    fn art_sources_step(&mut self);
    /// Old copies kept after moves (all of them, from this run or an earlier one).
    fn kept_old_copies(&mut self) -> usize;
    /// Of those, the ones ready to remove (`old_copies::Readiness::Ready`).
    fn ready_old_copies(&mut self) -> usize;
    /// Open the one page about those old copies; `resume_old_copies` runs when it is left.
    fn old_copies_step(&mut self);
    /// What that page said when it could not check the copies (shown once as a notice), if so.
    fn old_copies_note(&mut self) -> Option<String>;
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
        View::PickAccount(_) | View::ConfirmCopies { .. } => Action::Back,
        View::Step if wizard.step == Step::PortsFolder => Action::SkipSetup,
        View::Step => Action::Back,
    }
}

fn show_step(w: &mut Wizard, a: &mut dyn Adapters) {
    w.view = View::Step;
    a.show(w);
}

/// Arrive at `step` going forward. Two steps have no page: Your ports opens Import ports at once, and the
/// old copy question is asked only when some old copy is ready to remove, else it is passed over (and the last page
/// says the copies are kept until each port has been checked).
fn enter(w: &mut Wizard, step: Step, a: &mut dyn Adapters) {
    w.step = step;
    match step {
        Step::YourPorts => start_import(w, a),
        Step::GameFiles => {
            // No installed port needs a game file: nothing to ask about.
            let needing = a.ports_needing_game_files();
            w.summary.game_files_needed = Some(needing);
            if needing == 0 {
                return enter(w, Step::Steam, a);
            }
            show_step(w, a);
        }
        Step::OldCopies => {
            let before = a.kept_old_copies();
            if before == 0 {
                return enter(w, Step::Done, a);
            }
            if a.ready_old_copies() == 0 {
                w.summary.old_copies_waiting = Some(before);
                return enter(w, Step::Done, a);
            }
            // Recorded first: the page can tell the wizard it is done before `old_copies_step` returns.
            w.handoff = Some(HandOff::OldCopies { before });
            w.view = View::Step;
            a.old_copies_step();
        }
        _ => show_step(w, a),
    }
}

fn advance(w: &mut Wizard, a: &mut dyn Adapters) {
    if let Some(next) = w.step.next() {
        enter(w, next, a);
    }
}

fn start_import(w: &mut Wizard, a: &mut dyn Adapters) {
    // Recorded first: the import screen can tell the wizard it is done before `import_step` returns.
    w.handoff = Some(HandOff::Import { installed_before: a.installed_count() });
    w.view = View::Step;
    a.import_step();
}

/// Adds the ports to Steam (or queues them), then goes on; a problem is told and the page stays. Queued in
/// Desktop Mode, the page stays too: it now offers to close Steam and make the changes.
fn add_to_steam(w: &mut Wizard, a: &mut dyn Adapters) {
    match a.steam_add_all() {
        done @ SteamOutcome::Queued { game_mode: false, .. } => {
            w.summary.steam = Some(done);
            show_step(w, a);
        }
        done @ SteamOutcome::Added(_) => {
            w.summary.steam = Some(done);
            // Added with Steam closed, but other changes (repoints of moved ports) still wait: the page now offers
            // to make them, instead of leaving them waiting unseen.
            if a.steam_waiting() > 0 {
                show_step(w, a);
            } else {
                advance(w, a);
            }
        }
        done @ (SteamOutcome::Queued { .. } | SteamOutcome::Applied(_)) => {
            w.summary.steam = Some(done);
            advance(w, a);
        }
        SteamOutcome::Nothing => advance(w, a),
        SteamOutcome::Problem(text) => {
            w.view = View::Notice(text);
            a.show(w);
        }
    }
}

/// Connects the game files in the background; `resume` reads how it went.
fn start_connect(w: &mut Wizard, a: &mut dyn Adapters, copies: bool) {
    w.handoff = Some(HandOff::GameFilesConnect);
    w.view = View::Step;
    a.game_files_connect_step(copies);
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
        (View::PickAccount(_), Action::Back) => show_step(w, a),
        (View::ConfirmCopies { .. }, Action::Back) => show_step(w, a),
        (View::ConfirmCopies { .. }, Action::CopyGameFiles) => start_connect(w, a, true),
        (View::ConfirmCopies { .. }, Action::SkipGameFileCopies) => start_connect(w, a, false),
        (View::PickAccount(names), Action::ChooseAccount(i)) if i < names.len() => match a.steam_choose_account(i) {
            Ok(()) => {
                w.summary.steam_account_asked = true;
                w.view = View::Step;
                add_to_steam(w, a);
            }
            Err(e) => {
                w.view = View::Notice(format!("The Steam account could not be remembered.

{e}"));
                a.show(w);
            }
        },
        (View::Step, action) => press_in_step(w, action, a),
        // A press that does not belong to this view (a stale item) changes nothing.
        _ => {}
    }
}

fn press_in_step(w: &mut Wizard, action: Action, a: &mut dyn Adapters) {
    match action {
        Action::Next if matches!(w.step, Step::PortsFolder | Step::GameFiles | Step::Steam | Step::OldCopies) => advance(w, a),
        Action::Back => {
            if let Some(mut prev) = w.step.prev() {
                // A page that was never shown (no port needs game files) is stepped over.
                if prev == Step::GameFiles && w.summary.game_files_needed == Some(0) {
                    prev = Step::YourPorts;
                }
                // Going back shows the page again; it never starts the import over.
                w.step = prev;
                show_step(w, a);
            }
        }
        Action::Skip if w.step.skippable() => {
            if !w.summary.skipped.contains(&w.step) {
                w.summary.skipped.push(w.step);
            }
            advance(w, a);
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
            w.handoff = Some(HandOff::PortsFolder { before: a.ports_folder() });
            a.ports_folder_step(Some(path));
        }
        Action::ChooseFolder if w.step == Step::PortsFolder => {
            w.handoff = Some(HandOff::PortsFolder { before: a.ports_folder() });
            a.ports_folder_step(None);
        }
        Action::FindPorts if w.step == Step::YourPorts => start_import(w, a),
        Action::ConnectGameFiles if w.step == Step::GameFiles && w.summary.game_files.as_ref().is_some_and(|r| r.connectable > 0) => match a.game_files_plan() {
            Err(text) => {
                w.view = View::Notice(text);
                a.show(w);
            }
            Ok(plan) if plan.copy_ports == 0 => start_connect(w, a, false),
            Ok(plan) => {
                w.view = View::ConfirmCopies { ports: plan.copy_ports, bytes: plan.copy_bytes };
                a.show(w);
            }
        },
        Action::ChooseRomFolder if w.step == Step::GameFiles => {
            w.handoff = Some(HandOff::RomFolder);
            a.rom_folder_step();
        }
        Action::TryAgain if w.step == Step::Steam => show_step(w, a),
        Action::AddToSteam if w.step == Step::Steam => {
            if !w.summary.steam_account_asked {
                let accounts = a.steam_accounts();
                if accounts.len() > 1 {
                    w.view = View::PickAccount(accounts);
                    return a.show(w);
                }
            }
            add_to_steam(w, a);
        }
        Action::CloseSteam if w.step == Step::Steam => {
            w.handoff = Some(HandOff::SteamClose);
            a.steam_close_step();
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
pub fn menu_returns_to_wizard(handoff: &HandOff, menu: Menu) -> bool {
    match menu {
        Menu::Settings => true,
        Menu::PortsFolder => matches!(handoff, HandOff::PortsFolder { .. }),
        Menu::GameFiles => matches!(handoff, HandOff::RomFolder),
    }
}

/// The screen opened by a hand-off was closed: note what it did, then go on when it answered the question
/// of the page, or show the same page again when it did not (a cancelled picker, a folder with nothing in
/// it). With no hand-off pending (an unrelated dialog came and went) the current view is drawn again as it
/// was, so a question or a failed-save page is not turned back into the step.
pub fn resume(w: &mut Wizard, a: &mut dyn Adapters) {
    let Some(handoff) = w.handoff.take() else { return a.show(w) };
    match handoff {
        HandOff::Import { installed_before } => {
            let added = a.installed_count().saturating_sub(installed_before);
            // B on the first page of Import ports (or Cancel on its scan) is "back" to the ports folder question;
            // Skip and the end of an import go on.
            if a.import_went_back() {
                w.step = Step::PortsFolder;
                return show_step(w, a);
            }
            // Moved N, or skipped: leaving without moving anything is not "no ports were moved in".
            if added > 0 {
                w.summary.imported = Some(w.summary.imported.unwrap_or(0) + added);
            } else if w.summary.imported.is_none() && !w.summary.skipped.contains(&Step::YourPorts) {
                w.summary.skipped.push(Step::YourPorts);
            }
            w.view = View::Step;
            advance(w, a);
        }
        HandOff::PortsFolder { before } => {
            if a.ports_folder() != before {
                w.view = View::Step;
                advance(w, a);
            } else {
                show_step(w, a);
            }
        }
        HandOff::RomFolder => {
            w.summary.rom_folder_opened = true;
            // The results page is the answer; Continue goes on. A cancelled picker leaves the page as it was.
            if let Some(results) = a.game_files_result() {
                w.summary.game_files = Some(results);
            }
            show_step(w, a);
        }
        HandOff::GameFilesConnect => match a.game_files_connect_result() {
            Some(Ok(n)) => {
                w.summary.game_files_connected = Some(w.summary.game_files_connected.unwrap_or(0) + n);
                if let Some(r) = w.summary.game_files.as_mut() {
                    r.connectable = 0;
                }
                show_step(w, a);
            }
            Some(Err(text)) => {
                w.view = View::Notice(format!("Nothing was connected: {text}"));
                a.show(w);
            }
            None => show_step(w, a),
        },
        HandOff::Art => {
            w.summary.art_opened = true;
            show_step(w, a);
        }
        HandOff::SteamClose => match a.steam_close_result() {
            Some(r) => {
                w.summary.steam_started |= r.steam_started;
                if r.applied > 0 {
                    let before = match w.summary.steam {
                        Some(SteamOutcome::Applied(n)) => n,
                        _ => 0,
                    };
                    w.summary.steam = Some(SteamOutcome::Applied(before + r.applied));
                } else if r.waiting == 0 && !matches!(w.summary.steam, Some(SteamOutcome::Applied(_) | SteamOutcome::Added(_))) {
                    // Every change was dropped or already done: nothing waits, nothing was made.
                    w.summary.steam = Some(SteamOutcome::Nothing);
                }
                // Everything made: on to the next step. Something still waits: the page says so again.
                if r.waiting == 0 {
                    w.view = View::Step;
                    advance(w, a);
                } else {
                    show_step(w, a);
                }
            }
            None => show_step(w, a),
        },
        HandOff::OldCopies { before } => {
            let kept = a.kept_old_copies();
            w.summary.old_copies = Some(OldCopiesResult { removed: before.saturating_sub(kept), kept });
            w.view = View::Step;
            advance(w, a);
            // The copies could not be checked: say so once, on top of the last page.
            if let Some(note) = a.old_copies_note() {
                w.view = View::Notice(note);
                a.show(w);
            }
        }
    }
}

/// The import screen is done (its `finish`): resume only when the hand-off still pending is the import. A late
/// finish (the wizard already came back, or moved on to another hand-off) changes nothing.
pub fn resume_import(w: &mut Wizard, a: &mut dyn Adapters) {
    if matches!(w.handoff, Some(HandOff::Import { .. })) {
        resume(w, a);
    }
}

/// The Steam close flow is done: resume only when it is the pending hand-off.
pub fn resume_steam_close(w: &mut Wizard, a: &mut dyn Adapters) {
    if matches!(w.handoff, Some(HandOff::SteamClose)) {
        resume(w, a);
    }
}

/// The old copy page is done: resume only when it is the pending hand-off.
pub fn resume_old_copies(w: &mut Wizard, a: &mut dyn Adapters) {
    if matches!(w.handoff, Some(HandOff::OldCopies { .. })) {
        resume(w, a);
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;

    /// Records every call as text so a test can assert the exact sequence and arguments.
    pub struct Fake {
        pub calls: Vec<String>,
        pub done_error: Option<String>,
        pub installed: usize,
        pub folder: PathBuf,
        pub went_back: bool,
        pub needing: usize,
        pub note: Option<String>,
        pub plan: Result<GamePlan, String>,
        pub connected: Option<Result<usize, String>>,
        pub result: Option<super::super::game_files::Results>,
        pub kept: usize,
        /// Ready old copies; `None` = all the kept ones.
        pub ready: Option<usize>,
        pub accounts: Vec<String>,
        pub steam_result: SteamOutcome,
        pub close_result: Option<SteamCloseResult>,
        pub waiting: usize,
    }

    impl Default for Fake {
        fn default() -> Fake {
            Fake { calls: Vec::new(), done_error: None, installed: 0, folder: PathBuf::from("/home/deck/Library"), went_back: false, needing: 1, note: None, plan: Ok(GamePlan { ports: 2, copy_ports: 0, copy_bytes: 0 }), connected: Some(Ok(2)), result: None, kept: 0, ready: None, accounts: Vec::new(), steam_result: SteamOutcome::Added(2), close_result: None, waiting: 0 }
        }
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
        fn ports_folder(&mut self) -> PathBuf {
            self.folder.clone()
        }
        fn import_step(&mut self) {
            self.calls.push("import".into());
        }
        fn import_went_back(&mut self) -> bool {
            std::mem::take(&mut self.went_back)
        }
        fn rom_folder_step(&mut self) {
            self.calls.push("rom".into());
        }
        fn ports_needing_game_files(&mut self) -> usize {
            self.needing
        }
        fn game_files_result(&mut self) -> Option<super::super::game_files::Results> {
            self.result.take()
        }
        fn steam_accounts(&mut self) -> Vec<String> {
            self.accounts.clone()
        }
        fn steam_choose_account(&mut self, index: usize) -> Result<(), String> {
            self.calls.push(format!("account {index}"));
            Ok(())
        }
        fn steam_add_all(&mut self) -> SteamOutcome {
            self.calls.push("steam_add_all".into());
            self.steam_result.clone()
        }
        fn art_sources_step(&mut self) {
            self.calls.push("art".into());
        }
        fn steam_close_step(&mut self) {
            self.calls.push("steam_close".into());
        }
        fn steam_close_result(&mut self) -> Option<SteamCloseResult> {
            self.close_result.take()
        }
        fn steam_waiting(&mut self) -> usize {
            self.waiting
        }
        fn kept_old_copies(&mut self) -> usize {
            self.kept
        }
        fn ready_old_copies(&mut self) -> usize {
            self.ready.unwrap_or(self.kept)
        }
        fn old_copies_step(&mut self) {
            self.calls.push("old_copies".into());
        }
        fn game_files_plan(&mut self) -> Result<GamePlan, String> {
            self.plan.clone()
        }
        fn game_files_connect_step(&mut self, copies: bool) {
            self.calls.push(format!("connect copies={copies}"));
        }
        fn game_files_connect_result(&mut self) -> Option<Result<usize, String>> {
            self.connected.take()
        }
        fn old_copies_note(&mut self) -> Option<String> {
            self.note.take()
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
    use crate::fork::setup_wizard::game_files::Results as GameFilesResults;

    fn at(step: Step, mode: Mode) -> Wizard {
        let mut w = Wizard::new(mode);
        w.step = step;
        w
    }

    #[test]
    fn the_wizard_starts_on_the_ports_folder_question_with_no_welcome_page() {
        let w = Wizard::new(Mode::FirstRun);
        assert_eq!(w.step, Step::PortsFolder);
        assert_eq!(Step::ALL, [Step::PortsFolder, Step::YourPorts, Step::GameFiles, Step::Steam, Step::OldCopies, Step::Done]);
        assert_eq!((1..=5).collect::<Vec<_>>(), [Step::PortsFolder, Step::YourPorts, Step::GameFiles, Step::Steam, Step::Done].map(Step::number), "five counted steps");
        assert_eq!(Step::OldCopies.number(), Step::Steam.number(), "the old copy question is not counted");
    }

    #[test]
    fn keeping_the_folder_goes_on_to_your_ports_which_opens_import_at_once() {
        let (mut w, mut a) = (Wizard::new(Mode::FirstRun), Fake { installed: 2, ..Fake::default() });
        press(&mut w, Action::Next, &mut a);
        assert_eq!(w.step, Step::YourPorts);
        assert_eq!(a.calls, ["import"], "no page in between");
        assert_eq!(w.handoff, Some(HandOff::Import { installed_before: 2 }));
    }

    #[test]
    fn a_folder_choice_hands_over_and_goes_on_only_when_the_folder_changed() {
        let (mut w, mut a) = (Wizard::new(Mode::FirstRun), Fake::default());
        press(&mut w, Action::UseFolder("/run/media/deck/SD/Ports".into()), &mut a);
        assert_eq!(a.calls, ["ports_folder /run/media/deck/SD/Ports"]);
        assert_eq!(w.handoff, Some(HandOff::PortsFolder { before: "/home/deck/Library".into() }));
        // The flow was cancelled: same folder, same question.
        resume(&mut w, &mut a);
        assert_eq!((w.step, w.handoff.clone()), (Step::PortsFolder, None));
        assert_eq!(a.calls.last().unwrap(), "show PortsFolder Step");
        // This time the folder changed: on to Your ports, which opens Import ports.
        press(&mut w, Action::ChooseFolder, &mut a);
        a.folder = "/run/media/deck/SD/Ports".into();
        resume(&mut w, &mut a);
        assert_eq!(w.step, Step::YourPorts);
        assert_eq!(a.calls.last().unwrap(), "import");
    }

    #[test]
    fn your_ports_skip_or_finish_goes_on_and_back_returns_to_the_ports_folder_question() {
        let (mut w, mut a) = (at(Step::PortsFolder, Mode::FirstRun), Fake { installed: 3, ..Fake::default() });
        press(&mut w, Action::Next, &mut a);
        a.installed = 5;
        resume_import(&mut w, &mut a);
        assert_eq!((w.step, w.summary.imported), (Step::GameFiles, Some(2)));
        assert_eq!(a.calls.last().unwrap(), "show GameFiles Step");
        // Back from the first Import ports page is back to the ports folder question, not a skip.
        let (mut w, mut a) = (at(Step::PortsFolder, Mode::FirstRun), Fake::default());
        press(&mut w, Action::Next, &mut a);
        a.went_back = true;
        resume_import(&mut w, &mut a);
        assert_eq!((w.step, w.summary.skipped.clone()), (Step::PortsFolder, vec![]));
        assert_eq!(a.calls.last().unwrap(), "show PortsFolder Step");
        assert!(!a.went_back, "read once");
    }

    #[test]
    fn back_from_the_game_files_page_shows_your_ports_without_opening_import_again() {
        let (mut w, mut a) = (at(Step::GameFiles, Mode::FirstRun), Fake::default());
        press(&mut w, Action::Back, &mut a);
        assert_eq!(w.step, Step::YourPorts);
        assert_eq!(a.calls, ["show YourPorts Step"]);
        assert!(w.handoff.is_none());
        // From there the page can look for ports (FindPorts) or skip them.
        press(&mut w, Action::FindPorts, &mut a);
        assert_eq!(a.calls.last().unwrap(), "import");
    }

    #[test]
    fn the_game_files_page_shows_the_results_of_a_scan_and_goes_on_only_with_continue() {
        let (mut w, mut a) = (at(Step::GameFiles, Mode::FirstRun), Fake { needing: 4, ..Fake::default() });
        press(&mut w, Action::ChooseRomFolder, &mut a);
        // The picker was cancelled: no result, same page, no advance.
        resume(&mut w, &mut a);
        assert_eq!((w.step, w.summary.game_files.clone()), (Step::GameFiles, None));
        assert!(w.summary.rom_folder_opened);
        // A scan came back: the results page is the same step, shown again.
        press(&mut w, Action::ChooseRomFolder, &mut a);
        a.result = Some(GameFilesResults { total: 4, found: 3, missing: vec![("Zelda".into(), "Ocarina".into())], ..Default::default() });
        resume(&mut w, &mut a);
        assert_eq!(w.step, Step::GameFiles, "the results are read before going on");
        assert_eq!(w.summary.game_files.as_ref().map(|r| (r.found, r.total)), Some((3, 4)));
        assert_eq!(a.calls.last().unwrap(), "show GameFiles Step");
        // Another folder picked and cancelled keeps the earlier results and does not move on.
        press(&mut w, Action::ChooseRomFolder, &mut a);
        resume(&mut w, &mut a);
        assert_eq!((w.step, w.summary.game_files.is_some()), (Step::GameFiles, true));
        // Continue goes on.
        press(&mut w, Action::Next, &mut a);
        assert_eq!(w.step, Step::Steam);
    }

    #[test]
    fn game_files_are_skipped_when_no_installed_port_needs_any_and_back_steps_over_them() {
        let (mut w, mut a) = (at(Step::YourPorts, Mode::FirstRun), Fake { needing: 0, ..Fake::default() });
        press(&mut w, Action::Skip, &mut a);
        assert_eq!(w.step, Step::Steam, "nothing to ask about");
        assert_eq!(w.summary.game_files_needed, Some(0));
        assert!(!w.summary.skipped.contains(&Step::GameFiles), "it was not skipped by the user");
        assert_eq!(a.calls.last().unwrap(), "show Steam Step");
        press(&mut w, Action::Back, &mut a);
        assert_eq!(w.step, Step::YourPorts, "back does not stop on a page that was never shown");
        // With ports that need files the page is shown and Back returns to it.
        let (mut w, mut a) = (at(Step::YourPorts, Mode::FirstRun), Fake { needing: 2, ..Fake::default() });
        press(&mut w, Action::Skip, &mut a);
        assert_eq!((w.step, w.summary.game_files_needed), (Step::GameFiles, Some(2)));
        w.step = Step::Steam;
        press(&mut w, Action::Back, &mut a);
        assert_eq!(w.step, Step::GameFiles);
    }

    #[test]
    fn adding_to_steam_goes_on_and_remembers_what_it_did_and_artwork_sources_stay_on_the_page() {
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake::default());
        press(&mut w, Action::OpenArtSources, &mut a);
        resume(&mut w, &mut a);
        assert_eq!((w.step, w.summary.art_opened), (Step::Steam, true));
        press(&mut w, Action::AddToSteam, &mut a);
        assert_eq!(a.calls.last().unwrap(), "show Done Step", "{:?}", a.calls);
        assert_eq!((w.step, w.summary.steam.clone()), (Step::Done, Some(SteamOutcome::Added(2))));
        assert!(!a.calls.contains(&"old_copies".to_string()), "nothing was kept, so no old copy question");
    }

    #[test]
    fn queued_ports_are_remembered_as_queued() {
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake { steam_result: SteamOutcome::Queued { count: 3, game_mode: true }, ..Fake::default() });
        press(&mut w, Action::AddToSteam, &mut a);
        assert_eq!(w.summary.steam, Some(SteamOutcome::Queued { count: 3, game_mode: true }));
        assert_eq!(w.step, Step::Done);
    }

    #[test]
    fn queued_in_desktop_mode_stays_on_the_steam_page_to_offer_closing_steam() {
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake { steam_result: SteamOutcome::Queued { count: 4, game_mode: false }, ..Fake::default() });
        press(&mut w, Action::AddToSteam, &mut a);
        assert_eq!((w.step, w.view.clone()), (Step::Steam, View::Step));
        assert_eq!(a.calls.last().unwrap(), "show Steam Step");
        assert_eq!(w.summary.steam, Some(SteamOutcome::Queued { count: 4, game_mode: false }));
    }

    #[test]
    fn closing_steam_hands_over_and_goes_on_only_when_nothing_is_left_waiting() {
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake::default());
        press(&mut w, Action::CloseSteam, &mut a);
        assert_eq!(a.calls, ["steam_close"]);
        assert_eq!(w.handoff, Some(HandOff::SteamClose));
        // Not now, a timeout, or Game Mode: nothing applied, same page.
        a.close_result = Some(SteamCloseResult { applied: 0, waiting: 4, steam_started: false });
        resume_steam_close(&mut w, &mut a);
        assert_eq!((w.step, w.summary.steam.clone()), (Step::Steam, None));
        assert_eq!(a.calls.last().unwrap(), "show Steam Step");
        // Applied, all of it: on to the last page, which says so.
        press(&mut w, Action::CloseSteam, &mut a);
        a.close_result = Some(SteamCloseResult { applied: 4, waiting: 0, steam_started: true });
        resume_steam_close(&mut w, &mut a);
        assert_eq!((w.step, w.summary.steam.clone(), w.summary.steam_started), (Step::Done, Some(SteamOutcome::Applied(4)), true));
        // Part of it: counted, and the page comes back.
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake::default());
        press(&mut w, Action::CloseSteam, &mut a);
        a.close_result = Some(SteamCloseResult { applied: 3, waiting: 1, steam_started: false });
        resume_steam_close(&mut w, &mut a);
        assert_eq!((w.step, w.summary.steam.clone()), (Step::Steam, Some(SteamOutcome::Applied(3))));
        // The flow could not run at all: the page again.
        press(&mut w, Action::CloseSteam, &mut a);
        resume_steam_close(&mut w, &mut a);
        assert_eq!(w.step, Step::Steam);
        // A late resume with another hand-off pending changes nothing.
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake::default());
        press(&mut w, Action::OpenArtSources, &mut a);
        resume_steam_close(&mut w, &mut a);
        assert_eq!(w.handoff, Some(HandOff::Art));
        // Close Steam belongs to the Steam step only.
        let (mut w, mut a) = (at(Step::GameFiles, Mode::FirstRun), Fake::default());
        press(&mut w, Action::CloseSteam, &mut a);
        assert!(a.calls.is_empty() && w.handoff.is_none());
    }

    #[test]
    fn added_with_steam_closed_but_repoints_still_waiting_stays_to_offer_them() {
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake { waiting: 4, ..Fake::default() });
        press(&mut w, Action::AddToSteam, &mut a);
        assert_eq!((w.step, w.summary.steam.clone()), (Step::Steam, Some(SteamOutcome::Added(2))));
        assert_eq!(a.calls.last().unwrap(), "show Steam Step");
        // Everything dropped by the close flow: nothing waits, nothing was made.
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake::default());
        press(&mut w, Action::CloseSteam, &mut a);
        a.close_result = Some(SteamCloseResult { applied: 0, waiting: 0, steam_started: false });
        resume_steam_close(&mut w, &mut a);
        assert_eq!((w.step, w.summary.steam.clone()), (Step::Done, Some(SteamOutcome::Nothing)));
    }

    #[test]
    fn a_problem_adding_is_told_on_the_page_and_the_wizard_does_not_go_on() {
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake { steam_result: SteamOutcome::Problem("Steam could not be read.".into()), ..Fake::default() });
        press(&mut w, Action::AddToSteam, &mut a);
        assert_eq!(w.step, Step::Steam);
        assert_eq!(w.view, View::Notice("Steam could not be read.".into()));
        assert_eq!(w.summary.steam, None);
        press(&mut w, Action::Ok, &mut a);
        assert_eq!(w.view, View::Step);
    }

    #[test]
    fn nothing_to_add_goes_on_without_a_note() {
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake { steam_result: SteamOutcome::Nothing, ..Fake::default() });
        press(&mut w, Action::AddToSteam, &mut a);
        assert_eq!((w.step, w.summary.steam.clone()), (Step::Done, None));
        // The page offers Continue when there is nothing to add: Next goes on from Steam too.
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake::default());
        press(&mut w, Action::Next, &mut a);
        assert_eq!(w.step, Step::Done);
    }

    #[test]
    fn several_accounts_are_asked_about_once_before_adding_and_back_returns_to_the_page() {
        let names = vec!["Tim".to_string(), "Kid".to_string()];
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake { accounts: names.clone(), ..Fake::default() });
        press(&mut w, Action::AddToSteam, &mut a);
        assert_eq!(w.view, View::PickAccount(names.clone()));
        assert!(!a.calls.contains(&"steam_add_all".to_string()), "nothing is added before the account is chosen");
        assert_eq!(back_action(&w), Action::Back);
        press(&mut w, Action::Back, &mut a);
        assert_eq!((w.view.clone(), w.step), (View::Step, Step::Steam));
        press(&mut w, Action::AddToSteam, &mut a);
        press(&mut w, Action::ChooseAccount(1), &mut a);
        assert_eq!(&a.calls[a.calls.len() - 3..], ["account 1", "steam_add_all", "show Done Step"]);
        assert!(w.summary.steam_account_asked);
        // A stale choice past the end does nothing.
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake { accounts: names, ..Fake::default() });
        press(&mut w, Action::AddToSteam, &mut a);
        press(&mut w, Action::ChooseAccount(5), &mut a);
        assert_eq!(w.view, View::PickAccount(vec!["Tim".into(), "Kid".into()]));
        // One account is never asked about.
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake { accounts: vec!["Tim".into()], ..Fake::default() });
        press(&mut w, Action::AddToSteam, &mut a);
        assert_eq!(w.step, Step::Done);
    }

    #[test]
    fn the_old_copy_question_is_asked_only_when_this_run_kept_copies() {
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake { kept: 3, ..Fake::default() });
        press(&mut w, Action::Skip, &mut a);
        assert_eq!(w.step, Step::OldCopies);
        assert_eq!(a.calls, ["old_copies"]);
        assert_eq!(w.handoff, Some(HandOff::OldCopies { before: 3 }));
        // Two were removed, one stays.
        a.kept = 1;
        resume_old_copies(&mut w, &mut a);
        assert_eq!(w.step, Step::Done);
        assert_eq!(w.summary.old_copies, Some(OldCopiesResult { removed: 2, kept: 1 }));
        // Skipping with nothing kept goes straight to the last page.
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake::default());
        press(&mut w, Action::Skip, &mut a);
        assert_eq!((w.step, a.calls.clone()), (Step::Done, vec!["show Done Step".to_string()]));
        assert_eq!(w.summary.old_copies, None);
    }

    #[test]
    fn the_old_copy_question_waits_until_one_is_ready_and_the_last_page_says_so() {
        // Tim's Deck now: 4 old copies kept, none ready (Steam not updated, or not started yet).
        let (mut w, mut a) = (at(Step::Steam, Mode::Rerun), Fake { kept: 4, ready: Some(0), ..Fake::default() });
        press(&mut w, Action::Skip, &mut a);
        assert_eq!(w.step, Step::Done);
        assert!(!a.calls.contains(&"old_copies".to_string()), "no question while nothing is ready");
        assert_eq!((w.summary.old_copies, w.summary.old_copies_waiting), (None, Some(4)));
        // One ready: the page comes.
        let (mut w, mut a) = (at(Step::Steam, Mode::Rerun), Fake { kept: 4, ready: Some(1), ..Fake::default() });
        press(&mut w, Action::Skip, &mut a);
        assert_eq!((w.step, a.calls.clone()), (Step::OldCopies, vec!["old_copies".to_string()]));
    }

    #[test]
    fn a_late_old_copy_finish_changes_nothing_unless_it_is_the_pending_hand_off() {
        let (mut w, mut a) = (at(Step::Done, Mode::FirstRun), Fake::default());
        resume_old_copies(&mut w, &mut a);
        assert!(a.calls.is_empty());
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake::default());
        press(&mut w, Action::OpenArtSources, &mut a);
        resume_old_copies(&mut w, &mut a);
        assert_eq!(w.handoff, Some(HandOff::Art));
    }

    #[test]
    fn back_from_the_last_page_goes_to_steam_never_to_the_old_copy_question() {
        let (mut w, mut a) = (at(Step::Done, Mode::FirstRun), Fake { kept: 2, ..Fake::default() });
        press(&mut w, Action::Back, &mut a);
        assert_eq!(w.step, Step::Steam);
        assert!(!a.calls.contains(&"old_copies".to_string()));
        for (from, to) in [(Step::Steam, Step::GameFiles), (Step::GameFiles, Step::YourPorts), (Step::YourPorts, Step::PortsFolder)] {
            let mut w = at(from, Mode::FirstRun);
            press(&mut w, Action::Back, &mut a);
            assert_eq!(w.step, to);
        }
        let mut w = at(Step::PortsFolder, Mode::FirstRun);
        press(&mut w, Action::Back, &mut a);
        assert_eq!(w.step, Step::PortsFolder, "nothing before the first question");
    }

    #[test]
    fn only_your_ports_game_files_and_steam_can_be_skipped_and_skipping_moves_on() {
        for step in Step::ALL {
            assert_eq!(step.skippable(), matches!(step, Step::YourPorts | Step::GameFiles | Step::Steam), "{step:?}");
        }
        let (mut w, mut a) = (at(Step::GameFiles, Mode::FirstRun), Fake::default());
        press(&mut w, Action::Skip, &mut a);
        assert_eq!(w.step, Step::Steam);
        press(&mut w, Action::Skip, &mut a);
        assert_eq!(w.step, Step::Done);
        assert_eq!(w.summary.skipped, [Step::GameFiles, Step::Steam]);
        // A skip on a step that cannot be skipped does nothing.
        let mut w = at(Step::PortsFolder, Mode::FirstRun);
        press(&mut w, Action::Skip, &mut a);
        assert_eq!(w.step, Step::PortsFolder);
        assert!(w.summary.skipped.is_empty());
        // Skipping Your ports opens nothing: the page's Skip means "not now".
        let (mut w, mut a) = (at(Step::YourPorts, Mode::FirstRun), Fake::default());
        press(&mut w, Action::Skip, &mut a);
        assert_eq!(w.step, Step::GameFiles);
        assert_eq!(w.summary.skipped, [Step::YourPorts]);
        assert!(!a.calls.contains(&"import".to_string()));
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
        assert_eq!((w.view.clone(), w.step), (View::Step, Step::PortsFolder));
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
    fn b_is_back_on_every_page_and_the_skip_question_on_the_first() {
        let mut w = Wizard::new(Mode::FirstRun);
        assert_eq!(back_action(&w), Action::SkipSetup);
        for step in [Step::YourPorts, Step::GameFiles, Step::Steam, Step::OldCopies, Step::Done] {
            w.step = step;
            assert_eq!(back_action(&w), Action::Back, "{step:?}");
        }
        w.view = View::ConfirmSkipSetup;
        assert_eq!(back_action(&w), Action::No);
        w.view = View::Notice("n".into());
        assert_eq!(back_action(&w), Action::Ok);
        w.view = View::SaveFailed("e".into());
        assert_eq!(back_action(&w), Action::Ok);
    }

    #[test]
    fn b_always_does_something_and_never_marks_setup_done() {
        // Every view of every step, first run and rerun: pressing what B maps to never marks setup done unless
        // it is the confirmed skip's Yes, which B never maps to.
        for mode in [Mode::FirstRun, Mode::Rerun] {
            for step in Step::ALL {
                for view in [View::Step, View::ConfirmSkipSetup, View::Notice("n".into()), View::SaveFailed("e".into())] {
                    let (mut w, mut a) = (at(step, mode), Fake::default());
                    w.view = view.clone();
                    let action = back_action(&w);
                    press(&mut w, action.clone(), &mut a);
                    assert!(!a.calls.iter().any(|c| c == "mark_done"), "{mode:?} {step:?} {view:?} {action:?}");
                    let moved = !a.calls.is_empty() || w.step != step || w.view != view;
                    // A step with no page of its own (the old copy question) is never on screen as a view to press B on.
                    assert!(moved || step == Step::OldCopies && view == View::Step, "B did nothing at {mode:?} {step:?} {view:?}");
                }
            }
        }
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
            Action::ConnectGameFiles,
            Action::CopyGameFiles,
            Action::SkipGameFileCopies,
            Action::AddToSteam,
            Action::CloseSteam,
            Action::TryAgain,
            Action::ChooseAccount(0),
            Action::OpenArtSources,
            Action::Finish,
            Action::Yes,
            Action::No,
            Action::Ok,
            Action::CloseAnyway,
        ];
        let views = [View::Step, View::ConfirmSkipSetup, View::Notice("n".into()), View::SaveFailed("e".into()), View::PickAccount(vec!["a".into(), "b".into()]), View::ConfirmCopies { ports: 1, bytes: 1 }];
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
    fn no_hand_off_resume_or_walk_through_the_steps_marks_setup_done() {
        let (mut w, mut a) = (Wizard::new(Mode::FirstRun), Fake { kept: 1, needing: 2, ..Fake::default() });
        press(&mut w, Action::Next, &mut a);
        resume_import(&mut w, &mut a);
        press(&mut w, Action::Next, &mut a);
        press(&mut w, Action::AddToSteam, &mut a);
        assert_eq!(w.step, Step::OldCopies, "the old copy question is open");
        resume_old_copies(&mut w, &mut a);
        assert_eq!(w.step, Step::Done);
        assert!(!a.calls.iter().any(|c| c == "mark_done" || c == "close"), "{:?}", a.calls);
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
        let (mut w, mut a) = (at(Step::PortsFolder, Mode::FirstRun), Fake { installed: 3, ..Fake::default() });
        press(&mut w, Action::Next, &mut a);
        assert_eq!(w.handoff, Some(HandOff::Import { installed_before: 3 }));
        // The screen's finish and the wizard's own watcher can both come back; only the first reads the result.
        a.installed = 5;
        resume(&mut w, &mut a);
        assert_eq!(w.summary.imported, Some(2));
        a.installed = 9;
        resume(&mut w, &mut a);
        assert_eq!(w.summary.imported, Some(2));
        assert_eq!((w.step, w.view.clone()), (Step::GameFiles, View::Step));
    }

    #[test]
    fn a_late_import_finish_resumes_only_the_import_hand_off() {
        // Another hand-off is pending (the game files chooser): the import's late finish must not take it.
        let (mut w, mut a) = (at(Step::GameFiles, Mode::FirstRun), Fake::default());
        press(&mut w, Action::ChooseRomFolder, &mut a);
        let calls = a.calls.len();
        resume_import(&mut w, &mut a);
        assert_eq!(w.handoff, Some(HandOff::RomFolder), "the game files hand-off is still pending");
        assert!(!w.summary.rom_folder_opened);
        assert_eq!(a.calls.len(), calls, "and nothing was drawn");
        // Nothing pending at all (the wizard already came back): also nothing.
        let (mut w, mut a) = (at(Step::YourPorts, Mode::FirstRun), Fake::default());
        resume_import(&mut w, &mut a);
        assert!(a.calls.is_empty());
    }

    #[test]
    fn leaving_import_ports_without_moving_anything_is_a_skip_not_zero_moved_and_ports_removed_meanwhile_never_count() {
        let (mut w, mut a) = (at(Step::PortsFolder, Mode::FirstRun), Fake { installed: 3, ..Fake::default() });
        press(&mut w, Action::Next, &mut a);
        resume(&mut w, &mut a);
        assert_eq!(w.summary.imported, None);
        assert_eq!(w.summary.skipped, [Step::YourPorts], "Skip or Cancel reads as skipped on the last page");
        w.step = Step::YourPorts;
        press(&mut w, Action::FindPorts, &mut a);
        a.installed = 1;
        resume(&mut w, &mut a);
        assert_eq!((w.summary.imported, w.summary.skipped.clone()), (None, vec![Step::YourPorts]), "listed once");
        // Going back is neither.
        let (mut w, mut a) = (at(Step::PortsFolder, Mode::FirstRun), Fake::default());
        press(&mut w, Action::Next, &mut a);
        a.went_back = true;
        resume(&mut w, &mut a);
        assert_eq!((w.summary.imported, w.summary.skipped.clone()), (None, vec![]));
    }

    #[test]
    fn a_second_visit_to_your_ports_adds_to_the_total() {
        let (mut w, mut a) = (at(Step::YourPorts, Mode::FirstRun), Fake { installed: 7, ..Fake::default() });
        press(&mut w, Action::FindPorts, &mut a);
        a.installed = 10;
        resume(&mut w, &mut a);
        assert_eq!(w.summary.imported, Some(3));
        w.step = Step::YourPorts;
        press(&mut w, Action::FindPorts, &mut a);
        a.installed = 11;
        resume(&mut w, &mut a);
        assert_eq!(w.summary.imported, Some(4));
    }

    #[test]
    fn a_folder_action_belongs_to_the_ports_folder_step_only() {
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake::default());
        press(&mut w, Action::UseFolder("/x".into()), &mut a);
        press(&mut w, Action::ChooseFolder, &mut a);
        assert!(a.calls.is_empty());
    }

    #[test]
    fn a_hand_off_action_from_the_wrong_step_does_nothing() {
        let (mut w, mut a) = (at(Step::PortsFolder, Mode::FirstRun), Fake::default());
        for action in [Action::FindPorts, Action::ChooseRomFolder, Action::AddToSteam, Action::OpenArtSources] {
            press(&mut w, action, &mut a);
        }
        assert!(a.calls.is_empty() && w.handoff.is_none());
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
    fn a_menu_only_returns_to_the_wizard_for_the_hand_off_that_leads_to_it() {
        let all = [
            HandOff::PortsFolder { before: "/a".into() },
            HandOff::Import { installed_before: 0 },
            HandOff::RomFolder,
            HandOff::GameFilesConnect,
            HandOff::Art,
            HandOff::SteamClose,
            HandOff::OldCopies { before: 1 },
        ];
        for h in all {
            // The ports-folder flow opens Game files itself ("Fix in Game files"): that must not resume the wizard.
            assert_eq!(menu_returns_to_wizard(&h, Menu::GameFiles), h == HandOff::RomFolder, "{h:?}");
            assert_eq!(menu_returns_to_wizard(&h, Menu::PortsFolder), matches!(h, HandOff::PortsFolder { .. }), "{h:?}");
            assert!(menu_returns_to_wizard(&h, Menu::Settings), "{h:?}");
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
    #[test]
    fn a_failed_old_copy_check_is_shown_once_on_top_of_the_last_page() {
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake { kept: 2, note: Some("The old copies could not be checked (x).".into()), ..Fake::default() });
        press(&mut w, Action::Skip, &mut a);
        resume_old_copies(&mut w, &mut a);
        assert_eq!(w.step, Step::Done);
        assert_eq!(w.view, View::Notice("The old copies could not be checked (x).".into()));
        press(&mut w, Action::Ok, &mut a);
        assert_eq!((w.step, w.view.clone()), (Step::Done, View::Step));
        // Without a note there is no notice.
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake { kept: 2, ..Fake::default() });
        press(&mut w, Action::Skip, &mut a);
        resume_old_copies(&mut w, &mut a);
        assert_eq!(w.view, View::Step);
    }

    fn scanned(connectable: usize) -> Wizard {
        let mut w = at(Step::GameFiles, Mode::FirstRun);
        w.summary.game_files = Some(GameFilesResults { total: 3, in_folder: 0, found: 2, connectable, missing: vec![] });
        w
    }

    #[test]
    fn connecting_without_copies_runs_at_once_and_is_counted_when_it_comes_back() {
        let (mut w, mut a) = (scanned(2), Fake::default());
        press(&mut w, Action::ConnectGameFiles, &mut a);
        assert_eq!(a.calls, ["connect copies=false"]);
        assert_eq!(w.handoff, Some(HandOff::GameFilesConnect));
        resume(&mut w, &mut a);
        assert_eq!(w.summary.game_files_connected, Some(2));
        assert_eq!(w.summary.game_files.as_ref().map(|r| r.connectable), Some(0), "nothing is left to connect");
        assert_eq!((w.step, w.view.clone()), (Step::GameFiles, View::Step), "the page is shown again, Continue goes on");
    }

    #[test]
    fn copies_are_asked_about_with_their_size_and_only_made_after_a_yes() {
        let (mut w, mut a) = (scanned(2), Fake { plan: Ok(GamePlan { ports: 2, copy_ports: 1, copy_bytes: 800_000_000 }), ..Fake::default() });
        press(&mut w, Action::ConnectGameFiles, &mut a);
        assert_eq!(w.view, View::ConfirmCopies { ports: 1, bytes: 800_000_000 });
        assert!(a.calls.iter().all(|c| !c.starts_with("connect")), "nothing is written before the answer");
        assert_eq!(back_action(&w), Action::Back);
        press(&mut w, Action::Back, &mut a);
        assert_eq!(w.view, View::Step);
        press(&mut w, Action::ConnectGameFiles, &mut a);
        press(&mut w, Action::CopyGameFiles, &mut a);
        assert_eq!(a.calls.last().unwrap(), "connect copies=true");
        let (mut w, mut a) = (scanned(2), Fake { plan: Ok(GamePlan { ports: 2, copy_ports: 1, copy_bytes: 5 }), ..Fake::default() });
        press(&mut w, Action::ConnectGameFiles, &mut a);
        press(&mut w, Action::SkipGameFileCopies, &mut a);
        assert_eq!(a.calls.last().unwrap(), "connect copies=false");
    }

    #[test]
    fn a_plan_that_fails_or_a_failed_connect_is_told_and_nothing_is_counted() {
        let (mut w, mut a) = (scanned(1), Fake { plan: Err("Not enough free space.".into()), ..Fake::default() });
        press(&mut w, Action::ConnectGameFiles, &mut a);
        assert_eq!(w.view, View::Notice("Not enough free space.".into()));
        let (mut w, mut a) = (scanned(1), Fake { connected: Some(Err("the original changed".into())), ..Fake::default() });
        press(&mut w, Action::ConnectGameFiles, &mut a);
        resume(&mut w, &mut a);
        assert_eq!(w.view, View::Notice("Nothing was connected: the original changed".into()));
        assert_eq!(w.summary.game_files_connected, None);
        assert_eq!(w.summary.game_files.as_ref().map(|r| r.connectable), Some(1), "still offered");
    }

    #[test]
    fn connect_belongs_to_a_scan_with_something_to_connect() {
        let (mut w, mut a) = (scanned(0), Fake::default());
        press(&mut w, Action::ConnectGameFiles, &mut a);
        assert!(a.calls.is_empty());
        let (mut w, mut a) = (at(Step::GameFiles, Mode::FirstRun), Fake::default());
        press(&mut w, Action::ConnectGameFiles, &mut a);
        assert!(a.calls.is_empty(), "no scan yet");
        let (mut w, mut a) = (scanned(2), Fake::default());
        w.step = Step::Steam;
        press(&mut w, Action::ConnectGameFiles, &mut a);
        assert!(a.calls.is_empty());
    }

    #[test]
    fn try_again_on_the_steam_page_draws_it_again_and_changes_nothing_else() {
        let (mut w, mut a) = (at(Step::Steam, Mode::FirstRun), Fake::default());
        press(&mut w, Action::TryAgain, &mut a);
        assert_eq!((w.step, a.calls.clone()), (Step::Steam, vec!["show Steam Step".to_string()]));
        let (mut w, mut a) = (at(Step::GameFiles, Mode::FirstRun), Fake::default());
        press(&mut w, Action::TryAgain, &mut a);
        assert!(a.calls.is_empty());
    }

}
