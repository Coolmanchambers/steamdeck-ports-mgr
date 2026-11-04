use serde_json::Value;
use slint::Color;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb_u8(r, g, b)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeColors {
    pub search_background: Color,
    pub search_text: Color,
    pub list_background: Color,
    pub list_text: Color,
    pub selected_background: Color,
    pub selected_text: Color,
    pub border: Color,
}

impl ThemeColors {
    // Fork: the fork's "arc-dark": darker neutral surfaces and brighter secondary text (Codex review A1, A2).
    // The app refreshes themes.json from upstream, so this palette is kept in code and laid over the file's
    // "arc-dark" entry at load (`load`). ui/shared.slint carries the same values; a test keeps them in step.
    pub const FALLBACK: ThemeColors = ThemeColors {
        search_background: rgb(0x27, 0x2e, 0x38),
        search_text: rgb(0xb8, 0xc3, 0xd1),
        list_background: rgb(0x19, 0x1f, 0x27),
        list_text: rgb(0xed, 0xf2, 0xf7),
        selected_background: rgb(0x52, 0x94, 0xe2),
        selected_text: rgb(0xff, 0xff, 0xff),
        border: rgb(0x56, 0x62, 0x73),
    };
}

// Fork: the theme the fork's palette replaces, whatever themes.json says about it.
pub const FORK_THEME: &str = "arc-dark";

#[derive(Clone, Copy, Debug)]
pub struct SemanticPalette {
    pub brand_github: Color,
    pub brand_github_hover: Color,
    pub brand_discord: Color,
    pub brand_discord_hover: Color,
    pub success: Color,
    pub success_hover: Color,
    pub warning: Color,
    pub warning_hover: Color,
    pub danger: Color,
    pub danger_hover: Color,
    pub info: Color,
    pub info_hover: Color,
    pub border_strong: Color,
    pub text_on_accent: Color,
}

impl Default for SemanticPalette {
    fn default() -> Self {
        SemanticPalette {
            brand_github: rgb(0x24, 0x29, 0x2f),
            brand_github_hover: rgb(0x32, 0x38, 0x3f),
            brand_discord: rgb(0x58, 0x65, 0xf2),
            brand_discord_hover: rgb(0x6b, 0x76, 0xf5),
            // Fork: darker fills so white labels read at 4.5:1 or better (Codex review A4); `contrast` tests below.
            success: rgb(0x3a, 0x7a, 0x47),
            success_hover: rgb(0x40, 0x83, 0x4c),
            warning: rgb(0xa0, 0x58, 0x00),
            warning_hover: rgb(0xab, 0x60, 0x00),
            danger: rgb(0xdd, 0x2e, 0x44),
            danger_hover: rgb(0xe4, 0x58, 0x69),
            info: rgb(0x24, 0x70, 0xab),
            info_hover: rgb(0x2b, 0x7b, 0xb6),
            border_strong: rgb(0xff, 0xff, 0xff),
            text_on_accent: rgb(0xff, 0xff, 0xff),
        }
    }
}

pub struct ThemeConfig {
    pub themes: HashMap<String, ThemeColors>,
    pub current: ThemeColors,
    pub semantic: SemanticPalette,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        ThemeConfig {
            themes: HashMap::from([("arc-dark".to_string(), ThemeColors::FALLBACK)]),
            current: ThemeColors::FALLBACK,
            semantic: SemanticPalette::default(),
        }
    }
}

pub fn parse_hex_color(s: &str) -> Option<Color> {
    let s = s.strip_prefix('#')?;
    let digits: Vec<u8> = s.chars().map(|c| c.to_digit(16).map(|d| d as u8)).collect::<Option<_>>()?;
    match digits.as_slice() {
        [r1, r2, g1, g2, b1, b2] => Some(rgb(r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2)),
        [r, g, b] => Some(rgb(r * 17, g * 17, b * 17)),
        _ => None,
    }
}

fn parse_theme_entry(v: &Value) -> Option<ThemeColors> {
    let color = |key: &str| v.get(key).and_then(Value::as_str).and_then(parse_hex_color);
    Some(ThemeColors {
        search_background: color("search_background")?,
        search_text: color("search_text")?,
        list_background: color("list_background")?,
        list_text: color("list_text")?,
        selected_background: color("selected_background")?,
        selected_text: color("selected_text")?,
        border: color("border")?,
    })
}

pub fn load(path: &Path, cfg: &mut ThemeConfig, active_theme: &str) {
    let Ok(text) = fs::read_to_string(path) else { return };
    let Ok(data) = serde_json::from_str::<Value>(crate::core::config::strip_bom(&text)) else { return };
    let mut themes: HashMap<String, ThemeColors> = data
        .get("themes")
        .and_then(Value::as_object)
        .map(|entries| entries.iter().filter_map(|(name, v)| Some((name.clone(), parse_theme_entry(v)?))).collect())
        .unwrap_or_default();
    if themes.is_empty() {
        return;
    }
    // Fork: the file's "arc-dark" is upstream's lighter palette (and the app refreshes the file from upstream);
    // the fork's palette wins for that one name, and the default theme exists even if the file dropped it.
    themes.insert(FORK_THEME.to_string(), ThemeColors::FALLBACK);
    let first = themes.get(FORK_THEME).copied().unwrap_or(ThemeColors::FALLBACK);
    cfg.current = themes.get(active_theme).copied().unwrap_or(first);
    cfg.themes = themes;
}

pub fn preview_theme(cfg: &mut ThemeConfig, name: &str) {
    if let Some(t) = cfg.themes.get(name) {
        cfg.current = *t;
    }
}

pub fn list_theme_names(cfg: &ThemeConfig) -> Vec<String> {
    let mut names: Vec<String> = cfg.themes.keys().cloned().collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("ports_launcher_theme_test_{}_{}.json", std::process::id(), name));
        p
    }

    const SAMPLE: &str = r##"{
  "themes": {
    "night": {
      "search_background": "#404552",
      "search_text": "#7c818c",
      "list_background": "#383c4a",
      "list_text": "#d3dae3",
      "selected_background": "#5294e2",
      "selected_text": "#ffffff",
      "border": "#4b5162"
    },
    "day": {
      "search_background": "#ffffff",
      "search_text": "#000000",
      "list_background": "#eeeeee",
      "list_text": "#111111",
      "selected_background": "#3366cc",
      "selected_text": "#ffffff",
      "border": "#cccccc"
    }
  }
}"##;

    #[test]
    fn parse_hex_color_couvre_3_et_6_chiffres() {
        assert_eq!(parse_hex_color("#fff"), Some(rgb(255, 255, 255)));
        assert_eq!(parse_hex_color("#3a8ea0"), Some(rgb(0x3a, 0x8e, 0xa0)));
        assert_eq!(parse_hex_color("bogus"), None);
        assert_eq!(parse_hex_color("#12"), None);
        assert_eq!(parse_hex_color("#12345g"), None);
    }

    #[test]
    fn charge_le_theme_actif_demande_et_le_catalogue() {
        let path = temp_path("load_ok");
        fs::write(&path, SAMPLE).unwrap();
        let mut cfg = ThemeConfig::default();
        load(&path, &mut cfg, "night");
        assert_eq!(cfg.current.search_background, rgb(0x40, 0x45, 0x52));
        assert_eq!(cfg.themes.len(), 3, "the file's two plus the fork's arc-dark");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn theme_actif_inconnu_replie_sur_un_theme_du_catalogue() {
        let path = temp_path("load_unknown_active");
        fs::write(&path, SAMPLE).unwrap();
        let mut cfg = ThemeConfig::default();
        load(&path, &mut cfg, "does-not-exist");
        // Fork: an unknown theme falls back to the fork's default, not to whichever entry the map yields first.
        assert_eq!(cfg.current, ThemeColors::FALLBACK);
        let _ = fs::remove_file(&path);
    }

    // Fork: a file whose "themes" has no usable entry leaves the loaded set alone (as a missing file does).
    #[test]
    fn un_catalogue_sans_theme_utilisable_garde_l_etat_precedent() {
        let path = temp_path("load_unusable");
        fs::write(&path, SAMPLE).unwrap();
        let mut cfg = ThemeConfig::default();
        load(&path, &mut cfg, "day");
        fs::write(&path, r##"{"themes": {"broken": {"search_background": "nope"}}}"##).unwrap();
        load(&path, &mut cfg, "day");
        assert_eq!(cfg.themes.len(), 3);
        assert_eq!(cfg.current.search_background, rgb(0xff, 0xff, 0xff));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn fichier_absent_garde_l_etat_precedent() {
        let path = temp_path("missing");
        let _ = fs::remove_file(&path);
        let mut cfg = ThemeConfig::default();
        let before = cfg.current;
        load(&path, &mut cfg, "arc-dark");
        assert_eq!(cfg.current, before);
    }

    #[test]
    fn preview_change_current_sans_toucher_le_catalogue() {
        let path = temp_path("preview");
        fs::write(&path, SAMPLE).unwrap();
        let mut cfg = ThemeConfig::default();
        load(&path, &mut cfg, "night");
        preview_theme(&mut cfg, "day");
        assert_eq!(cfg.current.search_background, rgb(0xff, 0xff, 0xff));
        assert_eq!(cfg.themes.len(), 3);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn list_theme_names_est_triee() {
        let path = temp_path("list_names");
        fs::write(&path, SAMPLE).unwrap();
        let mut cfg = ThemeConfig::default();
        load(&path, &mut cfg, "night");
        assert_eq!(list_theme_names(&cfg), vec!["arc-dark".to_string(), "day".to_string(), "night".to_string()]);
        let _ = fs::remove_file(&path);
    }

    // Fork: the theme quick wins (Codex review 2026-11-02, section A).
    mod fork_palette {
        use super::super::*;
        use super::{temp_path, SAMPLE};

        const SHARED_SLINT: &str = include_str!("../../ui/shared.slint");
        const THEMES_JSON: &str = include_str!("../../themes.json");

        /// The `#rrggbb` of a `name: #...;` property line in `shared.slint`, the first one of that name.
        fn slint_color(name: &str) -> Color {
            let line = SHARED_SLINT
                .lines()
                .find(|l| l.trim_start().starts_with(&format!("in-out property <color> {}:", name)))
                .unwrap_or_else(|| panic!("shared.slint has no colour property named {name}"));
            let hex = line.split(':').nth(1).unwrap().trim().trim_end_matches(';');
            parse_hex_color(hex).unwrap_or_else(|| panic!("{name} in shared.slint is not a hex colour: {hex}"))
        }

        fn same(what: &str, rust: Color, slint: Color) {
            assert_eq!(
                (rust.red(), rust.green(), rust.blue()),
                (slint.red(), slint.green(), slint.blue()),
                "{what}: src/ui/theme.rs and ui/shared.slint drifted apart"
            );
        }

        fn luminance(c: Color) -> f64 {
            let lin = |v: u8| {
                let v = v as f64 / 255.0;
                if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * lin(c.red()) + 0.7152 * lin(c.green()) + 0.0722 * lin(c.blue())
        }

        /// WCAG 2 contrast ratio, 1.0 to 21.0.
        fn contrast(a: Color, b: Color) -> f64 {
            let (la, lb) = (luminance(a), luminance(b));
            (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
        }

        #[test]
        fn the_fork_palette_in_rust_and_slint_is_the_same() {
            let t = ThemeColors::FALLBACK;
            same("search-background", t.search_background, slint_color("search-background"));
            same("search-text", t.search_text, slint_color("search-text"));
            same("list-background", t.list_background, slint_color("list-background"));
            same("list-text", t.list_text, slint_color("list-text"));
            same("selected-background", t.selected_background, slint_color("selected-background"));
            same("selected-text", t.selected_text, slint_color("selected-text"));
            same("border-color", t.border, slint_color("border-color"));
            same("card-background", t.list_background, slint_color("card-background"));
            same("fallback-text", t.list_text, slint_color("fallback-text"));

            let s = SemanticPalette::default();
            same("success", s.success, slint_color("success"));
            same("success-hover", s.success_hover, slint_color("success-hover"));
            same("warning", s.warning, slint_color("warning"));
            same("warning-hover", s.warning_hover, slint_color("warning-hover"));
            same("danger", s.danger, slint_color("danger"));
            same("danger-hover", s.danger_hover, slint_color("danger-hover"));
            same("info", s.info, slint_color("info"));
            same("info-hover", s.info_hover, slint_color("info-hover"));
            same("text-on-accent", s.text_on_accent, slint_color("text-on-accent"));
            same("selection-border", s.border_strong, slint_color("selection-border"));
        }

        // Not covered: `danger_hover`. The spec left danger as it was (4.6:1), and a hover that is brighter than
        // its fill cannot also beat it; the hover is a transient state under the pointer.
        #[test]
        fn white_labels_read_on_the_filled_action_colours_and_their_hovers() {
            let s = SemanticPalette::default();
            for (name, fill) in [
                ("success", s.success),
                ("success-hover", s.success_hover),
                ("warning", s.warning),
                ("warning-hover", s.warning_hover),
                ("info", s.info),
                ("info-hover", s.info_hover),
                ("danger", s.danger),
            ] {
                let ratio = contrast(s.text_on_accent, fill);
                assert!(ratio >= 4.5, "{name}: white on the fill is {ratio:.2}:1, below WCAG AA 4.5:1");
            }
        }

        #[test]
        fn the_filled_colours_stand_out_from_the_dark_surface() {
            let (t, s) = (ThemeColors::FALLBACK, SemanticPalette::default());
            for (name, fill) in [("success", s.success), ("warning", s.warning), ("info", s.info), ("danger", s.danger)] {
                let ratio = contrast(fill, t.list_background);
                assert!(ratio >= 3.0, "{name}: the fill against the list background is {ratio:.2}:1, under 3:1 (icons and chips)");
            }
        }

        #[test]
        fn secondary_and_body_text_read_on_both_surfaces() {
            let t = ThemeColors::FALLBACK;
            for (name, bg) in [("list-background", t.list_background), ("search-background", t.search_background)] {
                let secondary = contrast(t.search_text, bg);
                assert!(secondary >= 4.5, "search-text on {name} is {secondary:.2}:1, below 4.5:1");
                let body = contrast(t.list_text, bg);
                assert!(body >= 7.0, "list-text on {name} is {body:.2}:1, below 7:1");
            }
        }

        #[test]
        fn the_fork_arc_dark_wins_over_the_file_and_the_other_themes_stay() {
            let path = temp_path("fork_arc_dark");
            fs::write(&path, THEMES_JSON).unwrap();
            let mut cfg = ThemeConfig::default();
            load(&path, &mut cfg, FORK_THEME);
            assert_eq!(cfg.current, ThemeColors::FALLBACK, "the active arc-dark is the fork's palette");
            assert_eq!(cfg.themes[FORK_THEME], ThemeColors::FALLBACK);
            assert!(cfg.themes.len() > 50, "the file's own themes are still offered ({})", cfg.themes.len());
            assert_ne!(cfg.themes["dracula"], ThemeColors::FALLBACK, "other themes come from the file as before");
            let _ = fs::remove_file(&path);
        }

        #[test]
        fn a_file_without_arc_dark_still_offers_it() {
            let path = temp_path("no_arc_dark");
            fs::write(&path, SAMPLE).unwrap();
            let mut cfg = ThemeConfig::default();
            load(&path, &mut cfg, FORK_THEME);
            assert_eq!(cfg.current, ThemeColors::FALLBACK);
            assert_eq!(cfg.themes.len(), 3);
            let _ = fs::remove_file(&path);
        }
    }
}
