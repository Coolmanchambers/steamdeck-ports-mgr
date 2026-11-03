//! The one list left of the ports folder screens, "Ports folder not available" at startup (labels, what each row
//! does, which row starts selected), and the text other screens show while the ports folder must wait. Built without
//! any UI so they can be tested; `src/app/ports_folder_screen.rs` draws them. Settings > Ports > Ports folder and the
//! move are pages (`ports_folder_pages`, `setup_wizard::ports_pages`).

use super::ports_folder::Missing;
use super::rom_screens::util::ellipsize;

/// What choosing a row does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// An information row: nothing.
    Info,
    TryAgain,
    ChooseAnother,
    ContinueWithout,
    Quit,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListModel {
    pub labels: Vec<String>,
    pub acts: Vec<Act>,
    /// The row selected when the list opens.
    pub select: usize,
}

impl ListModel {
    fn push(&mut self, label: impl Into<String>, act: Act) -> usize {
        self.labels.push(label.into());
        self.acts.push(act);
        self.acts.len() - 1
    }

    pub fn act(&self, index: usize) -> Act {
        self.acts.get(index).copied().unwrap_or(Act::Info)
    }

    pub fn row_of(&self, act: Act) -> Option<usize> {
        self.acts.iter().position(|a| *a == act)
    }
}

/// "Ports folder not available" at startup or after Try again. `place` is the folder the human way ("SD card > Ports").
pub fn missing(place: &str, why: &Missing, again: bool) -> ListModel {
    let mut m = ListModel::default();
    m.push(if again { "Still not there. Your ports folder:" } else { "Your ports folder is not available:" }, Act::Info);
    m.push(format!("  {}", ellipsize(place, 70)), Act::Info);
    let (question, retry) = match why {
        Missing::DriveNotMounted(_) => ("Is the SD card in?", "Try again (the card is in now)"),
        Missing::FolderGone => ("Was it moved, renamed or deleted?", "Try again"),
        Missing::NotAFolder => ("Something that is not a folder has its name.", "Try again"),
    };
    m.push(question, Act::Info);
    m.select = m.push(retry, Act::TryAgain);
    m.push("Choose another folder...", Act::ChooseAnother);
    m.push("Continue without it (installs, adopting and restores wait until it is back)", Act::ContinueWithout);
    m.push("Quit", Act::Quit);
    m
}

/// Why an action that writes into the ports folder must wait, if it must. `unavailable` is the
/// "not available" text when the ports folder is missing.
pub fn blocked_reason(moving: bool, unavailable: Option<String>) -> Option<String> {
    if moving {
        return Some("Installed ports are being moved to a new ports folder. Wait for that to finish, then try again.".to_string());
    }
    unavailable.map(|text| format!("{text}\nInstalls, adopting and restores wait until it is back. To use another folder: Settings > Ports > Ports folder."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn the_missing_folder_list() {
        let m = missing("SD card > Ports", &Missing::DriveNotMounted(PathBuf::from("/run/media/deck/SD")), false);
        assert_eq!(m.labels[1], "  SD card > Ports");
        assert_eq!(m.labels[m.select], "Try again (the card is in now)");
        assert_eq!(m.labels[2], "Is the SD card in?");
        assert_eq!(m.row_of(Act::Quit), Some(m.labels.len() - 1));
        assert_eq!(m.act(99), Act::Info);
    }

    #[test]
    fn blocked_reasons() {
        assert!(blocked_reason(false, None).is_none());
        assert!(blocked_reason(true, None).unwrap().contains("being moved"));
        let r = blocked_reason(false, Some("Your ports folder /x is not available. Is the SD card in?".into())).unwrap();
        assert!(r.contains("adopting and restores wait"));
    }
}
