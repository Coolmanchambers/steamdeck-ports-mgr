//! When an upload pass runs, as plain decisions the app's screens call (and tests check).
//!
//! A pass starts: right after each backup the app makes (Backup Saves, before an update, before a restore,
//! the save finder's Back up now), when the app starts (the catch-up for anything not yet uploaded, which
//! is also the queue after being offline), on "Back up now", on a timer after a failure (5, 15, then 60
//! minutes, or what the service asked), shortly after a zip that was not ready, and every ten minutes while
//! the app is open as a safety net. Nothing runs while the app is closed.

use super::net::backoff;
use super::service::ErrorKind;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// Settings > Backup Saves finished.
    SaveBackup,
    /// A backup made before an update (install finished or failed), an old copy's removal or a version change.
    UpdateBackup,
    /// The safety backup Restore saves makes before a restore.
    RestoreBackup,
    /// The save finder's "Back up now".
    FinderBackup,
    /// The person pressed Back up now.
    Manual,
    /// The app started.
    Start,
    /// The retry timer after a failure.
    Retry,
    /// A zip was not ready on the last pass.
    Deferred,
    /// The ten-minute check.
    Safety,
}

impl Why {
    /// A zip changed less than this many seconds ago waits for the next pass. A backup the app has just
    /// reported finished is complete (its zip is read back before it is reported), so those wait only a moment;
    /// a pass that nobody asked for after a specific backup waits a minute in case one is being written.
    pub fn min_age_secs(self) -> u64 {
        match self {
            Why::SaveBackup | Why::UpdateBackup | Why::RestoreBackup | Why::FinderBackup | Why::Manual => 3,
            Why::Start | Why::Retry | Why::Deferred | Why::Safety => 60,
        }
    }

    /// Whether the person asked for this pass (so its result is shown even when nothing needed copying).
    pub fn is_manual(self) -> bool {
        self == Why::Manual
    }
}

/// How long to wait before the next safety pass.
pub const SAFETY_EVERY: Duration = Duration::from_secs(10 * 60);

/// How long to wait before looking again at zips that were not ready.
pub const DEFERRED_AFTER: Duration = Duration::from_secs(70);

/// A pass runs only for a build that has a Google client and a person who is signed in.
pub fn should_run(client_present: bool, signed_in: bool) -> bool {
    client_present && signed_in
}

/// What to do after a pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// Nothing; wait for the next backup, the safety timer or the person.
    Wait,
    /// Run again after this long, for this reason.
    After(Duration, Why),
}

/// `failures_in_a_row` counts the failed passes before this one that retry by themselves.
pub fn after_pass(error: Option<(ErrorKind, Option<u64>)>, deferred: usize, failures_in_a_row: usize) -> Next {
    match error {
        Some((kind, retry_after)) if kind.retries_by_itself() => Next::After(backoff(failures_in_a_row, retry_after), Why::Retry),
        // Signed out, storage full, clock wrong: only the person can fix it (Back up now tries again).
        Some(_) => Next::Wait,
        None if deferred > 0 => Next::After(DEFERRED_AFTER, Why::Deferred),
        None => Next::Wait,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::cloud::fake::FakeService;
    use crate::fork::cloud::local::testing::{set_mtime, write_zip};
    use crate::fork::cloud::record::ServiceRecord;
    use crate::fork::cloud::sync::{run_pass, PassInput};

    const NOW: u64 = 2_000_000_000;

    #[test]
    fn only_a_build_with_a_client_and_a_signed_in_person_runs() {
        assert!(should_run(true, true));
        assert!(!should_run(false, true));
        assert!(!should_run(true, false));
    }

    #[test]
    fn retries_follow_5_15_60_minutes_and_the_services_own_wait() {
        let offline = |n| after_pass(Some((ErrorKind::Offline, None)), 0, n);
        assert_eq!(offline(0), Next::After(Duration::from_secs(300), Why::Retry));
        assert_eq!(offline(1), Next::After(Duration::from_secs(900), Why::Retry));
        assert_eq!(offline(2), Next::After(Duration::from_secs(3_600), Why::Retry));
        assert_eq!(after_pass(Some((ErrorKind::RateLimited, Some(40))), 0, 0), Next::After(Duration::from_secs(40), Why::Retry));
        assert_eq!(after_pass(Some((ErrorKind::ServiceDown, None)), 0, 1), Next::After(Duration::from_secs(900), Why::Retry));
    }

    #[test]
    fn failures_only_a_person_can_fix_do_not_retry() {
        for kind in [ErrorKind::SignedOut, ErrorKind::StorageFull, ErrorKind::ClockWrong] {
            assert_eq!(after_pass(Some((kind, None)), 0, 0), Next::Wait, "{kind:?}");
        }
    }

    #[test]
    fn a_clean_pass_waits_unless_a_zip_was_not_ready() {
        assert_eq!(after_pass(None, 0, 0), Next::Wait);
        assert_eq!(after_pass(None, 2, 0), Next::After(DEFERRED_AFTER, Why::Deferred));
    }

    #[test]
    fn a_pass_for_each_kind_of_backup_uploads_it() {
        // The three places the app writes backups, each just finished (a few seconds old): a pass started by
        // the matching hook picks it up straight away; one started by the timer would wait a minute.
        let dir = std::env::temp_dir().join(format!("dpm_cloud_hooks_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cases = [(Why::SaveBackup, "Global Backups/20261024.zip"), (Why::UpdateBackup, "Update Backups/soh/a.zip"), (Why::RestoreBackup, "Restore Backups/soh/b.zip"), (Why::FinderBackup, "Update Backups/soh/c.zip")];
        for (why, rel) in cases {
            let path = dir.join(rel);
            write_zip(&path, "a.sav", rel.as_bytes());
            set_mtime(&path, NOW - 10);
            let (mut fake, mut rec) = (FakeService::new(), ServiceRecord::default());
            let input = PassInput { saves_backup_dir: &dir, device: "deck", now: NOW, min_age: why.min_age_secs() };
            let report = run_pass(&mut fake, &mut rec, &input, &|| false, &mut |_| {}, &mut |_| {});
            assert!(report.uploaded >= 1 && fake.uploads().iter().any(|u| u.ends_with(rel)), "{why:?}: {report:?}");
            let timer = PassInput { min_age: Why::Safety.min_age_secs(), ..input };
            let (mut fake2, mut rec2) = (FakeService::new(), ServiceRecord::default());
            let again = run_pass(&mut fake2, &mut rec2, &timer, &|| false, &mut |_| {}, &mut |_| {});
            assert!(!fake2.uploads().iter().any(|u| u.ends_with(rel)) && again.deferred >= 1, "{why:?} would not wait for the timer");
            std::fs::remove_file(&path).unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manual_is_the_only_pass_the_person_asked_for() {
        for why in [Why::SaveBackup, Why::UpdateBackup, Why::RestoreBackup, Why::FinderBackup, Why::Start, Why::Retry, Why::Deferred, Why::Safety] {
            assert!(!why.is_manual());
        }
        assert!(Why::Manual.is_manual());
    }
}
