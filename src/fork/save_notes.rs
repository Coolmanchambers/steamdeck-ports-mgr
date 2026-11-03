//! Text for ports with a save folder that has no known Linux location (upstream bug O1, see
//! `core::platform_resolve::SaveLocation::UnknownOnLinux`). English only, like the rest of the fork. Back up saves
//! names those ports on its done page (`backup_pages`).

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_note_names_the_field_that_is_unknown() {
        assert_eq!(info_note(false, false), None);
        assert_eq!(info_note(true, false).as_deref(), Some("Save folder: unknown on Linux, no backup"));
        assert_eq!(info_note(false, true).as_deref(), Some("Save folder 2: unknown on Linux, no backup"));
        assert_eq!(info_note(true, true).as_deref(), Some("Save folders: unknown on Linux, no backup"));
    }
}
