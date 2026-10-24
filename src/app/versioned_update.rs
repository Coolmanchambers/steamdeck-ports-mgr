//! Fork: finishing an installed update whose release names its files with the version (see
//! `fork::versioned`). Called on the UI thread when an install is done.
//!
//! When Play started the update, the game launches at once, so the summary is saved to
//! `update-notes.json` and shown once no game runs and the app is in view: after the game exits, or at
//! the next start. It comes before the "Steam changes queued" offer, which waits for no dialog.

use super::dialogs::{dialog_is_open, open_message_dialog};
use super::state::AppState;
use crate::fork::port_mover::steam::SteamCtx;
use crate::fork::settings::{ForkSettings, FILE_NAME};
use crate::fork::store::SystemSteamCheck;
use crate::fork::versioned::{self, FinishInput};
use crate::ui::gamepad_router::GamepadRouter;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

const NOTES_FILE: &str = "update-notes.json";

fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"].iter().find_map(|v| std::env::var_os(v)).map(PathBuf::from).filter(|p| p.is_absolute())
}

/// Points the launch target and Steam at the new files and moves the old ones to `.previous/`.
/// Returns the text to show now, or `None` when the update superseded nothing. With `launching` (Play
/// started the update) the text is saved for later instead, when it needs attention, and `None` returned.
pub(crate) fn after_install(app: &Rc<AppState>, key: &str, tag: Option<&str>, launching: bool) -> Option<String> {
    let port = app.find_port(key)?;
    let game_dir = crate::core::path_safety::safe_join(&app.paths.library_dir(), &port.folder).ok()?;
    let favorite = app.state.borrow().get(key).and_then(|i| i.favorite_exe.clone());
    let home = home_dir();
    let settings = ForkSettings::load(&app.paths.config_dir.join(FILE_NAME));
    let steam_check = SystemSteamCheck::new();
    let ctx = home.as_ref().map(|home| SteamCtx {
        home,
        config_dir: &app.paths.config_dir,
        settings: &settings,
        steam: &steam_check,
        game_mode: crate::fork::steam_queue::game_mode_from_env(),
        now: SystemTime::now(),
    });
    let outcome = versioned::finish_update(&FinishInput { port: &port, game_dir: &game_dir, favorite: favorite.as_deref(), steam: ctx.as_ref() });
    if let Some(new_favorite) = &outcome.new_favorite {
        app.state.borrow_mut().set_favorite_exe(key, Some(new_favorite.clone()));
    }
    for (m, folder) in &outcome.moved {
        eprintln!("versioned update: {} moved {} to .previous/{folder} (replaced by {})", port.folder, m.old, m.new);
    }
    for name in &outcome.waiting {
        eprintln!("versioned update: {} keeps {name} until the queued Steam change is applied", port.folder);
    }
    for (name, why) in &outcome.kept {
        eprintln!("versioned update: {} kept {name} in place: {why}", port.folder);
    }
    let text = outcome.summary(port.display_name(), &port.folder, tag)?;
    if launching {
        if outcome.needs_attention() {
            match save_note(&app.paths.config_dir, port.display_name(), &text) {
                Ok(()) => notes_changed(),
                Err(e) => eprintln!("versioned update: could not keep the note for later ({e})"),
            }
        }
        return None;
    }
    Some(text)
}

fn notes_path(config_dir: &Path) -> PathBuf {
    config_dir.join(NOTES_FILE)
}

pub(crate) fn load_notes(config_dir: &Path) -> Vec<(String, String)> {
    let Ok(text) = std::fs::read_to_string(notes_path(config_dir)) else { return Vec::new() };
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&text) else { return Vec::new() };
    items
        .iter()
        .filter_map(|v| Some((v.get("port")?.as_str()?.to_string(), v.get("text")?.as_str()?.to_string())))
        .collect()
}

fn write_notes(config_dir: &Path, notes: &[(String, String)]) -> std::io::Result<()> {
    let path = notes_path(config_dir);
    if notes.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let value: Vec<Value> = notes.iter().map(|(port, text)| json!({"port": port, "text": text})).collect();
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&value).unwrap_or_default())?;
    std::fs::rename(&tmp, &path)
}

/// Keeps one note per port (a newer update's note replaces an older one).
pub(crate) fn save_note(config_dir: &Path, port: &str, text: &str) -> std::io::Result<()> {
    let mut notes = load_notes(config_dir);
    notes.retain(|(p, _)| p != port);
    notes.push((port.to_string(), text.to_string()));
    write_notes(config_dir, &notes)
}

/// Takes the oldest saved note off the list.
pub(crate) fn take_note(config_dir: &Path) -> Option<(String, String)> {
    let mut notes = load_notes(config_dir);
    if notes.is_empty() {
        return None;
    }
    let first = notes.remove(0);
    write_notes(config_dir, &notes).ok()?;
    Some(first)
}

thread_local! {
    /// Saved notes are only looked for on disk while this says there may be some.
    static MAY_HAVE_NOTES: RefCell<Option<bool>> = const { RefCell::new(None) };
}

/// Shows one saved update note if the app is free: no dialog, no game running, window in view.
/// Returns whether one was shown.
pub(crate) fn show_saved_note(app: &Rc<AppState>, router: &Rc<RefCell<GamepadRouter>>, app_in_view: bool) -> bool {
    if dialog_is_open(app) || !app.install_runtime.running_processes.borrow().is_empty() || !app_in_view {
        return false;
    }
    let config_dir = app.paths.config_dir.clone();
    let may = MAY_HAVE_NOTES.with(|m| *m.borrow_mut().get_or_insert_with(|| notes_path(&config_dir).exists()));
    if !may {
        return false;
    }
    match take_note(&config_dir) {
        Some((port, text)) => {
            open_message_dialog(app, router, &port, &text);
            true
        }
        None => {
            MAY_HAVE_NOTES.with(|m| *m.borrow_mut() = Some(false));
            false
        }
    }
}

/// A note was just saved: look on disk again.
pub(crate) fn notes_changed() {
    MAY_HAVE_NOTES.with(|m| *m.borrow_mut() = None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_are_kept_one_per_port_and_taken_oldest_first() {
        let dir = std::env::temp_dir().join(format!("dpm_update_notes_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(take_note(&dir), None);
        save_note(&dir, "Dusklight", "first").unwrap();
        save_note(&dir, "Other", "other").unwrap();
        save_note(&dir, "Dusklight", "second").unwrap();
        assert_eq!(take_note(&dir), Some(("Other".into(), "other".into())));
        assert_eq!(take_note(&dir), Some(("Dusklight".into(), "second".into())));
        assert_eq!(take_note(&dir), None);
        assert!(!notes_path(&dir).exists(), "the file goes once the last note is shown");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
