use super::*;
use crate::core::models::port_from_value;
use crate::fork::steam_queue::{self, ApplyInputs};
use crate::fork::vdf::fixtures::*;
use serde_json::json;
use std::io;

const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];
const JPG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3];
const ICO: &[u8] = &[0, 0, 1, 0, 1, 0, 1, 2];

fn no_steam() -> io::Result<Vec<String>> {
    Ok(Vec::new())
}

fn steam_up() -> io::Result<Vec<String>> {
    Ok(vec!["77 steam".into()])
}

fn put(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn real_png(side: u32) -> Vec<u8> {
    let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(side, side, image::Rgba([200, 10, 10, 255])));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn port(folder: &str, icon_url: bool) -> Port {
    let mut v = json!({"name": format!("{folder} Port"), "folder": folder, "source": "https://example.com/x.zip"});
    if icon_url {
        v["icon"] = json!("https://example.com/icon.png");
    }
    port_from_value(&v).unwrap()
}

struct World {
    root: PathBuf,
    home: PathBuf,
    config: PathBuf,
    cache: PathBuf,
    library: PathBuf,
    vdf: PathBuf,
    grid: PathBuf,
}

impl World {
    fn new(name: &str) -> World {
        let root = std::env::temp_dir().join(format!("dpm_steam_icons_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let home = root.join("home");
        let user = home.join(".local/share/Steam/userdata/1234");
        let w = World {
            config: root.join("app"),
            cache: root.join("app/cache"),
            library: root.join("sd/Ports"),
            vdf: user.join("config/shortcuts.vdf"),
            grid: user.join("config/grid"),
            home,
            root,
        };
        fs::create_dir_all(&w.cache).unwrap();
        fs::create_dir_all(&w.library).unwrap();
        fs::create_dir_all(&w.grid).unwrap();
        w
    }

    fn dir(&self, folder: &str) -> PathBuf {
        self.library.join(folder)
    }

    fn write_vdf(&self, entries: &[Vec<u8>]) {
        put(&self.vdf, &file_of(entries));
    }

    /// A shortcut for a port's program (`<folder>/run`) with the icon text as given.
    fn entry(&self, index: usize, app_id: u32, folder: &str, icon: &str, options: &str) -> Vec<u8> {
        let exe = format!("\"{}\"", self.dir(folder).join("run").display());
        let start = format!("\"{}\"", self.dir(folder).display());
        steam_entry_with(index, app_id, &format!("{folder} shortcut"), &exe, &start, icon, options)
    }

    fn inputs(&self) -> PlanInputs<'_> {
        PlanInputs { config_dir: &self.config, cache_dir: &self.cache, library_dir: &self.library }
    }

    fn backup(&self, stamp: &str, entries: &[Vec<u8>]) {
        let name = format!("shortcuts.vdf.bak-{stamp}");
        put(&self.vdf.with_file_name(name), &file_of(entries));
    }

    fn list(&self) -> Vec<shortcuts::Shortcut> {
        shortcuts::list(&ShortcutsFile::load(&self.vdf).unwrap().doc)
    }

    fn plan_for(&self, ports: &[Port]) -> Plan {
        let findings = diagnose(&self.home, &self.library, ports).unwrap();
        plan(&self.inputs(), &findings, ports)
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn which_files_steam_can_show() {
    let w = World::new("showable");
    let dir = &w.root;
    put(&dir.join("a.png"), PNG);
    put(&dir.join("b.JPG"), JPG);
    put(&dir.join("c.jpeg"), JPG);
    put(&dir.join("d.ico"), ICO);
    put(&dir.join("e.tga"), b"anything");
    for ok in ["a.png", "b.JPG", "c.jpeg", "d.ico", "e.tga"] {
        assert!(is_showable_image(&dir.join(ok)), "{ok}");
    }
    put(&dir.join("empty.png"), b"");
    put(&dir.join("text.png"), b"<html>404</html>");
    put(&dir.join("pic.webp"), b"RIFFxxxxWEBP");
    put(&dir.join("noext"), PNG);
    put(&dir.join("jpg-named-png.png"), JPG);
    for bad in ["empty.png", "text.png", "pic.webp", "noext", "jpg-named-png.png", "missing.png"] {
        assert!(!is_showable_image(&dir.join(bad)), "{bad}");
    }
    assert!(!is_showable_image(dir), "a folder is not a picture");
}

#[test]
fn every_icon_state() {
    let w = World::new("states");
    let port_dir = w.dir("Zelda");
    let grid = &w.grid;
    put(&port_dir.join("art/icon.png"), PNG);
    put(&grid.join("77_icon.png"), PNG);
    put(&w.root.join("elsewhere/icon.png"), PNG);
    put(&port_dir.join("art/bad.png"), b"nope");
    let state = |stored: &str| icon_state(stored, &port_dir, grid);
    assert_eq!(state(""), IconState::Empty);
    assert_eq!(state("\"\""), IconState::Empty, "only quote marks is nothing");
    assert_eq!(state(&port_dir.join("art/icon.png").to_string_lossy()), IconState::Ok);
    assert_eq!(state(&grid.join("77_icon.png").to_string_lossy()), IconState::Ok, "Steam's own grid folder is fine");
    let outside = w.root.join("elsewhere/icon.png").to_string_lossy().into_owned();
    assert_eq!(state(&outside), IconState::Outside(outside.clone()));
    let missing = port_dir.join("art/gone.png").to_string_lossy().into_owned();
    assert_eq!(state(&missing), IconState::Missing(missing.clone()));
    let bad = port_dir.join("art/bad.png").to_string_lossy().into_owned();
    assert_eq!(state(&bad), IconState::NotAnImage(bad.clone()));
    let quoted_there = port_dir.join("art/icon.png").to_string_lossy().into_owned();
    assert_eq!(state(&format!("\"{quoted_there}\"")), IconState::Quoted { path: quoted_there.clone(), exists: true });
    assert_eq!(state(&format!("\"{missing}\"")), IconState::Quoted { path: missing.clone(), exists: false });
    for fixable in [IconState::Empty, IconState::Missing(String::new()), IconState::NotAnImage(String::new()), IconState::Quoted { path: String::new(), exists: true }] {
        assert!(fixable.needs_fix());
    }
    assert!(!IconState::Ok.needs_fix() && !IconState::Outside(String::new()).needs_fix());
}

#[test]
fn the_wording_names_the_path() {
    assert_eq!(IconState::Missing("/run/media/x/tp.png".into()).wording(), "icon file missing (/run/media/x/tp.png)");
    assert!(IconState::Quoted { path: "/a.png".into(), exists: true }.wording().contains("quote marks"));
    assert!(IconState::Quoted { path: "/a.png".into(), exists: false }.wording().contains("missing"));
    assert_eq!(IconState::Empty.wording(), "no icon is set");
}

#[test]
fn diagnose_lists_each_shortcut_of_a_port_and_the_longest_folder_wins() {
    let w = World::new("diagnose");
    put(&w.dir("Zelda/art/i.png"), PNG);
    let nested = port("Zelda/mods", false);
    let ports = [port("Zelda", false), nested, port("Other", false)];
    let foreign = steam_entry_with(2, 9, "Chiaki", "\"/usr/bin/chiaki\"", "\"/usr/bin/\"", "", "");
    w.write_vdf(&[
        w.entry(0, 1, "Zelda", &w.dir("Zelda/art/i.png").to_string_lossy(), ""),
        w.entry(1, 2, "Zelda/mods", "", ""),
        foreign,
        w.entry(3, 3, "Other", "\"/nowhere/x.png\"", ""),
    ]);
    let findings = diagnose(&w.home, &w.library, &ports).unwrap();
    let summary: Vec<(&str, u32, bool)> = findings.iter().map(|f| (f.port_key.as_str(), f.app_id.unwrap(), f.state.needs_fix())).collect();
    assert_eq!(summary, [("Zelda", 1, false), ("Zelda/mods", 2, true), ("Other", 3, true)], "Chiaki belongs to no port; the nested folder's shortcut is the nested port's");
    assert_eq!(findings[0].state, IconState::Ok);
    assert_eq!(findings[1].state, IconState::Empty);
    assert!(matches!(findings[2].state, IconState::Quoted { exists: false, .. }));
    assert_eq!((findings[0].account_id, findings[0].shortcut_name.as_str()), (1234, "Zelda shortcut"));
    assert!(findings[0].describe().starts_with("Zelda Port: icon is fine"));
}

#[test]
fn an_unreadable_shortcuts_file_is_an_error_not_no_findings() {
    let w = World::new("unreadable");
    put(&w.vdf, b"not a vdf at all");
    assert!(diagnose(&w.home, &w.library, &[port("Zelda", false)]).is_err());
}

#[test]
fn a_quoted_icon_whose_file_is_there_is_fixed_by_dropping_the_quotes() {
    let w = World::new("unquote");
    let icon = w.dir("Zelda/art/i.png");
    put(&icon, PNG);
    w.write_vdf(&[w.entry(0, 5, "Zelda", &format!("\"{}\"", icon.display()), "-x")]);
    let ports = [port("Zelda", false)];
    let p = w.plan_for(&ports);
    assert_eq!(p.items.len(), 1);
    assert_eq!(p.items[0].replacement, Replacement::Use { path: icon.clone(), source: Source::Unquoted });
    assert!(p.items[0].line().contains("without the quote marks"));
}

#[test]
fn source_one_is_the_icon_the_same_app_id_had_in_a_backup() {
    let w = World::new("backup");
    let good = w.dir("Zelda/art/i.png");
    put(&good, PNG);
    let gone = w.dir("Zelda/art/gone.png");
    w.write_vdf(&[w.entry(0, 5, "Zelda", &gone.to_string_lossy(), "")]);
    // The newest backup holds the same dead icon (no use); an older one holds the good one; a still older one another.
    w.backup("20261020-100000", &[w.entry(0, 5, "Zelda", &gone.to_string_lossy(), "")]);
    w.backup("20261019-100000", &[w.entry(0, 5, "Zelda", &good.to_string_lossy(), "")]);
    w.backup("20261018-100000", &[w.entry(0, 5, "Zelda", &w.dir("Zelda/other.png").to_string_lossy(), "")]);
    // Backdate so "newest first" follows the stamps.
    for (stamp, age) in [("20261020-100000", 100), ("20261019-100000", 200), ("20261018-100000", 300)] {
        let f = fs::File::options().write(true).open(w.vdf.with_file_name(format!("shortcuts.vdf.bak-{stamp}"))).unwrap();
        f.set_modified(SystemTime::now() - std::time::Duration::from_secs(age * 1000)).unwrap();
    }
    let p = w.plan_for(&[port("Zelda", true)]);
    assert_eq!(p.items[0].replacement, Replacement::Use { path: good, source: Source::Backup("shortcuts.vdf.bak-20261019-100000".into()) });
}

#[test]
fn a_backup_icon_in_an_old_folder_is_taken_from_the_port_s_current_folder() {
    let w = World::new("moved");
    let old = w.root.join("home/Applications/dpm/Library/Zelda");
    put(&old.join("art/i.png"), PNG);
    let new_icon = w.dir("Zelda/art/i.png");
    put(&new_icon, PNG);
    fs::write(
        w.config.join("port-moves.json"),
        json!({"version": 1, "old_copies": [{"port_key": "Zelda", "name": "Zelda", "old_path": old.to_string_lossy(), "new_path": w.dir("Zelda").to_string_lossy()}]}).to_string(),
    )
    .unwrap();
    w.write_vdf(&[w.entry(0, 5, "Zelda", &w.dir("Zelda/gone.png").to_string_lossy(), "")]);
    w.backup("20261019-100000", &[w.entry(0, 5, "Zelda", &old.join("art/i.png").to_string_lossy(), "")]);
    let p = w.plan_for(&[port("Zelda", false)]);
    assert_eq!(p.items[0].replacement, Replacement::Use { path: new_icon, source: Source::BackupMoved("shortcuts.vdf.bak-20261019-100000".into()) }, "the old copy still exists, but it is not where the icon should live");
}

#[test]
fn a_backup_icon_that_only_exists_in_an_old_copy_is_not_used() {
    let w = World::new("old_only");
    let old = w.root.join("home/Applications/dpm/Library/Zelda");
    put(&old.join("art/i.png"), PNG);
    fs::write(
        w.config.join("port-moves.json"),
        json!({"version": 1, "old_copies": [{"port_key": "Zelda", "name": "Zelda", "old_path": old.to_string_lossy(), "new_path": w.dir("Zelda").to_string_lossy()}]}).to_string(),
    )
    .unwrap();
    w.write_vdf(&[w.entry(0, 5, "Zelda", "", "")]);
    w.backup("20261019-100000", &[w.entry(0, 5, "Zelda", &old.join("art/i.png").to_string_lossy(), "")]);
    let p = w.plan_for(&[port("Zelda", false)]);
    assert!(matches!(p.items[0].replacement, Replacement::Nothing(_)), "{:?}", p.items[0].replacement);
}

#[test]
fn source_two_is_an_icon_already_in_the_grid_folder_and_it_beats_the_catalog() {
    let w = World::new("grid");
    w.write_vdf(&[w.entry(0, 5, "Zelda", "", "")]);
    put(&w.grid.join("5_icon.png"), PNG);
    put(&w.cache.join("Zelda_icon.png"), &real_png(64));
    let p = w.plan_for(&[port("Zelda", true)]);
    assert_eq!(p.items[0].replacement, Replacement::Use { path: w.grid.join("5_icon.png"), source: Source::Grid });
}

#[test]
fn a_backup_beats_the_grid_which_beats_the_catalog() {
    let w = World::new("order");
    let good = w.dir("Zelda/art/i.png");
    put(&good, PNG);
    w.write_vdf(&[w.entry(0, 5, "Zelda", "", "")]);
    put(&w.grid.join("5_icon.png"), PNG);
    put(&w.cache.join("Zelda_icon.png"), &real_png(64));
    w.backup("20261019-100000", &[w.entry(0, 5, "Zelda", &good.to_string_lossy(), "")]);
    let ports = [port("Zelda", true)];
    assert!(matches!(w.plan_for(&ports).items[0].replacement, Replacement::Use { source: Source::Backup(_), .. }));
    fs::remove_file(w.vdf.with_file_name("shortcuts.vdf.bak-20261019-100000")).unwrap();
    assert!(matches!(w.plan_for(&ports).items[0].replacement, Replacement::Use { source: Source::Grid, .. }));
    fs::remove_file(w.grid.join("5_icon.png")).unwrap();
    assert!(matches!(w.plan_for(&ports).items[0].replacement, Replacement::WriteCatalog { .. }));
}

#[test]
fn source_four_says_why_nothing_was_found() {
    let w = World::new("nothing");
    w.write_vdf(&[w.entry(0, 5, "Zelda", "", "")]);
    let p = w.plan_for(&[port("Zelda", true)]);
    match &p.items[0].replacement {
        Replacement::Nothing(why) => assert!(why.contains("not been downloaded"), "{why}"),
        other => panic!("{other:?}"),
    }
    let p = w.plan_for(&[port("Zelda", false)]);
    match &p.items[0].replacement {
        Replacement::Nothing(why) => assert!(why.contains("catalog has none"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(p.fixable(), 0);
    assert!(p.items[0].line().contains("cannot be fixed"));
}

#[test]
fn a_grid_file_that_cannot_be_shown_is_left_alone_and_said() {
    let w = World::new("grid_junk");
    w.write_vdf(&[w.entry(0, 5, "Zelda", "", "")]);
    put(&w.grid.join("5_icon.png"), b"junk the user put here");
    put(&w.cache.join("Zelda_icon.png"), &real_png(64));
    let ports = [port("Zelda", true)];
    let p = w.plan_for(&ports);
    assert!(matches!(&p.items[0].replacement, Replacement::Nothing(why) if why.contains("left alone")), "{:?}", p.items[0].replacement);
    let r = resolve(&p);
    assert!(r.fixes.is_empty());
    assert_eq!(fs::read(w.grid.join("5_icon.png")).unwrap(), b"junk the user put here");
}

#[test]
fn a_shortcut_without_an_app_id_cannot_be_fixed() {
    let w = World::new("no_id");
    // An entry with no appid field: build it by hand.
    let exe = format!("\"{}\"", w.dir("Zelda").join("run").display());
    let entry = object(
        "0",
        &[str_field("AppName", b"Zelda shortcut"), str_field("Exe", exe.as_bytes()), str_field("StartDir", b""), str_field("icon", b""), str_field("LaunchOptions", b"")],
    );
    w.write_vdf(&[entry]);
    let p = w.plan_for(&[port("Zelda", true)]);
    assert!(matches!(&p.items[0].replacement, Replacement::Nothing(why) if why.contains("app ID")));
}

#[test]
fn the_catalog_icon_is_written_to_the_grid_and_user_art_is_never_overwritten() {
    let w = World::new("catalog");
    w.write_vdf(&[w.entry(0, 5, "Zelda", "", "")]);
    put(&w.cache.join("Zelda_icon.png"), &real_png(64));
    let p = w.plan_for(&[port("Zelda", true)]);
    assert!(matches!(p.items[0].replacement, Replacement::WriteCatalog { .. }));
    assert!(!w.grid.join("5_icon.png").exists(), "planning writes nothing");
    let r = resolve(&p);
    assert_eq!(r.fixes.len(), 1, "{r:?}");
    assert_eq!(r.fixes[0].to, w.grid.join("5_icon.png").to_string_lossy());
    assert_eq!(r.fixes[0].from, "");
    assert!(is_showable_image(&w.grid.join("5_icon.png")));
    // The art the user chose appears between planning and resolving: it stays as it is.
    fs::remove_file(w.grid.join("5_icon.png")).unwrap();
    put(&w.grid.join("5_icon.jpg"), JPG);
    let r = resolve(&p);
    assert!(r.fixes.is_empty(), "{r:?}");
    assert_eq!(r.left.len(), 1);
    assert_eq!(fs::read(w.grid.join("5_icon.jpg")).unwrap(), JPG);
    assert!(!w.grid.join("5_icon.png").exists());
}

#[test]
fn fix_doc_changes_only_the_icon_field_and_only_while_it_is_as_found() {
    let w = World::new("fixdoc");
    let bytes = file_of(&[w.entry(0, 5, "Zelda", "\"/gone/i.png\"", "SteamDeck=1 %command%"), w.entry(1, 6, "Other", "/keep/me.png", "-keep")]);
    let before = Document::parse(&bytes).unwrap();
    let mut doc = Document::parse(&bytes).unwrap();
    let fix = IconFix { account_id: 1234, account_label: String::new(), port_key: "Zelda".into(), name: "Zelda shortcut".into(), app_id: 5, from: "\"/gone/i.png\"".into(), to: "/new/i.png".into() };
    let result = fix_doc(&mut doc, &[&fix]);
    assert_eq!(result.changed, [("Zelda shortcut".to_string(), 5, "\"/gone/i.png\"".to_string(), "/new/i.png".to_string())]);
    let (a, b) = (shortcuts::list(&before), shortcuts::list(&doc));
    assert_eq!(b[0].icon, "/new/i.png");
    assert_eq!((&b[0].app_name, &b[0].exe, &b[0].start_dir, &b[0].launch_options, b[0].app_id), (&a[0].app_name, &a[0].exe, &a[0].start_dir, &a[0].launch_options, a[0].app_id));
    assert_eq!(b[1], a[1], "the other shortcut is untouched");
    // Run again: already done, nothing changes and nothing is reported.
    let again = fix_doc(&mut doc, &[&fix]);
    assert_eq!(again, FixResult::default());
    // The icon changed since the problem was found: left alone.
    let mut doc = Document::parse(&bytes).unwrap();
    let stale = IconFix { from: "/something/else.png".into(), ..fix.clone() };
    let result = fix_doc(&mut doc, &[&stale]);
    assert!(result.changed.is_empty());
    assert_eq!(result.skipped.len(), 1);
    assert_eq!(shortcuts::list(&doc)[0].icon, "\"/gone/i.png\"");
    // A shortcut that is gone.
    let gone = IconFix { app_id: 999, ..fix };
    assert_eq!(fix_doc(&mut doc, &[&gone]).skipped, [("Zelda shortcut".to_string(), "no longer in Steam".to_string())]);
}

fn fix_for(app_id: u32, from: &str, to: &str) -> IconFix {
    IconFix { account_id: 1234, account_label: "Tim".into(), port_key: "Zelda".into(), name: "Zelda shortcut".into(), app_id, from: from.into(), to: to.into() }
}

#[test]
fn with_steam_closed_the_file_is_written_with_a_backup_and_only_the_icon_changes() {
    let w = World::new("write");
    w.write_vdf(&[w.entry(0, 5, "Zelda", "\"/gone/i.png\"", "-opt")]);
    let fix = fix_for(5, "\"/gone/i.png\"", "/new/i.png");
    let settings = ForkSettings::load(&w.config.join("fork-settings.json"));
    let report = apply(std::slice::from_ref(&fix), &w.home, &no_steam, &settings, false, SystemTime::now());
    assert_eq!(report.written.len(), 1, "{report:?}");
    assert!(report.queued.is_empty() && report.failed.is_empty());
    let now = &w.list()[0];
    assert_eq!((now.icon.as_str(), now.launch_options.as_str(), now.app_id), ("/new/i.png", "-opt", Some(5)));
    let backups = backups::list_backups(&w.vdf).unwrap();
    assert_eq!(backups.len(), 1, "one backup of the file as it was");
    assert!(Document::parse(&fs::read(&backups[0].path).unwrap()).is_ok());
}

#[test]
fn with_steam_running_or_in_game_mode_nothing_is_written_and_the_fixes_are_queued() {
    let w = World::new("queue");
    w.write_vdf(&[w.entry(0, 5, "Zelda", "", "")]);
    let before = fs::read(&w.vdf).unwrap();
    let fix = fix_for(5, "", "/new/i.png");
    let settings = ForkSettings::load(&w.config.join("fork-settings.json"));
    let up = apply(std::slice::from_ref(&fix), &w.home, &steam_up, &settings, false, SystemTime::now());
    assert_eq!(up.queued, [fix.clone()]);
    assert!(up.written.is_empty());
    let game = apply(std::slice::from_ref(&fix), &w.home, &no_steam, &settings, true, SystemTime::now());
    assert_eq!(game.queued, [fix.clone()], "Game Mode never writes, whatever the process check says");
    assert_eq!(fs::read(&w.vdf).unwrap(), before, "shortcuts.vdf untouched");
    assert!(backups::list_backups(&w.vdf).unwrap().is_empty());

    // Queue them, read them back, apply them with Steam closed.
    let queue_file = steam_queue::queue_path(&w.config);
    steam_queue::queue_changes(&queue_file, queue_entries(&[fix.clone()], false), false).unwrap();
    let loaded = steam_queue::load(&queue_file);
    assert!(loaded.problem.is_none(), "{:?}", loaded.problem);
    assert_eq!(loaded.queue.len(), 1);
    let entry = &loaded.queue.entries[0];
    assert_eq!((entry.kind, entry.icon.clone()), (QueuedKind::Icon, Some(IconChange { app_id: 5, from: String::new(), to: "/new/i.png".into() })));
    assert!(!loaded.queue.contains(1234, "Zelda"), "an icon repair is not an add or update");
    let inputs = ApplyInputs { home: &w.home, library_dir: &w.library, catalog: &[], favorite_exe: &|_| None, cache_dir: None, settings: &settings, game_mode: false };
    let still_up = steam_queue::apply_queue(&loaded.queue, &inputs, &steam_up, SystemTime::now());
    assert!(still_up.refused && !still_up.wrote);
    assert_eq!(still_up.remaining.len(), 1, "with Steam running the entry stays queued");
    assert_eq!(fs::read(&w.vdf).unwrap(), before);
    let applied = steam_queue::apply_queue(&loaded.queue, &inputs, &no_steam, SystemTime::now());
    assert!(applied.wrote && !applied.refused, "{}", applied.text);
    assert!(applied.remaining.is_empty());
    assert_eq!(applied.done, 1);
    assert_eq!(w.list()[0].icon, "/new/i.png");
    assert!(applied.text.contains("icon is now /new/i.png"), "{}", applied.text);
    // Applying the same entry again finds it done.
    let again = steam_queue::apply_queue(&loaded.queue, &inputs, &no_steam, SystemTime::now());
    assert!(!again.wrote && again.remaining.is_empty(), "{}", again.text);
    let game = ApplyInputs { game_mode: true, ..ApplyInputs { home: &w.home, library_dir: &w.library, catalog: &[], favorite_exe: &|_| None, cache_dir: None, settings: &settings, game_mode: true } };
    assert!(steam_queue::apply_queue(&loaded.queue, &game, &no_steam, SystemTime::now()).refused);
}

#[test]
fn a_second_queued_repair_for_the_same_shortcut_replaces_the_first() {
    let _w = World::new("upsert");
    let mut q = steam_queue::SteamQueue::default();
    for to in ["/a.png", "/b.png"] {
        for e in queue_entries(&[fix_for(5, "", to)], false) {
            q.upsert(e);
        }
    }
    for e in queue_entries(&[fix_for(6, "", "/c.png")], false) {
        q.upsert(e);
    }
    assert_eq!(q.entries.iter().map(|e| e.icon.as_ref().unwrap().to.as_str()).collect::<Vec<_>>(), ["/b.png", "/c.png"]);
}

#[test]
fn the_ports_own_ownership_rule_ignores_a_shortcut_outside_every_port() {
    let dirs = [(0usize, PathBuf::from("/lib/A")), (1, PathBuf::from("/lib/A/inner"))];
    assert_eq!(owner_of(Path::new("/lib/A/inner/run"), &dirs), Some(1));
    assert_eq!(owner_of(Path::new("/lib/A/run"), &dirs), Some(0));
    assert_eq!(owner_of(Path::new("/lib/AB/run"), &dirs), None, "whole path components only");
}
