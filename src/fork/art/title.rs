//! Deciding whether a search result is the game that was asked for.
//!
//! A fuzzy title search on a source is only trusted when the answer is the same title: same words,
//! ignoring case, punctuation, a leading "the" and trailing annotations like "(Unofficial)". Anything
//! looser (one title inside the other, a sequel, a remaster) is [`Confidence::Medium`] or lower and is
//! never used without the user's say-so.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    Low,
    /// One title's words are all in the other: maybe the same game, maybe a sequel.
    Medium,
    /// The same title.
    High,
}

impl Confidence {
    pub fn label(self) -> &'static str {
        match self {
            Confidence::Low => "low",
            Confidence::Medium => "medium",
            Confidence::High => "high",
        }
    }

    pub fn from_label(s: &str) -> Confidence {
        match s {
            "high" => Confidence::High,
            "medium" => Confidence::Medium,
            _ => Confidence::Low,
        }
    }

    /// Only this is used without asking.
    pub fn is_silent(self) -> bool {
        self == Confidence::High
    }
}

/// Words of a title: lowercase letters and digits, "&" as "and", brackets and what is inside them
/// dropped, "the" dropped, sorted so word order does not matter.
pub fn tokens(title: &str) -> Vec<String> {
    let mut depth = 0u32;
    let mut cleaned = String::with_capacity(title.len());
    for c in title.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            '&' if depth == 0 => cleaned.push_str(" and "),
            c if depth == 0 => cleaned.push(c),
            _ => {}
        }
    }
    let mut out: Vec<String> = cleaned
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .filter(|w| w != "the")
        .collect();
    out.sort();
    out
}

pub fn confidence(query: &str, candidate: &str) -> Confidence {
    let (q, c) = (tokens(query), tokens(candidate));
    if q.is_empty() || c.is_empty() {
        return Confidence::Low;
    }
    if q == c {
        return Confidence::High;
    }
    let contains_all = |big: &[String], small: &[String]| small.iter().all(|w| big.contains(w));
    if contains_all(&c, &q) || contains_all(&q, &c) {
        return Confidence::Medium;
    }
    Confidence::Low
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_title_is_high_whatever_the_decoration() {
        assert_eq!(confidence("Ship of Harkinian", "Ship of Harkinian"), Confidence::High);
        assert_eq!(confidence("Ship of Harkinian", "ship of harkinian (Unofficial)"), Confidence::High);
        assert_eq!(confidence("The Legend of Zelda: Ocarina of Time", "Legend of Zelda, The - Ocarina of Time"), Confidence::High);
        assert_eq!(confidence("Sonic & Knuckles", "Sonic and Knuckles"), Confidence::High);
        assert_eq!(confidence("Half-Life 2", "Half Life 2"), Confidence::High);
    }

    #[test]
    fn a_title_inside_another_is_only_medium_and_a_different_game_is_low() {
        assert_eq!(confidence("Sonic Unleashed Recompiled", "Sonic Unleashed"), Confidence::Medium);
        assert_eq!(confidence("Zelda", "The Legend of Zelda: Majora's Mask"), Confidence::Medium);
        assert_eq!(confidence("Ship of Harkinian", "Doom"), Confidence::Low);
        assert_eq!(confidence("", "Doom"), Confidence::Low);
        assert_eq!(confidence("Doom", "Doom II"), Confidence::Medium);
        assert_eq!(confidence("Doom 2", "Doom 3"), Confidence::Low);
    }

    #[test]
    fn only_high_is_used_silently() {
        assert!(Confidence::High.is_silent());
        assert!(!Confidence::Medium.is_silent());
        assert!(!Confidence::Low.is_silent());
        for c in [Confidence::Low, Confidence::Medium, Confidence::High] {
            assert_eq!(Confidence::from_label(c.label()), c);
        }
        assert!(Confidence::High > Confidence::Medium);
    }
}
