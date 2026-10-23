//! The fallback for a port that could not be moved because there is not enough room: link it into the
//! ports folder where it stands (the old "adopt"). Nothing moves; `fork::adopt` does the work and its
//! rules hold (the install is not touched, no `source`, no updates through the link).
//!
//! It is only ever offered after the user has seen why the move was refused, and it asks first.

use super::plan::MoveRequest;
use crate::core::models::Port;
use crate::fork::adopt;
use crate::fork::screens::listed;
use crate::fork::store::backup_stamp;
use std::path::Path;

#[derive(Debug)]
pub struct LinkDone {
    /// The new catalog entries (registered in `ports.local.json`), parsed.
    pub linked: Vec<Port>,
    pub text: String,
}

/// What the confirm dialog says before linking.
pub fn confirm_text(requests: &[&MoveRequest]) -> String {
    let names: Vec<String> = requests.iter().map(|r| format!("  {}", r.port_name)).collect();
    format!(
        "Link {} instead of moving:\n{}\n\nEach stays where it is. The ports folder gets a link to it and ports.local.json an entry (backed up first). \
         Nothing is copied or changed, and a linked port is not updated by this app.",
        if requests.len() == 1 { "1 port".to_string() } else { format!("{} ports", requests.len()) },
        listed(&names)
    )
}

/// Links each port where it stands; a failure is reported by name and the others still go ahead.
pub fn link_instead(requests: &[&MoveRequest], catalog: &[Port], library_dir: &Path, local_json: &Path) -> LinkDone {
    let mut linked = Vec::new();
    let mut failures = Vec::new();
    let mut lines = Vec::new();
    for request in requests {
        let Some(port) = catalog.iter().find(|p| p.key() == request.port_key) else { continue };
        match adopt::adopt(&request.from, port, library_dir, local_json, &backup_stamp()) {
            Ok(_) => {
                if let Ok(mut registered) = crate::core::models::port_from_value(&adopt::local_entry(port)) {
                    registered.user_managed = true;
                    linked.push(registered);
                }
                lines.push(format!("  {}", request.port_name));
            }
            Err(e) => failures.push(format!("{}: {e}", request.port_name)),
        }
    }
    let mut text = failures.join("\n");
    if !lines.is_empty() {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&format!("Linked {}:\n{}", lines.len(), listed(&lines)));
    }
    if !linked.is_empty() {
        text.push_str("\n\nThey now show as installed. They stay where they are, and linked ports are not checked for updates.");
    }
    LinkDone { linked, text }
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::plan::build_requests;
    use super::super::scan::fixtures::*;
    use super::super::scan::{Confidence, Found, Size};
    use super::*;
    use crate::fork::adopt::MatchKind;
    use std::fs;

    fn found(h: &Home, dir: &str, name: &str, port_key: &str, folder: &str) -> Found {
        Found {
            path: h.home.join(dir),
            dir_name: dir.to_string(),
            port_key: port_key.into(),
            port_name: name.into(),
            catalog_folder: folder.into(),
            how: MatchKind::FolderName,
            confidence: Confidence::Exact,
            size: Size::default(),
            steam: vec![],
        }
    }

    #[test]
    fn linking_leaves_the_install_alone_and_registers_the_port() {
        let h = fake_home("link_ok");
        let catalog = catalog();
        let f = found(&h, "Applications/SoH", "Ship of Harkinian", catalog[0].key(), "SoH");
        let reqs = build_requests(&[&f], &h.library);
        let refs: Vec<&MoveRequest> = reqs.iter().collect();
        let before = fs::read(h.home.join("Applications/SoH/saves/file1.sav")).unwrap();
        let local = h.home.join("ports.local.json");
        let done = link_instead(&refs, &catalog, &h.library, &local);
        assert_eq!(done.linked.len(), 1);
        assert!(done.linked[0].user_managed);
        assert!(done.text.starts_with("Linked 1:\n  Ship of Harkinian"), "{}", done.text);
        assert!(done.text.contains("not checked for updates"));
        assert!(fs::symlink_metadata(h.library.join("SoH")).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(h.library.join("SoH/saves/file1.sav")).unwrap(), before);
        assert!(h.home.join("Applications/SoH/soh.appimage").exists(), "the original stays");
    }

    #[test]
    fn a_linked_port_shows_up_on_the_add_to_steam_screen_with_its_ports_folder_path() {
        use crate::fork::screens::SteamScreen;
        use crate::fork::settings::ForkSettings;
        let h = fake_home("link_steam");
        let catalog = catalog();
        let f = found(&h, "Applications/SoH", "Ship of Harkinian", catalog[0].key(), "SoH");
        let reqs = build_requests(&[&f], &h.library);
        let done = link_instead(&reqs.iter().collect::<Vec<_>>(), &catalog, &h.library, &h.home.join("ports.local.json"));
        let merged = crate::core::config::merge_local_catalog(catalog, done.linked);
        let screen = SteamScreen::load(&h.home, &h.library, &merged, &ForkSettings::load(&h.home.join("s.json")), &|_| None);
        let row = screen.rows.iter().find(|r| r.name == "Ship of Harkinian").expect("listed as installed");
        assert_eq!(row.wanted.as_ref().unwrap().exe, h.library.join("SoH/soh.appimage").to_str().unwrap());
        assert!(row.by_name(), "the hand-made shortcut is found by name and can be repointed");
    }

    #[test]
    fn a_failure_is_named_and_does_not_stop_the_others() {
        let h = fake_home("link_partial");
        let catalog = catalog();
        fs::create_dir_all(h.library.join("dusklight")).unwrap();
        let a = found(&h, "Games/dusk-old", "Dusklight", catalog[1].key(), "dusklight");
        let b = found(&h, "Applications/SoH", "Ship of Harkinian", catalog[0].key(), "SoH");
        let reqs = build_requests(&[&a, &b], &h.library);
        let refs: Vec<&MoveRequest> = reqs.iter().collect();
        let done = link_instead(&refs, &catalog, &h.library, &h.home.join("ports.local.json"));
        assert_eq!(done.linked.len(), 1);
        assert!(done.text.starts_with("Dusklight: "), "{}", done.text);
        assert!(done.text.contains("already exists"), "{}", done.text);
        assert!(done.text.contains("Linked 1:\n  Ship of Harkinian"), "{}", done.text);
    }

    #[test]
    fn the_confirm_text_says_nothing_is_copied() {
        let h = fake_home("link_confirm");
        let f = found(&h, "Applications/SoH", "Ship of Harkinian", "k", "SoH");
        let reqs = build_requests(&[&f], &h.library);
        let text = confirm_text(&reqs.iter().collect::<Vec<_>>());
        assert!(text.starts_with("Link 1 port instead of moving:\n  Ship of Harkinian"), "{text}");
        assert!(text.contains("Nothing is copied or changed"));
    }
}
