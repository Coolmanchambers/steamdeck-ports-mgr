//! Sizes and dates for the "Restore saves" pages and the screens that share them (cloud backup, save finder). The
//! pages themselves are `super::pages`. English only, like the fork's other screens.

use super::Backup;

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
        // A second "Back up saves" of the day (`2026-11-04-2`, data safety review) reads "2026-11-04 (2)".
        let day_like = name.is_ascii() && b.len() > 11 && b[4] == b'-' && b[7] == b'-' && b[10] == b'-' && name[..10].bytes().all(|c| c.is_ascii_digit() || c == b'-');
        if let Some(n) = day_like.then(|| &name[11..]).filter(|n| n.bytes().all(|c| c.is_ascii_digit())) {
            return format!("{} ({n})", &name[..10]);
        }
        return name.to_string();
    }
    let time = name[11..19].replace('-', ":");
    match name.get(19..).and_then(|r| r.strip_prefix('-')) {
        Some(n) if !n.is_empty() => format!("{} {time} ({n})", &name[..10]),
        _ => format!("{} {time}", &name[..10]),
    }
}

#[cfg(test)]
mod tests {
    use super::super::BackupKind;
    use super::*;
    use std::path::PathBuf;
    use std::time::SystemTime;

    fn backup(name: &str, kind: BackupKind) -> Backup {
        Backup { kind, zip: PathBuf::from(format!("/b/{name}.zip")), prefix: String::new(), name: name.into(), modified: SystemTime::UNIX_EPOCH, size: 1536, files: 3, fields: vec![], problem: None }
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
        assert_eq!(when(&backup("2026-10-21_14-05-00", BackupKind::BeforeUpdate)), "2026-10-21 14:05:00");
        assert_eq!(when(&backup("2026-10-21_14-05-00-2", BackupKind::BeforeUpdate)), "2026-10-21 14:05:00 (2)");
        assert_eq!(when(&backup("2026-10-21", BackupKind::AllPorts)), "2026-10-21");
        assert_eq!(when(&backup("2026-10-21-2", BackupKind::AllPorts)), "2026-10-21 (2)", "a second backup that day");
        assert_eq!(when(&backup("2026-10-21-cloud", BackupKind::AllPorts)), "2026-10-21-cloud");
        assert_eq!(when(&backup("odd name", BackupKind::AllPorts)), "odd name");
    }
}
