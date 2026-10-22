//! The text the "Restore saves" screens show, kept apart from the UI so it can be tested. English only,
//! like the fork's other screens.

use super::{system_name, Backup, Interrupted, RestoreDone, RestorePlan, SaveTarget, Target};
use std::path::Path;

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// "2026-10-21_14-05-00" -> "2026-10-21 14:05:00", "2026-10-21_14-05-00-2" -> "2026-10-21 14:05:00 (2)";
/// anything else (a whole-library backup is just a date) is shown as it is.
pub fn when(backup: &Backup) -> String {
    let name = backup.name.as_str();
    let b = name.as_bytes();
    let stamp_like = name.is_ascii()
        && b.len() >= 19
        && b[10] == b'_'
        && b[13] == b'-'
        && b[16] == b'-'
        && name[..10].bytes().all(|c| c.is_ascii_digit() || c == b'-')
        && name[11..19].bytes().all(|c| c.is_ascii_digit() || c == b'-');
    if !stamp_like {
        return name.to_string();
    }
    let time = name[11..19].replace('-', ":");
    match name.get(19..).and_then(|r| r.strip_prefix('-')) {
        Some(n) if !n.is_empty() => format!("{} {time} ({n})", &name[..10]),
        _ => format!("{} {time}", &name[..10]),
    }
}

fn field_list(backup: &Backup) -> String {
    backup.fields.iter().map(|f| f.label()).collect::<Vec<_>>().join(", ")
}

/// One row of the backup list.
pub fn backup_row(backup: &Backup) -> String {
    match &backup.problem {
        Some(problem) => format!("{}  {}  - cannot be used: {problem}", when(backup), backup.kind.label()),
        None => format!("{}  {}  {}  ({})", when(backup), backup.kind.label(), human_size(backup.size), field_list(backup)),
    }
}

pub fn target_row(target: &SaveTarget) -> String {
    match &target.target {
        Target::Folder(path) => format!("{}: {}", target.field.label(), path.display()),
        Target::NotForThisSystem => format!("{}: no save folder known for {}", target.field.label(), system_name()),
        Target::Refused(why) => format!("{}: not used, it {why}", target.field.label()),
        Target::NotListed => format!("{}: the catalog does not say where it keeps its saves", target.field.label()),
    }
}

/// One list row per folder a restore left behind (shown above the backups).
/// The folder's own name only, so the row fits; the details give the full path.
pub fn interrupted_row(item: &Interrupted) -> String {
    let name = item.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if item.holds_previous {
        format!("A restore was interrupted: your previous saves are in {name} (select for details)")
    } else {
        format!("An unfinished restore copy was left in {name} (select for details)")
    }
}

pub fn interrupted_text(item: &Interrupted) -> String {
    if item.holds_previous {
        format!(
            "A restore was interrupted. Your previous saves are in {}.\n\nIf {} looks wrong or is missing, copy that folder's contents back into it (in Dolphin, Ctrl+H shows hidden folders). The newest \"Before restore\" row in this list holds the same saves; restoring it puts them back. The app never deletes this folder; remove it yourself once your saves are right.",
            item.path.display(),
            item.folder.display()
        )
    } else {
        format!(
            "A restore stopped before it finished and left an unfinished copy of a backup at {}.\n\nIt is not your current saves ({} is). You can delete it once you have checked that folder. The app never deletes it.",
            item.path.display(),
            item.folder.display()
        )
    }
}

/// True when at least one save field points at a folder the app may restore into.
pub fn any_restorable(targets: &[SaveTarget]) -> bool {
    targets.iter().any(|t| matches!(t.target, Target::Folder(_)))
}

/// Shown instead of the list when there is nowhere to restore to.
pub fn nothing_restorable_text(port_name: &str, targets: &[SaveTarget]) -> String {
    if targets.is_empty() || targets.iter().all(|t| t.target == Target::NotListed) {
        return format!("{port_name} does not say where it keeps its saves, so there is nothing to restore.");
    }
    let mut text = if targets.iter().all(|t| t.target == Target::NotForThisSystem) {
        format!(
            "No save folder known for {} for {port_name}. The catalog only names a location on another system, so the app will not guess, and no restore is offered.\n",
            system_name()
        )
    } else {
        format!("{port_name} has no save folder the app can restore into:\n")
    };
    for t in targets {
        text.push_str(&format!("\n{}", target_row(t)));
    }
    text
}

pub fn confirm_text(port_name: &str, backup: &Backup, plan: &RestorePlan, saves_backup_dir: &Path, port_folder: &str) -> String {
    let mut text = format!(
        "Restore {port_name}'s saves from {} ({}, {}, {} files)?\n\nEverything in these folders is replaced by the backup's copy:",
        when(backup),
        backup.kind.label(),
        human_size(backup.size),
        backup.files
    );
    for (field, path) in &plan.replace {
        text.push_str(&format!("\n  {}: {}", field.label(), path.display()));
    }
    text.push_str("\nFiles saved since then are removed from the folder (they stay in the zip made first).");
    for (field, why) in &plan.skipped {
        text.push_str(&format!("\n\nNot restored, {}: {why}.", field.label()));
    }
    text.push_str(&format!(
        "\n\nYour current saves are zipped first, to {}. If that backup fails, nothing is changed. Quit the game before restoring.",
        saves_backup_dir.join(super::RESTORE_BACKUPS_DIR).join(port_folder).display()
    ));
    text
}

pub fn done_text(port_name: &str, done: &RestoreDone) -> String {
    let mut text = format!("{port_name}'s saves were restored.\n");
    for f in &done.folders {
        text.push_str(&format!("\n{}: {} ({} files)", f.field.label(), f.path.display(), f.files));
        if let Some(old) = &f.kept_old {
            text.push_str(&format!("\n  The folder it replaced was kept at {}.", old.display()));
        }
    }
    for (field, why) in &done.skipped {
        text.push_str(&format!("\n\nNot restored, {}: {why}.", field.label()));
    }
    match &done.safety_backup {
        Some(zip) => text.push_str(&format!(
            "\n\nYour saves from before the restore are in {}. To undo, open Info > Restore saves... and pick the newest \"Before restore\" row.",
            zip.display()
        )),
        None => text.push_str("\n\nThere were no current saves to back up first."),
    }
    text
}

/// One row of the Settings list of ports that have backups.
pub fn port_row(name: &str, backups: &[Backup]) -> String {
    let newest = backups.first().map(when).unwrap_or_default();
    let count = backups.len();
    let noun = if count == 1 { "backup" } else { "backups" };
    format!("{name} - {count} {noun}, newest {newest}")
}

#[cfg(test)]
mod tests {
    use super::super::{BackupKind, RestoredFolder, SaveField};
    use super::*;
    use std::path::PathBuf;
    use std::time::SystemTime;

    fn backup(name: &str, kind: BackupKind, fields: Vec<SaveField>) -> Backup {
        Backup { kind, zip: PathBuf::from(format!("/b/{name}.zip")), prefix: String::new(), name: name.into(), modified: SystemTime::UNIX_EPOCH, size: 1536, files: 3, fields, problem: None }
    }

    #[test]
    fn sizes_read_like_a_file_manager() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn stamps_become_dates() {
        assert_eq!(when(&backup("2026-10-21_14-05-00", BackupKind::BeforeUpdate, vec![])), "2026-10-21 14:05:00");
        assert_eq!(when(&backup("2026-10-21_14-05-00-2", BackupKind::BeforeUpdate, vec![])), "2026-10-21 14:05:00 (2)");
        assert_eq!(when(&backup("2026-10-21", BackupKind::AllPorts, vec![])), "2026-10-21");
        assert_eq!(when(&backup("odd name", BackupKind::AllPorts, vec![])), "odd name");
    }

    #[test]
    fn rows_say_kind_size_and_folders() {
        let b = backup("2026-10-21_14-05-00", BackupKind::BeforeUpdate, vec![SaveField::Save, SaveField::Save2]);
        assert_eq!(backup_row(&b), "2026-10-21 14:05:00  Before update  1.5 KB  (Save folder, Save folder 2)");
        let mut bad = b.clone();
        bad.problem = Some("it holds a file name that points outside its folder (../x)".into());
        assert!(backup_row(&bad).contains("cannot be used"));
    }

    #[test]
    fn no_linux_folder_is_said_plainly() {
        let targets = vec![SaveTarget { field: SaveField::Save, target: Target::NotForThisSystem }];
        assert!(!any_restorable(&targets));
        let text = nothing_restorable_text("Game", &targets);
        assert!(text.contains(&format!("No save folder known for {}", system_name())), "{text}");
        assert!(text.contains("will not guess"));
        assert!(nothing_restorable_text("Game", &[]).contains("does not say where it keeps its saves"));
    }

    #[test]
    fn confirm_names_the_folders_and_the_safety_backup() {
        let b = backup("2026-10-21_14-05-00", BackupKind::BeforeUpdate, vec![SaveField::Save, SaveField::Save2]);
        let plan = RestorePlan { replace: vec![(SaveField::Save, PathBuf::from("/g/Save"))], skipped: vec![(SaveField::Save2, "no save folder known for Linux".into())] };
        let text = confirm_text("Game", &b, &plan, Path::new("/sb"), "Game");
        assert!(text.contains("Save folder: /g/Save") || text.contains("Save folder: \\g\\Save"), "{text}");
        assert!(text.contains("Not restored, Save folder 2"));
        assert!(text.contains("zipped first"));
        assert!(text.contains("nothing is changed"));
    }

    #[test]
    fn done_text_points_at_the_undo() {
        let done = RestoreDone {
            safety_backup: Some(PathBuf::from("/sb/Restore Backups/Game/x.zip")),
            folders: vec![RestoredFolder { field: SaveField::Save, path: PathBuf::from("/g/Save"), files: 2, kept_old: None }],
            skipped: vec![],
        };
        let text = done_text("Game", &done);
        assert!(text.contains("(2 files)"));
        assert!(text.contains("Before restore"));
    }

    #[test]
    fn port_rows_count_backups() {
        let list = vec![backup("2026-10-21_14-05-00", BackupKind::BeforeUpdate, vec![SaveField::Save]), backup("2026-10-20", BackupKind::AllPorts, vec![SaveField::Save])];
        assert_eq!(port_row("Game", &list), "Game - 2 backups, newest 2026-10-21 14:05:00");
    }
}
