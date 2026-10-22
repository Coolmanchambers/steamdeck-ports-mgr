//! Text for ports whose catalog entry gives no Linux save folder (upstream bug O1, see
//! `core::platform_resolve::SaveLocation::UnknownOnLinux`). English only, like the rest of the fork.

const MAX_NAMES: usize = 8;

/// Line under the Info dialog's version text.
pub const INFO_NOTE: &str = "Save folder: unknown on Linux, no backup";

/// Appended to the Save Backup summary; empty when every installed port has a known folder.
pub fn no_linux_folder_note(names: &[String]) -> String {
    if names.is_empty() {
        return String::new();
    }
    let shown = names.iter().take(MAX_NAMES).map(String::as_str).collect::<Vec<_>>().join(", ");
    let more = match names.len().saturating_sub(MAX_NAMES) {
        0 => String::new(),
        n => format!(" and {n} more"),
    };
    format!(
        "\n\nNot backed up: no Linux save folder is known for {} installed port(s): {shown}{more}. The catalog lists a Windows or Android folder only; a Linux path in ports.local.json fixes it.",
        names.len()
    )
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
        assert!(note.contains("for 10 installed port(s): P1, P2, P3, P4, P5, P6, P7, P8 and 2 more."), "{note}");
        assert!(!note.contains("P9"));
        let one = no_linux_folder_note(&["Osu".to_string()]);
        assert!(one.contains("for 1 installed port(s): Osu."), "{one}");
    }
}
