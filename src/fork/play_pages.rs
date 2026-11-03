//! Play's two questions, as pages: "Could not start <name>" (the port's folder or program is gone, or the system would not
//! start it) and "Which program starts <name>?" (a port with several programs, from Play or from Fix problems). They replace
//! upstream's "Error - X" two-button dialog and its bare list of file names ("Choose the program to start (remembered;
//! change it in Info)", "Choose favorite executable").
//!
//! Pure data and words; `src/app/play_screen.rs` runs them.

use crate::fork::art::thumbs::ArtRef;
use crate::fork::install_wizard::TITLE_FIT;
use crate::fork::page::{Card, Focus, Page, PageButton, PageOption};
use crate::fork::port_page::paths::{human_path, Places};
use crate::fork::rom_screens::util::ellipsize;
use std::path::{Path, PathBuf};

/// What a choice on one of these pages means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayAct {
    /// Download the port again (the install wizard's "Update this port?").
    Reinstall,
    /// The one-port game files page.
    GameFiles,
    /// Program `n` of the list.
    Program(usize),
    /// Play asks which program to start every time.
    AskEveryTime,
    /// B, the Back button: nothing starts, nothing changes.
    Back,
}

/// Why a port did not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// The port's folder is not there (the card was taken out, the folder was moved or deleted).
    FolderGone,
    /// The program it starts (its file name) is not there.
    ProgramGone(String),
    /// No program was found in its folder.
    NoProgram,
    /// The folder name in the port list is not one the app will use.
    BadFolderName,
    /// The system would not start the program: its file name, and why in a few plain words.
    WontStart { file: String, why: String },
}

/// The window title of both pages ("Play: Dusklight"; the main window's layer names the part before the colon).
pub fn window_title(name: &str) -> String {
    format!("Play: {name}")
}

fn base(name: &str, art: &Option<ArtRef>, title: &str) -> Page<PlayAct> {
    Page {
        window_title: window_title(name),
        large: true,
        header: name.to_string(),
        title: title.to_string(),
        card: Some(Card { art: art.clone(), qr: None, title: name.to_string(), lines: Vec::new() }),
        initial: Focus::Option(0),
        ..Page::default()
    }
}

fn option(title: &str, detail: &str, action: PlayAct) -> PageOption<PlayAct> {
    PageOption { title: title.to_string(), detail: detail.to_string(), choice: None, enabled: true, recommended: false, action }
}

fn recommended(mut o: PageOption<PlayAct>) -> PageOption<PlayAct> {
    o.recommended = true;
    o
}

/// "Could not start <name>", or "Could not start this port" when the name does not fit the title line.
pub fn could_not_start_title(name: &str) -> String {
    let full = format!("Could not start {name}");
    if full.chars().count() <= TITLE_FIT { full } else { "Could not start this port".to_string() }
}

/// A few plain words for why the system said no to starting a program.
pub fn wont_start_reason(kind: std::io::ErrorKind, os_error: Option<i32>) -> String {
    use std::io::ErrorKind;
    match (kind, os_error) {
        (ErrorKind::PermissionDenied, _) => "It is not allowed to run.".to_string(),
        (ErrorKind::NotFound, _) => "A file it needs is missing.".to_string(),
        (_, Some(8)) => "It is not a program this Deck can run.".to_string(),
        _ => "The system gave no reason.".to_string(),
    }
}

/// "Could not start <name>": what is wrong, and the ways to put it right. `needs_files`: the port names game files, so
/// connecting them is one of the ways.
pub fn could_not_start_page(name: &str, art: &Option<ArtRef>, problem: &Problem, needs_files: bool) -> Page<PlayAct> {
    let mut p = base(name, art, &could_not_start_title(name));
    let mut reinstall = true;
    p.paragraphs.push(match problem {
        Problem::FolderGone => format!("{name}'s folder is not there any more. It may have been moved, or the SD card it was on is out."),
        Problem::ProgramGone(file) => format!("The program it starts, {file}, is not there. The game files may be missing or incomplete."),
        Problem::NoProgram => "No program was found in its folder. The game files may be missing or incomplete.".to_string(),
        Problem::BadFolderName => {
            reinstall = false;
            "The folder name in the port list is not one this app can use, so it cannot be started. Its entry in the port list needs fixing.".to_string()
        }
        Problem::WontStart { file, why } => format!("The system would not start {file}. {why}"),
    });
    if reinstall {
        p.options.push(recommended(option("Reinstall", "Downloads it again; saves in its folder are backed up first", PlayAct::Reinstall)));
    }
    if needs_files {
        p.options.push(option("Game files", "Find the game files it needs and connect them", PlayAct::GameFiles));
    }
    p.options.push(option("Back", "Closes this page", PlayAct::Back));
    p.b_does(&PlayAct::Back)
}

// -- Which program ------------------------------------------------------------------------------------------------------

/// One program in the list: its file name and the folder it is in, the human way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramChoice {
    pub file: String,
    pub place: String,
}

/// At most this many programs are listed.
pub const MAX_PROGRAMS: usize = 30;

fn lower_alnum(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Each program as the page lists it.
pub fn choices_of(candidates: &[PathBuf], places: &Places) -> Vec<ProgramChoice> {
    candidates
        .iter()
        .map(|c| ProgramChoice {
            file: c.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "?".to_string()),
            place: c.parent().map(|d| human_path(d, places)).unwrap_or_default(),
        })
        .collect()
}

/// The program most likely to be the one: the only one in the shallowest folder, else the only one among those whose name
/// is the port's own (its folder or its name). `None` when nothing tells them apart.
pub fn likely_program(game_dir: &Path, candidates: &[PathBuf], folder: &str, name: &str) -> Option<usize> {
    let depth = |c: &PathBuf| crate::fork::program_detect::depth_in(game_dir, c);
    let shallow = candidates.iter().map(depth).min()?;
    let top: Vec<usize> = (0..candidates.len()).filter(|i| depth(&candidates[*i]) == shallow).collect();
    if let [only] = top.as_slice() {
        return Some(*only);
    }
    let mine = [lower_alnum(folder), lower_alnum(name)];
    let named: Vec<usize> = top
        .into_iter()
        .filter(|i| candidates[*i].file_stem().is_some_and(|s| mine.iter().any(|m| !m.is_empty() && *m == lower_alnum(&s.to_string_lossy()))))
        .collect();
    match named.as_slice() {
        [only] => Some(*only),
        _ => None,
    }
}

/// "Which program starts <name>?": one option per program (file name, then its folder), the likely one recommended.
/// `ask_every_time`: Fix problems and Port options may also leave it unchosen (Play then asks each time); Play's own
/// question has no such answer. `hidden`: programs beyond the list's end.
pub fn which_program_page(name: &str, art: &Option<ArtRef>, choices: &[ProgramChoice], likely: Option<usize>, ask_every_time: bool) -> Page<PlayAct> {
    let full = format!("Which program starts {name}?");
    let title = if full.chars().count() <= TITLE_FIT { full } else { "Which program should start?".to_string() };
    let mut p = base(name, art, &title);
    p.paragraphs.push(format!("{name} has more than one program. The one you choose is remembered. You can change it later under Program to start in its Port options."));
    for (i, c) in choices.iter().take(MAX_PROGRAMS).enumerate() {
        let mut o = option(&ellipsize(&c.file, 40), &ellipsize(&c.place, 66), PlayAct::Program(i));
        o.recommended = likely == Some(i);
        p.options.push(o);
    }
    if choices.len() > MAX_PROGRAMS {
        p.footnote = format!("{} more programs are not shown (the ones deepest in its folders).", choices.len() - MAX_PROGRAMS);
    }
    if ask_every_time {
        p.options.push(option("Ask every time", "Play asks which program to start each time", PlayAct::AskEveryTime));
    }
    p.buttons[crate::fork::page::LEFT] = Some(PageButton { label: "Back".to_string(), action: PlayAct::Back });
    p.initial = Focus::Option(likely.filter(|i| *i < MAX_PROGRAMS).unwrap_or(0));
    p.b_does(&PlayAct::Back)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::page::{action_at, initial_focus};

    const NAMES: [&str; 4] = ["X", "The Wind Waker HD", "Ship of Harkinian (Shipwright)", "The Legend of Zelda: The Wind Waker HD (ZeldaWWHDRecomp)"];

    fn problems() -> Vec<Problem> {
        vec![
            Problem::FolderGone,
            Problem::ProgramGone("ZeldaWWHDRecomp-linux-x86_64.AppImage".into()),
            Problem::NoProgram,
            Problem::BadFolderName,
            Problem::WontStart { file: "ZeldaWWHDRecomp-linux-x86_64.AppImage".into(), why: "It is not allowed to run.".into() },
        ]
    }

    fn game_dir() -> PathBuf {
        PathBuf::from("/run/media/deck/c14f9023-6854-443f-9348-735551234567/Ports/soh")
    }

    fn places() -> Places {
        Places { home: Some(PathBuf::from("/home/deck")), media_roots: vec![PathBuf::from("/run/media/deck"), PathBuf::from("/run/media")], ..Places::default() }
    }

    fn candidates() -> Vec<PathBuf> {
        let d = game_dir();
        vec![d.join("soh.sh"), d.join("soh-updater.sh"), d.join("a-folder-with-a-very-long-name/another-folder-with-a-longer-name/and-one-more-folder-name/deep/Tool.AppImage")]
    }

    fn all_pages() -> Vec<Page<PlayAct>> {
        let mut pages = Vec::new();
        for name in NAMES {
            for problem in problems() {
                for needs in [false, true] {
                    pages.push(could_not_start_page(name, &None, &problem, needs));
                }
            }
            let choices = choices_of(&candidates(), &places());
            for ask in [false, true] {
                pages.push(which_program_page(name, &None, &choices, Some(0), ask));
                pages.push(which_program_page(name, &None, &choices, None, ask));
            }
        }
        pages
    }

    /// The page's title is one line (about 40 characters) whatever the name; an option's title (40) and detail (66) are one
    /// line each; the words are plain.
    #[test]
    fn every_page_fits_with_the_longest_real_names_and_paths() {
        for p in all_pages() {
            assert!(p.title.chars().count() <= TITLE_FIT, "title too long ({}): {}", p.title.chars().count(), p.title);
            assert!(p.card.as_ref().is_some_and(|c| NAMES.contains(&c.title.as_str())), "the card names the port in full");
            for o in &p.options {
                assert!(o.title.chars().count() <= 40, "{:?}: option title too long: {}", p.title, o.title);
                assert!(o.detail.chars().count() <= 66, "{:?}: detail too long ({}): {}", p.title, o.detail.chars().count(), o.detail);
                assert!(!o.title.is_empty());
            }
            for b in p.buttons.iter().flatten() {
                assert!(b.label.chars().count() <= 16);
            }
            for t in p.paragraphs.iter().chain(p.options.iter().map(|o| &o.detail)).chain(p.options.iter().map(|o| &o.title)) {
                assert!(!t.contains('\u{2014}') && !t.contains("--"), "{t}");
                for jargon in ["executable", "favorite", "Favorite", "exe ", "upstream", "json", "asset", "os error"] {
                    assert!(!t.contains(jargon), "jargon {jargon:?}: {t}");
                }
            }
            assert!(!p.title.starts_with("Error") && !p.title.contains("Invalid"), "{}", p.title);
        }
    }

    #[test]
    fn the_title_says_what_happened_and_falls_back_for_a_long_name() {
        assert_eq!(could_not_start_title("Dusklight"), "Could not start Dusklight");
        assert_eq!(could_not_start_title(NAMES[3]), "Could not start this port");
        assert_eq!(could_not_start_title(&"a".repeat(24)).chars().count(), 40, "a name of 24 characters just fits");
        assert_eq!(could_not_start_title(&"a".repeat(25)), "Could not start this port");
    }

    #[test]
    fn could_not_start_offers_reinstall_game_files_and_back() {
        let p = could_not_start_page("Dusklight", &None, &Problem::ProgramGone("dusk.sh".into()), true);
        assert_eq!(p.paragraphs, ["The program it starts, dusk.sh, is not there. The game files may be missing or incomplete."]);
        let opts: Vec<(&str, &str)> = p.options.iter().map(|o| (o.title.as_str(), o.detail.as_str())).collect();
        assert_eq!(
            opts,
            [("Reinstall", "Downloads it again; saves in its folder are backed up first"), ("Game files", "Find the game files it needs and connect them"), ("Back", "Closes this page")]
        );
        assert_eq!(p.options.iter().map(|o| o.action).collect::<Vec<_>>(), [PlayAct::Reinstall, PlayAct::GameFiles, PlayAct::Back]);
        assert!(p.options[0].recommended);
        assert_eq!(action_at(&p, initial_focus(&p)), Some(PlayAct::Reinstall));
        assert_eq!(p.back_hint, "Back");
        assert_eq!(p.window_title, "Play: Dusklight");
        assert_eq!(p.card.as_ref().unwrap().title, "Dusklight");
    }

    #[test]
    fn game_files_are_offered_only_to_a_port_that_names_some() {
        let p = could_not_start_page("Dusklight", &None, &Problem::FolderGone, false);
        assert_eq!(p.options.iter().map(|o| o.action).collect::<Vec<_>>(), [PlayAct::Reinstall, PlayAct::Back]);
    }

    #[test]
    fn a_bad_folder_name_cannot_be_reinstalled_and_a_refusal_has_no_suggested_fix() {
        let bad = could_not_start_page("Dusklight", &None, &Problem::BadFolderName, true);
        assert_eq!(bad.options.iter().map(|o| o.action).collect::<Vec<_>>(), [PlayAct::GameFiles, PlayAct::Back]);
        assert!(bad.options.iter().all(|o| !o.recommended));
        let refused = could_not_start_page("Dusklight", &None, &Problem::WontStart { file: "dusk".into(), why: "It is not allowed to run.".into() }, false);
        assert_eq!(refused.paragraphs[0], "The system would not start dusk. It is not allowed to run.");
        assert_eq!(wont_start_reason(std::io::ErrorKind::PermissionDenied, Some(13)), "It is not allowed to run.");
        assert_eq!(wont_start_reason(std::io::ErrorKind::Other, Some(8)), "It is not a program this Deck can run.");
        assert_eq!(wont_start_reason(std::io::ErrorKind::Other, None), "The system gave no reason.");
    }

    #[test]
    fn the_program_question_lists_each_program_with_its_folder() {
        let choices = choices_of(&candidates(), &places());
        assert_eq!(choices[0], ProgramChoice { file: "soh.sh".into(), place: "SD card > Ports > soh".into() });
        let p = which_program_page("Ship of Harkinian", &None, &choices, Some(0), false);
        assert_eq!(p.title, "Which program starts Ship of Harkinian?");
        assert_eq!(p.options.iter().map(|o| o.title.as_str()).collect::<Vec<_>>(), ["soh.sh", "soh-updater.sh", "Tool.AppImage"]);
        assert_eq!(p.options[0].detail, "SD card > Ports > soh");
        assert!(p.options[2].detail.chars().count() <= 66 && p.options[2].detail.contains("..."), "a deep folder is cut to its line");
        assert!(p.options[0].recommended && !p.options[1].recommended);
        assert!(p.paragraphs[0].contains("remembered"));
        assert_eq!(action_at(&p, initial_focus(&p)), Some(PlayAct::Program(0)));
        assert_eq!(p.back_hint, "Back");
        assert!(p.options.iter().all(|o| o.action != PlayAct::AskEveryTime), "Play's own question has no ask-every-time answer");
        assert_eq!(which_program_page(NAMES[3], &None, &choices, None, false).title, "Which program should start?");
    }

    #[test]
    fn fix_problems_and_port_options_may_leave_it_to_ask_every_time() {
        let choices = choices_of(&candidates(), &places());
        let p = which_program_page("Dusklight", &None, &choices, None, true);
        let last = p.options.last().unwrap();
        assert_eq!((last.title.as_str(), last.detail.as_str(), last.action), ("Ask every time", "Play asks which program to start each time", PlayAct::AskEveryTime));
        assert!(p.options.iter().all(|o| !o.recommended), "nothing tells them apart");
    }

    #[test]
    fn a_long_list_is_cut_and_says_so() {
        let many: Vec<PathBuf> = (0..40).map(|i| game_dir().join(format!("p{i}.sh"))).collect();
        let p = which_program_page("X", &None, &choices_of(&many, &places()), None, true);
        assert_eq!(p.options.len(), MAX_PROGRAMS + 1);
        assert_eq!(p.footnote, "10 more programs are not shown (the ones deepest in its folders).");
    }

    #[test]
    fn the_likely_program_is_the_only_shallowest_or_the_one_named_like_the_port() {
        let d = game_dir();
        let deep = d.join("jdk/bin/java");
        assert_eq!(likely_program(&d, &[deep.clone(), d.join("launch.sh")], "x", "X"), Some(1), "the only one in the shallowest folder");
        assert_eq!(likely_program(&d, &[d.join("a.sh"), d.join("b.sh")], "x", "X"), None, "nothing tells them apart");
        assert_eq!(likely_program(&d, &[d.join("soh-updater.sh"), d.join("SoH.sh")], "soh", "Ship of Harkinian"), Some(1), "named like the port's folder");
        assert_eq!(likely_program(&d, &[d.join("soh.sh"), d.join("soh.AppImage")], "soh", "Ship"), None, "two with its name");
        assert_eq!(likely_program(&d, &[], "x", "X"), None);
    }
}
