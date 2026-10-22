//! Text for ports with a save folder that has no known Linux location (upstream bug O1, see
//! `core::platform_resolve::SaveLocation::UnknownOnLinux`). English only, like the rest of the fork.

const MAX_NAMES: usize = 8;

/// Why a location is unknown, and how to fix it; shared by the texts below.
const WHY: &str = "The catalog gives only a Windows or Android location (or a %VAR% that is not set here); a Linux path in ports.local.json fixes it.";

/// The line under the Info dialog's status lines, for an installed port; `None` when nothing is unknown.
pub fn info_note(save_unknown: bool, save2_unknown: bool) -> Option<String> {
    let label = match (save_unknown, save2_unknown) {
        (true, true) => "Save folders",
        (true, false) => "Save folder",
        (false, true) => "Save folder 2",
        (false, false) => return None,
    };
    Some(format!("{label}: unknown on Linux, no backup"))
}

/// Appended to the uninstall confirmation of a port with such a folder.
pub const UNINSTALL_NOTE: &str =
    "\n\nSaves are not kept: no Linux save folder is known for this port, so nothing is copied before the game folder is deleted. Copy any save inside the game folder yourself first.";

/// Appended to the Save Backup summary; empty when every installed port's save folders are known.
pub fn no_linux_folder_note(names: &[String]) -> String {
    if names.is_empty() {
        return String::new();
    }
    let shown = names.iter().take(MAX_NAMES).map(String::as_str).collect::<Vec<_>>().join(", ");
    let more = match names.len().saturating_sub(MAX_NAMES) {
        0 => String::new(),
        n => format!(" and {n} more"),
    };
    format!("\n\nNot backed up: a save folder of {} installed port(s) has no known Linux location: {shown}{more}. {WHY}", names.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_is_empty_without_names() {
        assert_eq!(no_linux_folder_note(&[]), "");
    }

    #[test]
    fn note_lists_names_and_counts_the_rest() {
        let names: Vec<String> = (1..=10).map(|i| format!("P{i}")).collect();
        let note = no_linux_folder_note(&names);
        assert!(note.contains("a save folder of 10 installed port(s) has no known Linux location: P1, P2, P3, P4, P5, P6, P7, P8 and 2 more."), "{note}");
        assert!(!note.contains("P9"));
        let one = no_linux_folder_note(&["osu!".to_string()]);
        assert!(one.starts_with("\n\nNot backed up: a save folder of 1 installed port(s) has no known Linux location: osu!. The catalog gives only"), "{one}");
    }

    #[test]
    fn info_note_names_the_field_that_is_unknown() {
        assert_eq!(info_note(false, false), None);
        assert_eq!(info_note(true, false).as_deref(), Some("Save folder: unknown on Linux, no backup"));
        assert_eq!(info_note(false, true).as_deref(), Some("Save folder 2: unknown on Linux, no backup"));
        assert_eq!(info_note(true, true).as_deref(), Some("Save folders: unknown on Linux, no backup"));
    }
}
