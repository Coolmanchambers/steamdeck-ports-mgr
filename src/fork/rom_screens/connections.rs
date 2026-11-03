//! The recorded connections ("Connected ports"), with whether each still holds on disk. The words the page shows are
//! `settings_pages::connection_words`.

use super::util::file_name;
use crate::fork::rom_library::{check_mapping, Health, Mapping, Method};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnGroup {
    pub port_key: String,
    pub slot: String,
    pub health: Health,
    /// The port's name (its key when it is no longer in the catalog).
    pub name: String,
    /// The first file's name, and how many more files the connection holds (a disc's tracks).
    pub file: String,
    pub more: usize,
    pub method: Method,
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
        groups.push(ConnGroup {
            port_key: m.port_key.clone(),
            slot: m.slot.clone(),
            health,
            name: port_name(&m.port_key).unwrap_or_else(|| m.port_key.clone()),
            file: file_name(&m.source),
            more: members.len() - 1,
            method: m.method,
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
    fn a_disc_reads_as_one_connection_and_its_health_is_kept() {
        let tmp = temp_dir();
        let cue = tmp.path().join("Game.cue");
        let bin = tmp.path().join("Game (Track 1).bin");
        write(&cue, b"cue");
        write(&bin, b"bin");
        let ms = vec![mapping("p", "disc", cue.clone(), Method::UseInPlace), mapping("p", "disc", bin.clone(), Method::UseInPlace)];
        let names = |k: &str| (k == "p").then(|| "Severed Chains".to_string());
        let groups = connection_groups(&ms, &names);
        assert_eq!(groups.len(), 1);
        assert_eq!((groups[0].name.as_str(), groups[0].file.as_str(), groups[0].more, groups[0].method, groups[0].health), ("Severed Chains", "Game.cue", 1, Method::UseInPlace, Health::Ok));
        assert_eq!(super::super::settings_pages::connection_words(&groups[0]), "Game.cue and 1 more: read where it is");
        fs::remove_file(&bin).unwrap();
        let broken = connection_groups(&ms, &names);
        assert_eq!(broken[0].health, Health::SourceMissing);
        assert!(super::super::settings_pages::connection_words(&broken[0]).ends_with("your file is gone (card or drive out?)"));
    }

    #[test]
    fn an_unknown_port_is_listed_by_its_key() {
        let tmp = temp_dir();
        let f = tmp.path().join("a.z64");
        write(&f, b"x");
        let groups = connection_groups(&[mapping("gone/port", "s", f, Method::Symlink)], &|_| None);
        assert_eq!((groups[0].name.as_str(), groups[0].file.as_str()), ("gone/port", "a.z64"));
    }
}
