use super::*;
use crate::fork::rom_library::testutil::{temp_dir, write};
use crate::fork::rom_library::{parse_instructions, Mapping, Method, RomLibraryStore};
use crate::fork::rom_screens::report::tests_support::port;
use crate::fork::setup_wizard::game_files::in_folder_keys;

const DUSK_TEXT: &str = "Required files: Legend of Zelda, The - Twilight Princess PAL or NTSC";
const TP_RVZ: &str = "Legend of Zelda, The - Twilight Princess (USA).rvz";

fn dusk() -> Port {
    port("Twilight Princess", "dusklight", &["gc"], DUSK_TEXT)
}

fn connect(config: &Path, port: &Port) {
    let slot = parse_instructions(&port.instructions).slots[0].id();
    let mut store = RomLibraryStore::load(&config.join("rom_library.json")).unwrap();
    store
        .record(Mapping {
            port_key: port.key().to_string(),
            slot,
            method: Method::UseInPlace,
            source: PathBuf::from("/roms/tp.rvz"),
            dest: None,
            size: 1,
            mtime_ms: 0,
            crc32: None,
            created_by_us: false,
            connected_at: String::new(),
        })
        .unwrap();
}

#[test]
fn the_catalog_text_really_is_a_title_only_slot() {
    let slots = parse_instructions(DUSK_TEXT).slots;
    assert_eq!(slots.len(), 1);
    assert!(slots[0].is_vague());
}

#[test]
fn tims_case_a_disc_with_the_title_in_the_folder_is_not_missing() {
    let tmp = temp_dir();
    let dir = tmp.path().join("lib/dusklight");
    write(&dir.join("Dusklight-v1.4.1-linux-x86_64.AppImage"), b"x");
    write(&dir.join(TP_RVZ), b"disc");
    assert_eq!(files_state(&dusk(), &dir, tmp.path()), FilesState::Have);
    assert!(in_folder_keys([(&dusk(), dir)].into_iter(), tmp.path()).contains(dusk().key()));
}

#[test]
fn the_same_folder_without_the_disc_cannot_be_checked_so_it_is_not_a_failure() {
    let tmp = temp_dir();
    let dir = tmp.path().join("lib/dusklight");
    write(&dir.join("Dusklight-v1.4.1-linux-x86_64.AppImage"), b"x");
    assert_eq!(files_state(&dusk(), &dir, tmp.path()), FilesState::Unverifiable);
    assert!(in_folder_keys([(&dusk(), dir)].into_iter(), tmp.path()).is_empty(), "no files counted as found");
}

#[test]
fn only_a_game_file_with_every_word_of_the_title_counts() {
    let tmp = temp_dir();
    let dir = tmp.path().join("dusklight");
    write(&dir.join("Legend of Zelda, The - Wind Waker (USA).rvz"), b"d");
    write(&dir.join("Twilight Princess notes.txt"), b"t");
    write(&dir.join("Twilight Princess.png"), b"t");
    assert_eq!(files_state(&dusk(), &dir, tmp.path()), FilesState::Unverifiable, "another game, a text file and a picture");
    write(&dir.join("deep/er/than/usual/x.rvz"), b"d");
    assert_eq!(files_state(&dusk(), &dir, tmp.path()), FilesState::Unverifiable);
    write(&dir.join("isos/gc/Twilight Princess, Legend of Zelda.iso"), b"d");
    assert_eq!(files_state(&dusk(), &dir, tmp.path()), FilesState::Have, "found a few folders down, words in any order");
}

#[test]
fn a_connection_recorded_for_the_slot_counts_whatever_is_in_the_folder() {
    let tmp = temp_dir();
    let dir = tmp.path().join("dusklight");
    write(&dir.join("x.AppImage"), b"x");
    assert_eq!(files_state(&dusk(), &dir, tmp.path()), FilesState::Unverifiable);
    connect(tmp.path(), &dusk());
    assert_eq!(files_state(&dusk(), &dir, tmp.path()), FilesState::Have);
    assert!(in_folder_keys([(&dusk(), dir)].into_iter(), tmp.path()).contains(dusk().key()));
}

#[test]
fn a_connection_for_another_port_does_not_count() {
    let tmp = temp_dir();
    let dir = tmp.path().join("dusklight");
    write(&dir.join("x.AppImage"), b"x");
    connect(tmp.path(), &port("Other", "other", &[], DUSK_TEXT));
    assert_eq!(files_state(&dusk(), &dir, tmp.path()), FilesState::Unverifiable);
}

#[test]
fn a_named_file_that_is_absent_is_still_red() {
    let tmp = temp_dir();
    let dir = tmp.path().join("zelda");
    let p = port("Zelda", "zelda", &["n64"], "Required files: oot.z64 (CRC32: 12345678)");
    write(&dir.join("readme.txt"), b"x");
    assert_eq!(files_state(&p, &dir, tmp.path()), FilesState::Missing);
    write(&dir.join("sub/OOT.Z64"), b"x");
    assert_eq!(files_state(&p, &dir, tmp.path()), FilesState::Have);
}

#[test]
fn a_named_file_missing_wins_over_a_title_that_cannot_be_checked() {
    let tmp = temp_dir();
    let dir = tmp.path().join("both");
    let p = port("Both", "both", &[], "Required files: oot.z64 (CRC32: 12345678), Twilight Princess");
    write(&dir.join("Twilight Princess (USA).iso"), b"d");
    assert_eq!(files_state(&p, &dir, tmp.path()), FilesState::Missing, "the named file is missing, so red");
    write(&dir.join("oot.z64"), b"r");
    assert_eq!(files_state(&p, &dir, tmp.path()), FilesState::Have);
}

#[test]
fn the_harbourmasters_ports_keep_their_generated_archive_rule() {
    let tmp = temp_dir();
    let dir = tmp.path().join("Shipwright");
    let p = port("Ship of Harkinian", "Shipwright", &["n64"], "Required files: Legend of Zelda, The - Ocarina of Time (USA).z64 (CRC32: 12345678)");
    write(&dir.join("soh.appimage"), b"x");
    assert_eq!(files_state(&p, &dir, tmp.path()), FilesState::Missing);
    write(&dir.join("oot.o2r"), b"archive");
    assert_eq!(files_state(&p, &dir, tmp.path()), FilesState::Have);
    assert!(in_folder_keys([(&p, dir)].into_iter(), tmp.path()).contains(p.key()));
}

#[test]
fn a_slot_with_a_serial_or_checksum_only_has_its_game_only_when_connected() {
    let tmp = temp_dir();
    let dir = tmp.path().join("ps");
    let p = port("PS", "ps", &["ps1"], "Required files: SLUS-00972. BIOS: Regional SCPH");
    write(&dir.join("x.sh"), b"x");
    assert_eq!(files_state(&p, &dir, tmp.path()), FilesState::Missing);
}

#[test]
fn significant_words_drop_regions_filler_tags_and_apostrophes() {
    assert_eq!(significant_words("Legend of Zelda, The - Twilight Princess PAL or NTSC"), ["legend", "zelda", "twilight", "princess"]);
    assert_eq!(significant_words("The Legend of Zelda: Majora's Mask (USA) [!]"), ["legend", "zelda", "majoras", "mask"]);
    assert_eq!(significant_words("Game X (Europe) (En,Fr)"), ["game"]);
    assert!(significant_words("USA or Europe").is_empty());
}

#[test]
fn a_title_with_no_significant_words_is_never_found_by_name() {
    let tmp = temp_dir();
    let dir = tmp.path().join("g");
    let p = port("G", "g", &[], "Required files: PAL or NTSC disc");
    assert!(parse_instructions(&p.instructions).slots.iter().all(|s| s.is_vague()));
    write(&dir.join("anything.iso"), b"d");
    assert_ne!(files_state(&p, &dir, tmp.path()), FilesState::Have);
}

#[test]
fn only_a_port_that_needs_files_and_lacks_them_counts_as_missing_for_the_wizard() {
    assert!(!FilesState::counts_as_missing(None), "a port that needs no game files is never missing");
    assert!(!FilesState::counts_as_missing(Some(FilesState::Have)));
    assert!(FilesState::counts_as_missing(Some(FilesState::Missing)));
    assert!(FilesState::counts_as_missing(Some(FilesState::Unverifiable)));
}
