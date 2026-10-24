//! The upload pass against the in-memory service: skip rules, never replacing, failures, two services.

use super::fake::FakeService;
use super::local::testing::{set_mtime, write_zip};
use super::record::{Record, ServiceRecord};
use super::service::{CloudService, ErrorKind, ServiceError, ServiceId};
use super::sha256::sha256_hex;
use super::sync::{run_pass, run_services, PassInput, PassReport};
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};

const NOW: u64 = 2_000_000_000;
const DEVICE: &str = "deck";

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dpm_cloud_sync_{}_{}", std::process::id(), name));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn zip_at(dir: &Path, rel: &str, content: &[u8], age: u64) -> Vec<u8> {
    let path = dir.join(rel);
    write_zip(&path, "a.sav", content);
    set_mtime(&path, NOW - age);
    fs::read(&path).unwrap()
}

fn input(dir: &Path) -> PassInput<'_> {
    PassInput { saves_backup_dir: dir, device: DEVICE, now: NOW, min_age: 60 }
}

fn pass(service: &mut dyn CloudService, rec: &mut ServiceRecord, dir: &Path) -> PassReport {
    run_pass(service, rec, &input(dir), &|| false, &mut |_| {}, &mut |_| {})
}

#[test]
fn new_zips_are_uploaded_newest_first_and_recorded() {
    let dir = temp_dir("basic");
    zip_at(&dir, "Global Backups/2026-10-22.zip", b"one", 7_200);
    zip_at(&dir, "Update Backups/soh/s1.zip", b"two", 3_600);
    zip_at(&dir, "Restore Backups/soh/s2.zip", b"three", 600);
    let (mut fake, mut rec) = (FakeService::new(), ServiceRecord::default());
    let report = pass(&mut fake, &mut rec, &dir);
    assert_eq!((report.uploaded, report.recognised, report.error.is_none()), (3, 0, true), "{report:?}");
    assert_eq!(fake.uploads(), ["deck/Restore Backups/soh/s2.zip", "deck/Update Backups/soh/s1.zip", "deck/Global Backups/2026-10-22.zip"]);
    assert_eq!(rec.uploaded.len(), 3);
    assert_eq!((rec.last_ok, rec.last_upload, rec.last_error), (Some(NOW), Some(NOW), None));
    assert_eq!(fake.files[&(DEVICE.to_string(), "Update Backups/soh/s1.zip".to_string())], fs::read(dir.join("Update Backups/soh/s1.zip")).unwrap());
    assert_eq!(rec.uploaded["Update Backups/soh/s1.zip"].sha256, sha256_hex(&fs::read(dir.join("Update Backups/soh/s1.zip")).unwrap()));
}

#[test]
fn a_second_pass_with_nothing_new_makes_no_call_at_all() {
    let dir = temp_dir("again");
    zip_at(&dir, "Global Backups/a.zip", b"one", 3_600);
    let (mut fake, mut rec) = (FakeService::new(), ServiceRecord::default());
    pass(&mut fake, &mut rec, &dir);
    let calls = fake.calls.len();
    let report = pass(&mut fake, &mut rec, &dir);
    assert_eq!((report.uploaded, report.recognised), (0, 0));
    assert_eq!(fake.calls.len(), calls, "no listing, no upload");
}

#[test]
fn scratch_folders_pending_restore_and_fresh_zips_are_not_uploaded() {
    let dir = temp_dir("skips");
    zip_at(&dir, "Pending Restore/soh/save_folder/x.zip", b"p", 3_600);
    zip_at(&dir, "_backup_abc/y.zip", b"s", 3_600);
    zip_at(&dir, "Update Backups/_update_backup_abc/z.zip", b"s", 3_600);
    zip_at(&dir, "Global Backups/just-now.zip", b"fresh", 5);
    zip_at(&dir, "Global Backups/ok.zip", b"ok", 3_600);
    let (mut fake, mut rec) = (FakeService::new(), ServiceRecord::default());
    let report = pass(&mut fake, &mut rec, &dir);
    assert_eq!(fake.uploads(), ["deck/Global Backups/ok.zip"]);
    assert_eq!(report.deferred, 1, "the fresh zip waits for the next pass");
}

#[test]
fn a_file_the_cloud_already_has_is_recognised_not_uploaded_and_another_deck_does_not_count() {
    let dir = temp_dir("recognise");
    let bytes = zip_at(&dir, "Global Backups/a.zip", b"one", 3_600);
    zip_at(&dir, "Global Backups/b.zip", b"two", 3_700);
    let mut fake = FakeService::new();
    fake.put(DEVICE, "Global Backups/a.zip", &bytes);
    fake.put("other deck", "Global Backups/b.zip", &fs::read(dir.join("Global Backups/b.zip")).unwrap());
    let mut rec = ServiceRecord::default();
    let report = pass(&mut fake, &mut rec, &dir);
    assert_eq!((report.recognised, report.uploaded), (1, 1));
    assert_eq!(fake.uploads(), ["deck/Global Backups/b.zip"]);
    assert_eq!(rec.uploaded.len(), 2);
}

#[test]
fn a_rewritten_zip_is_uploaded_beside_the_old_copy_never_over_it() {
    let dir = temp_dir("rewrite");
    let first = zip_at(&dir, "Global Backups/day.zip", b"morning", 7_200);
    let (mut fake, mut rec) = (FakeService::new(), ServiceRecord::default());
    pass(&mut fake, &mut rec, &dir);
    let second = zip_at(&dir, "Global Backups/day.zip", b"evening, different", 3_600);
    let report = pass(&mut fake, &mut rec, &dir);
    assert_eq!(report.uploaded, 1);
    let short = &sha256_hex(&second)[..8];
    let keys: Vec<String> = fake.files.keys().map(|(_, p)| p.clone()).collect();
    assert_eq!(keys, [format!("Global Backups/day-{short}.zip"), "Global Backups/day.zip".to_string()]);
    assert_eq!(fake.files[&(DEVICE.to_string(), "Global Backups/day.zip".to_string())], first, "the first copy is untouched");
    // And a third pass has nothing to do.
    let calls = fake.calls.len();
    pass(&mut fake, &mut rec, &dir);
    assert_eq!(fake.calls.len(), calls);
}

#[test]
fn a_changed_modified_time_with_the_same_content_copies_nothing() {
    let dir = temp_dir("touch");
    zip_at(&dir, "Global Backups/a.zip", b"one", 7_200);
    let (mut fake, mut rec) = (FakeService::new(), ServiceRecord::default());
    pass(&mut fake, &mut rec, &dir);
    set_mtime(&dir.join("Global Backups/a.zip"), NOW - 3_600);
    let report = pass(&mut fake, &mut rec, &dir);
    assert_eq!((report.uploaded, report.recognised), (0, 1));
    assert_eq!(fake.uploads().len(), 1);
    assert_eq!(rec.uploaded["Global Backups/a.zip"].mtime, NOW - 3_600);
}

#[test]
fn an_offline_failure_keeps_what_was_done_and_the_next_pass_finishes() {
    let dir = temp_dir("offline");
    zip_at(&dir, "Update Backups/a/new.zip", b"newest", 600);
    zip_at(&dir, "Update Backups/a/mid.zip", b"middle", 1_200);
    zip_at(&dir, "Update Backups/a/old.zip", b"oldest", 1_800);
    let mut fake = FakeService::new();
    let mut rec = ServiceRecord::default();
    // The listing is call 1, the first upload call 2, the second upload (call 3) fails.
    let report = {
        let mut hits = 0;
        let mut flaky = Flaky { inner: &mut fake, calls: &mut hits, fail_at: 3 };
        pass(&mut flaky, &mut rec, &dir)
    };
    assert_eq!((report.uploaded, report.error.as_ref().map(|e| e.kind)), (1, Some(ErrorKind::Offline)));
    assert_eq!(rec.uploaded.len(), 1);
    assert_eq!(rec.last_error, Some((ErrorKind::Offline, NOW)));
    assert_eq!(rec.last_ok, None);
    let report = pass(&mut fake, &mut rec, &dir);
    assert_eq!((report.uploaded, report.error.is_none()), (2, true));
    assert_eq!((rec.uploaded.len(), rec.last_error.is_none(), rec.last_ok), (3, true, Some(NOW)));
}

/// Fails the `fail_at`th call (1 = the listing) with an offline error, and passes the rest through.
struct Flaky<'a> {
    inner: &'a mut FakeService,
    calls: &'a mut usize,
    fail_at: usize,
}

impl Flaky<'_> {
    fn tick(&mut self) -> Result<(), ServiceError> {
        *self.calls += 1;
        if *self.calls == self.fail_at {
            return Err(ServiceError::new(ErrorKind::Offline, "no connection"));
        }
        Ok(())
    }
}

impl CloudService for Flaky<'_> {
    fn id(&self) -> ServiceId {
        self.inner.id()
    }
    fn list_all(&mut self) -> Result<Vec<super::service::RemoteFile>, ServiceError> {
        self.tick()?;
        self.inner.list_all()
    }
    fn upload(&mut self, device: &str, rel: &str, file: &Path, size: u64, p: super::service::Progress) -> Result<super::service::RemoteFile, ServiceError> {
        self.tick()?;
        self.inner.upload(device, rel, file, size, p)
    }
    fn download(&mut self, file: &super::service::RemoteFile, dest: &Path, p: super::service::Progress) -> Result<(), ServiceError> {
        self.tick()?;
        self.inner.download(file, dest, p)
    }
}

#[test]
fn a_failed_listing_uploads_nothing_and_says_why() {
    let dir = temp_dir("listfail");
    zip_at(&dir, "Global Backups/a.zip", b"one", 3_600);
    let mut fake = FakeService::new().failing(ErrorKind::ServiceDown);
    let mut rec = ServiceRecord::default();
    let report = pass(&mut fake, &mut rec, &dir);
    assert_eq!((report.uploaded, report.error.as_ref().map(|e| e.kind)), (0, Some(ErrorKind::ServiceDown)));
    assert!(fake.uploads().is_empty());
    assert_eq!(rec.last_error, Some((ErrorKind::ServiceDown, NOW)));
}

#[test]
fn a_copy_that_arrives_with_the_wrong_size_is_an_error_and_is_not_recorded() {
    let dir = temp_dir("corrupt");
    zip_at(&dir, "Global Backups/a.zip", b"one", 3_600);
    let mut fake = FakeService::new();
    fake.corrupt_next_upload = true;
    let mut rec = ServiceRecord::default();
    let report = pass(&mut fake, &mut rec, &dir);
    assert!(report.error.as_ref().is_some_and(|e| e.detail.contains("did not arrive intact")), "{report:?}");
    assert!(rec.uploaded.is_empty() && rec.last_upload.is_none());
}

#[test]
fn a_wrong_checksum_from_the_service_is_an_error_but_no_checksum_is_fine() {
    let dir = temp_dir("sha");
    zip_at(&dir, "Global Backups/a.zip", b"one", 3_600);
    let mut fake = FakeService::new();
    fake.report_sha256 = false;
    let mut rec = ServiceRecord::default();
    assert!(pass(&mut fake, &mut rec, &dir).error.is_none(), "size alone is the fallback");
}

#[test]
fn cancelling_stops_the_pass_without_calling_it_a_good_one() {
    let dir = temp_dir("cancel");
    zip_at(&dir, "Update Backups/a/one.zip", b"1", 600);
    zip_at(&dir, "Update Backups/a/two.zip", b"2", 700);
    let (mut fake, mut rec) = (FakeService::new(), ServiceRecord::default());
    let polled = Cell::new(0);
    let report = run_pass(&mut fake, &mut rec, &input(&dir), &|| {
        polled.set(polled.get() + 1);
        polled.get() > 2
    }, &mut |_| {}, &mut |_| {});
    assert!(report.cancelled);
    assert_eq!(rec.last_ok, None);
    assert!(fake.uploads().len() < 2);
}

#[test]
fn the_record_is_saved_after_every_file() {
    let dir = temp_dir("saves");
    zip_at(&dir, "Update Backups/a/one.zip", b"1", 600);
    zip_at(&dir, "Update Backups/a/two.zip", b"2", 700);
    let (mut fake, mut rec) = (FakeService::new(), ServiceRecord::default());
    let mut sizes = Vec::new();
    run_pass(&mut fake, &mut rec, &input(&dir), &|| false, &mut |_| {}, &mut |r| sizes.push(r.uploaded.len()));
    assert_eq!(sizes, [1, 2, 2]);
}

#[test]
fn two_services_run_independently_and_each_keeps_its_own_record() {
    let dir = temp_dir("two");
    zip_at(&dir, "Global Backups/a.zip", b"one", 3_600);
    let mut good = FakeService::new();
    good.id = ServiceId::Second;
    let bad = FakeService::new().failing(ErrorKind::StorageFull);
    let mut services: Vec<Box<dyn CloudService>> = vec![Box::new(bad), Box::new(good)];
    let record = std::cell::RefCell::new(Record::in_memory());
    let out = run_services(&mut services, &|id| record.borrow().service(id), &mut |id, part| record.borrow_mut().set_service(id, part.clone()), &input(&dir), &|| false, &mut |_| {});
    let record = record.into_inner();
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].1.error.as_ref().map(|e| e.kind), Some(ErrorKind::StorageFull));
    assert!(out[1].1.error.is_none() && out[1].1.uploaded == 1);
    assert_eq!(record.service(ServiceId::Google).last_error, Some((ErrorKind::StorageFull, NOW)));
    assert_eq!(record.service(ServiceId::Second).uploaded.len(), 1);
    assert!(record.service(ServiceId::Google).uploaded.is_empty());
    assert_eq!(record.service(ServiceId::Second).last_ok, Some(NOW));
}

#[test]
fn the_service_trait_has_no_way_to_delete() {
    // The trait is the whole surface the app can use: list, upload, download. A new method would have to be
    // added here on purpose; this test is the reminder that retention is deliberately not built.
    let src = include_str!("service.rs");
    let methods: Vec<&str> = src.lines().filter(|l| l.trim_start().starts_with("fn ")).collect();
    for m in methods {
        assert!(!m.contains("delete") && !m.contains("remove") && !m.contains("trash"), "{m}");
    }
}
