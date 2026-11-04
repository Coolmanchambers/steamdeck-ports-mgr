//! Settings > App: Themes, Language, Tools and Files as pages instead of upstream's type-to-search lists. Themes and
//! Language are choice pages with the current one marked ("Same as the system" is Recommended for the language);
//! Tools is a page of links (each shows its address and a QR code first, as the Port page does); Files names each of
//! the app's own files by what it is ("Your port list", "Extra ports you added") and opens it in a text editor (Desktop
//! Mode only: the card is left out in Game Mode).
//!
//! What is saved is unchanged: the theme's name, the language's code (empty for the system's), nothing for Tools.
//! Pure data and words; `src/app/settings_choices_screen.rs` runs them.

use crate::fork::page::{Focus, Page, PageButton, PageOption, LEFT};
use crate::fork::rom_screens::util::ellipsize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceAct {
    Pick(usize),
    Back,
}

/// The app's own files the Files page offers: the file's name, what the page calls it, and what it holds.
pub const FILES: [(&str, &str, &str); 4] = [
    ("ports.json", "Your port list", "Every port the app knows about"),
    ("ports.local.json", "Extra ports you added", "Ports added by hand or from a link"),
    ("state.json", "Your app settings", "Choices, play time, favorites and your Discord text"),
    ("themes.json", "Themes", "The colours of each look"),
];

fn option(title: impl Into<String>, detail: impl Into<String>, action: ChoiceAct) -> PageOption<ChoiceAct> {
    PageOption { title: title.into(), detail: detail.into(), choice: None, enabled: true, recommended: false, action }
}

fn base(window: &str, header: &str, title: &str) -> Page<ChoiceAct> {
    let mut p = Page { window_title: window.to_string(), large: true, header: header.to_string(), title: title.to_string(), ..Page::default() };
    p.buttons[LEFT] = Some(PageButton { label: "Back".to_string(), action: ChoiceAct::Back });
    p
}

/// Themes: one choice per theme, the one in use marked and the cursor on it.
pub fn theme_page(header: &str, title: &str, names: &[String], current: &str) -> Page<ChoiceAct> {
    let mut p = base("Settings: App: Themes", header, title);
    p.paragraphs.push("Choose how the app looks. It changes at once.".to_string());
    for (i, name) in names.iter().enumerate() {
        let on = name == current;
        p.options.push(PageOption { choice: Some(on), ..option(ellipsize(name, 40), if on { "In use now" } else { "" }, ChoiceAct::Pick(i)) });
    }
    p.initial = Focus::Option(names.iter().position(|n| n == current).unwrap_or(0));
    p
}

/// Language: "Same as the system" first (Recommended), then each language by its own name; the one in use marked.
/// `languages` are (code, name); `current` is the saved code (empty: the system's).
pub fn language_page(header: &str, title: &str, system_label: &str, languages: &[(&str, &str)], current: &str) -> Page<ChoiceAct> {
    let mut p = base("Settings: App: Language", header, title);
    p.paragraphs.push("Choose the app's language.".to_string());
    let sys_on = current.is_empty();
    p.options.push(PageOption { choice: Some(sys_on), recommended: true, ..option(system_label, if sys_on { "In use now" } else { "Follows the Deck's own language" }, ChoiceAct::Pick(0)) });
    for (i, (code, name)) in languages.iter().enumerate() {
        let on = *code == current;
        p.options.push(PageOption { choice: Some(on), ..option(*name, if on { "In use now" } else { "" }, ChoiceAct::Pick(i + 1)) });
    }
    let at = if sys_on { 0 } else { languages.iter().position(|(c, _)| *c == current).map_or(0, |i| i + 1) };
    p.initial = Focus::Option(at);
    p
}

/// The code a language choice saves.
pub fn language_code<'a>(languages: &[(&'a str, &str)], pick: usize) -> Option<&'a str> {
    if pick == 0 { Some("") } else { languages.get(pick - 1).map(|(c, _)| *c) }
}

/// The web address without its scheme, for a card's detail.
fn host_and_path(url: &str) -> String {
    let bare = url.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/');
    ellipsize(bare, 66)
}

/// Tools: one card per link the app's port list recommends. `tools` are (name, address).
pub fn tools_page(header: &str, title: &str, tools: &[(String, String)]) -> Page<ChoiceAct> {
    let mut p = base("Settings: App: Tools", header, title);
    if tools.is_empty() {
        p.paragraphs.push("The app's port list names no tools yet. They arrive with its next update.".to_string());
        p.initial = Focus::Button(LEFT);
        return p;
    }
    p.paragraphs.push("Tools the app's port list recommends. Choose one to see its web page.".to_string());
    for (i, (name, url)) in tools.iter().enumerate() {
        p.options.push(option(ellipsize(name, 40), host_and_path(url), ChoiceAct::Pick(i)));
    }
    p.initial = Focus::Option(0);
    p
}

/// Files: the app's own files by what they are; choosing one opens it in a text editor.
pub fn files_page(header: &str, title: &str) -> Page<ChoiceAct> {
    let mut p = base("Settings: App: Files", header, title);
    p.paragraphs.push("The app keeps these files. Choosing one opens it in a text editor. Change them only if you know what you are doing.".to_string());
    for (i, (_, name, what)) in FILES.iter().enumerate() {
        p.options.push(option(*name, *what, ChoiceAct::Pick(i)));
    }
    p.initial = Focus::Option(0);
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::page::{action_at, initial_focus};
    use crate::fork::rom_screens::settings_pages::tests::{assert_fits, shown};

    const LANGS: [(&str, &str); 3] = [("en", "English"), ("fr", "Fran\u{e7}ais"), ("ja", "\u{65e5}\u{672c}\u{8a9e}")];

    #[test]
    fn the_language_page_starts_with_the_system_recommended_and_marks_the_one_in_use() {
        let p = language_page("Settings", "Language", "Same as the system", &LANGS, "");
        assert_eq!((p.options[0].title.as_str(), p.options[0].recommended, p.options[0].choice), ("Same as the system", true, Some(true)));
        assert_eq!(action_at(&p, initial_focus(&p)), Some(ChoiceAct::Pick(0)));
        let fr = language_page("Settings", "Language", "Same as the system", &LANGS, "fr");
        assert_eq!(fr.options.iter().map(|o| o.choice).collect::<Vec<_>>(), [Some(false), Some(false), Some(true), Some(false)]);
        assert_eq!(action_at(&fr, initial_focus(&fr)), Some(ChoiceAct::Pick(2)));
        assert_eq!((language_code(&LANGS, 0), language_code(&LANGS, 2), language_code(&LANGS, 9)), (Some(""), Some("fr"), None), "what is saved: the same codes as before");
    }

    #[test]
    fn the_theme_page_marks_the_theme_in_use() {
        let names = vec!["Default".to_string(), "Steam Deck".to_string(), "A theme with a very long name that goes on and on".to_string()];
        let p = theme_page("Settings", "Themes", &names, "Steam Deck");
        assert_eq!(p.options[1].choice, Some(true));
        assert_eq!(p.options[1].detail, "In use now");
        assert_eq!(action_at(&p, initial_focus(&p)), Some(ChoiceAct::Pick(1)));
        assert_fits(&p);
    }

    #[test]
    fn files_are_named_by_what_they_are_never_by_their_file_name() {
        let p = files_page("Settings", "Files");
        let titles: Vec<&str> = p.options.iter().map(|o| o.title.as_str()).collect();
        assert_eq!(titles, ["Your port list", "Extra ports you added", "Your app settings", "Themes"]);
        for t in shown(&p) {
            assert!(!t.contains(".json"), "{t}");
        }
        assert_eq!(FILES.iter().map(|f| f.0).collect::<Vec<_>>(), ["ports.json", "ports.local.json", "state.json", "themes.json"], "the same four files as upstream's list");
    }

    #[test]
    fn tools_are_cards_with_their_address_and_an_empty_list_says_why() {
        let tools = vec![("7-Zip".to_string(), "https://www.7-zip.org/".to_string())];
        let p = tools_page("Settings", "Tools", &tools);
        assert_eq!((p.options[0].title.as_str(), p.options[0].detail.as_str()), ("7-Zip", "www.7-zip.org"));
        let none = tools_page("Settings", "Tools", &[]);
        assert!(none.options.is_empty() && none.paragraphs[0].contains("names no tools yet"));
    }

    #[test]
    fn every_page_fits_and_says_it_plainly() {
        let long = vec![("A tool with a really very long name, longer than the card".to_string(), format!("https://example.org/{}", "x".repeat(90)))];
        for p in [
            theme_page("Settings", "Themes", &["Default".into()], "x"),
            language_page("Settings", "Language", "Same as the system", &LANGS, "ja"),
            tools_page("Settings", "Tools", &long),
            files_page("Settings", "Files"),
        ] {
            assert_fits(&p);
            assert_eq!(p.buttons[LEFT].as_ref().map(|b| b.action), Some(ChoiceAct::Back));
            for t in shown(&p) {
                assert!(!t.contains('\u{2014}') && !t.contains("catalog") && !t.contains(".json"), "{t}");
            }
        }
    }
}
