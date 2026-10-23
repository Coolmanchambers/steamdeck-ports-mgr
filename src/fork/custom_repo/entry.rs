//! The port-list entry for a project added from a GitHub link. Like a Deck-ready only project
//! (`fork::deck_ready::entry`), it is the user's own `ports.local.json` entry that keeps its `source`, so the
//! normal installer downloads it and the normal update check follows it. Nothing is written here: the entry is
//! built, parsed and checked, then handed to the import as a [`NewEntry`], which writes it (the file backed up
//! first) just before the move and takes it off again if the port does not move.

use super::check::Checked;
use super::parse::Repo;
use crate::core::models::{port_from_value, Port};
use crate::fork::import::targets::NewEntry;
use serde_json::{json, Value};
use std::path::Path;

fn sanitize(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' }).collect();
    cleaned.trim_matches(['-', '.']).to_string()
}

/// The `Library/` folder for the project: its name, or `name-owner` when that is taken. `taken` says whether
/// a folder name is used already (by a port, or by something in the ports folder).
pub fn folder_for(repo: &Repo, taken: impl Fn(&str) -> bool) -> Option<String> {
    let base = sanitize(&repo.name);
    let owner = sanitize(&repo.owner);
    [base.clone(), format!("{base}-{owner}")].into_iter().find(|c| !c.is_empty() && c != "-" && !c.starts_with('-') && !taken(c))
}

/// The same check the Deck-ready screen uses: a port the app knows already has this folder (any case), or
/// something exists at `<ports folder>/<folder>`.
pub fn folder_taken<'a>(ports_folder: &Path, known_folders: impl IntoIterator<Item = &'a str>, folder: &str) -> bool {
    crate::fork::deck_ready::entry::folder_taken(ports_folder, known_folders, folder)
}

/// The JSON entry for `ports.local.json`.
pub fn local_entry(checked: &Checked, folder: &str) -> Value {
    let url = checked.repo.url();
    let mut instructions = format!("Added from a GitHub link. Game files are not included; the project page says what it needs: {url}");
    if !checked.description.is_empty() {
        instructions = format!("{}\n\n{instructions}", checked.description);
    }
    json!({
        "name": checked.name,
        "folder": folder,
        "source": url,
        "website": url,
        "instructions": instructions,
    })
}

/// What the import writes for the project: built, then parsed, so an entry the app would not read is
/// refused here and never reaches the file. The parsed port must come from this very project.
pub fn new_entry(checked: &Checked, folder: &str) -> Result<(NewEntry, Port), String> {
    if folder.is_empty() || folder == "." || folder == ".." || folder.contains(['/', '\\']) {
        return Err(format!("{folder:?} is not a usable folder name."));
    }
    let value = local_entry(checked, folder);
    let mut port = port_from_value(&value).map_err(|_| format!("The entry for {} could not be built.", checked.name))?;
    if port.repo.as_deref().map(str::to_lowercase) != Some(checked.repo.slug().to_lowercase()) || port.folder != folder {
        return Err(format!("The entry for {} did not read back as that project.", checked.name));
    }
    port.user_managed = true;
    Ok((NewEntry { list_name: checked.name.clone(), value }, port))
}

/// The port the app already has for this project, if any (any case): picking it needs no new entry.
pub fn existing<'a>(ports: &'a [Port], repo: &Repo) -> Option<&'a Port> {
    let slug = repo.slug().to_lowercase();
    ports.iter().find(|p| p.repo.as_deref().is_some_and(|r| r.to_lowercase() == slug))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::custom_repo::parse::parse_link;
    use std::fs;
    use std::path::PathBuf;

    fn checked(slug: &str, name: &str, description: &str) -> Checked {
        Checked { repo: parse_link(slug).unwrap(), name: name.into(), description: description.into(), version: "v1.2".into(), date: Some("2026-10-17".into()), linux_file: "g-linux.AppImage".into() }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dpm_custom_repo_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_entry_keeps_its_source_so_the_normal_installer_and_updates_work() {
        let c = checked("github.com/some-owner/Cool-Port", "Cool Port", "A cool port.");
        let (new, port) = new_entry(&c, "Cool-Port").unwrap();
        assert_eq!(new.list_name, "Cool Port");
        assert_eq!(new.value["source"], "https://github.com/some-owner/Cool-Port");
        assert_eq!(new.value["folder"], "Cool-Port");
        assert!(new.value["instructions"].as_str().unwrap().starts_with("A cool port.\n\nAdded from a GitHub link."));
        assert_eq!(port.repo.as_deref(), Some("some-owner/Cool-Port"));
        assert_eq!(port.key(), "some-owner/Cool-Port");
        assert!(port.user_managed, "its button says Open folder, never Uninstall");
        assert_eq!(port.source_type, crate::core::models::SourceType::Github);
        assert!(new.value.get("preferred_asset").is_none(), "the installer picks the file (and asks when it cannot)");
    }

    #[test]
    fn the_folder_is_the_project_name_or_name_and_owner_when_taken() {
        let repo = parse_link("github.com/some-owner/Cool-Port").unwrap();
        assert_eq!(folder_for(&repo, |_| false).as_deref(), Some("Cool-Port"));
        assert_eq!(folder_for(&repo, |f| f == "Cool-Port").as_deref(), Some("Cool-Port-some-owner"));
        assert_eq!(folder_for(&repo, |_| true), None);
        let odd = parse_link("github.com/o/.hidden.").unwrap();
        assert_eq!(folder_for(&odd, |_| false).as_deref(), Some("hidden"));
        let dots = parse_link("github.com/o/v1.2.3").unwrap();
        assert_eq!(folder_for(&dots, |_| false).as_deref(), Some("v1.2.3"));
    }

    #[test]
    fn a_folder_is_taken_by_a_known_port_in_any_case_or_by_something_in_the_ports_folder() {
        let dir = temp_dir("taken");
        fs::create_dir_all(dir.join("Existing")).unwrap();
        assert!(folder_taken(&dir, ["SoH"], "soh"));
        assert!(folder_taken(&dir, [], "Existing"));
        assert!(!folder_taken(&dir, ["SoH"], "Fresh"));
        assert!(folder_taken(&dir, [], "../escape"), "not a plain name");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_entry_that_would_not_read_back_as_that_project_is_refused_before_anything_is_written() {
        let c = checked("github.com/some-owner/Cool-Port", "Cool Port", "");
        for bad in ["", "../evil", "a/b", "..", ".", "a\\b"] {
            assert!(new_entry(&c, bad).is_err(), "{bad:?}");
        }
        let quiet = new_entry(&c, "Cool-Port").unwrap();
        assert!(!quiet.0.value["instructions"].as_str().unwrap().starts_with('\n'), "no blank first line without a description");
    }

    #[test]
    fn an_existing_port_for_the_project_is_found_in_any_case() {
        let mk = |v: serde_json::Value| port_from_value(&v).unwrap();
        let ports = vec![
            mk(json!({"name": "Shipwright", "folder": "Shipwright", "source": "https://github.com/HarbourMasters/Shipwright"})),
            mk(json!({"name": "Mine", "folder": "mine"})),
        ];
        assert_eq!(existing(&ports, &parse_link("github.com/harbourmasters/shipwright").unwrap()).map(|p| p.folder.as_str()), Some("Shipwright"));
        assert!(existing(&ports, &parse_link("github.com/someone/else").unwrap()).is_none());
    }

    /// The write itself is the Deck-ready path's: backed up first, byte for byte, and refused for a folder
    /// that is listed already.
    #[test]
    fn writing_the_entry_backs_up_the_file_byte_for_byte_and_never_overwrites_a_listed_folder() {
        let dir = temp_dir("write");
        let local = dir.join("ports.local.json");
        let original = "{\n  \"ports\": [\n    {\"name\": \"Mine\", \"folder\": \"mine\"}\n  ]\n}\n";
        fs::write(&local, original).unwrap();
        let c = checked("github.com/some-owner/Cool-Port", "Cool Port", "A cool port.");
        let (new, _) = new_entry(&c, "Cool-Port").unwrap();
        let backup = crate::fork::deck_ready::entry::add_to_local_file(&local, new.value.clone(), "T1").unwrap().expect("the old file is backed up");
        assert_eq!(fs::read_to_string(&backup).unwrap(), original, "the backup is the file as it was");
        let loaded = crate::core::config::load_local_config(&local);
        assert_eq!(loaded.iter().map(|p| p.folder.as_str()).collect::<Vec<_>>(), ["mine", "Cool-Port"]);
        assert!(loaded[1].repo.as_deref() == Some("some-owner/Cool-Port"));
        let again = crate::fork::deck_ready::entry::add_to_local_file(&local, new.value.clone(), "T2");
        assert!(again.is_err(), "a folder that is listed already is refused");
        // And the import's own undo takes exactly this entry off again.
        assert!(crate::fork::import::targets::remove_new_entry(&local, &new.value, "T3").unwrap());
        assert_eq!(crate::core::config::load_local_config(&local).len(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_local_file_is_created_without_a_backup() {
        let dir = temp_dir("write_new");
        let local = dir.join("ports.local.json");
        let c = checked("github.com/o/p", "P", "");
        let (new, _) = new_entry(&c, "p").unwrap();
        let backup = crate::fork::deck_ready::entry::add_to_local_file(&local, new.value, "T1").unwrap();
        assert!(backup.is_none());
        assert_eq!(crate::core::config::load_local_config(&local).len(), 1);
        let _ = fs::remove_dir_all(dir);
    }
}
