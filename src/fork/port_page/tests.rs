use super::paths::*;
use super::settings_files::*;
use super::*;
use crate::fork::folder_picker::mounts::Drive;
use crate::fork::page::{action_at, focusables, hints, initial_focus, HINT_GAP};
use std::fs;

fn sd(mount: &str, label: &str) -> Drive {
    Drive { mount_point: PathBuf::from(mount), kind: "SD card", label: label.into(), fs_type: "ext4".into() }
}

fn deck_places() -> Places {
    Places {
        drives: vec![sd("/run/media/deck/c14f9023-6854-443f-9348-735551234567", "c14f9023-6854-443f-9348-735551234567")],
        home: Some(PathBuf::from("/home/deck")),
        media_roots: vec![PathBuf::from("/run/media/deck"), PathBuf::from("/run/media"), PathBuf::from("/media")],
    }
}

// -- The human path formatter -----------------------------------------------------------------------------------

#[test]
fn an_sd_card_named_by_its_id_is_just_sd_card_never_the_uuid() {
    let p = deck_places();
    let roms = Path::new("/run/media/deck/c14f9023-6854-443f-9348-735551234567/Emulation/roms");
    assert_eq!(human_path(roms, &p), "SD card > Emulation > roms");
    assert_eq!(human_path(Path::new("/run/media/deck/c14f9023-6854-443f-9348-735551234567"), &p), "SD card");
    assert!(!human_path(roms, &p).contains("c14f9023"));
}

#[test]
fn a_named_card_or_stick_keeps_its_name_and_the_deepest_mount_wins() {
    let mut p = deck_places();
    p.drives = vec![sd("/run/media/deck/SD512", "SD512"), Drive { mount_point: "/run/media/deck/SD512/inner".into(), kind: "USB drive", label: "STICK".into(), fs_type: "vfat".into() }];
    assert_eq!(human_path(Path::new("/run/media/deck/SD512/Ports/dusk"), &p), "SD card (SD512) > Ports > dusk");
    assert_eq!(human_path(Path::new("/run/media/deck/SD512/inner/ROMs"), &p), "USB drive (STICK) > ROMs");
}

#[test]
fn a_card_that_is_not_mounted_now_is_still_not_shown_by_its_id() {
    let mut p = deck_places();
    p.drives.clear();
    assert_eq!(human_path(Path::new("/run/media/deck/0a1b2c3d-4e5f-6789-abcd-ef0123456789/Emulation/roms"), &p), "SD card > Emulation > roms");
    assert_eq!(human_path(Path::new("/run/media/deck/MyCard/Ports"), &p), "MyCard > Ports", "a card with a name keeps it");
    assert_eq!(human_path(Path::new("/run/media/1A2B-3C4D/x"), &p), "SD card > x", "the root without the user folder too");
}

#[test]
fn the_home_folder_is_internal_storage_and_other_paths_stay_as_they_are() {
    let p = deck_places();
    assert_eq!(human_path(Path::new("/home/deck/Emulation/roms"), &p), "Internal storage > Emulation > roms");
    assert_eq!(human_path(Path::new("/home/deck/.local/share/TwilitRealm"), &p), "Internal storage > .local > share > TwilitRealm");
    assert_eq!(human_path(Path::new("/home/deck"), &p), "Internal storage > Home");
    assert_eq!(human_path(Path::new("/opt/games"), &p), "/opt/games");
    assert_eq!(human_path(Path::new("/home/deckard/x"), &p), "/home/deckard/x", "a folder that only starts like home is not in it");
}

#[test]
fn sizes_read_like_a_file_manager() {
    assert_eq!(size_text(0), "0 bytes");
    assert_eq!(size_text(1), "1 byte");
    assert_eq!(size_text(2_400), "2.4 KB");
    assert_eq!(size_text(340_000_000), "340 MB");
    assert_eq!(size_text(1_234_000_000), "1.2 GB");
}

// -- Settings files ------------------------------------------------------------------------------------------------

#[test]
fn settings_names_are_settings_extensions_and_txt_only_when_named_like_settings() {
    for yes in ["config.toml", "settings.json", "Options.INI", "game.cfg", "user.conf", "prefs.yaml", "a.yml", "keys.xml", "controls.txt", "video_settings.txt"] {
        assert!(is_settings_name(yes), "{yes}");
    }
    for no in ["readme.txt", "notes.txt", "latest.log", "log.json", "crash_report.json", "game.o2r", "soh.appimage", ".hidden.json", "ports.json", "state.json", "fork-settings.json", "json", "x"] {
        assert!(!is_settings_name(no), "{no}");
    }
    assert!(is_settings_name("catalog.json"), "a catalog is not a log");
}

fn write(path: &Path, bytes: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![b'a'; bytes]).unwrap();
}

#[test]
fn settings_are_found_two_levels_deep_in_the_port_and_save_folders_minus_the_rest() {
    let tmp = tempfile::tempdir().unwrap();
    let game = tmp.path().join("dusk");
    let saves = tmp.path().join("saves");
    write(&game.join("config.toml"), 10);
    write(&game.join("cfg/controls.ini"), 10);
    write(&game.join("cfg/deeper/too_deep.ini"), 10);
    write(&game.join(".previous/v1/config.toml"), 10);
    write(&game.join("big.json"), (MAX_BYTES + 1) as usize);
    write(&game.join("game.log"), 10);
    write(&game.join("latest_log.json"), 10);
    write(&game.join("rom_hashes.json"), 10);
    write(&game.join("readme.txt"), 10);
    write(&saves.join("user.json"), 10);
    let found: Vec<String> = discover(&Input { game_dir: &game, save_dirs: &[saves.clone()], named: None, game_data: &["ROM_HASHES.json".to_string()] })
        .iter()
        .map(|f| f.path.strip_prefix(tmp.path()).unwrap().to_string_lossy().replace('\\', "/"))
        .collect();
    assert_eq!(found, ["dusk/config.toml", "dusk/cfg/controls.ini", "saves/user.json"]);
}

#[test]
fn a_local_entry_can_name_the_settings_exactly() {
    let tmp = tempfile::tempdir().unwrap();
    let game = tmp.path().join("dusk");
    write(&game.join("config.toml"), 10);
    write(&game.join("odd/settings.dat"), 10);
    write(&game.join("conf.d/a.dat"), 10);
    write(&game.join("conf.d/b.dat"), 10);
    let named = serde_json::json!(["odd/settings.dat", "conf.d", "missing.ini"]);
    let found: Vec<String> = discover(&Input { game_dir: &game, save_dirs: &[], named: Some(&named), game_data: &[] }).iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(found, ["settings.dat", "a.dat", "b.dat"], "only what it names: config.toml is left out, a missing file is skipped");
    let one = serde_json::json!({"linux": "config.toml", "windows": "%APPDATA%\\x"});
    assert_eq!(discover(&Input { game_dir: &game, save_dirs: &[], named: Some(&one), game_data: &[] }).len(), 1, "same shape as save: one per system");
}

#[test]
fn the_view_text_is_capped_and_refuses_binary() {
    let tmp = tempfile::tempdir().unwrap();
    let small = tmp.path().join("a.ini");
    fs::write(&small, "[video]\r\nwidth=1280\r\n").unwrap();
    assert_eq!(view_text(&small).unwrap(), ("[video]\nwidth=1280".to_string(), false));
    let long = tmp.path().join("b.ini");
    fs::write(&long, "x=1\n".repeat(VIEW_LINES + 10)).unwrap();
    let (text, cut) = view_text(&long).unwrap();
    assert!(cut && text.lines().count() == VIEW_LINES);
    let bin = tmp.path().join("c.json");
    fs::write(&bin, [b'{', 0, 1, 2]).unwrap();
    assert!(view_text(&bin).is_err());
    let huge = tmp.path().join("d.json");
    fs::write(&huge, "y".repeat(VIEW_BYTES * 2)).unwrap();
    let (text, cut) = view_text(&huge).unwrap();
    assert!(cut && text.len() <= VIEW_BYTES);
    assert!(view_text(&tmp.path().join("gone.ini")).is_err());
}

// -- Summaries -----------------------------------------------------------------------------------------------------

fn port() -> PortFacts {
    PortFacts { key: "dusk".into(), name: "Dusklight".into(), badge: "Ready".into(), update_on: true, tracks_releases: true, version: Some("v1.4.1".into()), ..PortFacts::default() }
}

#[test]
fn play_and_update_lines_say_what_is_true() {
    let mut p = port();
    assert_eq!(play_summary(&p), "Never played");
    p.last_played = Some("on 2026-10-01".into());
    assert_eq!(play_summary(&p), "Last played on 2026-10-01");
    p.last_played = Some("today".into());
    assert_eq!(play_summary(&p), "Last played today");
    p.running = true;
    assert_eq!(play_summary(&p), "Running now");
    let mut p = port();
    assert_eq!(updates_summary(&p), "v1.4.1, checks on");
    p.update_on = false;
    assert_eq!(updates_summary(&p), "v1.4.1, checks off");
    p.update_ready = true;
    assert_eq!(updates_summary(&p), "Update ready");
    p.update_check = UpdateCheck::UpToDate("v1.4.1".into());
    assert_eq!(updates_summary(&p), "Up to date (v1.4.1), checks off");
    p.update_check = UpdateCheck::Ready("v1.5.0".into());
    assert_eq!(updates_summary(&p), "Update ready: v1.5.0");
    p.tracks_releases = false;
    assert!(updates_summary(&p).starts_with("Not tracked"));
}

#[test]
fn the_program_line_names_the_pick_the_catalog_or_says_it_asks() {
    let dir = PathBuf::from("/lib/dusk");
    let mut f = ProgramFacts { game_dir: dir.clone(), candidates: vec![dir.join("Dusklight-v1.4.1-linux-x86_64.AppImage")], ..ProgramFacts::default() };
    assert_eq!(program_summary(&f), "Dusklight-v1.4.1-linux-x86_64.AppImage");
    f.candidates.push(dir.join("tool.sh"));
    assert_eq!(program_summary(&f), "Asks every time");
    f.candidates = vec![dir.join("run.sh"), dir.join("jdk/bin/java")];
    assert_eq!(program_summary(&f), "run.sh", "only the shallowest programs are offered");
    f.favorite = Some("bin/game".into());
    f.favorite_found = true;
    assert_eq!(program_summary(&f), "game");
    f.favorite_found = false;
    assert_eq!(program_summary(&f), "game is gone; asks every time");
    assert_eq!(program_summary(&ProgramFacts::default()), "No program found");
}

#[test]
fn the_saves_line_names_the_place_the_files_and_the_backup() {
    let p = deck_places();
    let folder = |exists, files| SaveFolder { path: "/home/deck/.local/share/TwilitRealm".into(), source: SaveSource::Catalog, exists, files, capped: false };
    let mut f = SavesFacts { folders: vec![folder(true, 3)], last_backup_days: Some(0), ..SavesFacts::default() };
    assert_eq!(saves_summary(&f, &p), "Internal storage > .local > share > TwilitRealm, 3 files, backed up today");
    f.folders = vec![folder(true, 0)];
    assert_eq!(saves_summary(&f, &p), "Internal storage > .local > share > TwilitRealm, no saves yet");
    f.folders = vec![folder(false, 0)];
    assert_eq!(saves_summary(&f, &p), "Not found yet");
    f.no_saves = true;
    assert_eq!(saves_summary(&f, &p), "No saves (you said so)");
}

#[test]
fn game_files_steam_and_settings_lines() {
    assert_eq!(game_files_summary(&GameFilesFacts { state: None, connected: vec![] }), "Needs no game files");
    assert_eq!(game_files_summary(&GameFilesFacts { state: Some(FilesState::Have), connected: vec!["Legend of Zelda (USA).rvz".into()] }), "Ready: Legend of Zelda (USA).rvz");
    assert_eq!(game_files_summary(&GameFilesFacts { state: Some(FilesState::Unverifiable), connected: vec![] }), "Add game files");
    let steam = |state| SteamFacts { state, ..SteamFacts::default() };
    assert_eq!(steam_summary(&steam(Ok(SteamState::InSteam))), "In Steam, icon fine");
    assert_eq!(steam_summary(&SteamFacts { icon_problem: Some("missing".into()), ..steam(Ok(SteamState::InSteam)) }), "In Steam, icon needs fixing");
    assert_eq!(steam_summary(&steam(Ok(SteamState::NotInSteam))), "Not in Steam");
    assert_eq!(steam_summary(&SteamFacts { at_old_copy: true, ..steam(Ok(SteamState::InSteam)) }), "Steam still starts the old copy");
    assert_eq!(steam_summary(&steam(Err("No Steam account folder was found.".into()))), "Steam was not found");
    let game = PathBuf::from("/lib/dusk");
    let file = |p: &str| SettingsFile { path: PathBuf::from(p), size: 1, modified: None };
    assert_eq!(settings_summary(&[], &game), "None found");
    assert_eq!(settings_summary(&[file("/lib/dusk/config.toml")], &game), "config.toml in the port folder");
    assert_eq!(settings_summary(&[file("/home/deck/.config/dusk/a.ini")], &game), "a.ini with the saves");
    assert_eq!(settings_summary(&[file("/lib/dusk/a.ini"), file("/lib/dusk/b.ini")], &game), "a.ini and 1 more");
}

#[test]
fn the_files_line_is_place_size_and_version() {
    let f = FilesFacts { dir: "/run/media/deck/c14f9023-6854-443f-9348-735551234567/Ports/dusklight".into(), size: Some(1_200_000_000), ..FilesFacts::default() };
    assert_eq!(files_summary(&f, Some("v1.4.1"), &deck_places()), "SD card > Ports > dusklight, 1.2 GB, v1.4.1");
}

// -- Pages and routing ----------------------------------------------------------------------------------------------

#[test]
fn every_card_routes_to_one_place_and_the_order_is_the_agreed_one() {
    let titles: Vec<&str> = Card::ALL.iter().map(|c| c.title()).collect();
    assert_eq!(titles, ["Play", "Fix problems", "Program to start", "Saves", "Port settings", "Game files", "Steam", "Updates", "Files", "Uninstall", "More"]);
    assert_eq!(Card::Play.route(), Route::Start);
    assert_eq!(Card::Fix.route(), Route::Checkup);
    assert_eq!(Card::GameFiles.route(), Route::GameFiles);
    for c in [Card::Program, Card::Saves, Card::Settings, Card::Steam, Card::Updates, Card::Files, Card::Uninstall, Card::More] {
        assert_eq!(c.route(), Route::Page, "{c:?}");
    }
    let icons: std::collections::HashSet<i32> = Card::ALL.iter().map(|c| c.icon().code()).collect();
    assert_eq!(icons.len(), Card::ALL.len(), "a picture of its own each");
}

#[test]
fn the_first_page_has_the_cover_a_card_per_area_and_b_goes_back() {
    let p = port();
    let page = first_page(&p, None, &deck_places());
    assert_eq!(page.window_title, "Port: Dusklight");
    assert_eq!(page.card.as_ref().unwrap().lines[0], "Ready");
    assert_eq!(page.options.iter().map(|o| o.action).collect::<Vec<_>>(), Card::ALL.iter().map(|c| Act::Open(*c)).collect::<Vec<_>>());
    assert_eq!(action_at(&page, initial_focus(&page)), Some(Act::Open(Card::Play)));
    assert_eq!(page.options[2].detail, "Looking...", "disk cards wait for the worker");
    assert_eq!(page.buttons[LEFT].as_ref().map(|b| b.action), Some(Act::Back));
    assert_eq!(hints(&page), ["A Choose", "B Back", "D-pad Move"].join(HINT_GAP));
    let disk = DiskFacts { checkup: Some(("2 things to fix".into(), 2)), ..DiskFacts::default() };
    let page = first_page(&p, Some(&disk), &deck_places());
    assert_eq!(page.options[1].detail, "2 things to fix");
    assert_eq!(page.options[2].detail, "No program found");
}

#[test]
fn the_card_pages_have_their_window_titles_and_safe_starting_places() {
    let p = port();
    let places = deck_places();
    let f = ProgramFacts { game_dir: "/lib/dusk".into(), candidates: vec!["/lib/dusk/a.sh".into(), "/lib/dusk/b.sh".into()], favorite: Some("b.sh".into()), favorite_found: true, ..ProgramFacts::default() };
    let prog = program_page(&p, &f, &places);
    assert_eq!(prog.window_title, "Port: Dusklight: Program to start");
    assert_eq!(prog.options[0].choice, Some(false));
    assert_eq!(prog.options[2].choice, Some(true));
    assert_eq!(action_at(&prog, initial_focus(&prog)), Some(Act::UseProgram(1)), "the cursor starts on the program in use");
    let un = uninstall_page(&p, None, None, &places);
    assert_eq!(action_at(&un, initial_focus(&un)), Some(Act::Back), "Keep it is where the cursor starts");
    assert!(un.options.iter().any(|o| o.action == Act::UninstallYes && o.enabled));
    let own = uninstall_page(&PortFacts { user_managed: true, ..port() }, None, None, &places);
    assert!(own.options.iter().all(|o| o.action != Act::UninstallYes), "a port you added yourself is never deleted here");
    let up = updates_page(&PortFacts { update_check: UpdateCheck::Ready("v1.5.0".into()), ..port() });
    assert_eq!(up.options[0].action, Act::UpdateNow);
    assert!(up.options[0].recommended);
    let switch = up.options.iter().position(|o| o.action == Act::ToggleUpdates).unwrap();
    assert_eq!(up.toggle_of(switch), Some(true));
    let none = updates_page(&port());
    assert!(none.options.iter().all(|o| o.action != Act::UpdateNow), "no Update now until one is found");
}

#[test]
fn game_mode_dims_what_needs_a_desktop() {
    let p = PortFacts { game_mode: true, ..port() };
    let files = files_page(&p, &FilesFacts::default(), &deck_places());
    assert!(!files.options[0].enabled && files.options[0].detail == "Only in Desktop Mode");
    let desk = files_page(&port(), &FilesFacts::default(), &deck_places());
    assert!(desk.options[0].enabled);
}

#[test]
fn the_view_page_shows_the_place_first_then_the_text() {
    let f = SettingsFile { path: "/run/media/deck/c14f9023-6854-443f-9348-735551234567/Ports/dusk/config.toml".into(), size: 20, modified: None };
    let page = view_page(&port(), 0, &f, &Ok(("width=1280".into(), true)), &deck_places());
    assert_eq!(page.paragraphs, ["SD card > Ports > dusk > config.toml", "width=1280"]);
    assert!(!page.footnote.is_empty(), "a cut file says so");
    assert_eq!(page.title, "config.toml");
}

#[test]
fn the_steam_page_offers_only_what_applies_and_the_link_page_shows_the_address_first() {
    let p = port();
    let s = steam_page(&p, &SteamFacts { state: Ok(SteamState::NotInSteam), ..SteamFacts::default() });
    assert_eq!(s.options.iter().map(|o| o.action).collect::<Vec<_>>(), [Act::SteamAdd]);
    let s = steam_page(&p, &SteamFacts { state: Ok(SteamState::InSteam), at_old_copy: true, icon_problem: Some("missing".into()), ..SteamFacts::default() });
    assert_eq!(s.options.iter().map(|o| o.action).collect::<Vec<_>>(), [Act::SteamRepoint, Act::SteamFixIcon]);
    let fine = steam_page(&p, &SteamFacts { state: Ok(SteamState::InSteam), ..SteamFacts::default() });
    assert!(fine.options.is_empty() && focusables(&fine).len() == 1, "only Back");
    let desk = link_page("Dusklight", "https://github.com/x/dusklight", false);
    assert_eq!(desk.paragraphs[0], "https://github.com/x/dusklight");
    assert_eq!(desk.options.iter().map(|o| o.action).collect::<Vec<_>>(), [LinkAct::OpenInBrowser]);
    assert!(desk.card.as_ref().unwrap().qr.is_some());
    let game = link_page("Dusklight", "https://github.com/x/dusklight", true);
    assert!(game.options.is_empty(), "no browser in Game Mode: the code and the text");
}

#[test]
fn the_project_link_is_a_web_page_never_a_download() {
    let port_of = |v: serde_json::Value| crate::core::models::port_from_value(&v).unwrap();
    assert_eq!(project_url(&port_of(serde_json::json!({"name": "A", "folder": "a", "source": "https://github.com/o/a"}))).as_deref(), Some("https://github.com/o/a"));
    assert_eq!(project_url(&port_of(serde_json::json!({"name": "B", "folder": "b", "source": "https://example.com/b.zip"}))), None, "a file link is not a project page");
    assert_eq!(project_url(&port_of(serde_json::json!({"name": "C", "folder": "c", "source": "https://example.com/c.zip", "website": "https://c.example"}))).as_deref(), Some("https://c.example"));
    assert_eq!(project_url(&port_of(serde_json::json!({"name": "D", "folder": "d", "source": "https://example.com/d.zip", "website": "file:///etc/passwd"}))), None);
}

#[test]
fn the_uninstall_confirm_page_says_what_is_kept_and_where() {
    let places = deck_places();
    let keep = crate::fork::uninstall_keep::KeepSummary { game_files: 3, older_versions: 1, bytes: 100_000_000, names: vec!["oot.o2r".into(), "oot-mq.o2r".into(), "zelda.z64".into()], ..Default::default() };
    let f = FilesFacts {
        dir: "/home/deck/Ports/shipwright".into(),
        size: Some(300_000_000),
        keep,
        kept_dir: "/home/deck/Saves Backup/Kept files/shipwright".into(),
        ..FilesFacts::default()
    };
    let saves = SavesFacts::default();
    let page = uninstall_page(&port(), Some(&f), Some(&saves), &places);
    assert!(page.facts.iter().any(|l| l.starts_with("3 game files and 1 older version will be moved to Kept files, saves are backed up.")), "{:?}", page.facts);
    assert!(page.facts.iter().any(|l| l.contains("Kept files") && l.contains("shipwright")), "{:?}", page.facts);
    assert!(page.paragraphs.iter().any(|l| l == "Kept: oot.o2r, oot-mq.o2r, zelda.z64."), "{:?}", page.paragraphs);
    assert!(page.paragraphs.iter().any(|l| l == "The rest of its folder is deleted: Internal storage > Ports > shipwright (200 MB)."), "the kept files are not counted as deleted: {:?}", page.paragraphs);
    assert!(page.paragraphs.iter().any(|l| l.starts_with("The app will not put them back when you reinstall")), "{:?}", page.paragraphs);
    let go = page.options.iter().find(|o| o.action == Act::UninstallYes).unwrap();
    assert_eq!(go.detail, "Keep the game files, delete the rest now");

    let plain = uninstall_page(&port(), Some(&FilesFacts { dir: "/home/deck/Ports/dusk".into(), ..FilesFacts::default() }), Some(&saves), &places);
    assert!(plain.facts.iter().all(|l| !l.contains("Kept files")));
    assert!(plain.paragraphs.iter().any(|l| l.starts_with("Its folder is deleted")));
}

#[test]
fn an_unreadable_game_files_record_disables_uninstall_and_says_why() {
    let f = FilesFacts { dir: "/home/deck/Ports/dusk".into(), keep_error: Some(crate::fork::uninstall_keep::UNREADABLE_RECORD.into()), ..FilesFacts::default() };
    let page = uninstall_page(&port(), Some(&f), Some(&SavesFacts::default()), &deck_places());
    assert!(page.facts.iter().any(|l| l.starts_with("The game files record could not be read")), "{:?}", page.facts);
    let go = page.options.iter().find(|o| o.action == Act::UninstallYes).unwrap();
    assert!(!go.enabled);
}

#[test]
fn a_card_mounted_by_its_device_name_is_sd_card() {
    let p = deck_places();
    assert_eq!(human_path(Path::new("/run/media/mmcblk0p1/Ports/dusk"), &p), "SD card > Ports > dusk");
    assert_eq!(human_path(Path::new("/run/media/sda1/ROMs"), &p), "USB drive > ROMs");
    assert_eq!(human_path(Path::new("/run/media/STICK/ROMs"), &p), "STICK > ROMs", "a real label stays");
    let mounted = Places { drives: vec![sd("/run/media/deck/mmcblk0p1", "mmcblk0p1")], ..deck_places() };
    assert_eq!(human_path(Path::new("/run/media/deck/mmcblk0p1/Emulation"), &mounted), "SD card > Emulation");
}

#[test]
fn the_fix_card_says_busy_instead_of_looking_forever_while_another_job_runs() {
    let p = port();
    let places = deck_places();
    assert_eq!(summary(Card::Fix, &p, None, &places), "Looking...", "the worker has not answered yet");
    let busy = DiskFacts { checkup: None, ..DiskFacts::default() };
    assert_eq!(summary(Card::Fix, &p, Some(&busy), &places), CHECKUP_BUSY);
    let fine = DiskFacts { checkup: Some(("Everything looks fine".into(), 0)), ..DiskFacts::default() };
    assert_eq!(summary(Card::Fix, &p, Some(&fine), &places), "Everything looks fine");
}
