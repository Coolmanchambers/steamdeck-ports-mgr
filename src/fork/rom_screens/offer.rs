//! The offer made once after a port installs: "this port needs game files, connect them now?"

use super::util::ellipsize;
use crate::fork::rom_library::{RomSpec, Slot};

/// Does the port name any file it cannot run without.
pub fn needs_game_files(spec: &RomSpec) -> bool {
    spec.requirement.slots.iter().any(|s| !s.optional)
}

pub fn offer_text(port_name: &str, spec: &RomSpec) -> String {
    let mut text = format!("{port_name} needs game files from your own copy of the game:\n");
    let wanted: Vec<&Slot> = spec.requirement.slots.iter().filter(|s| !s.optional).collect();
    for slot in wanted.iter().take(4) {
        let what = slot.alternatives.first().map(|a| a.display.as_str()).filter(|d| !d.is_empty()).unwrap_or(slot.label.as_str());
        text.push_str(&format!("\n  {}", ellipsize(what, 70)));
    }
    if wanted.len() > 4 {
        text.push_str(&format!("\n  and {} more", wanted.len() - 4));
    }
    text.push_str(
        "\n\nLook for them in your game files folder and connect them now? Nothing is moved or deleted, and you can do this later from Settings > Game files. You are only asked once per port.",
    );
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::rom_library::{parse_instructions, Access};

    fn spec(text: &str) -> RomSpec {
        RomSpec { requirement: parse_instructions(text), access: Access::Link, dest_subdir: String::new() }
    }

    #[test]
    fn only_ports_that_name_required_files_get_an_offer() {
        assert!(needs_game_files(&spec("Required files: Zelda (USA).z64 (CRC32: AABBCCDD)")));
        assert!(!needs_game_files(&spec("A fan made game. No files needed.")));
        assert!(!needs_game_files(&spec("")));
    }

    #[test]
    fn the_offer_names_the_file_and_says_nothing_is_moved() {
        let text = offer_text("Zelda Port", &spec("Required files: Zelda (USA).z64 (CRC32: AABBCCDD)"));
        assert!(text.contains("Zelda (USA).z64") && text.contains("Nothing is moved or deleted") && text.contains("once per port"), "{text}");
    }
}
