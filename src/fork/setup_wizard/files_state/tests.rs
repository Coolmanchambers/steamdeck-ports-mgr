use super::*;
use serde_json::json;
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

fn severed_text() -> String {
    let data: serde_json::Value = serde_json::from_str(include_str!("../../../../ports.json")).unwrap();
    data["ports"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p.get("-name").and_then(|n| n.as_str()).is_some_and(|n| n.contains("Severed-Chains")))
        .expect("Severed Chains is in the catalog")["instructions"]
        .as_str()
        .unwrap()
        .to_string()
}

fn severed() -> Port {
    port("The Legend of Dragoon (Severed-Chains)", "Severed-Chains", &["ps1"], &severed_text())
}

#[test]
fn severed_chains_with_its_discs_in_isos_is_not_missing_and_without_them_is_not_red() {
    let tmp = temp_dir();
    let dir = tmp.path().join("lib/Severed-Chains");
    write(&dir.join("launch"), b"#!/bin/sh\n");
    write(&dir.join("isos/.keep"), b"");
    assert_eq!(files_state(&severed(), &dir, tmp.path()), FilesState::Unverifiable, "nothing to check a title against: not red");
    write(&dir.join("isos/Legend of Dragoon (USA) (Disc 1).bin"), b"disc");
    write(&dir.join("isos/Legend of Dragoon (USA) (Disc 1).cue"), b"cue");
    assert_eq!(files_state(&severed(), &dir, tmp.path()), FilesState::Have);
    assert!(in_folder_keys([(&severed(), dir)].into_iter(), tmp.path()).contains(severed().key()));
}

#[test]
fn the_described_disc_adds_nothing_the_move_and_verify_code_must_find() {
    let files = GameFiles::for_port(&severed(), Path::new("/nowhere"));
    assert!(files.required.is_empty(), "no file name, so `required_not_found` and `must_hash` have nothing to look for");
    let tmp = temp_dir();
    assert!(crate::fork::port_mover::game_files::required_not_found(tmp.path(), &files).is_empty());
}

#[test]
fn a_catalog_filled_deck_ready_entry_reads_the_same_end_to_end() {
    use crate::fork::catalog_extras::RawCatalog;
    use crate::fork::deck_ready::entry::local_entry_with;
    use crate::fork::deck_ready::index::parse;
    use crate::fork::deck_ready::index::tests::{file, project};
    let mut p = project("sc", "deck_build", "game");
    p["name"] = json!("The Legend of Dragoon (Severed-Chains)");
    p["repo"] = json!("Legend-of-Dragoon-Modding/Severed-Chains");
    p["catalog_folder"] = json!("Severed-Chains");
    p["catalog_disabled"] = json!(true);
    p["evidence"]["deck_assets"] = json!(["Severed_Chains_Steam_Deck.tar.gz"]);
    let entry = parse(&file(vec![p])).unwrap().entries.remove(0);
    let raw = RawCatalog::parse(include_str!("../../../../ports.json"));
    let value = local_entry_with(&entry, "The Legend of Dragoon (Severed-Chains)", "Severed-Chains", &raw).unwrap();
    let mut port = crate::core::models::port_from_value(&value).unwrap();
    port.user_managed = true;
    let tmp = temp_dir();
    // The app's own files: the entry in ports.local.json is what `files_state` reads the requirement from.
    write(&tmp.path().join("ports.local.json"), json!({"ports": [value]}).to_string().as_bytes());
    let dir = tmp.path().join("lib/Severed-Chains");
    write(&dir.join("launch"), b"#!/bin/sh\n");
    assert_eq!(files_state(&port, &dir, tmp.path()), FilesState::Unverifiable);
    write(&dir.join("isos/The Legend of Dragoon (Disc 1).chd"), b"disc");
    assert_eq!(files_state(&port, &dir, tmp.path()), FilesState::Have);
    // The same entry from before the catalog fields were copied reads the same once they are filled in memory.
    let mut old = crate::core::models::port_from_value(&local_entry_without_catalog(&entry)).unwrap();
    old.user_managed = true;
    crate::fork::catalog_extras::apply_to_port(&mut old, &raw);
    assert_eq!(old.instructions, port.instructions);
}

fn local_entry_without_catalog(entry: &crate::fork::deck_ready::index::Entry) -> serde_json::Value {
    crate::fork::deck_ready::entry::local_entry(entry, "The Legend of Dragoon (Severed-Chains)", "Severed-Chains").unwrap()
}

#[test]
fn archives_data_and_cartridges_never_carry_a_title() {
    let tmp = temp_dir();
    let dir = tmp.path().join("sc");
    let p = severed();
    for name in ["The Legend of Dragoon.zip", "The Legend of Dragoon.7z", "The Legend of Dragoon.dat", "The Legend of Dragoon.jar", "The Legend of Dragoon.z64", "The Legend of Dragoon.img"] {
        write(&dir.join(name), b"x");
    }
    assert_eq!(files_state(&p, &dir, tmp.path()), FilesState::Unverifiable);
    // An .img counts only with its cue sheet.
    write(&dir.join("The Legend of Dragoon.cue"), b"cue");
    assert_eq!(files_state(&p, &dir, tmp.path()), FilesState::Have, "the cue sheet is a disc image itself");
}

#[test]
fn an_img_alone_is_not_a_disc_but_with_its_cue_it_is() {
    let tmp = temp_dir();
    let dir = tmp.path().join("g");
    let p = port("G", "g", &[], "Required files: Crash Team Racing");
    write(&dir.join("sub/Crash Team Racing.img"), b"x");
    assert_eq!(files_state(&p, &dir, tmp.path()), FilesState::Unverifiable);
    write(&dir.join("sub/Crash Team Racing.cue"), b"cue");
    assert_eq!(files_state(&p, &dir, tmp.path()), FilesState::Have);
}

#[test]
fn the_search_goes_three_folders_down_and_no_further() {
    let tmp = temp_dir();
    let dir = tmp.path().join("dusklight");
    write(&dir.join("a/b/c/d/Legend of Zelda, The - Twilight Princess (USA).rvz"), b"disc");
    assert_eq!(files_state(&dusk(), &dir, tmp.path()), FilesState::Unverifiable, "four folders down is too deep");
    write(&dir.join("a/b/c/Legend of Zelda, The - Twilight Princess (USA).rvz"), b"disc");
    assert_eq!(files_state(&dusk(), &dir, tmp.path()), FilesState::Have, "three folders down is found");
}
