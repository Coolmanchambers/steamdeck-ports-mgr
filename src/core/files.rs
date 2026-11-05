use std::fs;
use std::io;
use std::path::Path;

// Fork (data safety, D8): state files are synced to the disk before the rename and the folder after it, so a power cut
// right after a save leaves the old file or the new one, never an empty one. Caches use `write_atomic_unsynced`.
pub fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    write_atomic_with(path, contents, true)
}

// Fork (data safety, D8): the same without the syncs, for caches that can be fetched again (one sync per image on an
// SD card is slow).
pub fn write_atomic_unsynced(path: &Path, contents: &[u8]) -> io::Result<()> {
    write_atomic_with(path, contents, false)
}

fn write_atomic_with(path: &Path, contents: &[u8], sync: bool) -> io::Result<()> {
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    let written = if sync {
        fs::File::create(&tmp).and_then(|mut f| {
            io::Write::write_all(&mut f, contents)?;
            f.sync_all()
        })
    } else {
        fs::write(&tmp, contents)
    };
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    #[cfg(unix)]
    if sync {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()).and_then(|d| fs::File::open(d).ok()) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_atomic_remplace_le_contenu_sans_laisser_de_fichier_temporaire() {
        let dir = std::env::temp_dir().join(format!("ports_launcher_files_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        fs::write(&path, b"old").unwrap();

        write_atomic(&path, b"new").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert!(!dir.join("state.json.tmp").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    // Fork (data safety, D8): both kinds write the same bytes and leave no temporary file, also when the write fails.
    #[test]
    fn synced_and_unsynced_writes_end_the_same_and_clean_up_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        write_atomic(&path, b"synced").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"synced");
        write_atomic_unsynced(&path, b"cache").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"cache");
        fs::create_dir(dir.path().join("blocked")).unwrap();
        fs::create_dir(dir.path().join("blocked").join("x")).unwrap();
        assert!(write_atomic(&dir.path().join("blocked"), b"x").is_err(), "a folder is never replaced by a file");
        assert!(!dir.path().join("blocked.tmp").exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
