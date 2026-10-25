//! Matching a game title (what a catalog entry says when it names no file) against file names.
//!
//! One rule for every screen that asks "is this game here?": Home and the wizard (`setup_wizard::files_state`) and
//! Settings > Game files (`matching::name_basis`). A file carries a title when its name has every significant word of
//! the title; only disc and ROM images count, never archives or data files.

use super::system::is_rom_extension;

/// Words in a title that say nothing about the game: regions, formats, filler.
const TITLE_FILLER: [&str; 24] = [
    "pal", "ntsc", "ntscu", "ntscj", "usa", "us", "europe", "eur", "japan", "jpn", "world", "australia", "korea", "or", "and", "the", "of", "iso", "disc",
    "dump", "image", "version", "region", "an",
];

/// The words of a game title worth looking for in a file name: lowercase, no parenthesised tags, no region or filler
/// words, no one-letter words ("Majora's" is `majoras`).
pub fn significant_words(title: &str) -> Vec<String> {
    let mut plain = String::new();
    let mut depth = 0usize;
    for c in title.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            '\'' | '\u{2019}' => {}
            c if depth == 0 => plain.push(c),
            _ => {}
        }
    }
    plain
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 2 && !TITLE_FILLER.contains(&w.as_str()))
        .collect()
}

fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Whether a file name (without its extension) has every one of `words`. No words: never a match.
pub fn stem_has_words(stem: &str, words: &[String]) -> bool {
    if words.is_empty() {
        return false;
    }
    let key = squash(stem);
    words.iter().all(|w| key.contains(&squash(w)))
}

/// A disc image: one file of a disc (or the cue sheet of a set).
pub fn is_disc_image_extension(ext: &str) -> bool {
    matches!(ext.to_ascii_lowercase().as_str(), "iso" | "bin" | "cue" | "chd" | "img" | "rvz" | "ciso" | "gcm" | "wbfs" | "mdf" | "xiso" | "gdi" | "cdi" | "pbp" | "cso")
}

/// A game image a title can be found in: a disc or ROM image, not an archive or a data file. `img` is a disc only next
/// to its cue sheet, which the caller checks (`needs_cue`).
pub fn is_game_image_extension(ext: &str) -> bool {
    let e = ext.to_ascii_lowercase();
    is_rom_extension(&e) && !matches!(e.as_str(), "zip" | "7z" | "dat" | "swf" | "cdt" | "tap")
}

/// `img` alone is too common a name for data; it counts only with a cue sheet beside it.
pub fn needs_cue(ext: &str) -> bool {
    ext.eq_ignore_ascii_case("img")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn significant_words_drop_regions_filler_tags_and_apostrophes() {
        assert_eq!(significant_words("Legend of Zelda, The - Twilight Princess PAL or NTSC"), ["legend", "zelda", "twilight", "princess"]);
        assert_eq!(significant_words("The Legend of Zelda: Majora's Mask (USA) [!]"), ["legend", "zelda", "majoras", "mask"]);
        assert_eq!(significant_words("Game X (Europe) (En,Fr)"), ["game"]);
        assert!(significant_words("USA or Europe").is_empty());
    }

    #[test]
    fn a_stem_needs_every_word_and_an_empty_list_matches_nothing() {
        let words = significant_words("The Legend of Dragoon");
        assert!(stem_has_words("Legend of Dragoon (USA) (Disc 1)", &words));
        assert!(!stem_has_words("Legend of Zelda", &words));
        assert!(!stem_has_words("anything", &[]));
    }

    #[test]
    fn only_images_count() {
        for ok in ["iso", "bin", "cue", "chd", "rvz", "ISO", "z64", "gba"] {
            assert!(is_game_image_extension(ok), "{ok}");
        }
        for bad in ["zip", "7z", "dat", "txt", "png", "jar", "json", "swf"] {
            assert!(!is_game_image_extension(bad), "{bad}");
        }
        assert!(is_disc_image_extension("cue") && is_disc_image_extension("rvz") && !is_disc_image_extension("z64"));
        assert!(needs_cue("IMG") && !needs_cue("iso"));
    }
}
