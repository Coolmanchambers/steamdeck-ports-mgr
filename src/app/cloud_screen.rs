//! Settings > "Cloud backup...": the screens, and the background work behind them.
//!
//! What each page says is `crate::fork::cloud::view`; what is uploaded, restored and stored is the rest of
//! `crate::fork::cloud` (all tested without any UI). This file draws the pages through
//! `page_dialog::show_page_handle`, runs the sign-in, the cloud listing and the download on worker threads,
//! and starts the upload passes the hooks ask for ([`trigger`]).
//!
//! Pages: not set up in this build; "Keep a copy of your saves online?"; the sign-in page (QR code, short code,
//! the phone address; it moves on by itself); "Signed in"; the status page (Back up now, Get backups from the
//! cloud, Sign out); the restore list; "Copied" with the hand-off to Restore saves; the sign-out question.
//! Nothing is written to Google until the person signs in, and nothing in the cloud is ever deleted.

use super::dialogs::close_current_dialog;
use super::page_dialog::{show_page_handle, Handlers, PageHandle};
use super::state::AppState;
use crate::core::models::Port;
use crate::fork::cloud::endpoints::{self, Setup};
use crate::fork::cloud::hooks::{self, Next, Why};
use crate::fork::cloud::oauth::{self, DeviceCode, Poll};
use crate::fork::cloud::qr::QrMatrix;
use crate::fork::cloud::restore::{self, DownloadError, Downloaded, Place};
use crate::fork::cloud::service::{ErrorKind, RemoteFile, ServiceError, ServiceId};
use crate::fork::cloud::sync::{self, PassInput};
use crate::fork::cloud::view::{self, Act, Status};
use crate::fork::cloud::{self, record, store, tokens};
use crate::fork::page::Page;
use crate::fork::rom_screens::job::{Background, Ctl};
use crate::fork::save_restore::view::human_size;
use crate::ui::gamepad_router::GamepadRouter;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type Router = Rc<RefCell<GamepadRouter>>;

/// The one service there is today; the code that lists "every signed-in service" is already shaped for more.
const SERVICE: ServiceId = ServiceId::Google;

/// How many backups the restore list shows before "Show older".
const LIST_SHOWN: usize = 6;

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn utc_offset() -> i32 {
    use chrono::Offset;
    chrono::Local::now().offset().fix().local_minus_utc()
}

fn tokens_path(config_dir: &Path) -> PathBuf {
    config_dir.join(tokens::FILE_NAME)
}

fn record_path(config_dir: &Path) -> PathBuf {
    config_dir.join(record::FILE_NAME)
}

fn signed_in(app: &AppState) -> bool {
    !store::signed_in(&tokens_path(&app.paths.config_dir)).is_empty()
}

// -- The upload passes ----------------------------------------------------------------------------------------

/// What a finished pass tells the window thread.
struct PassOutcome {
    reports: Vec<(ServiceId, Summary)>,
}

struct Summary {
    deferred: usize,
    /// The failure: its kind and the wait the service asked for.
    error: Option<(ErrorKind, Option<u64>)>,
}

struct Running {
    job: Rc<Background<PassOutcome>>,
}

#[derive(Default)]
struct Runtime {
    running: Option<Running>,
    /// A trigger arrived while a pass ran: run another when it ends.
    dirty: Option<Why>,
    /// Failed passes in a row that retry by themselves (the retry delay grows with it).
    failures: usize,
}

thread_local! {
    static RUNTIME: RefCell<Runtime> = RefCell::new(Runtime::default());
    static PASS_TIMER: slint::Timer = slint::Timer::default();
    static RETRY_TIMER: slint::Timer = slint::Timer::default();
    static SAFETY_TIMER: slint::Timer = slint::Timer::default();
    /// The screens' own timer (sign-in polling, live status, the listing and the download).
    static FLOW_TIMER: slint::Timer = slint::Timer::default();
}

/// At app start: the catch-up pass for anything not uploaded yet (which is also the queue after being offline),
/// and the ten-minute safety check. Both do nothing unless the build has a Google client and the person is
/// signed in.
pub(crate) fn at_start(app: &Rc<AppState>) {
    trigger(app, Why::Start);
    let app = app.clone();
    SAFETY_TIMER.with(|t| t.start(slint::TimerMode::Repeated, hooks::SAFETY_EVERY, move || trigger(&app, Why::Safety)));
}

/// A backup was made, or a timer fired, or the person asked: run an upload pass now (or right after the one
/// that is running).
pub(crate) fn trigger(app: &Rc<AppState>, why: Why) {
    if !hooks::should_run(endpoints::available(), signed_in(app)) {
        return;
    }
    let running = RUNTIME.with(|r| r.borrow().running.is_some());
    if running {
        RUNTIME.with(|r| {
            let mut r = r.borrow_mut();
            // A manual request is never downgraded to a timer's.
            r.dirty = Some(if r.dirty == Some(Why::Manual) { Why::Manual } else { why });
        });
        return;
    }
    start_pass(app, why);
}

fn start_pass(app: &Rc<AppState>, why: Why) {
    let (config_dir, saves) = (app.paths.config_dir.clone(), app.paths.saves_backup_dir.clone());
    let job = Rc::new(Background::spawn(move |ctl| pass_job(&config_dir, &saves, why, ctl)));
    RUNTIME.with(|r| r.borrow_mut().running = Some(Running { job }));
    let app = app.clone();
    PASS_TIMER.with(|t| t.start(slint::TimerMode::Repeated, Duration::from_millis(250), move || poll_pass(&app)));
}

fn poll_pass(app: &Rc<AppState>) {
    let Some(job) = RUNTIME.with(|r| r.borrow().running.as_ref().map(|r| r.job.clone())) else { return PASS_TIMER.with(slint::Timer::stop) };
    let Some(result) = job.take() else { return };
    PASS_TIMER.with(slint::Timer::stop);
    RUNTIME.with(|r| r.borrow_mut().running = None);
    let outcome = result.unwrap_or_else(|panic| {
        cloud::log::note(format!("a pass stopped because of an error inside the app ({})", cloud::log::scrub(&panic)));
        PassOutcome { reports: vec![(SERVICE, Summary { deferred: 0, error: Some((ErrorKind::Other, None)) })] }
    });
    after_pass(app, &outcome);
}

fn after_pass(app: &Rc<AppState>, outcome: &PassOutcome) {
    let error = outcome.reports.iter().find_map(|(_, s)| s.error);
    let deferred: usize = outcome.reports.iter().map(|(_, s)| s.deferred).sum();
    let failures = RUNTIME.with(|r| r.borrow().failures);
    let next = hooks::after_pass(error, deferred, failures);
    RUNTIME.with(|r| r.borrow_mut().failures = if error.is_some_and(|(k, _)| k.retries_by_itself()) { failures + 1 } else { 0 });
    RETRY_TIMER.with(slint::Timer::stop);
    if let Next::After(wait, why) = next {
        let app = app.clone();
        RETRY_TIMER.with(|t| t.start(slint::TimerMode::SingleShot, wait, move || trigger(&app, why)));
    }
    if let Some(why) = RUNTIME.with(|r| r.borrow_mut().dirty.take()) {
        trigger(app, why);
    }
}

fn pass_job(config_dir: &Path, saves: &Path, why: Why, ctl: &Ctl) -> PassOutcome {
    let Some(setup) = endpoints::current() else { return PassOutcome { reports: Vec::new() } };
    let (tokens_file, record_file) = (tokens_path(config_dir), record_path(config_dir));
    // Each service is taken together with the name of the sign-in the pass starts under: if the person signs out
    // (or in again, as someone else) while it runs, it may not write into the record any more.
    let signed: Vec<(ServiceId, String)> = store::signed_in(&tokens_file).into_iter().filter_map(|(id, _)| Some((id, store::signin_name(&tokens_file, id)?))).collect();
    let mut services: Vec<_> = signed.iter().map(|(id, _)| endpoints::service(&setup, &tokens_file, *id)).collect();
    let device = cloud::device_name();
    let fraction = |f: f32| ctl.set_fraction(f);
    let input = PassInput { saves_backup_dir: saves, device: &device, now: unix_now(), min_age: why.min_age_secs(), fraction: Some(&fraction) };
    let reports = sync::run_services(
        &mut services,
        &|id| store::record_of(&record_file, id),
        &mut |id, part| {
            let Some((_, signin)) = signed.iter().find(|(s, _)| *s == id) else { return };
            if let Err(e) = store::persist_service(&record_file, &tokens_file, id, signin, part) {
                cloud::log::note(format!("{}: could not save the upload record ({})", id.key(), e.kind()));
            }
        },
        &input,
        &|| ctl.cancelled(),
        &mut |text| ctl.set_progress(text.to_string()),
    );
    ctl.set_progress(String::new());
    // Drive refused the token even after a fresh one was asked for: the status must say signed out, with the buttons
    // that go with it, not "up to date" with a failure line.
    for (id, r) in &reports {
        if r.error.as_ref().is_some_and(|e| e.kind == ErrorKind::SignedOut) {
            if let Some((_, signin)) = signed.iter().find(|(s, _)| s == id) {
                if let Err(e) = store::mark_signed_out(&tokens_file, *id, signin) {
                    cloud::log::note(format!("{}: could not record the sign-out ({})", id.key(), e.kind()));
                }
            }
        }
    }
    PassOutcome {
        reports: reports
            .into_iter()
            .map(|(id, r)| {
                if let Some(e) = &r.error {
                    cloud::log::note(format!("{}: pass stopped: {} (uploaded {}, recognised {})", id.key(), e, r.uploaded, r.recognised));
                } else {
                    cloud::log::note(format!("{}: pass done: uploaded {}, recognised {}, waiting {}, broken {}", id.key(), r.uploaded, r.recognised, r.deferred, r.broken));
                }
                (id, Summary { deferred: r.deferred, error: r.error.map(|e| (e.kind, e.retry_after)) })
            })
            .collect(),
    }
}

/// The home screen's small cloud backup status: the mark code (`fork::page::Mark::code`: 1 done, 3 waiting, 5 a
/// problem) and a few words; `(0, "")` when not signed in (nothing is shown).
pub(crate) fn home_status(app: &AppState) -> (i32, String) {
    let tokens_file = tokens_path(&app.paths.config_dir);
    if !store::tokens_of(&tokens_file, SERVICE).is_some_and(|t| t.signed_in()) {
        return (0, String::new());
    }
    if running_text().is_some() {
        return (3, "Backing up".to_string());
    }
    let record = store::record_of(&record_path(&app.paths.config_dir), SERVICE);
    if record.last_error.is_some() {
        (5, "Backup problem".to_string())
    } else if record.last_ok.is_some() {
        (1, "Backed up".to_string())
    } else {
        (3, "Not backed up yet".to_string())
    }
}

/// Fork (checkup): signed in, with a pass that copied (or found nothing new) within the last `within` seconds and no error
/// since: the saves are safe online, so a zip next to the app is not needed.
pub(crate) fn recent_success(app: &AppState, within: u64) -> bool {
    if !store::tokens_of(&tokens_path(&app.paths.config_dir), SERVICE).is_some_and(|t| t.signed_in()) {
        return false;
    }
    let record = store::record_of(&record_path(&app.paths.config_dir), SERVICE);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    record.last_error.is_none() && record.last_ok.is_some_and(|t| now.saturating_sub(t) <= within)
}

/// How far a running pass's copying is, when it has started copying.
fn running_fraction() -> Option<f32> {
    RUNTIME.with(|r| r.borrow().running.as_ref().and_then(|r| r.job.fraction()))
}

/// What a running pass is doing, for the status page.
fn running_text() -> Option<String> {
    RUNTIME.with(|r| r.borrow().running.as_ref().map(|r| r.job.progress())).map(|t| if t.is_empty() { "Checking for new backups".to_string() } else { t })
}

fn cancel_pass() {
    RUNTIME.with(|r| {
        if let Some(running) = &r.borrow().running {
            running.job.cancel();
        }
    });
}

// -- The screens ------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Intro,
    NotSetUp,
    GettingCode,
    SigningIn,
    Signed,
    Status,
    Listing,
    List,
    Copying,
    Copied,
    SignOutAsk,
    Notice,
}

/// What came back from copying backups to this Deck.
struct CopyOutcome {
    results: Vec<(RemoteFile, Result<Downloaded, DownloadError>)>,
}

struct Session {
    stage: Cell<Stage>,
    handle: RefCell<Option<PageHandle<Act>>>,
    code: RefCell<Option<DeviceCode>>,
    interval: Cell<u64>,
    next_poll: Cell<u64>,
    /// The line on the sign-in page about what is happening ("Waiting for you...", "That code ran out...").
    waiting: RefCell<String>,
    start_job: RefCell<Option<Rc<Background<Result<DeviceCode, ServiceError>>>>>,
    poll_job: RefCell<Option<Rc<Background<Poll>>>>,
    list_job: RefCell<Option<Rc<Background<Result<Vec<RemoteFile>, ServiceError>>>>>,
    copy_job: RefCell<Option<Rc<Background<CopyOutcome>>>>,
    /// The cloud backups, newest first.
    remote: RefCell<Vec<RemoteFile>>,
    shown: Cell<usize>,
    /// What to hand over to Restore saves after "Copied".
    copied_folder: RefCell<Option<String>>,
    copying_label: RefCell<String>,
    /// True when the copy is "Copy all my backups" (a summary), false for one chosen backup.
    copying_all: Cell<bool>,
    last_status: RefCell<Option<Page<Act>>>,
    ticks: Cell<u64>,
    /// Cancel was pressed on a copy: the file being copied still finishes, then the page says how far it got.
    cancelling: Cell<bool>,
    copying_total: Cell<usize>,
}

impl Session {
    fn new() -> Rc<Session> {
        Rc::new(Session {
            stage: Cell::new(Stage::Intro),
            handle: RefCell::new(None),
            code: RefCell::new(None),
            interval: Cell::new(5),
            next_poll: Cell::new(0),
            waiting: RefCell::new("Waiting for you to finish on your phone.".into()),
            start_job: RefCell::new(None),
            poll_job: RefCell::new(None),
            list_job: RefCell::new(None),
            copy_job: RefCell::new(None),
            remote: RefCell::new(Vec::new()),
            shown: Cell::new(LIST_SHOWN),
            copied_folder: RefCell::new(None),
            copying_label: RefCell::new(String::new()),
            copying_all: Cell::new(false),
            last_status: RefCell::new(None),
            ticks: Cell::new(0),
            cancelling: Cell::new(false),
            copying_total: Cell::new(0),
        })
    }

    fn cancel_jobs(&self) {
        if let Some(j) = self.start_job.borrow_mut().take() {
            j.cancel();
        }
        if let Some(j) = self.poll_job.borrow_mut().take() {
            j.cancel();
        }
        if let Some(j) = self.list_job.borrow_mut().take() {
            j.cancel();
        }
        if let Some(j) = self.copy_job.borrow_mut().take() {
            j.cancel();
        }
    }
}

/// Settings > "Cloud backup...". B on the first page returns to Settings.
pub(crate) fn open_from_settings(app: &Rc<AppState>, router: &Router) {
    let s = Session::new();
    show_home(app, router, &s);
}

fn handlers(s: &Rc<Session>) -> Handlers<Act> {
    let (s_act, s_back, s_exit) = (s.clone(), s.clone(), s.clone());
    Handlers {
        on_action: Rc::new(move |app, router, act| act_on(app, router, &s_act, act)),
        on_back: Rc::new(move |app, router| act_on(app, router, &s_back, Act::Back)),
        on_exit: Rc::new(move |app, router| leave(app, router, &s_exit)),
        on_input: None,
    }
}

fn show(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, stage: Stage, page: Page<Act>) {
    s.stage.set(stage);
    *s.last_status.borrow_mut() = None;
    let handle = show_page_handle(app, router, page, handlers(s));
    *s.handle.borrow_mut() = handle;
    ensure_timer(app, router, s);
}

/// A page, an explanation, and one way out.
fn notice(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, title: &str, paragraphs: Vec<String>) {
    show(app, router, s, Stage::Notice, view::message_page(title, paragraphs, "Back", Act::Back));
}

/// A failure told in plain words, with what to do about it.
fn notice_for(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, title: &str, error: &ServiceError) {
    cloud::log::note(format!("screen: {title}: {error}"));
    let mut lines = vec![view::one_off_text(SERVICE, error.kind)];
    if error.kind == ErrorKind::Other && !error.detail.is_empty() {
        lines.push(format!("Details: {}.", cloud::log::scrub(&error.detail)));
    }
    notice(app, router, s, title, lines);
}

fn leave(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    FLOW_TIMER.with(slint::Timer::stop);
    s.cancel_jobs();
    close_current_dialog(app, router);
    super::settings_menu_screen::open_group(app, router, crate::fork::settings_menu::Group::Saves);
}

/// The first page, from what is known: not set up, the intro, or the status.
fn show_home(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    let entry = store::tokens_of(&tokens_path(&app.paths.config_dir), SERVICE);
    let signed = entry.as_ref().is_some_and(|t| t.signed_in());
    if !endpoints::available() && !signed {
        return show(app, router, s, Stage::NotSetUp, view::not_set_up_page());
    }
    if entry.is_some() {
        return show_status(app, router, s);
    }
    show(app, router, s, Stage::Intro, view::intro_page());
}

fn status_page_now(app: &AppState) -> Page<Act> {
    let tokens_file = tokens_path(&app.paths.config_dir);
    let entry = store::tokens_of(&tokens_file, SERVICE);
    let signed = entry.as_ref().is_some_and(|t| t.signed_in());
    let account = entry.and_then(|t| t.email).map_or_else(|| "your account".to_string(), |e| oauth::mask_email(&e));
    let record = store::record_of(&record_path(&app.paths.config_dir), SERVICE);
    let running = running_text();
    let lines = if signed { view::status_lines(&Status { service: SERVICE, record: &record, now: unix_now(), offset_secs: utc_offset(), running: running.as_deref() }) } else { vec![view::problem_text(SERVICE, ErrorKind::SignedOut)] };
    let page = view::status_page(&account, lines, running.is_some() && signed, !signed);
    // While a pass runs: how far its copying is (a moving bar while it checks and looks).
    if running.is_some() && signed { page.with_progress(running_fraction()) } else { page }
}

fn show_status(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    let page = status_page_now(app);
    show(app, router, s, Stage::Status, page.clone());
    *s.last_status.borrow_mut() = Some(page);
}

fn act_on(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, act: Act) {
    match act {
        Act::SignIn | Act::NewCode => start_sign_in(app, router, s),
        Act::Cancel => match s.stage.get() {
            Stage::GettingCode | Stage::SigningIn => {
                s.cancel_jobs();
                show_home(app, router, s);
            }
            Stage::Listing => {
                s.cancel_jobs();
                show_status(app, router, s);
            }
            // A file cannot be stopped half way: the one being copied finishes, then the page says how far it got.
            Stage::Copying => {
                if let Some(job) = s.copy_job.borrow().as_ref() {
                    job.cancel();
                }
                s.cancelling.set(true);
                if let Some(h) = s.handle.borrow().as_ref() {
                    h.update(view::copying_back_page(&s.copying_label.borrow(), "Stopping after this file..."));
                }
            }
            _ => leave(app, router, s),
        },
        Act::Back => match s.stage.get() {
            Stage::Intro | Stage::NotSetUp | Stage::Status => leave(app, router, s),
            Stage::GettingCode | Stage::SigningIn | Stage::Listing | Stage::Copying => act_on(app, router, s, Act::Cancel),
            Stage::Signed | Stage::Notice => show_home(app, router, s),
            Stage::List | Stage::Copied | Stage::SignOutAsk => show_status(app, router, s),
        },
        Act::BackUpNow => {
            trigger(app, Why::Manual);
            show_status(app, router, s);
        }
        Act::NotNow | Act::KeepSignedIn => show_status(app, router, s),
        Act::ForgetAccount => forget_account(app, router, s),
        Act::GetBackups => start_listing(app, router, s),
        Act::Pick(i) => {
            let file = s.remote.borrow().get(i).cloned();
            if let Some(file) = file {
                let label = label_of(app, s, &file);
                s.copying_all.set(false);
                start_copy(app, router, s, vec![file], label);
            }
        }
        Act::CopyAll => {
            let missing: Vec<RemoteFile> = restore::missing_locally(&s.remote.borrow(), &app.paths.saves_backup_dir).into_iter().cloned().collect();
            s.copying_all.set(true);
            start_copy(app, router, s, missing, "all your backups".to_string());
        }
        Act::ShowOlder => {
            s.shown.set(usize::MAX);
            show_list(app, router, s);
        }
        Act::RestoreSaves => hand_over_to_restore(app, router, s),
        Act::SignOutAsk => show(app, router, s, Stage::SignOutAsk, view::signing_out_page(SERVICE)),
        Act::SignOut => sign_out(app, router, s),
        Act::OpenOnDeck => {
            let url = s.code.borrow().as_ref().map(|c| c.verification_url.clone());
            if let Some(url) = url {
                crate::core::launch::open_url(&url);
            }
        }
    }
}

// -- Timer ----------------------------------------------------------------------------------------------------------

fn ensure_timer(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    let (app, router, s) = (app.clone(), router.clone(), s.clone());
    FLOW_TIMER.with(|t| t.start(slint::TimerMode::Repeated, Duration::from_millis(250), move || tick(&app, &router, &s)));
}

fn tick(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    // The window was replaced by something else: stop working for it.
    if s.handle.borrow().as_ref().is_none_or(|h| !h.is_open()) {
        FLOW_TIMER.with(slint::Timer::stop);
        s.cancel_jobs();
        return;
    }
    let n = s.ticks.get() + 1;
    s.ticks.set(n);
    match s.stage.get() {
        Stage::GettingCode => tick_getting_code(app, router, s),
        Stage::SigningIn => tick_signing_in(app, router, s),
        Stage::Status if n % 4 == 0 => {
            let page = status_page_now(app);
            if s.last_status.borrow().as_ref() != Some(&page) {
                if let Some(h) = s.handle.borrow().as_ref() {
                    h.update(page.clone());
                }
                *s.last_status.borrow_mut() = Some(page);
            }
        }
        Stage::Listing => tick_listing(app, router, s),
        Stage::Copying => tick_copying(app, router, s),
        _ => {}
    }
}

// -- Signing in -----------------------------------------------------------------------------------------------------

fn start_sign_in(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    let Some(setup) = endpoints::current() else { return show(app, router, s, Stage::NotSetUp, view::not_set_up_page()) };
    s.cancel_jobs();
    // Before any code is shown: a sign-in must never be stored where other accounts could read it.
    if let Err(why) = tokens::private_check(&app.paths.config_dir) {
        return notice(app, router, s, "Cloud backup can't be used here", vec![why, "Nothing was changed.".into()]);
    }
    let (cfg, net, now) = (setup.oauth(), setup.net.clone(), unix_now());
    *s.start_job.borrow_mut() = Some(Rc::new(Background::spawn(move |_| oauth::start(&net, &cfg, now))));
    show(app, router, s, Stage::GettingCode, view::getting_code_page());
}

fn tick_getting_code(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    let Some(job) = s.start_job.borrow().clone() else { return };
    let Some(result) = job.take() else { return };
    s.start_job.borrow_mut().take();
    match result {
        Ok(Ok(code)) => show_code(app, router, s, code),
        Ok(Err(e)) => notice_for(app, router, s, "Could not start the sign-in", &e),
        Err(panic) => notice(app, router, s, "Could not start the sign-in", vec![cloud::log::scrub(&panic)]),
    }
}

fn code_page(s: &Session, code: &DeviceCode, now: u64) -> Page<Act> {
    let minutes = (code.expires_at.saturating_sub(now) + 59) / 60;
    let desktop = !crate::fork::steam_queue::game_mode_from_env();
    view::signing_in_page(SERVICE, &code.user_code, QrMatrix::encode(&code.verification_url), minutes.max(1), desktop, &s.waiting.borrow())
}

fn show_code(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, code: DeviceCode) {
    let now = unix_now();
    s.interval.set(code.interval);
    s.next_poll.set(now + code.interval);
    let page = code_page(s, &code, now);
    *s.code.borrow_mut() = Some(code);
    show(app, router, s, Stage::SigningIn, page);
    *s.waiting.borrow_mut() = "Waiting for you to finish on your phone.".to_string();
}

fn tick_signing_in(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    let Some(code) = s.code.borrow().clone() else { return };
    let now = unix_now();
    let pending = s.poll_job.borrow().clone();
    if let Some(job) = pending {
        let Some(result) = job.take() else { return };
        s.poll_job.borrow_mut().take();
        return on_poll(app, router, s, result, now);
    }
    if now >= code.expires_at {
        return on_poll(app, router, s, Ok(Poll::Expired), now);
    }
    // The minutes left on the page change slowly: redraw about every 15 seconds.
    if s.ticks.get() % 60 == 0 {
        if let Some(h) = s.handle.borrow().as_ref() {
            h.update(code_page(s, &code, now));
        }
    }
    if now >= s.next_poll.get() {
        let Some(setup) = endpoints::current() else { return };
        let (cfg, net) = (setup.oauth(), setup.net.clone());
        *s.poll_job.borrow_mut() = Some(Rc::new(Background::spawn(move |_| oauth::poll(&net, &cfg, &code, unix_now()))));
    }
}

fn on_poll(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, result: Result<Poll, String>, now: u64) {
    match result {
        Ok(Poll::Pending) => s.next_poll.set(now + s.interval.get()),
        Ok(Poll::SlowDown) => {
            s.interval.set(oauth::next_interval(s.interval.get(), &Poll::SlowDown));
            s.next_poll.set(now + s.interval.get());
        }
        Ok(Poll::Approved(t)) => finish_sign_in(app, router, s, t, now),
        Ok(Poll::Denied) => notice(app, router, s, "Sign-in was cancelled", vec!["You said no on your phone, so nothing was connected. Nothing was changed.".into()]),
        Ok(Poll::Expired) => {
            *s.waiting.borrow_mut() = "That code ran out. Here is a new one.".to_string();
            start_sign_in(app, router, s);
        }
        Ok(Poll::Failed(e)) if e.kind.retries_by_itself() => {
            // Keep waiting until the code runs out; say what is wrong meanwhile.
            s.next_poll.set(now + s.interval.get().max(10));
            *s.waiting.borrow_mut() = view::problem_text(SERVICE, e.kind);
            if let (Some(h), Some(code)) = (s.handle.borrow().as_ref(), s.code.borrow().as_ref()) {
                h.update(code_page(s, code, now));
            }
        }
        Ok(Poll::Failed(e)) => notice_for(app, router, s, "The sign-in did not work", &e),
        Err(panic) => notice(app, router, s, "The sign-in did not work", vec![cloud::log::scrub(&panic)]),
    }
}

fn finish_sign_in(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, tokens: oauth::Tokens, now: u64) {
    let account = tokens.email.as_deref().map_or_else(|| "your account".to_string(), oauth::mask_email);
    match store::sign_in(&tokens_path(&app.paths.config_dir), &record_path(&app.paths.config_dir), SERVICE, tokens, now) {
        Ok(()) => show(app, router, s, Stage::Signed, view::signed_in_page(SERVICE, &account)),
        Err(e) => {
            cloud::log::note(format!("could not save the sign-in ({})", e.kind()));
            notice(app, router, s, "Could not save the sign-in", vec![format!("This Deck could not store it ({}). You are not signed in; nothing was copied.", e.kind())]);
        }
    }
}

/// A revoked sign-in the person does not want to renew: the remembered account goes, back to the first page.
fn forget_account(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    cancel_pass();
    RETRY_TIMER.with(slint::Timer::stop);
    match store::sign_out(&tokens_path(&app.paths.config_dir), &record_path(&app.paths.config_dir), SERVICE) {
        Ok(()) => show_home(app, router, s),
        Err(e) => notice(app, router, s, "Could not forget the account", vec![format!("This Deck could not remove it ({}).", e.kind())]),
    }
}

fn sign_out(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    cancel_pass();
    RETRY_TIMER.with(slint::Timer::stop);
    match store::sign_out(&tokens_path(&app.paths.config_dir), &record_path(&app.paths.config_dir), SERVICE) {
        Ok(()) => notice(app, router, s, "Signed out", vec![format!("Signed out of {}. Backups already there stay there, and this Deck has stopped copying.", SERVICE.name())]),
        Err(e) => notice(app, router, s, "Could not sign out", vec![format!("This Deck could not remove the sign-in ({}). You are still signed in.", e.kind())]),
    }
}

// -- Getting backups back -------------------------------------------------------------------------------------------

fn start_listing(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    let Some(setup) = endpoints::current() else { return show(app, router, s, Stage::NotSetUp, view::not_set_up_page()) };
    s.cancel_jobs();
    let tokens_file = tokens_path(&app.paths.config_dir);
    *s.list_job.borrow_mut() = Some(Rc::new(Background::spawn(move |_| endpoints::service(&setup, &tokens_file, SERVICE).list_all())));
    s.shown.set(LIST_SHOWN);
    show(app, router, s, Stage::Listing, view::listing_page(SERVICE));
}

fn tick_listing(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    let Some(job) = s.list_job.borrow().clone() else { return };
    let Some(result) = job.take() else { return };
    s.list_job.borrow_mut().take();
    match result {
        Ok(Ok(files)) => {
            let mut files: Vec<RemoteFile> = files.into_iter().filter(|f| Place::parse(&f.path).is_some()).collect();
            view::newest_first(&mut files);
            *s.remote.borrow_mut() = files;
            show_list(app, router, s);
        }
        Ok(Err(e)) => notice_for(app, router, s, "Could not look at your backups", &e),
        Err(panic) => notice(app, router, s, "Could not look at your backups", vec![cloud::log::scrub(&panic)]),
    }
}

fn port_name(app: &AppState, folder: &str) -> Option<String> {
    app.catalog.borrow().iter().find(|p| p.folder == folder).map(|p| p.display_name().to_string())
}

fn label_of(app: &AppState, s: &Session, file: &RemoteFile) -> String {
    let Some(place) = Place::parse(&file.path) else { return file.path.clone() };
    let many_decks = s.remote.borrow().iter().map(|f| f.device.as_str()).collect::<std::collections::BTreeSet<_>>().len() > 1;
    view::entry_label(file, &place, place.folder().and_then(|f| port_name(app, f)).as_deref(), many_decks, utc_offset())
}

fn show_list(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    let remote = s.remote.borrow().clone();
    let shown = s.shown.get().min(remote.len());
    let rows: Vec<(String, String)> = remote.iter().take(shown).map(|f| (label_of(app, s, f), human_size(f.size))).collect();
    let missing = restore::missing_locally(&remote, &app.paths.saves_backup_dir);
    let copy_all = (!missing.is_empty()).then(|| (missing.len(), missing.iter().map(|f| f.size).sum()));
    // "Copy all" is the recommended choice only on a Deck with no backups of its own (a new Deck).
    let scan = cloud::local::scan(&app.paths.saves_backup_dir, unix_now(), 0, &|_, _, _| false);
    let new_deck = scan.zips.is_empty() && scan.deferred == 0 && scan.broken == 0;
    show(app, router, s, Stage::List, view::restore_list_page(SERVICE, &rows, copy_all, new_deck, remote.len() > shown));
}

fn start_copy(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, files: Vec<RemoteFile>, label: String) {
    if files.is_empty() {
        return notice(app, router, s, "Nothing to copy", vec!["Everything in the cloud is already on this Deck.".into()]);
    }
    let Some(setup) = endpoints::current() else { return show(app, router, s, Stage::NotSetUp, view::not_set_up_page()) };
    let (config_dir, saves) = (app.paths.config_dir.clone(), app.paths.saves_backup_dir.clone());
    let Some(signin) = store::signin_name(&tokens_path(&config_dir), SERVICE) else {
        return notice_for(app, router, s, "Could not copy that backup", &ServiceError::new(ErrorKind::SignedOut, "not signed in"));
    };
    *s.copying_label.borrow_mut() = label.clone();
    *s.copied_folder.borrow_mut() = None;
    s.cancelling.set(false);
    s.copying_total.set(files.len());
    *s.copy_job.borrow_mut() = Some(Rc::new(Background::spawn(move |ctl| copy_job(&setup, &config_dir, &saves, &signin, files, ctl))));
    show(app, router, s, Stage::Copying, view::copying_back_page(&label, ""));
}

fn copy_job(setup: &Setup, config_dir: &Path, saves: &Path, signin: &str, files: Vec<RemoteFile>, ctl: &Ctl) -> CopyOutcome {
    let (tokens_file, record_file) = (tokens_path(config_dir), record_path(config_dir));
    let mut service = endpoints::service(setup, &tokens_file, SERVICE);
    let total = files.len();
    let mut results = Vec::new();
    for (i, file) in files.into_iter().enumerate() {
        if ctl.cancelled() {
            break;
        }
        ctl.set_progress(if total > 1 { format!("{} of {total}", i + 1) } else { String::new() });
        // What the download learns goes into the record as entries added to it (under the store's lock), never as the
        // whole part: an upload pass may be writing the same file.
        let mut added = cloud::record::ServiceRecord::default();
        ctl.set_fraction(i as f32 / total.max(1) as f32);
        let result = restore::download_one(service.as_mut(), &file, saves, &mut added, unix_now(), &mut |done, all| {
            ctl.set_fraction((i as f32 + if all == 0 { 1.0 } else { done as f32 / all as f32 }) / total.max(1) as f32);
        });
        let stop = matches!(&result, Err(DownloadError::Service(e)) if !e.kind.retries_by_itself() || e.kind == ErrorKind::Offline);
        if result.is_ok() {
            if let Err(e) = store::add_uploaded(&record_file, &tokens_file, SERVICE, signin, added.uploaded) {
                cloud::log::note(format!("could not note the copied backup in the record ({})", e.kind()));
            }
        }
        results.push((file, result));
        if stop {
            break;
        }
    }
    CopyOutcome { results }
}

fn tick_copying(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    let Some(job) = s.copy_job.borrow().clone() else { return };
    let Some(result) = job.take() else {
        let progress = if s.cancelling.get() { "Stopping after this file...".to_string() } else { job.progress() };
        if s.ticks.get() % 4 == 0 {
            if let Some(h) = s.handle.borrow().as_ref() {
                h.update(view::copying_back_page(&s.copying_label.borrow(), &progress).with_progress(job.fraction()));
            }
        }
        return;
    };
    s.copy_job.borrow_mut().take();
    if s.cancelling.replace(false) {
        let done = result.as_ref().map_or(0, |o| o.results.iter().filter(|(_, r)| r.is_ok()).count());
        let total = s.copying_total.get();
        return notice(app, router, s, "The copy was stopped", vec![format!("{done} of {total} copied. Nothing on this Deck was replaced."), "Press Back to return to Cloud backup.".into()]);
    }
    match result {
        Ok(outcome) => on_copied(app, router, s, outcome),
        Err(panic) => notice(app, router, s, "The copy stopped", vec![cloud::log::scrub(&panic), "Nothing on this Deck was replaced.".into()]),
    }
}

fn on_copied(app: &Rc<AppState>, router: &Router, s: &Rc<Session>, outcome: CopyOutcome) {
    let label = s.copying_label.borrow().clone();
    let first_error = outcome.results.iter().find_map(|(_, r)| r.as_ref().err());
    if !s.copying_all.get() && outcome.results.len() == 1 {
        return match &outcome.results[0].1 {
            Ok(done) => {
                let expected = done.place.rel();
                let kept_both = !done.already_here && done.rel != expected;
                let folder = done.place.folder().map(str::to_string);
                let name = folder.as_deref().and_then(|f| port_name(app, f));
                *s.copied_folder.borrow_mut() = folder;
                show(app, router, s, Stage::Copied, view::copied_page(&label, name.as_deref(), kept_both));
            }
            Err(DownloadError::Service(e)) => notice_for(app, router, s, "Could not copy that backup", e),
            Err(DownloadError::Refused(text)) => notice(app, router, s, "That backup was not copied", vec![text.clone(), "Nothing on this Deck was changed.".into()]),
        };
    }
    let copied = outcome.results.iter().filter(|(_, r)| r.as_ref().is_ok_and(|d| !d.already_here)).count();
    let already = outcome.results.iter().filter(|(_, r)| r.as_ref().is_ok_and(|d| d.already_here)).count();
    let failed = outcome.results.iter().filter(|(_, r)| r.is_err()).count();
    let mut page = view::copied_all_page(copied, already, failed);
    if let Some(e) = first_error {
        page.paragraphs.push(e.text());
    }
    show(app, router, s, Stage::Copied, page);
    // The summary's own button is Done, which returns to the status page.
    *s.copied_folder.borrow_mut() = None;
}

fn hand_over_to_restore(app: &Rc<AppState>, router: &Router, s: &Rc<Session>) {
    let folder = s.copied_folder.borrow().clone();
    let port: Option<Port> = folder.and_then(|f| app.catalog.borrow().iter().find(|p| p.folder == f && !p.is_android_only()).cloned());
    FLOW_TIMER.with(slint::Timer::stop);
    s.cancel_jobs();
    match port {
        Some(port) => super::save_restore_screen::show_port(app, router, &port, Rc::new(|app, router| open_from_settings(app, router))),
        None => super::save_restore_screen::open_from_settings(app, router),
    }
}
