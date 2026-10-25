//! "Add a port": the page the Home screen's plus tile (and Settings > Ports > Add a port) opens. One question, a few
//! big cards with a picture and one plain line each. Pure data for `fork::page`; what each card does is
//! `src/app/add_port_screen.rs`.

use crate::fork::page::{Focus, Icon, Page, PageButton, PageOption, LEFT};

pub const TITLE: &str = "Add a port";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// The catalog list (All ports).
    Browse,
    /// The Deck-ready list.
    DeckReady,
    /// Import ports: bring in ports that are already on this Deck.
    Import,
    /// A port that is not on the list, from its GitHub page.
    GithubLink,
    Back,
}

/// The cards, top to bottom. In big mode the catalog has no list of its own (All ports shows only the ports you have;
/// the Deck-ready list is the one to browse), so "Browse all ports" is left out there rather than leading to the same list.
pub fn acts(big_mode: bool) -> Vec<Act> {
    let mut acts = vec![Act::DeckReady];
    if !big_mode {
        acts.push(Act::Browse);
    }
    acts.push(Act::Import);
    acts.push(Act::GithubLink);
    acts
}

fn card(act: Act) -> (&'static str, &'static str, Icon) {
    match act {
        Act::DeckReady => ("Deck-ready ports", "Ports checked to run well on the Deck", Icon::DeckReady),
        Act::Browse => ("Browse all ports", "Every port this app knows about", Icon::Browse),
        Act::Import => ("Ports you already have", "Find ports on this Deck and bring them in", Icon::Import),
        Act::GithubLink => ("From a GitHub link", "For a port that is not on the lists", Icon::Link),
        Act::Back => ("Back", "", Icon::None),
    }
}

/// The page. `back_word` is what B does for the hint bar: "Home" from the Home screen, "Back" from Settings.
pub fn page(big_mode: bool, back_word: &str) -> Page<Act> {
    let mut p = Page::<Act> {
        window_title: TITLE.to_string(),
        large: true,
        header: TITLE.to_string(),
        title: "How do you want to add a port?".to_string(),
        ..Page::default()
    };
    for (i, act) in acts(big_mode).into_iter().enumerate() {
        let (title, detail, icon) = card(act);
        p.options.push(PageOption { title: title.to_string(), detail: detail.to_string(), choice: None, enabled: true, recommended: act == Act::DeckReady, action: act });
        p = p.icon(i, icon);
    }
    p.buttons[LEFT] = Some(PageButton { label: "Back".to_string(), action: Act::Back });
    p.back_hint = back_word.to_string();
    p.initial = Focus::Option(0);
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::page::{action_at, initial_focus};

    #[test]
    fn windowed_offers_all_four_ways_and_big_mode_leaves_out_the_duplicate_list() {
        assert_eq!(acts(false), [Act::DeckReady, Act::Browse, Act::Import, Act::GithubLink]);
        assert_eq!(acts(true), [Act::DeckReady, Act::Import, Act::GithubLink], "big mode has no catalog list: Deck-ready is the one to browse");
    }

    #[test]
    fn the_page_asks_one_question_with_one_recommended_card_that_has_the_cursor() {
        for big in [false, true] {
            let p = page(big, "Home");
            assert_eq!(p.title, "How do you want to add a port?");
            assert_eq!(p.options.iter().filter(|o| o.recommended).count(), 1);
            assert_eq!(action_at(&p, initial_focus(&p)), Some(Act::DeckReady));
            assert!(p.options.iter().all(|o| !o.detail.is_empty() && o.detail.len() < 60), "one short plain line each");
            assert!((0..p.options.len()).all(|i| p.icon_of(i) != Icon::None), "every card has a picture");
        }
    }

    #[test]
    fn b_says_where_it_goes_and_a_back_button_exists_for_the_pointer() {
        let p = page(false, "Home");
        assert!(crate::fork::page::hints(&p).contains("B Home"));
        assert_eq!(p.buttons[LEFT].as_ref().map(|b| b.action), Some(Act::Back));
        assert!(crate::fork::page::hints(&page(false, "Back")).contains("B Back"));
    }
}
