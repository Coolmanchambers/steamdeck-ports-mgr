//! "Add to Steam with artwork?" on screen (`crate::fork::steam_add` has the pages and the words), for a few named
//! ports: Fix problems' Steam fix (exactly the ports its problem names) and the Port page's Add to Steam. The Port
//! page's "Update the shortcut" and "Use that shortcut for this port" ask on a page of the same kind ([`open_change`]).
//! The install wizard's Steam step uses [`ask`] and [`add`] with its own page.
//!
//! - [`open`] / [`open_change`]: asks on one page, or, when there is nothing to ask (no account, several and none
//!   chosen, Steam's files unreadable, Steam's process check failing, already waiting, already in Steam), hands the
//!   note straight to `then`. Not now (and B, and the X) runs `not_now`.
//! - [`add`]: the covers and icons are fetched into the cache first (a shortcut's artwork is placed only from there),
//!   then `setup_wizard::steam::add_selected` on a screen of just these ports: written now with Steam closed
//!   (`shortcuts.vdf` backed up first, by `screens::apply_selected`), else queued and, outside Game Mode, the Close Steam
//!   flow, which closes Steam without asking again (the page already asked). Other changes already waiting with Steam
//!   closed: these are queued behind them and the Close Steam flow makes them all. `then` gets the notes.
//!
//! No new Steam code: every write goes through `add_selected` and `steam_close_screen`, which never write `shortcuts.vdf`
//! while Steam runs.

use super::page_dialog::{show_page, Handlers};
use super::state::AppState;
use crate::core::models::Port;
use crate::fork::art::thumbs::ArtRef;
use crate::fork::page::{Mark, Page};
use crate::fork::rom_screens::job::Background;
use crate::fork::screens::SteamScreen;
use crate::fork::settings::ForkSettings;
use crate::fork::setup_wizard::flow::SteamOutcome;
use crate::fork::steam_add::{self, AddAct, Ask, Caller, Change, Facts, Job, Shown};
use crate::fork::steam_sync::Action;
use crate::fork::store::{SteamProcessCheck, SystemSteamCheck};
use crate::ui::gamepad_router::GamepadRouter;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

type Router = Rc<RefCell<GamepadRouter>>;
/// What runs with the notes once the flow ends (added, queued, skipped and why).
pub(crate) type Then = Rc<dyn Fn(&Rc<AppState>, &Router, Vec<(Mark, String)>)>;
/// Where Not now, B and the X go.
pub(crate) type Back = Rc<dyn Fn(&Rc<AppState>, &Router)>;

const WORK_TICK: Duration = Duration::from_millis(150);

thread_local! {
    static WORK_TIMER: slint::Timer = slint::Timer::default();
    static WORK: RefCell<Vec<Box<dyn FnMut() -> bool>>> = RefCell::new(Vec::new());
}

/// Which rows of the screen are written: every port not in Steam, or the one shortcut of this port key.
#[derive(Clone)]
enum Pick {
    Missing,
    One(String),
}

impl Pick {
    fn job(&self) -> Job {
        match self {
            Pick::Missing => Job::Add,
            Pick::One(_) => Job::Change,
        }
    }

    fn select(&self, screen: &mut SteamScreen) {
        match self {
            Pick::Missing => screen.select_all_missing(),
            Pick::One(key) => {
                for row in &mut screen.rows {
                    row.selected = row.key == *key && row.can_change();
                }
            }
        }
    }
}

fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"].iter().find_map(|v| std::env::var_os(v)).map(PathBuf::from).filter(|p| p.is_absolute())
}

/// Polls `done` until it returns true (it has handled its job's answer); one timer for all.
fn when_done(done: impl FnMut() -> bool + 'static) {
    WORK.with(|w| w.borrow_mut().push(Box::new(done)));
    WORK_TIMER.with(|t| {
        if !t.running() {
            t.start(slint::TimerMode::Repeated, WORK_TICK, || {
                let mut now = WORK.with(|w| std::mem::take(&mut *w.borrow_mut()));
                now.retain_mut(|done| !done());
                let left = WORK.with(|w| {
                    let mut w = w.borrow_mut();
                    now.append(&mut w);
                    *w = now;
                    !w.is_empty()
                });
                if !left {
                    WORK_TIMER.with(slint::Timer::stop);
                }
            });
        }
    });
}

/// The ports that can go into Steam now: still installed (one may have been uninstalled while a page was open) and
/// not Android only.
fn steam_ports(app: &AppState, ports: &[Port]) -> Vec<Port> {
    ports.iter().filter(|p| steam_add::can_go_in(app.is_installed(p), p.is_android_only())).cloned().collect()
}

fn shown(ports: &[Port]) -> Vec<Shown> {
    ports.iter().map(|p| Shown { name: p.display_name().to_string(), art: Some(ArtRef::of_port(p)) }).collect()
}

/// The Add to Steam screen's own load for just these ports, with the queue read as well.
fn steam_screen(app: &AppState, ports: &[Port]) -> Option<SteamScreen> {
    let home = home_dir()?;
    let settings = ForkSettings::load(&super::steam_screens::settings_path(app));
    let state = app.state.borrow();
    let favorite = |key: &str| state.get(key).and_then(|i| i.favorite_exe.clone());
    let queue = crate::fork::steam_queue::queue_path(&app.paths.config_dir);
    let game_mode = crate::fork::steam_queue::game_mode_from_env();
    Some(crate::fork::setup_wizard::steam::load_screen(&home, &app.paths.library_dir(), ports, &settings, &favorite, &app.paths.cache_dir, &queue, game_mode))
}

/// Where Steam stands for the picked rows (`screen` has them selected).
fn facts(screen: &SteamScreen, pick: &Pick) -> Facts {
    let steam = SystemSteamCheck::new();
    let info = crate::fork::setup_wizard::steam::info(screen, &steam);
    let (to_add, queued) = match pick {
        Pick::Missing => (info.to_add, info.queued),
        Pick::One(_) => {
            let queued = match screen.active_account() {
                Some(a) if info.problem.is_none() => screen.rows.iter().filter(|r| r.selected && screen.queue.contains(a.account_id, &r.key)).count(),
                _ => 0,
            };
            (if info.problem.is_some() { 0 } else { screen.selected_count() }, queued)
        }
    };
    Facts {
        accounts: screen.accounts.len(),
        chosen: screen.account.is_some(),
        problem: info.problem,
        to_add,
        queued,
        other_by_name: matches!(pick, Pick::Missing) && screen.rows.iter().any(|r| r.can_change() && r.by_name()),
        immediate: info.immediate,
        game_mode: info.game_mode,
        waiting: info.waiting,
        check_failed: !info.game_mode && steam.running_steam_processes().is_err(),
    }
}

fn ask_picked(app: &AppState, ports: &[Port], pick: &Pick, caller: Caller) -> (Ask, Option<SteamScreen>) {
    if ports.is_empty() {
        return (Ask::Skip(None), None);
    }
    let Some(mut screen) = steam_screen(app, ports) else { return (Ask::Skip(None), None) };
    pick.select(&mut screen);
    let facts = facts(&screen, pick);
    (steam_add::ask(&facts, ports.len(), caller, pick.job()), Some(screen))
}

/// Whether to ask about adding `ports`, or the note that says why not. Reads Steam's files; writes nothing.
pub(crate) fn ask(app: &AppState, ports: &[Port], caller: Caller) -> Ask {
    ask_picked(app, &steam_ports(app, ports), &Pick::Missing, caller).0
}

fn quiet<A: Clone + 'static>() -> Handlers<A> {
    Handlers { on_action: Rc::new(|_, _, _| {}), on_back: Rc::new(|_, _| {}), on_exit: Rc::new(|_, _| {}), on_input: None }
}

fn ask_handlers(on_add: Rc<dyn Fn(&Rc<AppState>, &Router)>, not_now: Back) -> Handlers<AddAct> {
    let (n0, n1, n2) = (not_now.clone(), not_now.clone(), not_now);
    let on_action = move |app: &Rc<AppState>, router: &Router, act: AddAct| match act {
        AddAct::Add => on_add(app, router),
        AddAct::NotNow => n0(app, router),
    };
    Handlers { on_action: Rc::new(on_action), on_back: Rc::new(move |app, router| n1(app, router)), on_exit: Rc::new(move |app, router| n2(app, router)), on_input: None }
}

/// "Add to Steam with artwork?" for `ports`, or straight to `then` with the note when there is nothing to ask.
pub(crate) fn open(app: &Rc<AppState>, router: &Router, ports: Vec<Port>, caller: Caller, then: Then, not_now: Back) {
    let ports = steam_ports(app, &ports);
    let names: Vec<String> = ports.iter().map(|p| p.display_name().to_string()).collect();
    let (running, game_mode, others) = match ask_picked(app, &ports, &Pick::Missing, caller).0 {
        Ask::Skip(note) => {
            eprintln!("steam add: nothing to ask for {}: {note:?}", names.join(", "));
            return then(app, router, note.into_iter().collect());
        }
        Ask::Ask { running, game_mode, others } => (running, game_mode, others),
    };
    let page = steam_add::page(&shown(&ports), game_mode, running, others, caller);
    let on_add: Rc<dyn Fn(&Rc<AppState>, &Router)> = Rc::new(move |app, router| {
        eprintln!("steam add: adding {}", names.join(", "));
        run(app, router, ports.clone(), Pick::Missing, steam_add::working_page(&shown(&ports)), then.clone());
    });
    show_page(app, router, page, ask_handlers(on_add, not_now));
}

/// What the picked shortcut's change is, in the page's words.
fn change_of(screen: &SteamScreen) -> Change {
    let Some(Action::Update { changes, by_name, .. }) = screen.rows.iter().find(|r| r.selected).and_then(|r| r.plan.as_ref()).map(|p| &p.action) else {
        return Change::default();
    };
    let program = changes.iter().find(|c| c.field == "program");
    Change {
        by_name: *by_name,
        from: program.map(|c| steam_add::program_name(&c.from)),
        to: program.map(|c| steam_add::program_name(&c.to)),
        fields: changes.iter().map(|c| c.field.to_string()).collect(),
    }
}

/// The Port page's "Update the shortcut" / "Use that shortcut for this port": what changes on one page, then the same
/// write, or queue and Close Steam flow.
pub(crate) fn open_change(app: &Rc<AppState>, router: &Router, port: Port, then: Then, not_now: Back) {
    let ports = steam_ports(app, std::slice::from_ref(&port));
    let pick = Pick::One(port.key().to_string());
    let (asked, screen) = ask_picked(app, &ports, &pick, Caller::PortPage);
    let (running, game_mode, others) = match asked {
        Ask::Skip(note) => {
            eprintln!("steam add: no shortcut change to ask about for {}: {note:?}", port.display_name());
            return then(app, router, note.into_iter().collect());
        }
        Ask::Ask { running, game_mode, others } => (running, game_mode, others),
    };
    let change = screen.as_ref().map(change_of).unwrap_or_default();
    let page = steam_add::change_page(&shown(&ports)[0], &change, game_mode, running, others);
    let on_add: Rc<dyn Fn(&Rc<AppState>, &Router)> = Rc::new(move |app, router| {
        eprintln!("steam add: changing the shortcut of {}", port.display_name());
        run(app, router, ports.clone(), pick.clone(), steam_add::working_page(&shown(&ports)), then.clone());
    });
    show_page(app, router, page, ask_handlers(on_add, not_now));
}

/// Yes: `working` is shown while the covers and icons are fetched, then the ports are added (or queued, and the Close
/// Steam flow closes Steam and makes them). `then` gets what happened.
pub(crate) fn add<A: Clone + 'static>(app: &Rc<AppState>, router: &Router, ports: Vec<Port>, working: Page<A>, then: Then) {
    let ports = steam_ports(app, &ports);
    run(app, router, ports, Pick::Missing, working, then);
}

fn run<A: Clone + 'static>(app: &Rc<AppState>, router: &Router, ports: Vec<Port>, pick: Pick, working: Page<A>, then: Then) {
    show_page(app, router, working, quiet());
    let cache = app.paths.cache_dir.clone();
    let wanted: Vec<(String, Option<String>, Option<String>)> = ports.iter().map(|p| (p.folder.clone(), p.image.clone(), p.icon.clone())).collect();
    let job = Background::spawn(move |_| {
        for (folder, image, icon) in wanted {
            if let Some(url) = image {
                crate::core::image_cache::cache_image(&url, &cache, &folder);
            }
            if let Some(url) = icon {
                crate::core::image_cache::cache_image(&url, &cache, &format!("{folder}_icon"));
            }
        }
    });
    let (app, router) = (app.clone(), router.clone());
    when_done(move || {
        if job.take().is_none() {
            return false;
        }
        let (n, kind) = (ports.len(), pick.job());
        let names: Vec<String> = ports.iter().map(|p| p.display_name().to_string()).collect();
        let outcome = match steam_screen(&app, &ports) {
            None => SteamOutcome::Problem("could not find your home folder".to_string()),
            Some(mut screen) => {
                pick.select(&mut screen);
                let settings = ForkSettings::load(&super::steam_screens::settings_path(&app));
                let queue = crate::fork::steam_queue::queue_path(&app.paths.config_dir);
                crate::fork::setup_wizard::steam::add_selected(&screen, &SystemSteamCheck::new(), &settings, &queue, SystemTime::now(), &crate::fork::store::backup_stamp(), true)
            }
        };
        eprintln!("steam add: Steam for {}: {outcome:?}", names.join(", "));
        // The fetch says nothing when it fails: what is in the cache now decides the words.
        let names_art = ports.iter().any(|p| p.image.is_some() || p.icon.is_some());
        let art_missing = ports.iter().any(|p| crate::fork::steam_queue::port_art_missing(p, &app.paths.cache_dir));
        let done_line = match kind {
            Job::Add => steam_add::added_note(names_art, art_missing, n),
            Job::Change => steam_add::CHANGED_NOTE,
        };
        let note = match outcome {
            SteamOutcome::Added(_) | SteamOutcome::Applied(_) => (Mark::Done, done_line.to_string()),
            SteamOutcome::Nothing => (Mark::Done, steam_add::already_note(kind).to_string()),
            SteamOutcome::Queued { game_mode: true, .. } => (Mark::Waiting, steam_add::desktop_mode_note(n, kind).to_string()),
            SteamOutcome::Queued { game_mode: false, .. } => {
                let then = then.clone();
                // Stop waiting, B, a blocked or failed close: whatever was not made waits, and the note says so.
                let after: super::steam_close_screen::Then = Rc::new(move |app, router, done| {
                    let line = if done.waiting == 0 && done.applied > 0 { (Mark::Done, done_line) } else { (Mark::Waiting, steam_add::waiting_note(n, kind)) };
                    then(app, router, vec![(line.0, line.1.to_string())]);
                });
                // The page already asked: Steam is closed now, without a second question.
                super::steam_close_screen::open(&app, &router, super::steam_close_screen::Opts { ask_first: false, ..Default::default() }, after);
                return true;
            }
            SteamOutcome::Problem(text) => (Mark::Problem, steam_add::problem_note(&text, kind)),
        };
        then(&app, &router, vec![note]);
        true
    });
}
