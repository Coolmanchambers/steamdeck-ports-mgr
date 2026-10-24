//! The import's one question about all its old copies, instead of one per port: "Remove the 4 old copies?"
//! (`crate::fork::import::old_batch` says what the page says). Settings > Ports folder keeps its per-copy
//! questions; only Import ports comes here.
//!
//! Every old copy is checked first (`removal_check`, on the worker thread, as the per-copy question does);
//! the ones that fail stay and are listed with the reason. Keep is the default and is what B does. Remove
//! them removes the checked copies one after another through the same `old_copies::remove` (which checks again
//! and zips the saves first), and says what happened to each.

use super::*;
use crate::fork::art::thumbs::ArtRef;
use crate::fork::import::old_batch::{Batch, BatchAction, Checked};
use crate::app::page_dialog::{show_page, Handlers};

/// Gets the closing text when the question has been dealt with, or `None` when it was abandoned because
/// another dialog took the screen while the copies were being checked (they stay listed under Old copies).
type Then = Rc<dyn Fn(&Rc<AppState>, &Router, Option<String>)>;

fn art_of(app: &AppState, key: &str) -> Option<ArtRef> {
    app.catalog.borrow().iter().find(|p| p.key() == key).map(ArtRef::of_port)
}

/// Asks about `copies` on one page, then `then(Some(summary + what happened))`.
pub(crate) fn prompt_old_copies_batch_then(app: &Rc<AppState>, router: &Router, copies: VecDeque<OldCopy>, summary: String, then: impl Fn(&Rc<AppState>, &Router, Option<String>) + 'static) {
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
                let check = parts.check(&copy);
                (copy, check)
            })
            .collect::<Vec<_>>()
    }));
    open_progress_dialog(app, router, "Checking the old copies", "Checking the old copies...");
    // Only this dialog is closed when the check ends, never another progress dialog (an install's).
    let own = match &*app.dialog_nav.dialogs.borrow() {
        DialogSlot::Progress(d) => Some(d.as_weak()),
        _ => None,
    };
    CHECK_RUNNING.with(|c| c.set(true));
    let (app2, router2) = (app.clone(), router.clone());
    let pending = RefCell::new(Some((summary, then)));
    CHECK_TIMER.with(|timer| {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(150), move || {
            let Some(result) = job.take() else {
                let text = job.progress();
                if !text.is_empty() {
                    if let DialogSlot::Progress(d) = &*app2.dialog_nav.dialogs.borrow() {
                        update_progress_dialog(&app2, d, &text);
                    }
                }
                return;
            };
            CHECK_TIMER.with(|t| t.stop());
            CHECK_RUNNING.with(|c| c.set(false));
            let Some((summary, then)) = pending.borrow_mut().take() else { return };
            let ours = own.as_ref().and_then(slint::Weak::upgrade);
            let showing_ours = match (&*app2.dialog_nav.dialogs.borrow(), &ours) {
                (DialogSlot::Progress(d), Some(o)) => std::ptr::eq(d.window(), o.window()),
                _ => false,
            };
            if showing_ours {
                close_current_dialog(&app2, &router2);
            } else if super::super::dialogs::dialog_is_open(&app2) {
                // Something else is on screen now: leave it; the old copies stay listed under Old copies.
                eprintln!("ports folder: old-copy question skipped, another dialog is open");
                return then(&app2, &router2, None);
            }
            match result {
                Ok(checked) => {
                    let items = checked.into_iter().map(|(copy, check)| Checked { art: art_of(&app2, &copy.port_key), copy, check }).collect();
                    ask(&app2, &router2, Rc::new(Batch::new(items)), summary, then);
                }
                Err(e) => then(&app2, &router2, Some(format!("{summary}\n\nThe old copies could not be checked ({e}), so they stay. Remove them later in Settings, Ports folder, Old copies.").trim_start().to_string())),
            }
        });
    });
}

fn with_summary(summary: &str, text: &str) -> String {
    if summary.is_empty() { text.to_string() } else { format!("{summary}\n\n{text}") }
}

/// The page itself. Keep, OK, B and the X all keep the old copies; only Remove them removes.
fn ask(app: &Rc<AppState>, router: &Router, batch: Rc<Batch>, summary: String, then: Then) {
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
            })
        },
        on_back: keep.clone(),
        on_exit: keep,
        on_input: None,
    };
    // In the setup wizard this is its old copy question, between Steam and the last page.
    let page = if crate::app::setup_wizard_screen::active() { batch.page().in_wizard(4, crate::fork::setup_wizard::flow::Step::COUNTED) } else { batch.page() };
    if !show_page(app, router, page, handlers) {
        // The page could not be built: nothing is removed, and the flow goes on.
        then(app, router, Some(with_summary(&summary, &batch.kept_text())));
    }
}

/// Removes the copies that passed their check, one after another, behind a progress dialog.
fn remove_all(app: &Rc<AppState>, router: &Router, batch: Rc<Batch>, summary: Rc<String>, then: Then) {
    let wanted: Vec<(OldCopy, Option<Port>)> = batch.removable().into_iter().map(|c| (c.copy.clone(), find_port(app, &c.copy.port_key))).collect();
    let parts = RemovalParts::of(app);
    let saves_dir = app.paths.saves_backup_dir.clone();
    open_progress_dialog(app, router, "Removing the old copies", "Removing the old copies...");
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
                if let (false, DialogSlot::Progress(d)) = (text.is_empty(), &*app.dialog_nav.dialogs.borrow()) {
                    update_progress_dialog(&app, d, &text);
                }
                return;
            };
            JOB_TIMER.with(|t| t.stop());
            close_current_dialog(&app, &router);
            if job_finished() {
                return;
            }
            let mut lines = match result {
                Ok(lines) => lines,
                Err(e) => vec![format!("The old copies could not be removed ({e}). Nothing else was touched.")],
            };
            lines.extend(batch.kept().iter().map(|c| format!("The old copy of {} stays: {}", c.copy.name, c.check.as_ref().err().cloned().unwrap_or_default())));
            then(&app, &router, Some(with_summary(&summary, &lines.join("\n"))));
        });
    });
}
