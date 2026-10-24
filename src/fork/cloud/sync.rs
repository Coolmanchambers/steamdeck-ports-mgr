//! One upload pass for one cloud service: scan `Saves Backup`, work out what is not in the cloud yet, copy
//! it, and write what happened in the record after every file (so an interrupted pass loses nothing).
//!
//! A pass stops at the first failure of the service and says why; the caller decides when to try again.
//! Several services run one after the other with a pass each (`run_services`), so one failing never stops the
//! other, and each keeps its own times and error in its own part of the record.

use super::local::{self, LocalZip};
use super::plan::{self, Candidate, Step};
use super::record::{ServiceRecord, Uploaded};
use super::service::{CloudService, ErrorKind, ServiceError, ServiceId};
use super::sha256::file_sha256;
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

pub struct PassInput<'a> {
    pub saves_backup_dir: &'a Path,
    /// This Deck's folder name in the cloud.
    pub device: &'a str,
    pub now: u64,
    /// A zip changed more recently than this many seconds ago is left for the next pass.
    pub min_age: u64,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PassReport {
    pub uploaded: usize,
    pub recognised: usize,
    /// Zips not ready yet (just written, not readable yet): look again soon.
    pub deferred: usize,
    pub broken: usize,
    /// Zips whose every candidate cloud name was taken by a different file (not uploaded).
    pub unplaced: usize,
    pub bytes: u64,
    pub cancelled: bool,
    pub error: Option<ServiceError>,
}

fn mtime_of(path: &Path) -> Option<(u64, u64)> {
    let meta = fs::metadata(path).ok()?;
    let secs = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some((meta.len(), secs))
}

fn unchanged(zip: &LocalZip) -> bool {
    mtime_of(&zip.path) == Some((zip.size, zip.mtime))
}

fn short_name(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

pub fn run_pass(
    service: &mut dyn CloudService,
    rec: &mut ServiceRecord,
    input: &PassInput,
    cancel: &dyn Fn() -> bool,
    say: &mut dyn FnMut(&str),
    save: &mut dyn FnMut(&ServiceRecord),
) -> PassReport {
    let mut report = PassReport::default();
    let scan = local::scan(input.saves_backup_dir, input.now, input.min_age, &|rel, size, mtime| rec.uploaded.get(rel).is_some_and(|u| u.size == size && u.mtime == mtime));
    report.deferred = scan.deferred;
    report.broken = scan.broken;
    let to_check: Vec<&LocalZip> = scan.zips.iter().filter(|z| plan::needs_check(z, rec)).collect();
    if !to_check.is_empty() {
        let name = service.id().name();
        let mut candidates = Vec::new();
        for zip in to_check {
            if cancel() {
                report.cancelled = true;
                return finish(rec, input, report, save);
            }
            say(&format!("Checking {}", short_name(&zip.rel)));
            match file_sha256(&zip.path) {
                Ok(sha256) if unchanged(zip) => candidates.push(Candidate { zip: zip.clone(), sha256 }),
                // Changed while it was read, or gone: the next pass sees it again.
                _ => report.deferred += 1,
            }
        }
        if !candidates.is_empty() {
            say(&format!("Looking at {name}"));
            let remote = match service.list_all() {
                Ok(all) => all.into_iter().filter(|f| f.device == input.device).collect::<Vec<_>>(),
                Err(e) => {
                    report.error = Some(e);
                    return finish(rec, input, report, save);
                }
            };
            let planned = plan::plan(&candidates, rec, &remote);
            for zip in &planned.unplaced {
                super::log::note(format!("{}: no free cloud name for {}; it was not uploaded", service.id().key(), short_name(&zip.rel)));
            }
            report.unplaced = planned.unplaced.len();
            let steps = planned.steps;
            for step in steps {
                if cancel() {
                    report.cancelled = true;
                    break;
                }
                match step {
                    Step::Recognise { zip, sha256, cloud_path } => {
                        rec.uploaded.insert(zip.rel.clone(), Uploaded { size: zip.size, mtime: zip.mtime, sha256, cloud_path, at: input.now });
                        report.recognised += 1;
                        save(rec);
                    }
                    Step::Upload { zip, sha256, cloud_path } => {
                        if !unchanged(&zip) {
                            report.deferred += 1;
                            continue;
                        }
                        say(&format!("Copying {} to {name}", short_name(&zip.rel)));
                        let mut on_bytes = |_done: u64, _all: u64| {};
                        match service.upload(input.device, &cloud_path, &zip.path, zip.size, &mut on_bytes) {
                            Ok(file) => {
                                if file.size != zip.size || file.sha256.as_deref().is_some_and(|h| !h.eq_ignore_ascii_case(&sha256)) {
                                    super::log::note(format!("{}: {} arrived with a different size or checksum", service.id().key(), short_name(&cloud_path)));
                                    report.error = Some(ServiceError::other(format!("{} did not arrive intact", short_name(&cloud_path))));
                                    break;
                                }
                                rec.uploaded.insert(zip.rel.clone(), Uploaded { size: zip.size, mtime: zip.mtime, sha256, cloud_path, at: input.now });
                                rec.last_upload = Some(input.now);
                                report.uploaded += 1;
                                report.bytes += zip.size;
                                save(rec);
                            }
                            Err(e) => {
                                report.error = Some(e);
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
    finish(rec, input, report, save)
}

fn finish(rec: &mut ServiceRecord, input: &PassInput, report: PassReport, save: &mut dyn FnMut(&ServiceRecord)) -> PassReport {
    match &report.error {
        Some(e) => rec.last_error = Some((e.kind, input.now)),
        None if !report.cancelled => {
            rec.last_error = None;
            rec.last_ok = Some(input.now);
        }
        None => {}
    }
    save(rec);
    report
}

/// Every service in turn, each with its own part of the record (`load` reads it, `persist` writes it after
/// every file). A failure of one is in its report; the next still runs.
pub fn run_services(
    services: &mut [Box<dyn CloudService>],
    load: &dyn Fn(ServiceId) -> ServiceRecord,
    persist: &mut dyn FnMut(ServiceId, &ServiceRecord),
    input: &PassInput,
    cancel: &dyn Fn() -> bool,
    say: &mut dyn FnMut(&str),
) -> Vec<(ServiceId, PassReport)> {
    let mut out = Vec::new();
    for service in services.iter_mut() {
        let id = service.id();
        let mut part = load(id);
        let report = run_pass(service.as_mut(), &mut part, input, cancel, say, &mut |p| persist(id, p));
        out.push((id, report));
    }
    out
}

/// True when this failure should not be retried until the person does something.
pub fn needs_person(kind: ErrorKind) -> bool {
    !kind.retries_by_itself()
}
