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
    /// A revoked sign-in: drop the remembered account and go back to the first page.
    ForgetAccount,
    /// Open the sign-in page in this Deck's own browser (Desktop Mode).
    OpenOnDeck,
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

/// First page: not signed in. B (the Back button) is the way out.
pub fn intro_page() -> Page<Act> {
    let mut page = base("Keep a copy of your saves online?");
    page.paragraphs = vec!["Your save backups stay on this Deck. Cloud backup also copies them to your own Google Drive, so a lost or broken Deck doesn't lose them.".into()];
    page.facts = vec!["The app can only see the folder it makes in your Drive. It can't read your other files.".into()];
    page.options = vec![option("Sign in to Google Drive", "You finish on your phone. No password is typed here.", true, Act::SignIn)];
    page.buttons[LEFT] = button("Back", Act::Back);
    page
}

pub fn getting_code_page() -> Page<Act> {
    let mut page = base("Getting a sign-in code...");
    page.paragraphs = vec!["Asking Google for a short code. This takes a moment.".into()];
    page.buttons[LEFT] = button("Cancel", Act::Cancel);
    page.initial = Focus::Button(LEFT);
    page
}

/// The code to type and the QR code to scan. `minutes_left` is how long the code still works. The QR code opens
/// Google's page on the phone; the code still has to be typed there. The card comes first, big, with the facts
/// beside it.
pub fn signing_in_page(service: ServiceId, user_code: &str, qr: Option<QrMatrix>, minutes_left: u64, desktop: bool, waiting: &str) -> Page<Act> {
    let name = service.name();
    let mut page = base(format!("Sign in to {name}"));
    page.paragraphs = vec![format!(
        "On your phone, scan the code to open Google, then type {user_code}. Or go to {} and type it. This page moves on by itself when you are done. The code works for {minutes_left} more minute{}.",
        service.device_page(),
        if minutes_left == 1 { "" } else { "s" }
    )];
    page.card = Some(Card {
        art: None,
        qr,
        title: user_code.to_string(),
        lines: vec!["Scan to open Google on your phone, then type this code.".into(), waiting.to_string(), "The app can only see the folder it makes. It can't read your other files.".into()],
    });
    page.footnote = "If you press Cancel, a sign-in approved later on your phone is ignored.".into();
    page.buttons[LEFT] = button("Cancel", Act::Cancel);
    page.buttons[MIDDLE] = button("New code", Act::NewCode);
    if desktop {
        page.buttons[crate::fork::page::EXTRA] = button("Open on this Deck", Act::OpenOnDeck);
    }
    page.initial = Focus::Button(LEFT);
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

/// A failure of something the person just asked for (signing in, looking at the cloud, copying a backup back).
/// Nothing retries these by itself, so none of them says it will: the way on is Back, then try again.
pub fn one_off_text(service: ServiceId, kind: ErrorKind) -> String {
    let name = service.name();
    match kind {
        ErrorKind::Offline => format!("Could not reach {name}. Check the internet connection, then press Back and try again."),
        ErrorKind::SignedOut => format!("Signed out of {name}. Press Back, then sign in again."),
        ErrorKind::StorageFull => format!("Your {name} is full. Free up space, then press Back and try again."),
        ErrorKind::RateLimited => format!("{name} is busy. Press Back, then try again in a minute."),
        ErrorKind::ServiceDown => format!("{name} isn't answering. Press Back, then try again later."),
        ErrorKind::ClockWrong => "This Deck's clock looks wrong. Check the date and time, then press Back and try again.".to_string(),
        ErrorKind::Other => format!("{name} had a problem. Press Back, then try again."),
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

/// The page Settings > Cloud backup opens once signed in. A revoked sign-in says so, and offers to sign in again
/// or to forget the account.
pub fn status_page(account: &str, lines: Vec<String>, busy: bool, signed_out: bool) -> Page<Act> {
    let mut page = base("Cloud backup");
    let mut facts = vec![if signed_out { format!("Was signed in as {account}.") } else { format!("Signed in as {account}.") }];
    facts.extend(lines);
    page.facts = facts;
    if signed_out {
        page.options = vec![option("Sign in again", "You finish on your phone.", true, Act::SignIn), option("Forget this account", "Removes it from this Deck. Backups already online stay there.", false, Act::ForgetAccount)];
    } else {
        let backup = if busy {
            PageOption { title: "Backing up...".into(), detail: "Copying now. Please wait.".into(), choice: None, enabled: false, recommended: false, action: Act::BackUpNow }
        } else {
            option("Back up now", "Copies any backups that are not online yet.", true, Act::BackUpNow)
        };
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
    page
}

/// The date a backup's name says it was made (`2026-10-24.zip`, `20261024-101010.zip`).
pub fn name_date(name: &str) -> Option<chrono::NaiveDate> {
    let digits: String = name.chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
    let bytes = digits.as_bytes();
    let nums: Vec<u32> = if bytes.len() >= 10 && bytes[4] == b'-' && bytes[7] == b'-' {
        digits.split('-').take(3).filter_map(|p| p.parse().ok()).collect()
    } else if bytes.len() >= 8 && bytes.iter().take(8).all(u8::is_ascii_digit) {
        vec![digits[0..4].parse().unwrap_or(0), digits[4..6].parse().unwrap_or(0), digits[6..8].parse().unwrap_or(0)]
    } else {
        Vec::new()
    };
    match nums[..] {
        [y, m, d] => chrono::NaiveDate::from_ymd_opt(y as i32, m, d),
        _ => None,
    }
}

/// Newest first: by the date in the name, then by the time the service says the file changed, then by path (the
/// service's own time is when it was uploaded, which says little about a backup copied up late).
pub fn newest_first(files: &mut [RemoteFile]) {
    let date = |f: &RemoteFile| Place::parse(&f.path).and_then(|p| match p {
        Place::Global { name } | Place::Update { name, .. } | Place::Restore { name, .. } => name_date(&name),
    });
    files.sort_by(|a, b| date(b).cmp(&date(a)).then_with(|| b.modified.cmp(&a.modified)).then_with(|| b.path.cmp(&a.path)));
}

/// "Oct 24" from a backup's name (`2026-10-24.zip`, `20261024-101010.zip`), else from the service's time.
pub fn date_label(name: &str, modified: Option<u64>, offset_secs: i32) -> String {
    if let Some(date) = name_date(name) {
        return date.format("%b %-d").to_string();
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
/// bytes "Copy all my backups to this Deck" would bring (`None`: nothing is missing); it is the recommended choice
/// only on a Deck with no backups of its own (`copy_all_recommended`), otherwise the newest backup is.
pub fn restore_list_page(service: ServiceId, rows: &[(String, String)], copy_all: Option<(usize, u64)>, copy_all_recommended: bool, has_more: bool) -> Page<Act> {
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
    let all_first = copy_all.is_some() && copy_all_recommended;
    if let Some((files, bytes)) = copy_all {
        page.options.push(option("Copy all my backups to this Deck", format!("{files} file{}, {}. For a new Deck.", if files == 1 { "" } else { "s" }, human_size(bytes)), all_first, Act::CopyAll));
    }
    for (i, (label, detail)) in rows.iter().enumerate() {
        page.options.push(option(label.clone(), detail.clone(), i == 0 && !all_first, Act::Pick(i)));
    }
    if has_more {
        page.options.push(option("Show older", "", false, Act::ShowOlder));
    }
    // The cursor starts on the recommended row.
    page.initial = Focus::Option(if all_first { 0 } else { usize::from(copy_all.is_some()) });
    page.buttons[LEFT] = button("Back", Act::Back);
    page
}

pub fn copying_back_page(what: &str, progress: &str) -> Page<Act> {
    let mut page = base("Copying from the cloud...");
    page.paragraphs = vec![format!("Copying {what} to this Deck. Nothing on the Deck is replaced. Cancel stops after the file being copied right now.")];
    if !progress.is_empty() {
        page.facts = vec![progress.to_string()];
    }
    page.buttons[LEFT] = button("Cancel", Act::Cancel);
    page.initial = Focus::Button(LEFT);
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

/// After "Copy all": a summary, and the way on to Restore saves.
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
    page.options = vec![option("Restore saves", "Opens the list of backups on this Deck.", true, Act::RestoreSaves), option("Not now", "They stay in Restore saves for later.", false, Act::NotNow)];
    page.buttons[LEFT] = None;
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
    fn the_signing_in_page_leads_with_the_qr_card_and_words_it_honestly() {
        let qr = QrMatrix::encode("https://www.google.com/device");
        let page = signing_in_page(ServiceId::Google, "ABCD-EFGH", qr, 15, false, "Waiting for you to finish on your phone.");
        assert_eq!(page.title, "Sign in to Google Drive");
        let text = &page.paragraphs[0];
        assert!(text.starts_with("On your phone, scan the code to open Google, then type ABCD-EFGH."), "{text}");
        assert!(text.contains("go to google.com/device and type it") && text.contains("15 more minutes"), "{text}");
        let card = page.card.as_ref().unwrap();
        assert!(card.qr.is_some() && card.title == "ABCD-EFGH");
        assert!(card.lines[0].contains("Scan to open Google on your phone, then type this code"));
        assert!(card.lines.iter().any(|l| l.contains("Waiting for you")) && card.lines.iter().any(|l| l.contains("can't read your other files")));
        assert!(page.facts.is_empty(), "the card comes first, with its words beside it");
        assert!(page.footnote.contains("approved later on your phone is ignored"));
        assert!(page.buttons[LEFT].is_some() && page.buttons[MIDDLE].is_some() && page.buttons[crate::fork::page::EXTRA].is_none());
        let desktop = signing_in_page(ServiceId::Google, "A", None, 1, true, "w");
        assert!(desktop.buttons[crate::fork::page::EXTRA].is_some());
        assert!(desktop.paragraphs[0].contains("1 more minute."), "{:?}", desktop.paragraphs);
    }

    #[test]
    fn the_intro_has_one_choice_and_one_way_out() {
        let page = intro_page();
        assert_eq!(page.options.len(), 1);
        assert!(page.options[0].recommended && page.options[0].action == Act::SignIn);
        assert_eq!(page.buttons[LEFT].as_ref().map(|b| b.action.clone()), Some(Act::Back), "Back is the only way out, no second 'Not now'");
        assert!(page.paragraphs[0].contains("lost or broken Deck"));
    }

    #[test]
    fn waiting_pages_have_no_recommended_button_so_cancel_is_not_the_blue_one() {
        let waiting = [
            getting_code_page(),
            listing_page(ServiceId::Google),
            copying_back_page("x", "1 of 2"),
            signing_in_page(ServiceId::Google, "ABCD-EFGH", None, 15, false, "w"),
        ];
        for page in waiting {
            assert_eq!(page.primary, None, "{}", page.title);
            assert!(page.buttons[LEFT].as_ref().is_some_and(|b| b.action == Act::Cancel), "{}", page.title);
            assert_eq!(page.initial, Focus::Button(LEFT), "the cursor starts on Cancel");
        }
        assert!(copying_back_page("x", "").paragraphs[0].contains("Cancel stops after the file being copied right now"));
    }

    #[test]
    fn failures_of_one_off_actions_never_promise_a_retry() {
        for kind in [ErrorKind::Offline, ErrorKind::SignedOut, ErrorKind::StorageFull, ErrorKind::RateLimited, ErrorKind::ServiceDown, ErrorKind::ClockWrong, ErrorKind::Other] {
            let t = one_off_text(ServiceId::Google, kind);
            assert!(t.contains("Back") && !t.contains("by itself") && !t.contains("shortly") && !t.contains("Trying again"), "{kind:?}: {t}");
        }
        assert_eq!(one_off_text(ServiceId::Google, ErrorKind::Offline), "Could not reach Google Drive. Check the internet connection, then press Back and try again.");
    }

    #[test]
    fn the_restore_list_is_newest_first_by_the_date_in_the_name() {
        let f = |path: &str, modified: u64| RemoteFile { device: "d".into(), path: path.into(), size: 1, sha256: None, id: "i".into(), modified: Some(modified) };
        // Uploaded in the wrong order: the oldest backup went up last.
        let mut files = vec![
            f("Global Backups/2026-10-20.zip", 900),
            f("Update Backups/soh/20261023-090000.zip", 100),
            f("Global Backups/2026-10-22.zip", 500),
            f("Update Backups/soh/20261023-120000.zip", 200),
            f("Update Backups/odd/whatever.zip", 999),
        ];
        newest_first(&mut files);
        let order: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(order, ["Update Backups/soh/20261023-120000.zip", "Update Backups/soh/20261023-090000.zip", "Global Backups/2026-10-22.zip", "Global Backups/2026-10-20.zip", "Update Backups/odd/whatever.zip"]);
        assert_eq!(name_date("20261023-090000.zip"), chrono::NaiveDate::from_ymd_opt(2026, 10, 23));
        assert_eq!(name_date("2026-13-45.zip"), None);
    }

    #[test]
    fn the_status_page_offers_back_up_restore_and_sign_out_and_a_revoked_one_offers_sign_in_or_forgetting() {
        let page = status_page("t***@gmail.com", vec!["Google Drive: up to date.".into()], false, false);
        let acts: Vec<Act> = page.options.iter().map(|o| o.action.clone()).collect();
        assert_eq!(acts, [Act::BackUpNow, Act::GetBackups, Act::SignOutAsk]);
        assert!(page.facts[0] == "Signed in as t***@gmail.com.");
        let out = status_page("t***@gmail.com", vec![problem_text(ServiceId::Google, ErrorKind::SignedOut)], false, true);
        let acts: Vec<Act> = out.options.iter().map(|o| o.action.clone()).collect();
        assert_eq!(acts, [Act::SignIn, Act::ForgetAccount]);
        assert!(out.options[0].recommended && !out.options[1].recommended);
        assert_eq!(out.facts[0], "Was signed in as t***@gmail.com.");
        assert!(out.facts.iter().all(|f| !f.contains("Signed in as")), "{:?}", out.facts);
        let busy = status_page("a", vec![], true, false);
        assert!(busy.options[0].title.contains("Backing up") && !busy.options[0].enabled, "a running pass cannot be started again");
        let focus = crate::fork::page::focusables(&busy);
        assert!(!focus.contains(&Focus::Option(0)) && focus.contains(&Focus::Option(1)), "the cursor skips the disabled row");
        assert_eq!(crate::fork::page::initial_focus(&busy), crate::fork::page::focus_number(&busy, Focus::Option(1)));
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
    fn the_restore_list_recommends_copy_all_only_for_a_deck_with_no_backups_of_its_own() {
        let rows = vec![("All games, Oct 24".to_string(), "".to_string()), ("Ship, before update, Oct 20".to_string(), "".to_string())];
        let new_deck = restore_list_page(ServiceId::Google, &rows, Some((4, 3 * 1024 * 1024)), true, true);
        let acts: Vec<Act> = new_deck.options.iter().map(|o| o.action.clone()).collect();
        assert_eq!(acts, [Act::CopyAll, Act::Pick(0), Act::Pick(1), Act::ShowOlder]);
        assert_eq!(new_deck.options.iter().filter(|o| o.recommended).count(), 1);
        assert!(new_deck.options[0].recommended && new_deck.options[0].detail.starts_with("4 files, 3"), "{}", new_deck.options[0].detail);
        assert!(new_deck.paragraphs[0].contains("Nothing on the Deck is replaced"));
        assert_eq!(new_deck.initial, Focus::Option(0), "the cursor starts on Copy all");
        let old_deck = restore_list_page(ServiceId::Google, &rows, Some((1, 100)), false, false);
        assert_eq!(old_deck.initial, Focus::Option(1), "the cursor starts on the newest backup, past Copy all");
        assert!(!old_deck.options[0].recommended && old_deck.options[1].recommended, "the newest backup is the recommended one");
        assert_eq!(old_deck.options.iter().filter(|o| o.recommended).count(), 1);
        let none = restore_list_page(ServiceId::Google, &rows, None, false, false);
        assert!(none.options[0].recommended && none.options[0].action == Act::Pick(0));
        assert_eq!(none.initial, Focus::Option(0));
        let empty = restore_list_page(ServiceId::Google, &[], None, false, false);
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
    fn the_summary_counts_what_happened_and_goes_on_to_restore_saves() {
        let page = copied_all_page(3, 1, 2);
        assert_eq!(page.paragraphs[0], "3 backups copied to this Deck.");
        assert_eq!(page.paragraphs[1], "1 was already here.");
        assert!(page.paragraphs[2].contains("2 could not be copied"));
        assert_eq!(copied_all_page(1, 0, 0).paragraphs[0], "1 backup copied to this Deck.");
        let acts: Vec<Act> = page.options.iter().map(|o| o.action.clone()).collect();
        assert_eq!(acts, [Act::RestoreSaves, Act::NotNow]);
        assert!(page.options[0].recommended);
    }

    #[test]
    fn the_not_set_up_page_says_so_in_the_designs_words() {
        let page = not_set_up_page();
        assert_eq!(page.title, "Cloud backup isn't set up in this build");
        assert_eq!(page.title, format!("{}", super::super::build::NOT_SET_UP.trim_end_matches('.')));
    }
}
