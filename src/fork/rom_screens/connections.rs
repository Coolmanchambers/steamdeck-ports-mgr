//! The list of recorded connections ("Connected ports"), with whether each still holds on disk.

use super::util::file_name;
use crate::fork::rom_library::{check_mapping, Health, Mapping, Method};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnGroup {
    pub port_key: String,
    pub slot: String,
    pub label: String,
    pub health: Health,
}

fn health_words(h: Health) -> &'static str {
    match h {
        Health::Ok => "OK",
        Health::SourceMissing => "game file missing (drive unplugged?)",
        Health::SourceChanged => "game file changed since connecting",
        Health::DestMissing => "link or copy is gone",
        Health::DestChanged => "link or copy was changed",
    }
}

/// One group per (port, file slot): a disc's several files read as one connection. `port_name` maps a
/// port key to its name (the key itself when the port is no longer in the catalog).
pub fn connection_groups(mappings: &[Mapping], port_name: &dyn Fn(&str) -> Option<String>) -> Vec<ConnGroup> {
    let mut groups: Vec<ConnGroup> = Vec::new();
    let mut seen: Vec<(String, String)> = Vec::new();
    for m in mappings {
        let id = (m.port_key.clone(), m.slot.clone());
        if seen.contains(&id) {
            continue;
        }
        seen.push(id);
        let members: Vec<&Mapping> = mappings.iter().filter(|o| o.port_key == m.port_key && o.slot == m.slot).collect();
        let health = members.iter().map(|o| check_mapping(o)).find(|h| *h != Health::Ok).unwrap_or(Health::Ok);
        let name = port_name(&m.port_key).unwrap_or_else(|| m.port_key.clone());
        let more = if members.len() > 1 { format!(" +{} more", members.len() - 1) } else { String::new() };
        let how = match m.method {
            Method::UseInPlace => "used in place",
            Method::Symlink => "linked",
            Method::Copy => "copied",
        };
        groups.push(ConnGroup {
            port_key: m.port_key.clone(),
            slot: m.slot.clone(),
            label: format!("{name}: {}{more} ({how}) - {}", file_name(&m.source), health_words(health)),
            health,
        });
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::rom_library::testutil::*;
    use std::fs;
    use std::path::PathBuf;

    fn mapping(port: &str, slot: &str, source: PathBuf, method: Method) -> Mapping {
        let meta = fs::metadata(&source).unwrap();
        Mapping {
            port_key: port.into(),
            slot: slot.into(),
            method,
            source,
            dest: None,
            size: meta.len(),
            mtime_ms: meta.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64,
            crc32: None,
            created_by_us: false,
            connected_at: String::new(),
        }
    }

    #[test]
    fn a_disc_reads_as_one_connection_and_health_shows_in_the_words() {
        let tmp = temp_dir();
        let cue = tmp.path().join("Game.cue");
        let bin = tmp.path().join("Game (Track 1).bin");
        write(&cue, b"cue");
        write(&bin, b"bin");
        let ms = vec![mapping("p", "disc", cue.clone(), Method::UseInPlace), mapping("p", "disc", bin.clone(), Method::UseInPlace)];
        let names = |k: &str| (k == "p").then(|| "Severed Chains".to_string());
        let groups = connection_groups(&ms, &names);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].label, "Severed Chains: Game.cue +1 more (used in place) - OK");
        fs::remove_file(&bin).unwrap();
        let broken = connection_groups(&ms, &names);
        assert_eq!(broken[0].health, Health::SourceMissing);
        assert!(broken[0].label.ends_with("game file missing (drive unplugged?)"));
    }

    #[test]
    fn an_unknown_port_is_listed_by_its_key() {
        let tmp = temp_dir();
        let f = tmp.path().join("a.z64");
        write(&f, b"x");
        let groups = connection_groups(&[mapping("gone/port", "s", f, Method::Symlink)], &|_| None);
        assert!(groups[0].label.starts_with("gone/port: a.z64 (linked)"));
    }
}
