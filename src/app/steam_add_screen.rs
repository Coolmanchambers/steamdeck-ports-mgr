//! "Add to Steam with artwork?" on screen (`crate::fork::steam_add` has the page and the words), for a few named ports:
//! Fix problems' Steam fix (exactly the ports its problem names) and the Port page's Add to Steam. The install wizard's
//! Steam step uses [`ask`] and [`add`] with its own page.
//!
//! - [`open`]: asks on one page, or, when there is nothing to ask (no account, several and none chosen, Steam's files
//!   unreadable, already waiting, already in Steam), hands the note straight to `then`. Not now (and B, and the X) runs
//!   `not_now`.
//! - [`add`]: the covers and icons are fetched into the cache first (a shortcut's artwork is placed only from there),
//!   then the setup wizard's add (`setup_wizard::steam::add_all` on a screen of just these ports): written now with
//!   Steam closed (`shortcuts.vdf` backed up first, by `screens::apply_selected`), else queued and, outside Game Mode,
//!   the Close Steam flow (it asks first). `then` gets the notes that say what happened.
//!
//! No new Steam code: every write goes through `add_all` and `steam_close_screen`, which never write `shortcuts.vdf`
//! while Steam runs.

use super::page_dialog::{show_page, Handlers};
use super::state::AppState;
use crate::core::models::Port;
use crate::fork::art::thumbs::ArtRef;
use crate::fork::page::{Mark, Page};
use crate::fork::rom_screens::job::Background;
use crate::fork::settings::ForkSettings;
use crate::fork::setup_wizard::flow::SteamOutcome;
use crate::fork::steam_add::{self, AddAct, Ask, Facts, Shown};
use crate::fork::store::SystemSteamCheck;
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

/// The ports that can go into Steam (an Android-only port cannot).
fn steam_ports(ports: &[Port]) -> Vec<Port> {
    ports.iter().filter(|p| !p.is_android_only()).cloned().collect()
}

fn shown(ports: &[Port]) -> Vec<Shown> {
    ports.iter().map(|p| Shown { name: p.display_name().to_string(), art: Some(ArtRef::of_port(p)) }).collect()
}

/// The Add to Steam screen's own load for just these ports, with the queue read as well.
fn steam_screen(app: &AppState, ports: &[Port]) -> Option<crate::fork::screens::SteamScreen> {
    let home = home_dir()?;
    let settings = ForkSettings::load(&super::steam_screens::settings_path(app));
    let state = app.state.borrow();
    let favorite = |key: &str| state.get(key).and_then(|i| i.favorite_exe.clone());
    let queue = crate::fork::steam_queue::queue_path(&app.paths.config_dir);
    let game_mode = crate::fork::steam_queue::game_mode_from_env();
    Some(crate::fork::setup_wizard::steam::load_screen(&home, &app.paths.library_dir(), ports, &settings, &favorite, &app.paths.cache_dir, &queue, game_mode))
}

/// Whether to ask about adding `ports`, or the note that says why not. Reads Steam's files; writes nothing.
pub(crate) fn ask(app: &AppState, ports: &[Port]) -> Ask {
    let ports = steam_ports(ports);
    if ports.is_empty() {
        return Ask::Skip(None);
    }
    let Some(screen) = steam_screen(app, &ports) else { return Ask::Skip(None) };
    let info = crate::fork::setup_wizard::steam::info(&screen, &SystemSteamCheck::new());
    let facts = Facts {
        accounts: screen.accounts.len(),
        chosen: screen.account.is_some(),
        problem: info.problem,
        to_add: info.to_add,
        queued: info.queued,
        other_by_name: screen.rows.iter().any(|r| r.can_change() && r.by_name()),
        immediate: info.immediate,
        game_mode: info.game_mode,
    };
    steam_add::ask(&facts, ports.len())
}

fn quiet<A: Clone + 'static>() -> Handlers<A> {
    Handlers { on_action: Rc::new(|_, _, _| {}), on_back: Rc::new(|_, _| {}), on_exit: Rc::new(|_, _| {}), on_input: None }
}

/// "Add to Steam with artwork?" for `ports`, or straight to `then` with the note when there is nothing to ask.
pub(crate) fn open(app: &Rc<AppState>, router: &Router, ports: Vec<Port>, then: Then, not_now: Back) {
    let ports = steam_ports(&ports);
    let names: Vec<String> = ports.iter().map(|p| p.display_name().to_string()).collect();
    let (running, game_mode) = match ask(app, &ports) {
        Ask::Skip(note) => {
            eprintln!("steam add: nothing to ask for {}: {note:?}", names.join(", "));
            return then(app, router, note.into_iter().collect());
        }
        Ask::Ask { running, game_mode } => (running, game_mode),
    };
    let page = steam_add::page(&shown(&ports), game_mode, running);
    let (n1, n2) = (not_now.clone(), not_now.clone());
    let on_action = move |app: &Rc<AppState>, router: &Router, act: AddAct| match act {
        AddAct::Add => {
            eprintln!("steam add: adding {}", names.join(", "));
            add(app, router, ports.clone(), steam_add::working_page(&shown(&ports)), then.clone());
        }
        AddAct::NotNow => not_now(app, router),
    };
    let handlers = Handlers { on_action: Rc::new(on_action), on_back: Rc::new(move |app, router| n1(app, router)), on_exit: Rc::new(move |app, router| n2(app, router)), on_input: None };
    show_page(app, router, page, handlers);
}

/// Yes: `working` is shown while the covers and icons are fetched, then the ports are added (or queued, and the Close
/// Steam flow asks to close Steam). `then` gets what happened.
pub(crate) fn add<A: Clone + 'static>(app: &Rc<AppState>, router: &Router, ports: Vec<Port>, working: Page<A>, then: Then) {
    let ports = steam_ports(&ports);
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
        let n = ports.len();
        let names: Vec<String> = ports.iter().map(|p| p.display_name().to_string()).collect();
        let outcome = match steam_screen(&app, &ports) {
            None => SteamOutcome::Problem("could not find your home folder".to_string()),
            Some(mut screen) => {
                let settings = ForkSettings::load(&super::steam_screens::settings_path(&app));
                let queue = crate::fork::steam_queue::queue_path(&app.paths.config_dir);
                crate::fork::setup_wizard::steam::add_all(&mut screen, &SystemSteamCheck::new(), &settings, &queue, SystemTime::now(), &crate::fork::store::backup_stamp())
            }
        };
        eprintln!("steam add: Steam for {}: {outcome:?}", names.join(", "));
        // The fetch says nothing when it fails: what is in the cache now decides the words.
        let names_art = ports.iter().any(|p| p.image.is_some() || p.icon.is_some());
        let art_missing = ports.iter().any(|p| crate::fork::steam_queue::port_art_missing(p, &app.paths.cache_dir));
        let added = steam_add::added_note(names_art, art_missing, n);
        let note = match outcome {
            SteamOutcome::Added(_) | SteamOutcome::Applied(_) => (Mark::Done, added.to_string()),
            SteamOutcome::Nothing => (Mark::Done, "Already in Steam".to_string()),
            SteamOutcome::Queued { game_mode: true, .. } => (Mark::Waiting, steam_add::desktop_mode_note(n).to_string()),
            SteamOutcome::Queued { game_mode: false, .. } => {
                let then = then.clone();
                let after: super::steam_close_screen::Then = Rc::new(move |app, router, done| {
                    let line = if done.waiting == 0 && done.applied > 0 { (Mark::Done, added) } else { (Mark::Waiting, steam_add::waiting_note(n)) };
                    then(app, router, vec![(line.0, line.1.to_string())]);
                });
                let lead = Some(steam_add::close_lead(&names));
                super::steam_close_screen::open(&app, &router, super::steam_close_screen::Opts { ask_first: true, lead, ..Default::default() }, after);
                return true;
            }
            SteamOutcome::Problem(text) => (Mark::Problem, steam_add::problem_note(&text)),
        };
        then(&app, &router, vec![note]);
        true
    });
}
