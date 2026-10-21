//! Small text and disk helpers for the game-files screens.

use std::path::Path;

/// "7.3 GB", "512 KB", "0 B". Decimal units, one decimal from MB up, as drive makers print them.
pub fn human_bytes(n: u64) -> String {
    const UNITS: [(&str, u64); 3] = [("GB", 1_000_000_000), ("MB", 1_000_000), ("KB", 1_000)];
    for (name, size) in UNITS {
        if n >= size {
            let value = n as f64 / size as f64;
            return if name == "KB" { format!("{} {name}", n / size) } else { format!("{value:.1} {name}") };
        }
    }
    format!("{n} B")
}

/// Shorten a path from the middle so the ending (the part that tells folders apart) stays.
pub fn ellipsize(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max || max < 8 {
        return text.to_string();
    }
    let head = (max - 3) / 3;
    let tail = max - 3 - head;
    let start: String = chars[..head].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{start}...{end}")
}

/// The last component of a path as text; the whole path when it has none.
pub fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Free bytes on the drive holding `path`, when the system can tell. A best effort: `None` means
/// "unknown", never "none left".
pub fn free_bytes(path: &Path) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let mut probe = path;
        while !probe.exists() {
            probe = probe.parent()?;
        }
        let out = std::process::Command::new("df").arg("-Pk").arg(probe).output().ok()?;
        if !out.status.success() {
            return None;
        }
        parse_df(&String::from_utf8_lossy(&out.stdout))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        None
    }
}

/// The "Available" column of `df -Pk` output, in bytes.
pub fn parse_df(output: &str) -> Option<u64> {
    let line = output.lines().filter(|l| !l.trim().is_empty()).nth(1)?;
    // Names of the file system and the mount point may contain spaces, the numbers never do: find the
    // "NN%" capacity column and read the column before it.
    let fields: Vec<&str> = line.split_whitespace().collect();
    let capacity = fields.iter().position(|f| f.len() > 1 && f.ends_with('%') && f[..f.len() - 1].bytes().all(|b| b.is_ascii_digit()))?;
    if capacity < 3 {
        return None;
    }
    fields[capacity - 1].parse::<u64>().ok().map(|kb| kb.saturating_mul(1024))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_like_a_drive_label() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1_000), "1 KB");
        assert_eq!(human_bytes(512_000), "512 KB");
        assert_eq!(human_bytes(12_582_912), "12.6 MB");
        assert_eq!(human_bytes(7_300_000_000), "7.3 GB");
    }

    #[test]
    fn long_paths_keep_their_ending() {
        assert_eq!(ellipsize("short", 20), "short");
        let long = "/run/media/mmcblk0p1/Emulation/roms/Nintendo 64/Zelda";
        let short = ellipsize(long, 30);
        assert_eq!(short.chars().count(), 30);
        assert!(short.ends_with("64/Zelda") && short.starts_with("/run"));
    }

    #[test]
    fn df_output_is_read_from_the_right_even_with_spaces_in_the_name() {
        let out = "Filesystem     1024-blocks      Used Available Capacity Mounted on\n/dev/mmcblk0p1   244568380 100000000 144568380      41% /run/media/deck\n";
        assert_eq!(parse_df(out), Some(144_568_380 * 1024));
        let spaced = "Filesystem 1024-blocks Used Available Capacity Mounted on\nmy drive 100 40 60 40% /mnt/x y\n";
        assert_eq!(parse_df(spaced), Some(60 * 1024));
        assert_eq!(parse_df("nothing"), None);
        assert_eq!(parse_df(""), None);
    }
}
