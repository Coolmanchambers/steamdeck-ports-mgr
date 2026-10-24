//! The pages of Settings > Cloud backup, as plain data for `fork::page` (drawn by `app::page_dialog`). English
//! only, one question per page, big choices with the recommended one highlighted, no file paths (Tim's
//! "simple UI" rule). What the pages do is `src/app/cloud_screen.rs`.

use super::qr::QrMatrix;
use super::record::ServiceRecord;
use super::restore::Place;
use super::service::{ErrorKind, RemoteFile, ServiceId};
use crate::fork::page::{Card, Focus, Page, PageButton, PageOption, LEFT, MIDDLE, RIGHT};
use crate::fork::save_restore::view::human_size;
use chrono::{DateTime, Datelike, FixedOffset, TimeZone};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// "Sign in to Google Drive".
    SignIn,
    /// A new sign-in code.
    NewCode,
    /// Stop what is running and go back one step.
    Cancel,
    /// B.
    Back,
    BackUpNow,
    /// "Get backups from the cloud".
    GetBackups,
    /// "Copy all my backups to this Deck".
    CopyAll,
    /// A cloud backup on the restore list (index into the list).
    Pick(usize),
    ShowOlder,
    /// "Restore saves": hand over to the existing Restore saves screens.
    RestoreSaves,
    NotNow,
    /// "Sign out of Google Drive...": asks first.
    SignOutAsk,
    SignOut,
    KeepSignedIn,
    /// Open the sign-in page in this Deck's own browser (Desktop Mode).
    OpenOnDeck,
    Done,
}

const TITLE: &str = "Cloud backup";

fn base(title: impl Into<String>) -> Page<Act> {
    Page { window_title: TITLE.into(), large: true, header: TITLE.into(), title: title.into(), initial: Focus::Option(0), ..Page::default() }
}

fn option(title: impl Into<String>, detail: impl Into<String>, recommended: bool, action: Act) -> PageOption<Act> {
    PageOption { title: title.into(), detail: detail.into(), choice: None, enabled: true, recommended, action }
}

fn button(label: &str, action: Act) -> Option<PageButton<Act>> {
    Some(PageButton { label: label.into(), action })
}

/// A short message with one way out.
pub fn message_page(title: &str, paragraphs: Vec<String>, label: &str, act: Act) -> Page<Act> {
    let mut page = base(title);
    page.paragraphs = paragraphs;
    page.buttons[RIGHT] = button(label, act);
    page.primary = Some(RIGHT);
    page.initial = Focus::Button(RIGHT);
    page
}

/// The build has no Google client.
pub fn not_set_up_page() -> Page<Act> {
    message_page(
        "Cloud backup isn't set up in this build",
        vec!["This copy of the app was built without a Google Drive connection, so it cannot copy your saves online.".into(), "The builds on the project's releases page include it.".into()],
        "Back",
        Act::Back,
    )
}

/// First page: not signed in.
pub fn intro_page() -> Page<Act> {
    let mut page = base("Keep a copy of your saves online?");
    page.paragraphs = vec!["Your save backups stay on this Deck. Cloud backup also copies them to your own Google Drive, so a lost or broken Deck doesn't lose them.".into()];
    page.facts = vec!["The app can only see the folder it makes in your Drive. It can't read your other files.".into()];
    page.options = vec![option("Sign in to Google Drive", "You finish on your phone. No password is typed here.", true, Act::SignIn), option("Not now", "Nothing is copied anywhere.", false, Act::Back)];
    page.buttons[LEFT] = button("Back", Act::Back);
    page
}

pub fn getting_code_page() -> Page<Act> {
    let mut page = base("Getting a sign-in code...");
    page.paragraphs = vec!["Asking Google for a short code. This takes a moment.".into()];
    page.buttons[LEFT] = button("Cancel", Act::Cancel);
    page.initial = Focus::Button(LEFT);
    page.primary = Some(LEFT);
    page
}

/// The code to type and the QR code to scan. `minutes_left` is how long the code still works.
pub fn signing_in_page(service: ServiceId, user_code: &str, verification_url: &str, qr: Option<QrMatrix>, minutes_left: u64, desktop: bool, waiting: &str) -> Page<Act> {
    let name = service.name();
    let mut page = base(format!("Sign in to {name}"));
    page.paragraphs = vec![format!(
        "On your phone, scan the code, or go to {} and type {user_code}. This page moves on by itself when you are done. The code works for {minutes_left} more minute{}.",
        service.device_page(),
        if minutes_left == 1 { "" } else { "s" }
    )];
    page.card = Some(Card { art: None, qr, title: user_code.to_string(), lines: vec![format!("Scan, or open {verification_url} and type this code")] });
    page.facts = vec![waiting.to_string(), "The app can only see the folder it makes. It can't read your other files.".into()];
    page.buttons[LEFT] = button("Cancel", Act::Cancel);
    page.buttons[MIDDLE] = button("New code", Act::NewCode);
    if desktop {
        page.buttons[crate::fork::page::EXTRA] = button("Open on this Deck", Act::OpenOnDeck);
    }
    page.initial = Focus::Button(LEFT);
    page.primary = Some(LEFT);
    page
}

/// Signed in, with the offer to copy now.
pub fn signed_in_page(service: ServiceId, account: &str) -> Page<Act> {
    let mut page = base("Signed in");
    page.paragraphs = vec![format!("Signed in to {} as {account}. From now on, every save backup is copied there by itself while the app is open.", service.name())];
    page.options = vec![option("Back up now", "Copies the backups you already have.", true, Act::BackUpNow), option("Later", "", false, Act::NotNow)];
    page.buttons[LEFT] = None;
    page
}

/// "today, 3:41 PM", "yesterday, 3:41 PM", "Oct 24, 3:41 PM" in the given offset from UTC.
pub fn when(ts: u64, now: u64, offset_secs: i32) -> String {
    let offset = FixedOffset::east_opt(offset_secs).unwrap_or_else(|| FixedOffset::east_opt(0).expect("zero offset"));
    let at: DateTime<FixedOffset> = offset.timestamp_opt(ts as i64, 0).single().unwrap_or_else(|| offset.timestamp_opt(0, 0).unwrap());
    let today: DateTime<FixedOffset> = offset.timestamp_opt(now as i64, 0).single().unwrap_or(at);
    let time = at.format("%-I:%M %p").to_string();
    let day = at.date_naive();
    let this = today.date_naive();
    if day == this {
        format!("today, {time}")
    } else if this.pred_opt() == Some(day) {
        format!("yesterday, {time}")
    } else if day.year() == this.year() {
        format!("{}, {time}", at.format("%b %-d"))
    } else {
        format!("{}, {time}", at.format("%b %-d, %Y"))
    }
}

/// The plain message for a failure (section 5 of the design).
pub fn problem_text(service: ServiceId, kind: ErrorKind) -> String {
    let name = service.name();
    match kind {
        ErrorKind::Offline => format!("{name}: waiting for internet. It will try again by itself."),
        ErrorKind::SignedOut => format!("Signed out of {name}. Sign in again to keep copying."),
        ErrorKind::StorageFull => format!("Your {name} is full. Free up space, then press Back up now."),
        ErrorKind::RateLimited => format!("{name} is busy. Trying again shortly."),
        ErrorKind::ServiceDown => format!("{name} isn't answering. Trying again later."),
        ErrorKind::ClockWrong => "This Deck's clock looks wrong. Check the date and time.".to_string(),
        ErrorKind::Other => format!("{name} had a problem. Trying again later."),
    }
}

pub struct Status<'a> {
    pub service: ServiceId,
    pub record: &'a ServiceRecord,
    pub now: u64,
    pub offset_secs: i32,
    /// What a running pass is doing right now (`Copying ...`).
    pub running: Option<&'a str>,
}

/// What the card says about the service: one line for its state, one for the last copy.
pub fn status_lines(s: &Status) -> Vec<String> {
    let name = s.service.name();
    let rec = s.record;
    let mut lines = Vec::new();
    if let Some(doing) = s.running {
        lines.push(format!("{name}: {}", doing.trim_end_matches("...").trim()));
    } else if let Some((kind, _)) = rec.last_error {
        lines.push(problem_text(s.service, kind));
    } else if rec.last_ok.is_some() {
        lines.push(format!("{name}: up to date."));
    } else {
        lines.push(format!("{name}: nothing copied yet."));
    }
    if let Some(at) = rec.last_upload {
        lines.push(format!("Last copy {}.", when(at, s.now, s.offset_secs)));
    }
    lines
}

/// The page Settings > Cloud backup opens once signed in.
pub fn status_page(account: &str, lines: Vec<String>, busy: bool, signed_out: bool) -> Page<Act> {
    let mut page = base("Cloud backup");
    let mut facts = vec![format!("Signed in as {account}.")];
    facts.extend(lines);
    page.facts = facts;
    if signed_out {
        page.options = vec![option("Sign in again", "You finish on your phone.", true, Act::SignIn)];
    } else {
        let backup = if busy { option("Backing up...", "Copying now. Please wait.", true, Act::BackUpNow) } else { option("Back up now", "Copies any backups that are not online yet.", true, Act::BackUpNow) };
        page.options = vec![backup, option("Get backups from the cloud", "Copies one back to this Deck. Nothing here is replaced.", false, Act::GetBackups), option("Sign out of Google Drive...", "Backups already online stay there.", false, Act::SignOutAsk)];
    }
    page.buttons[LEFT] = button("Back", Act::Back);
    page
}

pub fn signing_out_page(service: ServiceId) -> Page<Act> {
    let mut page = base(format!("Sign out of {}?", service.name()));
    page.paragraphs = vec!["Backups already there stay there. This Deck stops copying, and you can sign in again any time.".into()];
    page.options = vec![option("Keep signed in", "", true, Act::KeepSignedIn), option("Sign out", "Removes the sign-in from this Deck.", false, Act::SignOut)];
    page.buttons[LEFT] = None;
    page
}

pub fn listing_page(service: ServiceId) -> Page<Act> {
    let mut page = base(format!("Looking at {}...", service.name()));
    page.paragraphs = vec!["Getting the list of backups. This takes a moment.".into()];
    page.buttons[LEFT] = button("Cancel", Act::Cancel);
    page.initial = Focus::Button(LEFT);
    page.primary = Some(LEFT);
    page
}

/// "Oct 24" from a backup's name (`2026-10-24.zip`, `20261024-101010.zip`), else from the service's time.
pub fn date_label(name: &str, modified: Option<u64>, offset_secs: i32) -> String {
    let digits: String = name.chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
    let nums: Vec<u32> = if digits.len() >= 10 && digits.as_bytes()[4] == b'-' && digits.as_bytes()[7] == b'-' {
        digits.split('-').take(3).filter_map(|p| p.parse().ok()).collect()
    } else if digits.len() >= 8 && digits.chars().take(8).all(|c| c.is_ascii_digit()) {
        vec![digits[0..4].parse().unwrap_or(0), digits[4..6].parse().unwrap_or(0), digits[6..8].parse().unwrap_or(0)]
    } else {
        Vec::new()
    };
    if let [y, m, d] = nums[..] {
        if let Some(date) = chrono::NaiveDate::from_ymd_opt(y as i32, m, d) {
            return date.format("%b %-d").to_string();
        }
    }
    let offset = FixedOffset::east_opt(offset_secs).unwrap_or_else(|| FixedOffset::east_opt(0).expect("zero offset"));
    modified.and_then(|ts| offset.timestamp_opt(ts as i64, 0).single()).map_or_else(|| "date unknown".to_string(), |t| t.format("%b %-d").to_string())
}

/// One row of the restore list: "All games, Oct 24" or "Ship of Harkinian, before update, Oct 20".
pub fn entry_label(file: &RemoteFile, place: &Place, port_name: Option<&str>, show_device: bool, offset_secs: i32) -> String {
    let name = match place {
        Place::Global { name } | Place::Update { name, .. } | Place::Restore { name, .. } => name.as_str(),
    };
    let date = date_label(name, file.modified, offset_secs);
    let what = match place {
        Place::Global { .. } => format!("All games, {date}"),
        Place::Update { folder, .. } => format!("{}, before update, {date}", port_name.unwrap_or(folder)),
        Place::Restore { folder, .. } => format!("{}, before restore, {date}", port_name.unwrap_or(folder)),
    };
    if show_device {
        format!("{what} (from {})", file.device)
    } else {
        what
    }
}

/// "Get a backup from the cloud": at most the rows given, newest first. `copy_all` is the number of files and
/// bytes "Copy all my backups to this Deck" would bring (`None`: nothing is missing).
pub fn restore_list_page(service: ServiceId, rows: &[(String, String)], copy_all: Option<(usize, u64)>, has_more: bool) -> Page<Act> {
    let mut page = base("Get a backup from the cloud");
    if rows.is_empty() && copy_all.is_none() {
        page.title = format!("No backups in {} yet", service.name());
        page.paragraphs = vec!["Nothing has been copied there yet. Press Back up now on the Cloud backup page first.".into()];
        page.buttons[RIGHT] = button("Back", Act::Back);
        page.primary = Some(RIGHT);
        page.initial = Focus::Button(RIGHT);
        return page;
    }
    page.paragraphs = vec!["Pick one to copy back to this Deck. Nothing on the Deck is replaced; you choose what to restore next.".into()];
    if let Some((files, bytes)) = copy_all {
        page.options.push(option("Copy all my backups to this Deck", format!("{files} file{}, {}. For a new Deck.", if files == 1 { "" } else { "s" }, human_size(bytes)), true, Act::CopyAll));
    }
    for (i, (label, detail)) in rows.iter().enumerate() {
        page.options.push(option(label.clone(), detail.clone(), i == 0 && copy_all.is_none(), Act::Pick(i)));
    }
    if has_more {
        page.options.push(option("Show older", "", false, Act::ShowOlder));
    }
    page.buttons[LEFT] = button("Back", Act::Back);
    page
}

pub fn copying_back_page(what: &str, progress: &str) -> Page<Act> {
    let mut page = base("Copying from the cloud...");
    page.paragraphs = vec![format!("Copying {what} to this Deck. Nothing on the Deck is replaced.")];
    if !progress.is_empty() {
        page.facts = vec![progress.to_string()];
    }
    page.buttons[LEFT] = button("Cancel", Act::Cancel);
    page.initial = Focus::Button(LEFT);
    page.primary = Some(LEFT);
    page
}

/// After one backup came back: offer the existing Restore saves.
pub fn copied_page(what: &str, port_name: Option<&str>, kept_both: bool) -> Page<Act> {
    let mut page = base("Copied");
    page.paragraphs = vec![match port_name {
        Some(name) => format!("Copied {what} to this Deck. Restore saves for {name} now?"),
        None => format!("Copied {what} to this Deck. Open Restore saves to put it back?"),
    }];
    if kept_both {
        page.facts = vec!["A different file with the same name was already here, so both are kept.".into()];
    }
    page.options = vec![option("Restore saves", "Opens the list of backups, with this one in it.", true, Act::RestoreSaves), option("Not now", "It stays in Restore saves for later.", false, Act::NotNow)];
    page.buttons[LEFT] = None;
    page
}

/// After "Copy all": a summary.
pub fn copied_all_page(copied: usize, already: usize, failed: usize) -> Page<Act> {
    let mut page = base("Copied");
    let mut lines = vec![format!("{copied} backup{} copied to this Deck.", if copied == 1 { "" } else { "s" })];
    if already > 0 {
        lines.push(format!("{already} {} already here.", if already == 1 { "was" } else { "were" }));
    }
    if failed > 0 {
        lines.push(format!("{failed} could not be copied. Nothing was replaced; you can try again."));
    }
    page.paragraphs = lines;
    page.paragraphs.push("Open Settings > Restore saves to put one back.".into());
    page.buttons[RIGHT] = button("Done", Act::Done);
    page.primary = Some(RIGHT);
    page.initial = Focus::Button(RIGHT);
    page
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::cloud::record::ServiceRecord;
    use crate::fork::page::{action_at, focusables};

    const NOW: u64 = 1_800_000_000; // 2027-01-15 08:00:00 UTC

    fn remote(path: &str, modified: Option<u64>, device: &str) -> RemoteFile {
        RemoteFile { device: device.into(), path: path.into(), size: 1, sha256: None, id: "i".into(), modified }
    }

    #[test]
    fn times_read_as_today_yesterday_and_dates() {
        assert_eq!(when(NOW - 60, NOW, 0), "today, 7:59 AM");
        assert_eq!(when(NOW - 86_400, NOW, 0), "yesterday, 8:00 AM");
        assert_eq!(when(NOW - 5 * 86_400, NOW, 0), "Jan 10, 8:00 AM");
        assert_eq!(when(NOW - 400 * 86_400, NOW, 0), "Dec 11, 2025, 8:00 AM");
        assert_eq!(when(NOW, NOW, -7 * 3600), "today, 1:00 AM", "the offset is the Deck's own");
        assert_eq!(when(NOW - 3600, NOW, 5 * 3600 + 1800), "today, 12:30 PM");
    }

    #[test]
    fn backup_dates_come_from_the_name_first() {
        assert_eq!(date_label("2026-10-24.zip", None, 0), "Oct 24");
        assert_eq!(date_label("20261020-101010.zip", None, 0), "Oct 20");
        assert_eq!(date_label("2026-10-24-cloud.zip", None, 0), "Oct 24");
        assert_eq!(date_label("whatever.zip", Some(NOW), 0), "Jan 15");
        assert_eq!(date_label("whatever.zip", None, 0), "date unknown");
        assert_eq!(date_label("2026-13-45.zip", Some(NOW), 0), "Jan 15", "an impossible date falls back");
    }

    #[test]
    fn rows_say_what_and_when_without_paths() {
        let g = remote("Global Backups/2026-10-24.zip", None, "deck");
        let u = remote("Update Backups/soh/20261020-101010.zip", None, "deck");
        let r = remote("Restore Backups/soh/20261021-101010.zip", None, "other");
        let place = |f: &RemoteFile| Place::parse(&f.path).unwrap();
        assert_eq!(entry_label(&g, &place(&g), None, false, 0), "All games, Oct 24");
        assert_eq!(entry_label(&u, &place(&u), Some("Ship of Harkinian"), false, 0), "Ship of Harkinian, before update, Oct 20");
        assert_eq!(entry_label(&u, &place(&u), None, false, 0), "soh, before update, Oct 20");
        assert_eq!(entry_label(&r, &place(&r), Some("Ship of Harkinian"), true, 0), "Ship of Harkinian, before restore, Oct 21 (from other)");
        for l in [entry_label(&g, &place(&g), None, false, 0), entry_label(&u, &place(&u), None, false, 0)] {
            assert!(!l.contains('/') && !l.contains(".zip"), "{l}");
        }
    }

    #[test]
    fn every_failure_has_a_plain_message_and_the_clock_one_names_no_service() {
        for kind in [ErrorKind::Offline, ErrorKind::SignedOut, ErrorKind::StorageFull, ErrorKind::RateLimited, ErrorKind::ServiceDown, ErrorKind::ClockWrong, ErrorKind::Other] {
            let t = problem_text(ServiceId::Google, kind);
            assert!(!t.is_empty() && !t.contains("HTTP") && !t.contains("error"), "{kind:?}: {t}");
        }
        assert_eq!(problem_text(ServiceId::Google, ErrorKind::Offline), "Google Drive: waiting for internet. It will try again by itself.");
        assert_eq!(problem_text(ServiceId::Google, ErrorKind::SignedOut), "Signed out of Google Drive. Sign in again to keep copying.");
        assert_eq!(problem_text(ServiceId::Google, ErrorKind::StorageFull), "Your Google Drive is full. Free up space, then press Back up now.");
        assert_eq!(problem_text(ServiceId::Google, ErrorKind::ClockWrong), "This Deck's clock looks wrong. Check the date and time.");
    }

    #[test]
    fn status_lines_cover_each_state() {
        let mut rec = ServiceRecord::default();
        let line = |rec: &ServiceRecord, running: Option<&str>| status_lines(&Status { service: ServiceId::Google, record: rec, now: NOW, offset_secs: 0, running });
        assert_eq!(line(&rec, None), ["Google Drive: nothing copied yet."]);
        rec.last_ok = Some(NOW - 10);
        rec.last_upload = Some(NOW - 600);
        assert_eq!(line(&rec, None), ["Google Drive: up to date.", "Last copy today, 7:50 AM."]);
        assert_eq!(line(&rec, Some("Copying a.zip to Google Drive")), ["Google Drive: Copying a.zip to Google Drive", "Last copy today, 7:50 AM."]);
        rec.last_error = Some((ErrorKind::Offline, NOW));
        assert_eq!(line(&rec, None)[0], "Google Drive: waiting for internet. It will try again by itself.");
    }

    #[test]
    fn the_signing_in_page_shows_the_code_the_address_and_the_qr_card() {
        let qr = QrMatrix::encode("https://www.google.com/device");
        let page = signing_in_page(ServiceId::Google, "ABCD-EFGH", "https://www.google.com/device", qr, 15, false, "Waiting for you to finish on your phone.");
        assert_eq!(page.title, "Sign in to Google Drive");
        assert!(page.paragraphs[0].contains("google.com/device") && page.paragraphs[0].contains("ABCD-EFGH") && page.paragraphs[0].contains("15 more minutes"), "{:?}", page.paragraphs);
        let card = page.card.as_ref().unwrap();
        assert!(card.qr.is_some() && card.title == "ABCD-EFGH");
        assert!(page.buttons[LEFT].is_some() && page.buttons[MIDDLE].is_some() && page.buttons[crate::fork::page::EXTRA].is_none());
        assert!(page.facts.iter().any(|f| f.contains("can't read your other files")));
        let desktop = signing_in_page(ServiceId::Google, "A", "u", None, 1, true, "w");
        assert!(desktop.buttons[crate::fork::page::EXTRA].is_some());
        assert!(desktop.paragraphs[0].contains("1 more minute."), "{:?}", desktop.paragraphs);
    }

    #[test]
    fn the_intro_has_one_recommended_choice_and_a_way_out() {
        let page = intro_page();
        assert!(page.options[0].recommended && page.options[0].action == Act::SignIn);
        assert_eq!(page.options[1].action, Act::Back);
        assert!(page.paragraphs[0].contains("lost or broken Deck"));
    }

    #[test]
    fn the_status_page_offers_back_up_restore_and_sign_out_and_a_revoked_one_offers_sign_in() {
        let page = status_page("t***@gmail.com", vec!["Google Drive: up to date.".into()], false, false);
        let acts: Vec<Act> = page.options.iter().map(|o| o.action.clone()).collect();
        assert_eq!(acts, [Act::BackUpNow, Act::GetBackups, Act::SignOutAsk]);
        assert!(page.facts[0].contains("t***@gmail.com") && !page.facts[0].contains("example"));
        let out = status_page("t***@gmail.com", vec![problem_text(ServiceId::Google, ErrorKind::SignedOut)], false, true);
        assert_eq!(out.options.len(), 1);
        assert_eq!(out.options[0].action, Act::SignIn);
        assert!(status_page("a", vec![], true, false).options[0].title.contains("Backing up"));
    }

    #[test]
    fn sign_out_asks_first_and_keeping_is_the_recommended_answer() {
        let page = signing_out_page(ServiceId::Google);
        assert_eq!(page.title, "Sign out of Google Drive?");
        assert!(page.paragraphs[0].contains("Backups already there stay there"));
        assert!(page.options[0].recommended && page.options[0].action == Act::KeepSignedIn);
        assert_eq!(page.options[1].action, Act::SignOut);
    }

    #[test]
    fn the_restore_list_leads_with_copy_all_and_marks_one_recommended() {
        let rows = vec![("All games, Oct 24".to_string(), "".to_string()), ("Ship, before update, Oct 20".to_string(), "".to_string())];
        let page = restore_list_page(ServiceId::Google, &rows, Some((4, 3 * 1024 * 1024)), true);
        let acts: Vec<Act> = page.options.iter().map(|o| o.action.clone()).collect();
        assert_eq!(acts, [Act::CopyAll, Act::Pick(0), Act::Pick(1), Act::ShowOlder]);
        assert_eq!(page.options.iter().filter(|o| o.recommended).count(), 1);
        assert!(page.options[0].detail.starts_with("4 files, 3"), "{}", page.options[0].detail);
        assert!(page.paragraphs[0].contains("Nothing on the Deck is replaced"));
        let none = restore_list_page(ServiceId::Google, &rows, None, false);
        assert!(none.options[0].recommended && none.options[0].action == Act::Pick(0));
        let empty = restore_list_page(ServiceId::Google, &[], None, false);
        assert!(empty.title.contains("No backups") && empty.options.is_empty());
    }

    #[test]
    fn copied_offers_restore_saves_and_says_when_both_files_were_kept() {
        let page = copied_page("Ship of Harkinian, before update, Oct 20", Some("Ship of Harkinian"), true);
        assert_eq!(action_at(&page, 0), Some(Act::RestoreSaves));
        assert_eq!(page.paragraphs[0], "Copied Ship of Harkinian, before update, Oct 20 to this Deck. Restore saves for Ship of Harkinian now?");
        assert!(page.facts[0].contains("both are kept"));
        assert!(!copied_page("x", None, false).paragraphs[0].is_empty());
        assert!(focusables(&page).len() >= 2);
    }

    #[test]
    fn the_summary_counts_what_happened() {
        let page = copied_all_page(3, 1, 2);
        assert_eq!(page.paragraphs[0], "3 backups copied to this Deck.");
        assert_eq!(page.paragraphs[1], "1 was already here.");
        assert!(page.paragraphs[2].contains("2 could not be copied"));
        assert_eq!(copied_all_page(1, 0, 0).paragraphs[0], "1 backup copied to this Deck.");
    }

    #[test]
    fn the_not_set_up_page_says_so_in_the_designs_words() {
        let page = not_set_up_page();
        assert_eq!(page.title, "Cloud backup isn't set up in this build");
        assert_eq!(page.title, format!("{}", super::super::build::NOT_SET_UP.trim_end_matches('.')));
    }
}
