//! The import's one question about all its old copies, instead of one per port: "Remove the 4 old copies?"
//! (`crate::fork::import::old_batch` says what the page says). Settings > Ports folder keeps its per-copy
//! questions; only Import ports comes here.
//!
//! Every old copy is checked first (`removal_check` and its readiness, on the worker thread); each port gets one
//! status line and only the Ready ones that passed are offered. Keep is the default and is what B does. Remove
//! them removes those one after another through the same `old_copies::remove` (which checks again, readiness
//! included, and zips the saves first), and says what happened to each. "It starts fine..." asks about each
//! port that waits for a first start (one card each), records the yes, and shows the page again.
//!
//! `note_launch_end` is the app's own Play: a run of 30 s or more from the new copy counts as a start.

use super::*;
use crate::fork::art::thumbs::ArtRef;
use crate::fork::import::old_batch::{confirm_page, working_page, Batch, BatchAction, Checked, ConfirmAction};
pub(crate) use crate::fork::import::old_batch::Origin;
use crate::app::page_dialog::{show_page, show_page_handle, Handlers, PageHandle};

/// Said when the old copies could not be checked at all (the setup wizard shows that text once, on its last page).
pub(crate) const CHECK_FAILED: &str = "could not be checked";

/// Gets the closing text when the question has been dealt with, or `None` when it was abandoned because
/// another dialog took the screen while the copies were being checked (they stay listed under Old copies).
type Then = Rc<dyn Fn(&Rc<AppState>, &Router, Option<String>)>;

const CHECKING: &str = "Checking the old copies";
const CHECKING_SENTENCE: &str = "Each one is looked at before anything can be removed. Nothing is changed.";
const REMOVING: &str = "Removing the old copies";
const REMOVING_SENTENCE: &str = "Saves are zipped to a backup first. Only the ready ones are removed.";

/// The check or the removal as a page in the guided window, so the old copies page that follows takes the same window
/// back (one window, no flash of the main list). Nothing to choose; B and the X do nothing while it runs, as with the
/// progress dialog this replaces.
fn working(origin: Origin, title: &str, sentence: &str, progress: &str) -> crate::fork::page::Page<BatchAction> {
    // A sliding bar: the check and the removal say which copy they are on, not how far in bytes.
    let page = working_page(origin, title, sentence, progress).with_progress(None);
    if crate::app::setup_wizard_screen::active() { page.in_wizard(4, crate::fork::setup_wizard::flow::Step::COUNTED) } else { page }
}

fn show_working(app: &Rc<AppState>, router: &Router, origin: Origin, title: &str, sentence: &str) -> Option<PageHandle<BatchAction>> {
    let nothing: Rc<dyn Fn(&Rc<AppState>, &Router)> = Rc::new(|_, _| {});
    let handlers = Handlers { on_action: Rc::new(|_, _, _: BatchAction| {}), on_back: nothing.clone(), on_exit: nothing, on_input: None };
    show_page_handle(app, router, working(origin, title, sentence, ""), handlers)
}

fn art_of(app: &AppState, key: &str) -> Option<ArtRef> {
    app.catalog.borrow().iter().find(|p| p.key() == key).map(ArtRef::of_port)
}

/// Asks about `copies` on one page, then `then(Some(summary + what happened))`.
pub(crate) fn prompt_old_copies_batch_then(app: &Rc<AppState>, router: &Router, copies: VecDeque<OldCopy>, summary: String, origin: Origin, then: impl Fn(&Rc<AppState>, &Router, Option<String>) + 'static) {
    let then: Then = Rc::new(then);
    if copies.is_empty() {
        return then(app, router, Some(summary));
    }
    // One check at a time: a second request (another screen) is not allowed to replace the first.
    if CHECK_RUNNING.with(Cell::get) {
        let text = "Another old copy is being checked right now. These old copies stay listed under Settings > Ports folder > Old copies, to remove later.";
        return then(app, router, Some(if summary.is_empty() { text.to_string() } else { format!("{summary}\n\n{text}") }));
    }
    let parts = RemovalParts::of(app);
    let items: Vec<OldCopy> = copies.into_iter().collect();
    let total = items.len();
    let job = Rc::new(Background::spawn(move |ctl| {
        items
            .into_iter()
            .enumerate()
            .map(|(i, copy)| {
                ctl.set_progress(format!("Checking the old copy of {} ({} of {total})...", copy.name, i + 1));
                let check = parts.removal_only(&copy);
                let ready = parts.readiness(&copy);
                (copy, check, ready)
            })
            .collect::<Vec<_>>()
    }));
    // Only this page is closed when the check ends, never something that replaced it (an install's progress dialog).
    let own = show_working(app, router, origin, CHECKING, CHECKING_SENTENCE);
    CHECK_RUNNING.with(|c| c.set(true));
    let (app2, router2) = (app.clone(), router.clone());
    let pending = RefCell::new(Some((summary, then)));
    CHECK_TIMER.with(|timer| {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(150), move || {
            let Some(result) = job.take() else {
                let text = job.progress();
                if let (false, Some(own)) = (text.is_empty(), &own) {
                    own.update(working(origin, CHECKING, CHECKING_SENTENCE, &text));
                }
                return;
            };
            CHECK_TIMER.with(|t| t.stop());
            CHECK_RUNNING.with(|c| c.set(false));
            let Some((summary, then)) = pending.borrow_mut().take() else { return };
            let showing_ours = own.as_ref().is_some_and(PageHandle::is_open);
            if showing_ours {
                close_current_dialog(&app2, &router2);
            } else if super::super::dialogs::dialog_is_open(&app2) {
                // Something else is on screen now: leave it; the old copies stay listed under Old copies.
                eprintln!("ports folder: old-copy question skipped, another dialog is open");
                return then(&app2, &router2, None);
            }
            match result {
                Ok(checked) => {
                    let items = checked.into_iter().map(|(copy, check, ready)| Checked { art: art_of(&app2, &copy.port_key), copy, check, ready }).collect();
                    ask(&app2, &router2, Rc::new(Batch::from(items, origin)), summary, then);
                }
                Err(e) => then(&app2, &router2, Some(format!("{summary}\n\nThe old copies {CHECK_FAILED} ({e}), so they stay. Remove them later in Settings, Ports folder, Old copies.").trim_start().to_string())),
            }
        });
    });
}

fn with_summary(summary: &str, text: &str) -> String {
    if summary.is_empty() { text.to_string() } else { format!("{summary}\n\n{text}") }
}

/// The page itself. Keep, OK, B and the X all keep the old copies; only Remove them removes.
/// Steam changes waiting now, for the page's "Close Steam and update shortcuts".
fn steam_waiting(app: &AppState) -> crate::fork::import::cards::SteamWaiting {
    use crate::fork::import::cards::SteamWaiting;
    match (crate::fork::steam_queue::load(&crate::fork::steam_queue::queue_path(&app.paths.config_dir)).queue.len(), crate::fork::steam_queue::game_mode_from_env()) {
        (0, _) => SteamWaiting::Nothing,
        (n, true) => SteamWaiting::GameMode(n),
        (n, false) => SteamWaiting::Desktop(n),
    }
}

/// What a Settings visit to the page ended with, on a page with one OK; `then` runs after it (B too).
pub(crate) fn result_page(app: &Rc<AppState>, router: &Router, text: &str, origin: Origin, then: Rc<dyn Fn(&Rc<AppState>, &Router)>) {
    let mut page = crate::fork::page::Page::<BatchAction> { window_title: format!("{}: done", origin.window_title()), large: true, header: origin.header().to_string(), title: "Old copies".to_string(), ..Default::default() };
    page.paragraphs = text.lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect();
    page.options = vec![crate::fork::page::PageOption { title: "OK".to_string(), detail: String::new(), choice: None, enabled: true, recommended: true, action: BatchAction::Ok }];
    page.initial = crate::fork::page::Focus::Option(0);
    let page = page.b_does(&BatchAction::Ok);
    let go = {
        let then = then.clone();
        Rc::new(move |app: &Rc<AppState>, router: &Router| {
            close_current_dialog(app, router);
            then(app, router);
        })
    };
    let on_action = {
        let go = go.clone();
        Rc::new(move |app: &Rc<AppState>, router: &Router, _: BatchAction| go(app, router))
    };
    let handlers = Handlers { on_action, on_back: go.clone(), on_exit: go.clone(), on_input: None };
    if !show_page(app, router, page, handlers) {
        go(app, router);
    }
}

fn ask(app: &Rc<AppState>, router: &Router, batch: Rc<Batch>, summary: String, then: Then) {
    let batch = Rc::new(Batch::from(batch.items.clone(), batch.origin).with_steam(steam_waiting(app)));
    // Whichever way the page is left, the closing step runs once.
    show_ask(app, router, batch, Rc::new(summary), then, Rc::new(Cell::new(false)));
}

fn show_ask(app: &Rc<AppState>, router: &Router, batch: Rc<Batch>, summary: Rc<String>, then: Then, done: Rc<Cell<bool>>) {
    let keep: Rc<dyn Fn(&Rc<AppState>, &Router)> = {
        let (batch, summary, then, done) = (batch.clone(), summary.clone(), then.clone(), done.clone());
        Rc::new(move |app, router| {
            if done.replace(true) {
                return;
            }
            close_current_dialog(app, router);
            then(app, router, Some(with_summary(&summary, &batch.kept_text())));
        })
    };
    let handlers = Handlers {
        on_action: {
            let (batch, summary, then, done, keep) = (batch.clone(), summary.clone(), then.clone(), done.clone(), keep.clone());
            Rc::new(move |app: &Rc<AppState>, router: &Router, action: BatchAction| match action {
                BatchAction::Keep | BatchAction::Ok => keep(app, router),
                BatchAction::Details => {
                    let text = format!("{}\n", batch.details_text(&super::home_dir().unwrap_or_default(), &app.paths.saves_backup_dir));
                    let again: crate::app::import_screen::Step = {
                        let (batch, summary, then, done) = (batch.clone(), summary.clone(), then.clone(), done.clone());
                        Rc::new(move |app, router| show_ask(app, router, batch.clone(), summary.clone(), then.clone(), done.clone()))
                    };
                    let on_ok = again.clone();
                    crate::app::import_screen::confirm_back(app, router, "Old copies", &text, "OK", false, move |app, router| on_ok(app, router), again);
                }
                BatchAction::Remove => {
                    if done.replace(true) {
                        return;
                    }
                    close_current_dialog(app, router);
                    remove_all(app, router, batch.clone(), summary.clone(), then.clone());
                }
                BatchAction::UpdateSteam => {
                    if done.replace(true) {
                        return;
                    }
                    // The shared flow, then the page again with what Steam says now.
                    let (batch, summary, then) = (batch.clone(), summary.clone(), then.clone());
                    let back: crate::app::steam_close_screen::Then = Rc::new(move |app, router, _| {
                        let copies: VecDeque<OldCopy> = batch.items.iter().map(|c| c.copy.clone()).collect();
                        let then = then.clone();
                        prompt_old_copies_batch_then(app, router, copies, summary.as_ref().clone(), batch.origin, move |app, router, text| then(app, router, text));
                    });
                    crate::app::steam_close_screen::open(app, router, crate::app::steam_close_screen::Opts::default(), back);
                }
                BatchAction::Confirm => {
                    if done.replace(true) {
                        return;
                    }
                    let asks: Vec<Checked> = batch.to_confirm().into_iter().cloned().collect();
                    confirm_cards(app, router, Rc::new(asks), 0, batch.clone(), summary.clone(), then.clone());
                }
            })
        },
        on_back: keep.clone(),
        on_exit: keep,
        on_input: None,
    };
    // In the setup wizard this is its old copy question, between Steam and the last page.
    // B keeps the old copies (OK on the page that only says they stay).
    let page = batch.page().b_does(&BatchAction::Ok).b_does(&BatchAction::Keep);
    let page = if crate::app::setup_wizard_screen::active() { page.in_wizard(4, crate::fork::setup_wizard::flow::Step::COUNTED) } else { page };
    if !show_page(app, router, page, handlers) {
        // The page could not be built: nothing is removed, and the flow goes on.
        then(app, router, Some(with_summary(&summary, &batch.kept_text())));
    }
}

/// "Does <port> start from its new place?" for `asks[index]`, then the next; after the last (or B), the page is
/// checked and shown again with what was said. A yes is recorded on the old copy's record
/// (`old_copies::mark_started`); nothing is removed here.
fn confirm_cards(app: &Rc<AppState>, router: &Router, asks: Rc<Vec<Checked>>, index: usize, batch: Rc<Batch>, summary: Rc<String>, then: Then) {
    let again = {
        let (batch, summary, then) = (batch.clone(), summary.clone(), then.clone());
        move |app: &Rc<AppState>, router: &Router| {
            close_current_dialog(app, router);
            let copies: VecDeque<OldCopy> = batch.items.iter().map(|c| c.copy.clone()).collect();
            let then = then.clone();
            let origin = batch.origin;
            prompt_old_copies_batch_then(app, router, copies, summary.as_ref().clone(), origin, move |app, router, text| then(app, router, text));
        }
    };
    let Some(item) = asks.get(index) else { return again(app, router) };
    let page = confirm_page(item, index, asks.len(), batch.origin);
    let page = if crate::app::setup_wizard_screen::active() { page.in_wizard(4, crate::fork::setup_wizard::flow::Step::COUNTED) } else { page };
    let back: Rc<dyn Fn(&Rc<AppState>, &Router)> = Rc::new(again.clone());
    let handlers = Handlers {
        on_action: {
            let (asks, again) = (asks.clone(), again.clone());
            Rc::new(move |app: &Rc<AppState>, router: &Router, action: ConfirmAction| {
                if action == ConfirmAction::Yes {
                    let at = chrono::Utc::now().timestamp();
                    match old_copies::mark_started(&app.paths.config_dir, &asks[index].copy.old_path, old_copies::StartedBy::User, at) {
                        Ok(_) => eprintln!("old copies: {} confirmed to start from its new place", asks[index].copy.name),
                        Err(e) => eprintln!("old copies: could not record the confirm for {}: {e}", asks[index].copy.name),
                    }
                }
                if index + 1 < asks.len() {
                    confirm_cards(app, router, asks.clone(), index + 1, batch.clone(), summary.clone(), then.clone());
                } else {
                    again(app, router);
                }
            })
        },
        // B (and the X) is "Not yet" for this one and the rest: the page comes back as it was, with any yes said.
        on_back: back.clone(),
        on_exit: back,
        on_input: None,
    };
    if !show_page(app, router, page, handlers) {
        again(app, router);
    }
}

/// The app's own launch of `key` ended (upstream's playtime code calls this before it forgets the start time):
/// a run of at least `old_copies::MIN_RUN_SECONDS` from the port's folder in the ports folder in use counts as
/// "started from its new place" for that port's old copies.
pub(crate) fn note_launch_end(app: &AppState, key: &str) {
    let Some(started) = app.install_runtime.launch_started_at.borrow().get(key).copied() else { return };
    let Some(folder) = app.catalog.borrow().iter().find(|p| p.key() == key).map(|p| p.folder.clone()) else { return };
    let Ok(dir) = crate::core::path_safety::safe_join(&app.paths.library_dir(), &folder) else { return };
    let ran = started.elapsed().as_secs();
    match old_copies::note_launch(&app.paths.config_dir, key, &dir, ran, chrono::Utc::now().timestamp()) {
        Ok(n) if n > 0 => eprintln!("old copies: {key} ran {ran} s from its new place; {n} old copy record(s) noted"),
        Ok(_) => {}
        Err(e) => eprintln!("old copies: could not note the run of {key}: {e}"),
    }
}

/// Removes the copies that passed their check, one after another, behind a progress dialog.
fn remove_all(app: &Rc<AppState>, router: &Router, batch: Rc<Batch>, summary: Rc<String>, then: Then) {
    let wanted: Vec<(OldCopy, Option<Port>)> = batch.removable().into_iter().map(|c| (c.copy.clone(), find_port(app, &c.copy.port_key))).collect();
    let parts = RemovalParts::of(app);
    let saves_dir = app.paths.saves_backup_dir.clone();
    let origin = batch.origin;
    let own = show_working(app, router, origin, REMOVING, REMOVING_SENTENCE);
    JOB_BUSY.with(|b| b.set(true));
    let total = wanted.len();
    let job = Rc::new(Background::spawn(move |ctl| {
        let mut lines: Vec<String> = Vec::new();
        for (i, (entry, port)) in wanted.into_iter().enumerate() {
            ctl.set_progress(format!("Removing the old copy of {} ({} of {total})...", entry.name, i + 1));
            let backup = |e: &OldCopy| -> Result<Option<PathBuf>, String> {
                match &port {
                    Some(port) => crate::core::save_backup::backup_before_update(port, &saves_dir, &e.old_path, &crate::fork::store::backup_stamp(), &mut |_| {}),
                    None => Err("the port is not in the catalog any more".into()),
                }
            };
            let saves = |e: &OldCopy, dir: &Path| parts.save_folders(e, dir);
            let ctx = RemovalCtx { home: &parts.home, config_dir: &parts.config_dir, library_dir: &parts.library, active_paths: &parts.active, running: &running_in, save_folders: &saves };
            lines.push(match old_copies::remove(&entry, &ctx, &backup) {
                Ok(removed) => {
                    let zip = if removed.saves_zip.is_some() { " Its saves were zipped to a backup first." } else { "" };
                    format!("Removed the old copy of {} ({} freed).{zip}", entry.name, human_bytes(removed.bytes))
                }
                Err(e) => format!("The old copy of {} was kept: {e}", entry.name),
            });
        }
        lines
    }));
    let (app, router) = (app.clone(), router.clone());
    JOB_TIMER.with(|timer| {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
            let Some(result) = job.take() else {
                let text = job.progress();
                if let (false, Some(own)) = (text.is_empty(), &own) {
                    own.update(working(origin, REMOVING, REMOVING_SENTENCE, &text));
                }
                return;
            };
            JOB_TIMER.with(|t| t.stop());
            // Only this page is closed (the result page that follows takes the window back); something that replaced
            // it stays.
            if own.as_ref().is_some_and(PageHandle::is_open) {
                close_current_dialog(&app, &router);
            }
            if job_finished() {
                return;
            }
            let mut lines = match result {
                Ok(lines) => lines,
                Err(e) => vec![format!("The old copies could not be removed ({e}). Nothing else was touched.")],
            };
            let kept = Batch::new(batch.kept().into_iter().cloned().collect()).kept_text();
            if !kept.is_empty() {
                lines.push(kept);
            }
            then(&app, &router, Some(with_summary(&summary, &lines.join("\n"))));
        });
    });
}
