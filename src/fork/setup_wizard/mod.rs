//! The first-run setup wizard: where ports go, ports you already have (Import ports), game files folder,
//! Steam, the old copies this run kept, and a summary. Settings > Setup wizard... runs it again any time.
//!
//! - `flow`: the state machine and the `Adapters` it calls (everything outside the wizard).
//! - `text`: what each view says.
//! - `ports_folder`: the quick choices and the checks for the ports folder.
//! - this file: `setup_done` in `fork-settings.json`, and whether the wizard opens at start.
//!
//! The screen that draws it is `src/app/setup_wizard_screen.rs`. `setup_done` is written only when the
//! user reaches the last page and presses Finish, or chooses Skip setup and confirms, so quitting
//! half-way brings the wizard back at the next start. Existing installs have no `setup_done`, so they
//! get the wizard once too.

pub mod flow;
pub mod places;
pub mod text;

use crate::fork::settings::ForkSettings;
use serde_json::Value;
use std::io;
use std::path::Path;

pub const KEY_SETUP_DONE: &str = "setup_done";

/// What the settings file says about setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupState {
    /// Never finished (no file, or no `setup_done`).
    NotDone,
    Done,
    /// The file exists but is not a JSON object, so setup could never be saved as done. The wizard does
    /// not open by itself then (it would come back at every start); Settings still opens it.
    Unreadable,
}

pub fn setup_state(settings_path: &Path) -> SetupState {
    match std::fs::read_to_string(settings_path) {
        Ok(text) => match serde_json::from_str::<Value>(text.strip_prefix('\u{FEFF}').unwrap_or(&text)) {
            Ok(Value::Object(map)) if map.get(KEY_SETUP_DONE).and_then(Value::as_bool) == Some(true) => SetupState::Done,
            Ok(Value::Object(_)) => SetupState::NotDone,
            _ => SetupState::Unreadable,
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => SetupState::NotDone,
        Err(_) => SetupState::Unreadable,
    }
}

/// True when the wizard should open by itself at start.
pub fn opens_at_start(settings_path: &Path) -> bool {
    setup_state(settings_path) == SetupState::NotDone
}

pub fn mark_done(settings_path: &Path) -> io::Result<()> {
    ForkSettings::load(settings_path).set(KEY_SETUP_DONE, Value::Bool(true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn path(dir: &tempfile::TempDir) -> std::path::PathBuf {
        dir.path().join("fork-settings.json")
    }

    #[test]
    fn a_new_install_and_an_old_one_without_the_key_both_open_the_wizard() {
        let d = tempfile::tempdir().unwrap();
        let p = path(&d);
        assert!(opens_at_start(&p), "no file at all");
        fs::write(&p, r#"{"steam_account_id": 1234, "adopt_folders": []}"#).unwrap();
        assert!(opens_at_start(&p), "an existing install's settings, no setup_done");
        fs::write(&p, r#"{"setup_done": false}"#).unwrap();
        assert!(opens_at_start(&p));
        fs::write(&p, r#"{"setup_done": "yes"}"#).unwrap();
        assert!(opens_at_start(&p), "only a real true counts");
    }

    #[test]
    fn setup_done_hides_the_wizard_at_start() {
        let d = tempfile::tempdir().unwrap();
        let p = path(&d);
        fs::write(&p, "\u{FEFF}{\"setup_done\": true}").unwrap();
        assert_eq!(setup_state(&p), SetupState::Done);
        assert!(!opens_at_start(&p));
    }

    #[test]
    fn marking_done_keeps_the_other_settings_and_is_remembered() {
        let d = tempfile::tempdir().unwrap();
        let p = path(&d);
        fs::write(&p, r#"{"steam_account_id": 1234, "rom_prompted_ports": ["a/b"]}"#).unwrap();
        mark_done(&p).unwrap();
        assert_eq!(setup_state(&p), SetupState::Done);
        let text = fs::read_to_string(&p).unwrap();
        assert!(text.contains("1234") && text.contains("a/b"), "{text}");
        mark_done(&p).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap().matches("setup_done").count(), 1);
    }

    #[test]
    fn marking_done_creates_the_file_when_there_is_none() {
        let d = tempfile::tempdir().unwrap();
        let p = path(&d);
        mark_done(&p).unwrap();
        assert!(!opens_at_start(&p));
    }

    #[test]
    fn a_settings_file_that_is_not_an_object_never_opens_the_wizard_by_itself_and_is_not_overwritten() {
        let d = tempfile::tempdir().unwrap();
        let p = path(&d);
        fs::write(&p, "[1, 2").unwrap();
        assert_eq!(setup_state(&p), SetupState::Unreadable);
        assert!(!opens_at_start(&p));
        assert!(mark_done(&p).is_err());
        assert_eq!(fs::read_to_string(&p).unwrap(), "[1, 2");
    }
}
