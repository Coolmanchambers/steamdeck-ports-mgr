//! Deciding what to upload. Pure: it looks at the local zips, the record and the cloud listing, and says,
//! for each zip, "already there", "recognise it as uploaded" or "upload it under this name".
//!
//! Nothing in the cloud is ever replaced. A zip whose path is new gets its own name. A zip whose path was
//! uploaded before but whose content has changed (a second Backup Saves on the same day rewrites the day's
//! zip) is uploaded next to the old copy, named `<stem>-<first 8 of its SHA-256>.zip`. A cloud file with the
//! same name and size is recognised instead of uploaded again (a new Deck, or a lost record).

use super::local::LocalZip;
use super::record::ServiceRecord;
use super::service::RemoteFile;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub zip: LocalZip,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Copy the file to `cloud_path`.
    Upload { zip: LocalZip, sha256: String, cloud_path: String },
    /// Already in the cloud at `cloud_path`: write it in the record, copy nothing.
    Recognise { zip: LocalZip, sha256: String, cloud_path: String },
}

impl Step {
    pub fn zip(&self) -> &LocalZip {
        match self {
            Step::Upload { zip, .. } | Step::Recognise { zip, .. } => zip,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Newest first.
    pub steps: Vec<Step>,
}

/// True when the file needs a closer look (hash, then maybe an upload): it is not recorded, or its size or
/// modified time is not what was recorded.
pub fn needs_check(zip: &LocalZip, rec: &ServiceRecord) -> bool {
    rec.uploaded.get(&zip.rel).is_none_or(|u| u.size != zip.size || u.mtime != zip.mtime)
}

fn with_suffix(rel: &str, suffix: &str) -> String {
    match rel.strip_suffix(".zip") {
        Some(stem) => format!("{stem}{suffix}.zip"),
        None => format!("{rel}{suffix}"),
    }
}

/// `remote` is the cloud listing for this Deck only.
pub fn plan(candidates: &[Candidate], rec: &ServiceRecord, remote: &[RemoteFile]) -> Plan {
    let mut ordered: Vec<&Candidate> = candidates.iter().collect();
    ordered.sort_by(|a, b| b.zip.mtime.cmp(&a.zip.mtime).then_with(|| a.zip.rel.cmp(&b.zip.rel)));
    let mut steps = Vec::new();
    for c in ordered {
        let recorded = rec.uploaded.get(&c.zip.rel);
        if let Some(u) = recorded.filter(|u| u.sha256 == c.sha256) {
            // The same content with a new modified time: nothing to copy.
            steps.push(Step::Recognise { zip: c.zip.clone(), sha256: c.sha256.clone(), cloud_path: u.cloud_path.clone() });
            continue;
        }
        let short = &c.sha256[..c.sha256.len().min(8)];
        // First choice: the file's own path, unless that path was already used for different content.
        let mut names = Vec::new();
        if recorded.is_none() {
            names.push(c.zip.rel.clone());
        }
        names.push(with_suffix(&c.zip.rel, &format!("-{short}")));
        names.extend((2..50).map(|n| with_suffix(&c.zip.rel, &format!("-{short}-{n}"))));
        for name in names {
            match remote.iter().find(|r| r.path == name) {
                None => {
                    steps.push(Step::Upload { zip: c.zip.clone(), sha256: c.sha256.clone(), cloud_path: name });
                    break;
                }
                Some(r) if r.size == c.zip.size && r.sha256.as_deref().is_none_or(|h| h == c.sha256) => {
                    steps.push(Step::Recognise { zip: c.zip.clone(), sha256: c.sha256.clone(), cloud_path: name });
                    break;
                }
                Some(_) => {}
            }
        }
    }
    Plan { steps }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::cloud::record::Uploaded;
    use std::path::PathBuf;

    fn zip(rel: &str, size: u64, mtime: u64) -> LocalZip {
        LocalZip { rel: rel.into(), path: PathBuf::from(rel), size, mtime }
    }

    fn cand(rel: &str, size: u64, mtime: u64, sha: &str) -> Candidate {
        Candidate { zip: zip(rel, size, mtime), sha256: sha.repeat(64 / sha.len()) }
    }

    fn remote(path: &str, size: u64, sha: Option<&str>) -> RemoteFile {
        RemoteFile { device: "deck".into(), path: path.into(), size, sha256: sha.map(|s| s.repeat(64 / s.len())), id: "id".into(), modified: None }
    }

    fn recorded(rec: &mut ServiceRecord, rel: &str, size: u64, mtime: u64, sha: &str, cloud: &str) {
        rec.uploaded.insert(rel.into(), Uploaded { size, mtime, sha256: sha.repeat(64 / sha.len()), cloud_path: cloud.into(), at: 1 });
    }

    #[test]
    fn a_new_zip_is_uploaded_under_its_own_path() {
        let p = plan(&[cand("Global Backups/a.zip", 10, 5, "aa")], &ServiceRecord::default(), &[]);
        assert_eq!(p.steps, vec![Step::Upload { zip: zip("Global Backups/a.zip", 10, 5), sha256: "aa".repeat(32), cloud_path: "Global Backups/a.zip".into() }]);
    }

    #[test]
    fn the_newest_goes_first() {
        let c = [cand("Global Backups/old.zip", 1, 10, "aa"), cand("Global Backups/new.zip", 1, 30, "bb"), cand("Global Backups/mid.zip", 1, 20, "cc")];
        let p = plan(&c, &ServiceRecord::default(), &[]);
        let order: Vec<&str> = p.steps.iter().map(|s| s.zip().rel.as_str()).collect();
        assert_eq!(order, ["Global Backups/new.zip", "Global Backups/mid.zip", "Global Backups/old.zip"]);
    }

    #[test]
    fn a_cloud_file_with_the_same_name_and_size_is_recognised_not_uploaded() {
        let p = plan(&[cand("Global Backups/a.zip", 10, 5, "aa")], &ServiceRecord::default(), &[remote("Global Backups/a.zip", 10, None)]);
        assert!(matches!(&p.steps[0], Step::Recognise { cloud_path, .. } if cloud_path == "Global Backups/a.zip"));
    }

    #[test]
    fn a_cloud_file_with_the_same_name_and_a_different_hash_is_not_the_same_file() {
        let p = plan(&[cand("Global Backups/a.zip", 10, 5, "aa")], &ServiceRecord::default(), &[remote("Global Backups/a.zip", 10, Some("bb"))]);
        match &p.steps[0] {
            Step::Upload { cloud_path, .. } => assert_eq!(cloud_path, "Global Backups/a-aaaaaaaa.zip"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_cloud_file_with_the_same_name_and_another_size_is_never_replaced() {
        let p = plan(&[cand("Global Backups/a.zip", 10, 5, "aa")], &ServiceRecord::default(), &[remote("Global Backups/a.zip", 99, None)]);
        assert!(matches!(&p.steps[0], Step::Upload { cloud_path, .. } if cloud_path == "Global Backups/a-aaaaaaaa.zip"));
    }

    #[test]
    fn the_versioned_name_is_recognised_on_a_later_pass_and_counted_up_when_taken() {
        let c = [cand("Global Backups/a.zip", 10, 5, "aa")];
        let taken = remote("Global Backups/a.zip", 99, None);
        let same = remote("Global Backups/a-aaaaaaaa.zip", 10, None);
        let p = plan(&c, &ServiceRecord::default(), &[taken.clone(), same]);
        assert!(matches!(&p.steps[0], Step::Recognise { cloud_path, .. } if cloud_path == "Global Backups/a-aaaaaaaa.zip"));
        let other = remote("Global Backups/a-aaaaaaaa.zip", 77, None);
        let p = plan(&c, &ServiceRecord::default(), &[taken, other]);
        assert!(matches!(&p.steps[0], Step::Upload { cloud_path, .. } if cloud_path == "Global Backups/a-aaaaaaaa-2.zip"));
    }

    #[test]
    fn a_recorded_zip_with_new_content_is_uploaded_beside_the_old_copy() {
        let mut rec = ServiceRecord::default();
        recorded(&mut rec, "Global Backups/a.zip", 10, 5, "aa", "Global Backups/a.zip");
        let p = plan(&[cand("Global Backups/a.zip", 12, 9, "bb")], &rec, &[remote("Global Backups/a.zip", 10, None)]);
        assert!(matches!(&p.steps[0], Step::Upload { cloud_path, .. } if cloud_path == "Global Backups/a-bbbbbbbb.zip"));
    }

    #[test]
    fn a_recorded_zip_with_the_same_content_and_a_new_mtime_copies_nothing() {
        let mut rec = ServiceRecord::default();
        recorded(&mut rec, "Global Backups/a.zip", 10, 5, "aa", "Global Backups/a-aaaaaaaa.zip");
        let p = plan(&[cand("Global Backups/a.zip", 10, 99, "aa")], &rec, &[]);
        assert!(matches!(&p.steps[0], Step::Recognise { cloud_path, .. } if cloud_path == "Global Backups/a-aaaaaaaa.zip"));
    }

    #[test]
    fn needs_check_is_false_only_for_an_unchanged_recorded_file() {
        let mut rec = ServiceRecord::default();
        recorded(&mut rec, "x.zip", 10, 5, "aa", "x.zip");
        assert!(!needs_check(&zip("x.zip", 10, 5), &rec));
        assert!(needs_check(&zip("x.zip", 11, 5), &rec));
        assert!(needs_check(&zip("x.zip", 10, 6), &rec));
        assert!(needs_check(&zip("y.zip", 10, 5), &rec));
    }
}
