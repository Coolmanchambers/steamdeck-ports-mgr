//! Restore saves as pages (Settings > Saves > Restore saves, the Port page's Saves > Restore, the cloud backup's "put
//! it back"). Replaces the old lists ("2026-10-30 14:22  before update  1.2 MB  (save)", "No backups of these saves
//! yet. One is made before every update, and by Settings > Backup Saves.", a confirm that listed raw paths, "The folder
//! it replaced was kept at /home/deck/...").
//!
//! The pages: which port (from Settings: the ports with backups), which backup (the date as the title, the kind and
//! size as the detail, newest first), "Put back the saves from 2026-10-30 14:22?" (what happens to the saves now: they
//! are backed up first, and where, the human way), the waiting page, the done page, and a page for anything that stops
//! it. What a restore does is `super::restore`, unchanged: the saves now are backed up before anything is replaced.
//!
//! Pure data and words; `src/app/save_restore_screen.rs` runs them. Places come in told the human way
//! (`port_page::paths`).

use super::{Backup, BackupKind, Interrupted, RestoreDone, RestoreError, RestorePlan, SaveTarget, Target};
use std::path::Path;
use crate::fork::page::{Focus, Mark, Page, PageButton, PageOption, LEFT};
use crate::fork::port_page::paths::{human_path, plain_paths, size_text, Places};
use crate::fork::rom_screens::util::ellipsize;

/// The window title of the Settings page (the ports with backups).
pub const SETTINGS_WINDOW: &str = "Settings: Saves: Restore saves";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RsAct {
    /// The port `n` of the Settings page.
    Port(usize),
    /// The backup `n` of a port's page.
    Backup(usize),
    /// The restore that stopped part way, `n`.
    Interrupted(usize),
    /// Yes, put the saves back.
    Restore,
    /// No, keep the saves as they are.
    Keep,
    Ok,
    Back,
}

fn option(title: &str, detail: impl Into<String>, action: RsAct) -> PageOption<RsAct> {
    PageOption { title: ellipsize(title, 40), detail: ellipsize(&detail.into(), 66), choice: None, enabled: true, recommended: false, action }
}

fn base(window: &str, header: &str, title: &str) -> Page<RsAct> {
    Page {
        window_title: window.to_string(),
        large: true,
        header: ellipsize(header, 60),
        title: ellipsize(title, 40),
        buttons: [Some(PageButton { label: "Back".to_string(), action: RsAct::Back }), None, None, None],
        initial: Focus::Option(0),
        back_hint: "Back".to_string(),
        ..Page::default()
    }
}

/// A page with only OK (B does the same).
fn with_ok(mut p: Page<RsAct>) -> Page<RsAct> {
    p.options = vec![option("OK", "", RsAct::Ok)];
    p.options[0].recommended = true;
    p.buttons = [None, None, None, None];
    p.initial = Focus::Option(0);
    p.b_does(&RsAct::Ok)
}

/// "2026-10-30 14:22", "2026-10-30 14:22 (2)", "2026-10-21": when the backup was made, without the seconds.
pub fn when_short(backup: &Backup) -> String {
    let full = super::view::when(backup);
    let b = full.as_bytes();
    if full.len() >= 19 && b.get(16) == Some(&b':') && b.get(13) == Some(&b':') {
        return format!("{}{}", &full[..16], &full[19..]);
    }
    full
}

/// What kind of backup it is, in words.
pub fn kind_words(kind: BackupKind) -> &'static str {
    match kind {
        BackupKind::BeforeUpdate => "Made before an update",
        BackupKind::AllPorts => "From Back up saves",
        BackupKind::BeforeRestore => "Made before a restore",
    }
}

// -- Which port (Settings) ---------------------------------------------------------------------------------------------

/// A port with backups, for the Settings page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortChoice {
    pub name: String,
    pub backups: usize,
    /// When the newest was made (`when_short`).
    pub newest: String,
    /// The port is in the ports list (else nothing can be put back: the app does not know where its saves go).
    pub known: bool,
}

/// Settings > Saves > Restore saves: one card per port that has backups. B goes back to the Saves page.
pub fn ports_page(rows: &[PortChoice]) -> Page<RsAct> {
    let mut p = base(SETTINGS_WINDOW, "Settings", "Restore saves");
    if rows.is_empty() {
        p.paragraphs.push("No saves have been backed up yet. A backup is made before every update, and by Back up saves.".to_string());
        p.initial = Focus::Button(LEFT);
        return p;
    }
    p.paragraphs.push("Choose a port to see its backups and put one back. Your saves now are always backed up first.".to_string());
    p.options = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let detail = if r.known {
                format!("{} {}, newest {}", r.backups, if r.backups == 1 { "backup" } else { "backups" }, r.newest)
            } else {
                "Not in your ports list now, so it cannot be put back".to_string()
            };
            option(&r.name, detail, RsAct::Port(i))
        })
        .collect();
    p
}

// -- Which backup ------------------------------------------------------------------------------------------------------

/// One backup as its card shows it: the date as the title, the kind and size as the detail.
pub fn backup_option(index: usize, b: &Backup) -> PageOption<RsAct> {
    let detail = match &b.problem {
        Some(_) => "Cannot be used (choose it to see why)".to_string(),
        None => format!("{}, {}", kind_words(b.kind), size_text(b.size)),
    };
    option(&when_short(b), detail, RsAct::Backup(index))
}

/// A port's backups, newest first. `saves` are its save folders, the human way; `interrupted` the restores that
/// stopped part way (each has a card that says where the saves from before it are).
pub fn backups_page(name: &str, backups: &[Backup], saves: &[String], interrupted: usize) -> Page<RsAct> {
    let mut p = base("Restore saves: choose a backup", name, "Choose a backup");
    match saves {
        [] => {}
        [one] => p.paragraphs.push(format!("Your saves are in {one}.")),
        many => p.paragraphs.push(format!("Your saves are in {}.", many.join(" and "))),
    }
    if backups.is_empty() {
        p.paragraphs.push("No backups of these saves yet. One is made before every update, and by Back up saves.".to_string());
    } else {
        p.paragraphs.push("Newest first. Your saves now are backed up before any backup is put back.".to_string());
    }
    for i in 0..interrupted {
        p.fact(Mark::Problem, "An earlier restore stopped part way");
        p.options.push(option("About the restore that stopped", "Where your saves from before it are", RsAct::Interrupted(i)));
    }
    p.options.extend(backups.iter().enumerate().map(|(i, b)| backup_option(i, b)));
    p.initial = if p.options.is_empty() { Focus::Button(LEFT) } else { Focus::Option(interrupted.min(p.options.len() - 1)) };
    p
}

/// Something to say about a port, then OK.
pub fn notice_page(header: &str, title: &str, text: &str) -> Page<RsAct> {
    let mut p = base("Restore saves: notice", header, title);
    p.paragraphs = text.split("\n\n").map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect();
    with_ok(p)
}

/// A restore that stopped part way: where the saves from before it are, and what to do.
pub fn interrupted_text(item: &Interrupted, places: &Places) -> String {
    let (at, folder) = (human_path(&item.path, places), human_path(&item.folder, places));
    if item.holds_previous {
        format!(
            "A restore stopped part way. Your saves from before it are in a hidden folder next to the save folder: {at}.\n\nIf the saves in {folder} look wrong, copy that hidden folder's contents back into it. The newest backup made before a restore holds the same saves; putting it back does the same.\n\nThe app never deletes that hidden folder. Remove it yourself once your saves are right."
        )
    } else {
        format!("A restore stopped before it finished and left an unfinished copy of a backup in {at}.\n\nIt is not your saves now ({folder} is). You can delete it once you have checked. The app never deletes it.")
    }
}

/// Where one save folder of the port points, in words.
pub fn target_line(t: &SaveTarget, places: &Places) -> String {
    match &t.target {
        Target::Folder(path) => format!("{}: {}", t.field.label(), human_path(path, places)),
        Target::NotForThisSystem => format!("{}: no save folder is known for {}", t.field.label(), super::system_name()),
        Target::Refused(why) => format!("{}: not used, it {}", t.field.label(), plain_paths(why, places)),
        Target::NotListed => format!("{}: the ports list does not say where it keeps its saves", t.field.label()),
    }
}

/// Said instead of the backups when there is nowhere to put saves back.
pub fn nothing_restorable_text(name: &str, targets: &[SaveTarget], places: &Places) -> String {
    if targets.is_empty() || targets.iter().all(|t| t.target == Target::NotListed) {
        return format!("{name} does not say where it keeps its saves, so there is nothing to put back.");
    }
    let mut text = if targets.iter().all(|t| t.target == Target::NotForThisSystem) {
        format!("No save folder is known for {name} on {}. The ports list only names a place on another system, so the app does not guess, and nothing can be put back.", super::system_name())
    } else {
        format!("{name} has no save folder the app can put saves back into.")
    };
    for t in targets {
        text.push_str(&format!("\n\n{}", target_line(t, places)));
    }
    text
}

// -- Put them back? ----------------------------------------------------------------------------------------------------

/// What the yes/no page says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AskFacts {
    pub name: String,
    /// `when_short` of the backup.
    pub when: String,
    pub kind: String,
    /// The save folders that get the backup's saves, the human way.
    pub folders: Vec<String>,
    /// What the backup holds that is not put back, and why ("Save folder 2: no save folder is known for Linux").
    pub skipped: Vec<String>,
    /// Where the saves now are backed up first, the human way.
    pub kept_at: String,
}

/// The question about `backup` for the port `name`: `plan` is what the restore would do, `kept_at` the folder the saves
/// now are backed up to first (Saves Backup > Restore Backups > the port's folder).
pub fn ask_facts(name: &str, backup: &Backup, plan: &RestorePlan, kept_at: &Path, places: &Places) -> AskFacts {
    AskFacts {
        name: name.to_string(),
        when: when_short(backup),
        kind: kind_words(backup.kind).to_string(),
        folders: plan.replace.iter().map(|(_, path)| human_path(path, places)).collect(),
        skipped: plan.skipped.iter().map(|(field, why)| format!("{} ({})", field.label(), plain_paths(why, places))).collect(),
        kept_at: human_path(kept_at, places),
    }
}

/// What the done page says after `done`.
pub fn done_facts(name: &str, when: &str, done: &RestoreDone, places: &Places) -> DoneFacts {
    DoneFacts {
        name: name.to_string(),
        when: when.to_string(),
        folders: done.folders.iter().map(|f| PutBack { folder: human_path(&f.path, places), files: f.files, kept_old: f.kept_old.as_ref().map(|p| human_path(p, places)) }).collect(),
        skipped: done.skipped.iter().map(|(field, why)| format!("{} ({})", field.label(), plain_paths(why, places))).collect(),
        backed_up_first: done.safety_backup.is_some(),
    }
}

/// "Put back the saves from 2026-10-30 14:22?": the saves now are backed up first (and where), then the folders get
/// the backup's saves. No is where the cursor starts, and what B does.
pub fn ask_page(f: &AskFacts) -> Page<RsAct> {
    let title = [format!("Put back the saves from {}?", f.when), format!("Put back saves from {}?", f.when)].into_iter().find(|t| t.chars().count() <= 40).unwrap_or_else(|| "Put back these saves?".to_string());
    let mut p = base("Restore saves: put back", &f.name, &title);
    p.paragraphs.push(format!("Your saves now are kept first: they are backed up to {}. If that backup does not work, nothing is changed.", f.kept_at));
    p.paragraphs.push(format!(
        "Then the {} below {} the saves from that backup ({}). Saves made since then are replaced; they stay in the backup made first, so you can put them back the same way.",
        if f.folders.len() == 1 { "folder" } else { "folders" },
        if f.folders.len() == 1 { "gets" } else { "get" },
        f.kind.chars().take(1).flat_map(char::to_lowercase).chain(f.kind.chars().skip(1)).collect::<String>()
    ));
    for folder in &f.folders {
        p.fact(Mark::ToDo, folder.clone());
    }
    for s in &f.skipped {
        p.paragraphs.push(format!("Not put back: {s}."));
    }
    p.paragraphs.push("Quit the game first.".to_string());
    p.options = vec![option("Yes, put them back", "Your saves now are backed up first", RsAct::Restore), option("No, keep my saves now", "Nothing changes", RsAct::Keep)];
    p.options[1].recommended = true;
    p.initial = Focus::Option(1);
    p.buttons = [None, None, None, None];
    p.b_does(&RsAct::Keep)
}

/// While the saves are put back: the step it is on. It cannot be stopped part way (B does nothing).
pub fn working_page(name: &str, step: &str) -> Page<RsAct> {
    let mut p = base("Restore saves: putting back", name, "Putting your saves back");
    p.paragraphs.push("Your saves now are backed up first. Do not turn the Deck off until this is done.".to_string());
    p.facts = step.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
    if p.facts.is_empty() {
        p.facts.push("Starting...".to_string());
    }
    p.buttons = [None, None, None, None];
    p.no_back = true;
    p.with_progress(None)
}

/// One folder that got the backup's saves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PutBack {
    /// The human way.
    pub folder: String,
    pub files: usize,
    /// The folder it replaced, when it was kept beside it (the human way).
    pub kept_old: Option<String>,
}

/// What the done page says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DoneFacts {
    pub name: String,
    pub when: String,
    pub folders: Vec<PutBack>,
    pub skipped: Vec<String>,
    /// The saves from before were backed up (there were some).
    pub backed_up_first: bool,
}

pub fn done_page(f: &DoneFacts) -> Page<RsAct> {
    let mut p = base("Restore saves: done", &f.name, "Your saves are back");
    p.paragraphs.push(format!("The saves from {} are back.", f.when));
    for d in &f.folders {
        p.fact(Mark::Done, format!("{}: {} {}", ellipsize(&d.folder, 70), d.files, if d.files == 1 { "file" } else { "files" }));
        if let Some(old) = &d.kept_old {
            p.paragraphs.push(format!("The folder it replaced is kept next to it: {old}."));
        }
    }
    for s in &f.skipped {
        p.paragraphs.push(format!("Not put back: {s}."));
    }
    p.paragraphs.push(if f.backed_up_first {
        format!("Your saves from before are in a backup. To go back to them, put back the newest one {}.", kind_words(BackupKind::BeforeRestore).to_lowercase())
    } else {
        "There were no saves to back up first.".to_string()
    });
    with_ok(p)
}

/// The restore did not happen, or stopped: what happened in plain words, every place the human way.
pub fn failed_page(name: &str, e: &RestoreError, places: &Places) -> Page<RsAct> {
    let title = match e {
        RestoreError::GameRunning(_) => "Close the game first",
        RestoreError::UnusableBackup(_) => "This backup cannot be used",
        RestoreError::NothingToRestore(_) => "Nothing to put back",
        RestoreError::RollbackFailed(_) => "The restore stopped part way",
        RestoreError::SafetyBackupFailed(_) | RestoreError::Failed(_) => "Your saves were not put back",
    };
    let mut p = notice_page(name, title, &plain_paths(&e.to_string(), places));
    p.window_title = "Restore saves: not done".to_string();
    if matches!(e, RestoreError::RollbackFailed(_)) {
        p.paragraphs.push(format!("Your saves from before are also in the backup made first: the newest one {}.", kind_words(BackupKind::BeforeRestore).to_lowercase()));
    }
    p
}

#[cfg(test)]
mod tests {
    use super::super::SaveField;
    use super::*;
    use crate::fork::page::{action_at, initial_focus};
    use crate::fork::rom_screens::settings_pages::tests::{assert_fits, assert_plain, shown};
    use std::path::PathBuf;
    use std::time::SystemTime;

    const LONG: &str = "The Legend of Zelda: The Wind Waker HD (ZeldaWWHDRecomp)";
    const PLACE: &str = "Internal storage > .local > share > Cemu > mlc01 > usr > save > 00050000 > 101c9400 > user";

    fn backup(name: &str, kind: BackupKind) -> Backup {
        Backup { kind, zip: PathBuf::from(format!("/home/deck/Saves Backup/{name}.zip")), prefix: String::new(), name: name.into(), modified: SystemTime::UNIX_EPOCH, size: 1_234_000, files: 3, fields: vec![SaveField::Save], problem: None }
    }

    fn places() -> Places {
        Places { home: Some(PathBuf::from("/home/deck")), media_roots: vec![PathBuf::from("/run/media/deck")], ..Places::default() }
    }

    fn all_pages() -> Vec<Page<RsAct>> {
        let mut bad = backup("2026-10-30_14-22-05-2", BackupKind::BeforeUpdate);
        bad.problem = Some("it holds a file name that points outside its folder (../x)".into());
        let backups = vec![backup("2026-10-30_14-22-05", BackupKind::BeforeUpdate), backup("2026-10-21", BackupKind::AllPorts), backup("2026-10-01_09-00-00", BackupKind::BeforeRestore), bad];
        let p = places();
        vec![
            ports_page(&[PortChoice { name: LONG.into(), backups: 12, newest: "2026-10-30 14:22 (2)".into(), known: true }, PortChoice { name: "gone".into(), backups: 1, newest: "x".into(), known: false }]),
            ports_page(&[]),
            backups_page(LONG, &backups, &[PLACE.into(), PLACE.into()], 2),
            backups_page("SoH", &[], &[], 0),
            notice_page(LONG, "Nothing to put back", &nothing_restorable_text(LONG, &[SaveTarget { field: SaveField::Save, target: Target::Folder("/home/deck/x".into()) }, SaveTarget { field: SaveField::Save2, target: Target::Refused("is the folder /home/deck itself".into()) }], &p)),
            ask_page(&AskFacts { name: LONG.into(), when: "2026-10-30 14:22 (2)".into(), kind: kind_words(BackupKind::AllPorts).into(), folders: vec![PLACE.into(), PLACE.into()], skipped: vec!["Save folder 2: no save folder is known for Linux".into()], kept_at: "Internal storage > Applications > Deck Ports Manager > Saves Backup > Restore Backups > zelda".into() }),
            working_page(LONG, "Backing up your current saves..."),
            working_page(LONG, ""),
            done_page(&DoneFacts { name: LONG.into(), when: "2026-10-30 14:22".into(), folders: vec![PutBack { folder: PLACE.into(), files: 1, kept_old: Some(PLACE.into()) }], skipped: vec!["x".into()], backed_up_first: true }),
            done_page(&DoneFacts { name: "SoH".into(), when: "2026-10-21".into(), folders: vec![], skipped: vec![], backed_up_first: false }),
            failed_page(LONG, &RestoreError::RollbackFailed("your previous saves are in /home/deck/.s.before-restore-1 (move them back to /home/deck/s)".into()), &p),
            failed_page(LONG, &RestoreError::Failed("Permission denied -- /run/media/deck/SD/Saves".into()), &p),
            failed_page(LONG, &RestoreError::GameRunning("cemu (process 12)".into()), &p),
        ]
    }

    #[test]
    fn every_restore_page_fits_and_says_it_plainly() {
        for p in all_pages() {
            assert_fits(&p);
            assert_plain(&p, &["ZeldaWWHDRecomp"]);
            assert!(p.window_title.starts_with("Restore saves: ") || p.window_title == SETTINGS_WINDOW, "{}", p.window_title);
            for t in shown(&p) {
                let lower = t.to_lowercase();
                for word in ["zip", "/run/media", "/home/", "~/", "(s)", "json", "--", "display()", "staging", "backup saves (all"] {
                    assert!(!lower.contains(word), "{word:?} in {t:?} ({})", p.title);
                }
            }
        }
    }

    #[test]
    fn the_backup_cards_have_the_date_as_title_and_kind_and_size_as_detail_newest_first() {
        let b = [backup("2026-10-30_14-22-05", BackupKind::BeforeUpdate), backup("2026-10-21", BackupKind::AllPorts)];
        let p = backups_page("Dusk", &b, &["Internal storage > Dusk > saves".into()], 0);
        assert_eq!(p.title, "Choose a backup");
        assert_eq!(p.header, "Dusk");
        assert_eq!((p.options[0].title.as_str(), p.options[0].detail.as_str()), ("2026-10-30 14:22", "Made before an update, 1.2 MB"));
        assert_eq!((p.options[1].title.as_str(), p.options[1].detail.as_str()), ("2026-10-21", "From Back up saves, 1.2 MB"));
        assert_eq!(p.paragraphs[0], "Your saves are in Internal storage > Dusk > saves.");
        assert_eq!(action_at(&p, initial_focus(&p)), Some(RsAct::Backup(0)));
        let stopped = backups_page("Dusk", &b, &[], 1);
        assert_eq!(stopped.options[0].action, RsAct::Interrupted(0));
        assert_eq!(action_at(&stopped, initial_focus(&stopped)), Some(RsAct::Backup(0)), "the cursor starts on the newest backup");
        assert_eq!(stopped.mark_of(0), Mark::Problem);
        let none = backups_page("Dusk", &[], &[], 0);
        assert_eq!(action_at(&none, initial_focus(&none)), Some(RsAct::Back));
        assert!(none.paragraphs.iter().any(|t| t.contains("by Back up saves")));
        assert_eq!(when_short(&backup("2026-10-21_14-05-00-2", BackupKind::BeforeUpdate)), "2026-10-21 14:05 (2)");
    }

    #[test]
    fn the_question_says_the_saves_now_are_kept_first_and_where_and_starts_on_no() {
        let p = ask_page(&AskFacts { name: "Dusk".into(), when: "2026-10-30 14:22".into(), kind: "Made before an update".into(), folders: vec!["Internal storage > Dusk > saves".into()], skipped: vec![], kept_at: "Internal storage > Saves Backup > Restore Backups > Dusk".into() });
        assert_eq!(p.title, "Put back saves from 2026-10-30 14:22?");
        assert_eq!(p.paragraphs[0], "Your saves now are kept first: they are backed up to Internal storage > Saves Backup > Restore Backups > Dusk. If that backup does not work, nothing is changed.");
        assert!(p.paragraphs[1].starts_with("Then the folder below gets the saves from that backup (made before an update)."), "{}", p.paragraphs[1]);
        assert_eq!(p.facts, ["Internal storage > Dusk > saves"]);
        assert_eq!(action_at(&p, initial_focus(&p)), Some(RsAct::Keep));
        assert_eq!(p.back_hint, "No, keep my saves now");
        assert_eq!(ask_page(&AskFacts { when: "2026-10-21".into(), folders: vec!["x".into()], ..AskFacts::default() }).title, "Put back the saves from 2026-10-21?");
    }

    #[test]
    fn the_done_and_failed_pages_say_where_things_are_the_human_way() {
        let d = done_page(&DoneFacts { name: "Dusk".into(), when: "2026-10-30 14:22".into(), folders: vec![PutBack { folder: "Internal storage > Dusk".into(), files: 2, kept_old: None }], skipped: vec![], backed_up_first: true });
        assert_eq!(d.title, "Your saves are back");
        assert_eq!(d.facts, ["Internal storage > Dusk: 2 files"]);
        assert!(d.paragraphs.last().unwrap().contains("put back the newest one made before a restore"));
        assert_eq!(d.back_hint, "OK");
        let f = failed_page("Dusk", &RestoreError::RollbackFailed("your previous saves are in /home/deck/a/.s.before-restore-1 (move them back to /home/deck/a/s)".into()), &places());
        assert_eq!(f.title, "The restore stopped part way");
        assert!(f.paragraphs[0].contains("Internal storage > a > .s.before-restore-1 (move them back to Internal storage > a > s)"), "{:?}", f.paragraphs);
        assert!(f.paragraphs[1].starts_with("Your saves from before are also in the backup made first"));
        let i = Interrupted { field: SaveField::Save, folder: "/home/deck/a/s".into(), path: "/home/deck/a/.s.before-restore-1".into(), holds_previous: true };
        let text = interrupted_text(&i, &places());
        assert!(text.contains("hidden folder next to the save folder: Internal storage > a > .s.before-restore-1.") && !text.contains("/home/"), "{text}");
    }
}
