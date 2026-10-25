use super::*;
use crate::core::executable_detect::{autodetect_executable, ExecutableSelectionError};
use std::fs;

fn temp_dir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("dpm_program_detect_{}_{}", std::process::id(), name));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

/// A 64-bit little-endian ELF header with one program header of the given type.
fn elf(e_type: u16, first_header: u32) -> Vec<u8> {
    let mut b = vec![0u8; 64 + 56];
    b[..4].copy_from_slice(&ELF_MAGIC);
    b[4] = 2;
    b[5] = 1;
    b[16..18].copy_from_slice(&e_type.to_le_bytes());
    b[32..40].copy_from_slice(&64u64.to_le_bytes());
    b[54..56].copy_from_slice(&56u16.to_le_bytes());
    b[56..58].copy_from_slice(&1u16.to_le_bytes());
    b[64..68].copy_from_slice(&first_header.to_le_bytes());
    b
}

fn pie_program() -> Vec<u8> {
    elf(ET_DYN, PT_INTERP)
}

fn shared_library() -> Vec<u8> {
    elf(ET_DYN, 1)
}

fn static_program() -> Vec<u8> {
    elf(ET_EXEC, 1)
}

/// Writes a file the way an exFAT card shows it: mode 0755, whatever it is.
fn put(dir: &Path, rel: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

fn names(files: &[PathBuf], root: &Path) -> Vec<String> {
    files.iter().map(|p| p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/")).collect()
}

#[test]
fn elf_programs_and_scripts_have_program_content() {
    let dir = temp_dir("content");
    assert!(is_program_content(&put(&dir, "static", &static_program())));
    assert!(is_program_content(&put(&dir, "pie", &pie_program())));
    assert!(is_program_content(&put(&dir, "run", b"#!/bin/sh\necho hi\n")));
}

#[test]
fn libraries_data_and_empty_files_do_not() {
    let dir = temp_dir("not_content");
    assert!(!is_program_content(&put(&dir, "libfoo.so", &static_program())));
    assert!(!is_program_content(&put(&dir, "libfoo.so.1.2.3", &pie_program())));
    assert!(!is_program_content(&put(&dir, "unnamed_lib", &shared_library())));
    assert!(!is_program_content(&put(&dir, "disc.rvz", b"RVZ\x01 not a program")));
    assert!(!is_program_content(&put(&dir, "empty", b"")));
    assert!(!is_program_content(&put(&dir, "short", b"\x7fEL")));
    assert!(!is_program_content(&dir.join("missing")));
}

#[test]
fn a_truncated_elf_header_is_not_a_program() {
    let dir = temp_dir("truncated");
    let mut bytes = pie_program();
    bytes.truncate(40);
    assert!(!is_program_content(&put(&dir, "cut", &bytes)));
}

#[test]
fn library_names() {
    assert!(is_library_name("libjvm.so"));
    assert!(is_library_name("libz.so.1"));
    assert!(is_library_name("LIBX.SO.6.0"));
    assert!(!is_library_name("sonic.sh"));
    assert!(!is_library_name("also"));
}

#[cfg(unix)]
#[test]
fn an_exfat_like_folder_gives_no_data_files() {
    use crate::core::executable_detect::list_candidates;
    let dir = temp_dir("exfat");
    for data in [
        "Legend of Zelda, The - Twilight Princess (USA).rvz",
        "disc1.bin",
        "disc1.cue",
        "libjvm.so",
        "libfoo.so.1",
        "app.jar",
        "notes.txt",
        "data/level.dat",
    ] {
        put(&dir, data, b"data that is not a program");
    }
    put(&dir, "libreal.so", &pie_program());
    put(&dir, "game", &pie_program());
    put(&dir, "tool.x", &static_program());
    put(&dir, "start", b"#!/bin/sh\n");
    put(&dir, "Port.AppImage", b"");
    put(&dir, "plain.sh", b"");
    let found = list_candidates(&dir);
    assert_eq!(names(&found, &dir), ["Port.AppImage", "game", "plain.sh", "start", "tool.x"]);
}

#[cfg(unix)]
#[test]
fn a_file_without_the_executable_bit_is_not_a_program_unless_named_for_one() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("nobit");
    let elf_file = put(&dir, "game", &pie_program());
    fs::set_permissions(&elf_file, fs::Permissions::from_mode(0o644)).unwrap();
    let script = put(&dir, "go.sh", b"");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(!is_linux_program(&elf_file));
    assert!(is_linux_program(&script));
}

#[cfg(unix)]
#[test]
fn dusklight_layout_gives_the_appimage_alone() {
    let dir = temp_dir("dusklight");
    let image = put(&dir, "Dusklight-v1.4.1-linux-x86_64.AppImage", b"\x7fELF-ish appimage");
    put(&dir, "Legend of Zelda, The - Twilight Princess (USA).rvz", b"rvz");
    put(&dir, "res/archive.bin", b"data");
    assert_eq!(autodetect_executable(&dir).unwrap(), image);
}

#[cfg(unix)]
#[test]
fn severed_chains_layout_gives_the_launch_script_alone() {
    let dir = temp_dir("severed");
    let launch = put(&dir, "launch.sh", b"#!/bin/sh\njdk/bin/java -jar x.jar\n");
    put(&dir, "jdk/bin/java", &pie_program());
    put(&dir, "jdk/bin/keytool", &pie_program());
    put(&dir, "jdk/lib/libjvm.so", &shared_library());
    put(&dir, "jdk/lib/server/libjsig.so", &shared_library());
    put(&dir, "jdk/release", b"JAVA_VERSION=21");
    put(&dir, "isos/disc1.bin", b"disc");
    put(&dir, "isos/disc1.cue", b"cue");
    put(&dir, "lib/engine.jar", b"PK");
    assert_eq!(autodetect_executable(&dir).unwrap(), launch);
}

#[cfg(unix)]
#[test]
fn only_the_shallowest_programs_are_offered_and_two_there_stay_ambiguous() {
    let dir = temp_dir("shallow");
    put(&dir, "a", &pie_program());
    put(&dir, "b", &pie_program());
    put(&dir, "deep/c", &pie_program());
    match autodetect_executable(&dir) {
        Err(ExecutableSelectionError::Ambiguous(found)) => assert_eq!(names(&found, &dir), ["a", "b"]),
        other => panic!("expected Ambiguous, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn a_program_only_deeper_down_is_found_when_nothing_is_shallower() {
    let dir = temp_dir("deeper");
    let game = put(&dir, "bin/game", &pie_program());
    put(&dir, "data/readme.txt", b"hi");
    assert_eq!(autodetect_executable(&dir).unwrap(), game);
}

#[cfg(unix)]
#[test]
fn a_folder_of_only_data_has_no_program() {
    let dir = temp_dir("only_data");
    put(&dir, "disc.rvz", b"x");
    put(&dir, "libfoo.so", &shared_library());
    assert!(matches!(autodetect_executable(&dir), Err(ExecutableSelectionError::NotFound)));
}

#[test]
fn depth_and_ordering() {
    let root = Path::new("/g");
    assert_eq!(depth_in(root, Path::new("/g/a")), 0);
    assert_eq!(depth_in(root, Path::new("/g/x/y/a")), 2);
    assert_eq!(depth_in(root, Path::new("/elsewhere/a")), usize::MAX);
    let files = vec![PathBuf::from("/g/jdk/bin/java"), PathBuf::from("/g/zz"), PathBuf::from("/g/aa"), PathBuf::from("/g/x/y")];
    assert_eq!(names(&shallowest(root, files.clone()), root), ["aa", "zz"]);
    let mut all = files;
    sort_shallow_first(root, &mut all);
    assert_eq!(names(&all, root), ["aa", "zz", "x/y", "jdk/bin/java"]);
    assert!(shallowest(root, Vec::new()).is_empty());
}

#[test]
fn the_pick_is_stored_relative_to_the_folder() {
    let game = Path::new("/lib/Port");
    assert_eq!(relative_pick(game, Path::new("/lib/Port/bin/run")).as_deref(), Some("bin/run"));
    assert_eq!(relative_pick(game, Path::new("/other/run")), None);
}

#[test]
fn remembering_the_pick_survives_a_reload_and_resolves_back() {
    use crate::core::state::StateManager;
    let dir = temp_dir("remember");
    let exe = dir.join("jdk").join("bin").join("java");
    let rel = relative_pick(&dir, &exe).unwrap();
    let state_path = dir.join("state.json");
    let mut state = StateManager::load(&state_path);
    state.set_favorite_exe("owner/port", Some(rel));
    let reloaded = StateManager::load(&state_path);
    let stored = reloaded.get("owner/port").unwrap().favorite_exe.clone().unwrap();
    assert_eq!(crate::core::path_safety::safe_join(&dir, &stored).unwrap(), exe);
}

#[cfg(unix)]
#[test]
fn severed_chains_launch_without_an_extension_wins_over_the_java_files() {
    let dir = temp_dir("severed_launch");
    let launch = put(&dir, "launch", b"#!/bin/sh\nexec jdk/bin/java -jar x.jar\n");
    put(&dir, "jdk/bin/java", &pie_program());
    put(&dir, "isos/disc1.bin", b"disc");
    put(&dir, "isos/disc1.cue", b"cue");
    assert_eq!(autodetect_executable(&dir).unwrap(), launch);
}

#[cfg(unix)]
#[test]
fn a_launcher_script_that_lost_its_shebang_still_counts_when_named_like_one() {
    let dir = temp_dir("no_shebang");
    let launch = put(&dir, "launch", b"cd \"$(dirname \"$0\")\"\nexec jdk/bin/java -jar x.jar\n");
    put(&dir, "jdk/bin/java", &pie_program());
    assert_eq!(autodetect_executable(&dir).unwrap(), launch, "a text file named launch, with the bit, is a launcher");
    for (name, bytes) in [
        ("data", b"just notes\n".as_slice()),
        ("start.txt", b"text\n"),
        ("startup-log", b"text\n"),
        ("play", b"\x00\x01binary"),
        ("start_here", b"Read this first.\n"),
        ("run_me", b"notes\n"),
        ("play_notes", b"notes\n"),
        ("run-game", b"notes\n"),
    ] {
        let d = temp_dir(&format!("not_launcher_{name}"));
        put(&d, name, bytes);
        assert!(!is_linux_program(&d.join(name)), "{name}");
    }
    let big = temp_dir("big_start");
    put(&big, "start", &vec![b'a'; 70_000]);
    assert!(!is_linux_program(&big.join("start")), "a big text file named start is data");
    assert!(launcher_name("Launch") && launcher_name("RUN") && launcher_name("play") && launcher_name("start"));
    for no in ["run-game", "run_me", "start_here", "play_notes", "launcher.sh", "running", "launch2"] {
        assert!(!launcher_name(no), "{no}");
    }
}

#[test]
fn a_damaged_elf_header_cannot_overflow_the_arithmetic() {
    let dir = temp_dir("overflow");
    let mut b = pie_program();
    b[32..40].copy_from_slice(&u64::MAX.to_le_bytes());
    b[54..56].copy_from_slice(&u16::MAX.to_le_bytes());
    b[56..58].copy_from_slice(&u16::MAX.to_le_bytes());
    assert!(!is_program_content(&put(&dir, "bad", &b)));
    let mut b = pie_program();
    b[32..40].copy_from_slice(&(u64::MAX - 10).to_le_bytes());
    assert!(!is_program_content(&put(&dir, "bad2", &b)));
}
