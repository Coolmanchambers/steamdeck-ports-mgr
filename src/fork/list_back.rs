//! B on a list goes up one level. Upstream's picker lists treat B as "close the dialog", so B on a sub-list (Artwork
//! sources' results, Restore saves, the waiting Steam changes) dropped the owner on the main window while the list's own
//! "Back" row went up one level. `app::dialogs::build_picker_dialog` asks [`back_row`] when B (the pad's B, or Escape)
//! comes: a list with a Back row does what that row does; a list without one still closes. The title bar's X never takes
//! the Back row: it leaves the list, as the X on the guided pages leaves the whole thing.

/// The row B stands for: the last row that reads "Back" or "Back to ..." (any case, spaces trimmed), if there is one.
/// The labels alone decide, so a row that is only a name reading "Back" (a port called that, say) would count too; the
/// lists this serves end with their own Back row, which is the last one and wins.
pub fn back_row<S: AsRef<str>>(labels: &[S]) -> Option<usize> {
    labels.iter().rposition(|l| {
        let l = l.as_ref().trim().to_lowercase();
        l == "back" || l.starts_with("back to ")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b_stands_for_the_back_row_when_there_is_one() {
        assert_eq!(back_row(&["SteamGridDB: set up", "Credits", "Back"]), Some(2));
        assert_eq!(back_row(&["Add a folder...", "Back to Import ports"]), Some(1));
        assert_eq!(back_row(&["  back  ", "x"]), Some(0));
        assert_eq!(back_row(&["Back", "middle", "Back"]), Some(2), "the last one, the list's own Back row");
    }

    #[test]
    fn a_list_without_a_back_row_keeps_closing() {
        assert_eq!(back_row(&["Builds: all", "System: All systems", "Done"]), None);
        assert_eq!(back_row(&["Backup of the old list", "Back up saves now", "Backing up..."]), None, "words that only start like it");
        assert_eq!(back_row::<&str>(&[]), None);
    }
}
