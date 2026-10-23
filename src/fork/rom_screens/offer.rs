//! When the app looks for a port's game files on its own: once per port, right after the port is
//! installed or adopted, and only when it names files it cannot run without.
//!
//! The search itself is `find::find_game_files_for_port`; this file only decides whether to start it,
//! so the old "connect now?" offer and the automatic search can never both appear. The memory of which
//! ports were already handled is `rom_prompted_ports` in `fork-settings.json` (the same key the earlier
//! offer used, so a port offered before is not asked again). "Later" does not forget a port: the port's
//! Info dialog (Game files...) and Settings > Game files > Find game files for a port always reach it.

use super::find::FindRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoSearch {
    Search,
    /// Why not, for the log and the tests.
    Skip(&'static str),
}

/// Decide whether to look for a port's game files right after it was installed or adopted.
/// `already_handled`: the search (or the earlier offer) was already made for this port.
pub fn after_install(req: &FindRequest, android_only: bool, already_handled: bool) -> AutoSearch {
    if android_only {
        AutoSearch::Skip("Android-only port")
    } else if !req.needs_files() {
        AutoSearch::Skip("needs no game files")
    } else if already_handled {
        AutoSearch::Skip("already looked for once")
    } else if req.already_connected() {
        AutoSearch::Skip("already connected")
    } else {
        AutoSearch::Search
    }
}

#[cfg(test)]
mod tests {
    use super::super::report::tests_support::port;
    use super::*;

    fn req(text: &str) -> FindRequest {
        FindRequest::new(port("Zelda Port", "ZeldaPort", &["n64"], text), true, None, vec![])
    }

    #[test]
    fn only_ports_that_name_required_files_are_searched_and_only_once() {
        let needs = req("Required files: Zelda (USA).z64 (CRC32: AABBCCDD)");
        assert_eq!(after_install(&needs, false, false), AutoSearch::Search);
        assert_eq!(after_install(&needs, false, true), AutoSearch::Skip("already looked for once"));
        assert_eq!(after_install(&needs, true, false), AutoSearch::Skip("Android-only port"));
        assert_eq!(after_install(&req("A fan made game. No files needed."), false, false), AutoSearch::Skip("needs no game files"));
        assert_eq!(after_install(&req(""), false, false), AutoSearch::Skip("needs no game files"));
        assert!(super::super::find::needs_game_files(&needs.spec));
    }
}
