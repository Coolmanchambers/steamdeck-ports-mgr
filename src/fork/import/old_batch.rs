//! The one page that asks about all the old copies an import left behind ("Remove the 4 old copies?"),
//! instead of one question per port. No UI code here; the screen is in
//! `src/app/ports_folder_screen/batch.rs`.
//!
//! Every old copy has already been through `removal_check`; the ones that did not pass are listed as staying,
//! with the reason, and are never offered. "Keep them" is the recommended choice, has the focus, and is what
//! B does: nothing is removed unless the user chooses "Remove them", and each removal then runs the same
//! checks and the same saves backup as removing a single old copy.

use crate::fork::art::thumbs::ArtRef;
use crate::fork::page::{Focus, Page, PageOption, Tile};
use crate::fork::port_mover::old_copies::OldCopy;
use crate::fork::rom_screens::util::{ellipsize, human_bytes};

/// An old copy with what its check said: `Ok` with the bytes it would free, or why it must stay.
#[derive(Debug, Clone)]
pub struct Checked {
    pub copy: OldCopy,
    pub check: Result<u64, String>,
    pub art: Option<ArtRef>,
}

impl Checked {
    pub fn removable(&self) -> bool {
        self.check.is_ok()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchAction {
    Keep,
    Remove,
    /// Nothing can be removed: just carry on.
    Ok,
}

pub const WINDOW_TITLE: &str = "Import ports: old copies";

pub struct Batch {
    pub items: Vec<Checked>,
}

impl Batch {
    pub fn new(items: Vec<Checked>) -> Batch {
        Batch { items }
    }

    pub fn removable(&self) -> Vec<&Checked> {
        self.items.iter().filter(|c| c.removable()).collect()
    }

    pub fn kept(&self) -> Vec<&Checked> {
        self.items.iter().filter(|c| !c.removable()).collect()
    }

    /// What removing them would free. A link's target stays, so a link frees nothing.
    pub fn freed_bytes(&self) -> u64 {
        self.removable().iter().filter(|c| !c.copy.link_only).filter_map(|c| c.check.as_ref().ok()).sum()
    }

    /// The names of the old copies that stay, for the closing text.
    fn names(items: &[&Checked]) -> String {
        items.iter().map(|c| c.copy.name.as_str()).collect::<Vec<_>>().join(", ")
    }

    /// What is said when nothing is removed (the user kept them, or none could go).
    pub fn kept_text(&self) -> String {
        let removable = self.removable();
        let mut parts: Vec<String> = Vec::new();
        if !removable.is_empty() {
            parts.push(format!("Kept the old {} of {}. Remove {} later in Settings, Ports folder, Old copies.", copies(removable.len()), Batch::names(&removable), if removable.len() == 1 { "it" } else { "them" }));
        }
        parts.extend(self.kept().iter().map(|c| format!("The old copy of {} stays: {}", c.copy.name, why(&c.check))));
        parts.join("\n")
    }

    /// The page. Keep is first, recommended and focused; Remove is never the default.
    pub fn page(&self) -> Page<BatchAction> {
        let removable = self.removable();
        let n = removable.len();
        let mut p = Page::<BatchAction> { window_title: WINDOW_TITLE.to_string(), large: true, header: crate::fork::import::cards::HEADER.to_string(), ..Page::default() };
        p.facts = self.kept().iter().map(|c| format!("{} stays: {}", c.copy.name, ellipsize(&why(&c.check), 90))).collect();
        if n == 0 {
            p.title = "The old copies stay".to_string();
            p.paragraphs = vec!["None of the old copies can be removed right now. They are kept, and nothing is lost.".to_string()];
            p.options = vec![PageOption { title: "OK".to_string(), detail: String::new(), choice: None, enabled: true, recommended: true, action: BatchAction::Ok }];
            p.initial = Focus::Option(0);
            return p;
        }
        p.title = if n == 1 { "Remove the old copy?".to_string() } else { format!("Remove the {n} old copies?") };
        let freed = self.freed_bytes();
        let size = if freed > 0 { format!(" The old {} {} {}.", copies(n), if n == 1 { "uses" } else { "use" }, human_bytes(freed)) } else { String::new() };
        p.paragraphs = vec![format!("Your ports were copied and checked.{size}")];
        p.tiles = removable.iter().map(|c| Tile::shown(c.art.clone(), c.copy.name.clone())).collect();
        let (keep, remove) = if n == 1 { ("Keep it", "Remove it") } else { ("Keep them", "Remove them") };
        p.options = vec![
            PageOption { title: keep.to_string(), detail: "Nothing is deleted. You can remove them later from Settings.".to_string(), choice: None, enabled: true, recommended: true, action: BatchAction::Keep },
            PageOption { title: remove.to_string(), detail: "Saves are zipped to a backup first".to_string(), choice: None, enabled: true, recommended: false, action: BatchAction::Remove },
        ];
        p.initial = Focus::Option(0);
        p
    }
}

fn copies(n: usize) -> &'static str {
    if n == 1 { "copy" } else { "copies" }
}

fn why(check: &Result<u64, String>) -> String {
    match check {
        Ok(_) => String::new(),
        Err(why) => why.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn copy(name: &str, bytes: u64, link_only: bool) -> OldCopy {
        OldCopy {
            port_key: name.to_lowercase(),
            name: name.to_string(),
            old_path: PathBuf::from(format!("/home/deck/Games/{name}")),
            new_path: PathBuf::from(format!("/run/media/sd/Ports/{name}")),
            bytes,
            moved_at: String::new(),
            link_only,
            switched: true,
            import_folder: None,
        }
    }

    fn ok(name: &str, bytes: u64) -> Checked {
        Checked { copy: copy(name, bytes, false), check: Ok(bytes), art: Some(ArtRef { folder: name.to_lowercase(), url: None }) }
    }

    fn stays(name: &str, why: &str) -> Checked {
        Checked { copy: copy(name, 5, false), check: Err(why.to_string()), art: None }
    }

    fn all_text(p: &Page<BatchAction>) -> String {
        let mut parts = vec![p.title.clone()];
        parts.extend(p.paragraphs.iter().cloned());
        parts.extend(p.facts.iter().cloned());
        parts.extend(p.tiles.iter().map(|t| t.label.clone()));
        parts.extend(p.options.iter().flat_map(|o| [o.title.clone(), o.detail.clone()]));
        parts.join("\n")
    }

    #[test]
    fn the_page_asks_once_for_all_the_old_copies_and_keeping_is_the_default() {
        let b = Batch::new(vec![ok("Dusklight", 3_000_000_000), ok("SoH", 200_000_000), ok("Abuse", 10_000_000), ok("Pinball", 1_000_000)]);
        let p = b.page();
        assert_eq!(p.title, "Remove the 4 old copies?");
        assert_eq!(p.window_title, "Import ports: old copies");
        assert_eq!(p.paragraphs, ["Your ports were copied and checked. The old copies use 3.2 GB."]);
        assert_eq!(p.tiles.iter().map(|t| t.label.as_str()).collect::<Vec<_>>(), ["Dusklight", "SoH", "Abuse", "Pinball"]);
        assert_eq!(p.options.iter().map(|o| (o.title.as_str(), o.action, o.recommended)).collect::<Vec<_>>(), [("Keep them", BatchAction::Keep, true), ("Remove them", BatchAction::Remove, false)]);
        assert_eq!(p.initial, Focus::Option(0), "Keep them has the focus");
        assert!(p.facts.is_empty());
    }

    #[test]
    fn one_old_copy_reads_in_the_singular() {
        let p = Batch::new(vec![ok("Dusklight", 3_000_000_000)]).page();
        assert_eq!(p.title, "Remove the old copy?");
        assert_eq!(p.paragraphs, ["Your ports were copied and checked. The old copy uses 3.0 GB."]);
        assert_eq!(p.options[0].title, "Keep it");
        assert_eq!(p.options[1].title, "Remove it");
    }

    #[test]
    fn a_copy_that_failed_its_check_is_listed_as_staying_and_never_offered() {
        let b = Batch::new(vec![ok("Dusklight", 100), stays("SoH", "Steam shortcuts still point into it")]);
        let p = b.page();
        assert_eq!(p.title, "Remove the old copy?", "only the one that passed counts");
        assert_eq!(p.tiles.len(), 1);
        assert_eq!(p.facts, ["SoH stays: Steam shortcuts still point into it"]);
        assert_eq!(b.removable().len(), 1);
        assert_eq!(b.kept().len(), 1);
    }

    #[test]
    fn when_nothing_can_go_the_page_says_so_and_has_only_ok() {
        let b = Batch::new(vec![stays("Dusklight", "a program is running from it"), stays("SoH", "saves newer than the new copy")]);
        let p = b.page();
        assert_eq!(p.title, "The old copies stay");
        assert_eq!(p.options.len(), 1);
        assert_eq!(p.options[0].action, BatchAction::Ok);
        assert!(p.tiles.is_empty());
        assert_eq!(p.facts.len(), 2);
        assert!(b.kept_text().contains("The old copy of Dusklight stays: a program is running from it"));
    }

    #[test]
    fn a_link_frees_nothing_so_the_size_sentence_leaves_it_out() {
        let link = Checked { copy: copy("Linked", 9_000_000_000, true), check: Ok(9_000_000_000), art: None };
        let b = Batch::new(vec![ok("Dusklight", 2_000_000_000), link.clone()]);
        assert_eq!(b.freed_bytes(), 2_000_000_000);
        assert!(b.page().paragraphs[0].contains("use 2.0 GB"));
        assert_eq!(Batch::new(vec![link]).page().paragraphs[0], "Your ports were copied and checked.");
    }

    #[test]
    fn keeping_them_says_which_stay_and_where_to_remove_them_later() {
        let b = Batch::new(vec![ok("Dusklight", 1), ok("SoH", 1), stays("Abuse", "no longer in the catalog")]);
        let t = b.kept_text();
        assert!(t.contains("Kept the old copies of Dusklight, SoH. Remove them later in Settings, Ports folder, Old copies."), "{t}");
        assert!(t.contains("The old copy of Abuse stays: no longer in the catalog"), "{t}");
        let one = Batch::new(vec![ok("Dusklight", 1)]).kept_text();
        assert!(one.starts_with("Kept the old copy of Dusklight. Remove it later"), "{one}");
    }

    #[test]
    fn a_long_reason_is_shortened_on_the_page_but_kept_whole_in_the_closing_text() {
        let long = "x".repeat(300);
        let b = Batch::new(vec![ok("A", 1), stays("B", &long)]);
        assert!(b.page().facts[0].len() < 140, "{}", b.page().facts[0]);
        assert!(b.kept_text().contains(&long));
    }

    #[test]
    fn no_page_shows_a_path_or_a_file_name() {
        let b = Batch::new(vec![ok("Dusklight", 100), stays("SoH", "Steam shortcuts still point into it")]);
        for p in [b.page(), Batch::new(vec![stays("SoH", "x")]).page()] {
            let t = all_text(&p);
            assert!(!t.contains("/home") && !t.contains("/run/") && !t.to_lowercase().contains("json"), "{t}");
        }
    }
}
