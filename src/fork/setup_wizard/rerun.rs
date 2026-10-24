//! A rerun of the setup wizard remembers what is done. Each step is Done, Pending (something is left to do) or
//! Waiting (nothing to do now; the old copies wait for their ports to be checked), worked out from the app as it
//! is now plus `setup_progress` in `fork-settings.json`, which records what the app cannot see later (that the
//! ports were looked for, how many moved in, what the last game files scan found, steps skipped).
//!
//! The first page of a rerun, "Your setup", shows one line per step; "Finish what's left" visits only the
//! Pending ones, in order, so nothing done is redone. No UI here.

use super::flow::Step;
use super::places::Kind;
use crate::fork::settings::ForkSettings;
use serde_json::{json, Map, Value};
use std::io;

pub const KEY: &str = "setup_progress";

/// What the wizard recorded in earlier runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    /// The ports folder question was answered (a place chosen or kept).
    pub ports_folder: bool,
    /// Your ports: `Some(true)` looked for, `Some(false)` skipped, `None` never reached.
    pub your_ports: Option<bool>,
    /// How many ports the wizard moved in, over every run.
    pub imported: usize,
    /// Game files: skipped.
    pub game_files_skipped: bool,
    /// What the last game files scan found: (have, total).
    pub game_files_found: Option<(usize, usize)>,
    /// Steam: skipped.
    pub steam_skipped: bool,
}

impl Progress {
    pub fn load(settings: &ForkSettings) -> Progress {
        let Some(v) = settings.get(KEY).and_then(Value::as_object) else { return Progress::default() };
        let flag = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
        let count = |k: &str| v.get(k).and_then(Value::as_u64).map_or(0, |n| n as usize);
        Progress {
            ports_folder: flag("ports_folder"),
            your_ports: match v.get("your_ports").and_then(Value::as_str) {
                Some("looked") => Some(true),
                Some("skipped") => Some(false),
                _ => None,
            },
            imported: count("imported"),
            game_files_skipped: flag("game_files_skipped"),
            game_files_found: match (v.get("game_files_have").and_then(Value::as_u64), v.get("game_files_total").and_then(Value::as_u64)) {
                (Some(have), Some(total)) => Some((have as usize, total as usize)),
                _ => None,
            },
            steam_skipped: flag("steam_skipped"),
        }
    }

    fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("ports_folder".into(), json!(self.ports_folder));
        if let Some(looked) = self.your_ports {
            m.insert("your_ports".into(), json!(if looked { "looked" } else { "skipped" }));
        }
        m.insert("imported".into(), json!(self.imported));
        m.insert("game_files_skipped".into(), json!(self.game_files_skipped));
        if let Some((have, total)) = self.game_files_found {
            m.insert("game_files_have".into(), json!(have));
            m.insert("game_files_total".into(), json!(total));
        }
        m.insert("steam_skipped".into(), json!(self.steam_skipped));
        Value::Object(m)
    }

    pub fn save(&self, settings: &mut ForkSettings) -> io::Result<()> {
        settings.set(KEY, self.to_json())
    }
}

/// The app as it is now, as the "Your setup" page needs it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    /// Where the ports folder is (`None`: a folder the user picked, or the default not chosen yet).
    pub ports_place: Option<Kind>,
    /// The ports folder is the default next to the app.
    pub ports_folder_is_default: bool,
    pub installed: usize,
    /// Old copies recorded after moves (each one a port moved in, even if no run recorded it).
    pub old_kept: usize,
    pub old_ready: usize,
    /// Installed ports that need a game file.
    pub game_files_needed: usize,
    /// The game files folder, as a place ("SD card > Emulation > roms"); `None` when none is set.
    pub rom_folder: Option<String>,
    pub steam_to_add: usize,
    pub steam_waiting: usize,
    pub steam_problem: bool,
    pub game_mode: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Done,
    /// Something is left to do: "Finish what's left" visits it.
    Pending,
    /// Nothing to do now.
    Waiting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepLine {
    pub step: Step,
    pub state: State,
    /// "Ports folder: SD card".
    pub text: String,
}

impl StepLine {
    /// The line with its mark, as the page shows it.
    pub fn marked(&self) -> String {
        let mark = match self.state {
            State::Done => "\u{2713}",
            State::Pending => "\u{2192}",
            State::Waiting => "\u{2026}",
        };
        format!("{mark} {}", self.text)
    }
}

fn place(kind: Option<Kind>) -> &'static str {
    match kind {
        Some(Kind::SdCard) => "SD card",
        Some(Kind::OtherDrive) => "USB drive",
        Some(Kind::Internal) => "internal storage",
        _ => "a folder you picked",
    }
}

fn ports(n: usize) -> String {
    if n == 1 { "1 port".to_string() } else { format!("{n} ports") }
}

/// One line per counted step (the old copies included), in the wizard's order.
pub fn lines(facts: &Facts, progress: &Progress) -> Vec<StepLine> {
    let line = |step, state, text: String| StepLine { step, state, text };
    let mut out = Vec::new();
    // Ports folder: chosen in a run, or anything but the default in use (a choice made somewhere).
    out.push(if progress.ports_folder || !facts.ports_folder_is_default {
        line(Step::PortsFolder, State::Done, format!("Ports folder: {}", place(facts.ports_place)))
    } else {
        line(Step::PortsFolder, State::Pending, "Ports folder: not chosen yet".to_string())
    });
    // Your ports: looked for in a run, or ports already in the ports folder.
    let moved_in = progress.imported.max(facts.old_kept);
    out.push(match progress.your_ports {
        _ if moved_in > 0 => line(Step::YourPorts, State::Done, format!("Your ports: {} moved in", moved_in)),
        _ if facts.installed > 0 => line(Step::YourPorts, State::Done, format!("Your ports: {} in your ports folder", ports(facts.installed))),
        Some(true) => line(Step::YourPorts, State::Done, "Your ports: looked for, none found".to_string()),
        Some(false) => line(Step::YourPorts, State::Pending, "Your ports: skipped".to_string()),
        None => line(Step::YourPorts, State::Pending, "Your ports: not looked for yet".to_string()),
    });
    // Game files.
    out.push(match (&facts.rom_folder, progress.game_files_found) {
        _ if facts.game_files_needed == 0 => line(Step::GameFiles, State::Done, "Game files: none of your ports need any".to_string()),
        (None, _) if progress.game_files_skipped => line(Step::GameFiles, State::Pending, "Game files: skipped, no folder set".to_string()),
        (None, _) => line(Step::GameFiles, State::Pending, "Game files: no folder set yet".to_string()),
        (Some(folder), Some((have, total))) => line(Step::GameFiles, State::Done, format!("Game files: {folder}, found for {have} of {total}")),
        (Some(folder), None) => line(Step::GameFiles, State::Pending, format!("Game files: {folder}, not looked at yet")),
    });
    // Steam.
    out.push(if facts.steam_problem {
        line(Step::Steam, State::Pending, "Steam: not set up (start Steam once and sign in)".to_string())
    } else if facts.steam_waiting > 0 {
        let wait = if facts.game_mode { "waiting for Desktop Mode" } else { "waiting for Steam to close" };
        line(Step::Steam, State::Pending, format!("Steam: {} {wait}", facts.steam_waiting))
    } else if facts.steam_to_add > 0 {
        line(Step::Steam, State::Pending, format!("Steam: {} not in Steam yet", ports(facts.steam_to_add)))
    } else if facts.installed == 0 {
        line(Step::Steam, if progress.steam_skipped { State::Pending } else { State::Done }, "Steam: no ports to add yet".to_string())
    } else {
        line(Step::Steam, State::Done, "Steam: your ports are in Steam".to_string())
    });
    // Old copies.
    out.push(if facts.old_kept == 0 {
        line(Step::OldCopies, State::Done, "Old copies: none kept".to_string())
    } else if facts.old_ready > 0 {
        line(Step::OldCopies, State::Pending, format!("Old copies: {} kept, {} ready to remove", facts.old_kept, facts.old_ready))
    } else {
        line(Step::OldCopies, State::Waiting, format!("Old copies: {} kept, 0 ready", facts.old_kept))
    });
    out
}

/// The steps "Finish what's left" visits, in order.
pub fn pending(lines: &[StepLine]) -> Vec<Step> {
    lines.iter().filter(|l| l.state == State::Pending).map(|l| l.step).collect()
}

/// What a step left behind in this run, for `setup_progress`. `imported_now`: ports moved in by this run so far;
/// `imported_before`: what earlier runs recorded.
pub fn record_step(progress: &mut Progress, step: Step, summary: &super::flow::Summary, imported_before: usize) {
    let skipped = summary.skipped.contains(&step);
    match step {
        Step::PortsFolder => progress.ports_folder = true,
        Step::YourPorts => {
            progress.your_ports = Some(!skipped || summary.imported.is_some());
            progress.imported = imported_before + summary.imported.unwrap_or(0);
        }
        Step::GameFiles => {
            progress.game_files_skipped = skipped;
            if let Some(r) = &summary.game_files {
                progress.game_files_found = Some((r.have(), r.total));
            }
        }
        Step::Steam => progress.steam_skipped = skipped,
        Step::OldCopies | Step::Done => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::setup_wizard::flow::Summary;
    use crate::fork::setup_wizard::game_files::Results;

    /// Tim's Deck after his first wizard run: ports folder on the SD card, his 4 SD-card ports moved in, their 4
    /// Steam repoints queued (Steam was running), the 4 old copies kept, no game files folder set. That run was
    /// on a build without `setup_progress`, so nothing is recorded.
    fn tims_deck() -> Facts {
        Facts {
            ports_place: Some(Kind::SdCard),
            ports_folder_is_default: false,
            installed: 4,
            old_kept: 4,
            old_ready: 0,
            game_files_needed: 4,
            rom_folder: None,
            steam_to_add: 0,
            steam_waiting: 4,
            steam_problem: false,
            game_mode: false,
        }
    }

    fn texts(lines: &[StepLine]) -> Vec<(Step, State, &str)> {
        lines.iter().map(|l| (l.step, l.state, l.text.as_str())).collect()
    }

    #[test]
    fn tims_deck_now_shows_what_is_done_and_finish_whats_left_visits_game_files_then_steam() {
        let l = lines(&tims_deck(), &Progress::default());
        assert_eq!(
            texts(&l),
            [
                (Step::PortsFolder, State::Done, "Ports folder: SD card"),
                (Step::YourPorts, State::Done, "Your ports: 4 moved in"),
                (Step::GameFiles, State::Pending, "Game files: no folder set yet"),
                (Step::Steam, State::Pending, "Steam: 4 waiting for Steam to close"),
                (Step::OldCopies, State::Waiting, "Old copies: 4 kept, 0 ready"),
            ]
        );
        assert_eq!(pending(&l), [Step::GameFiles, Step::Steam], "the ports folder and the import are never redone");
        assert_eq!(l[0].marked(), "\u{2713} Ports folder: SD card");
        assert_eq!(l[2].marked(), "\u{2192} Game files: no folder set yet");
        assert_eq!(l[4].marked(), "\u{2026} Old copies: 4 kept, 0 ready");
    }

    #[test]
    fn the_same_deck_after_steam_was_updated_and_the_game_files_scanned() {
        let f = Facts { steam_waiting: 0, rom_folder: Some("SD card > Emulation > roms".into()), old_ready: 2, ..tims_deck() };
        let p = Progress { game_files_found: Some((2, 4)), ..Progress::default() };
        let l = lines(&f, &p);
        assert_eq!(l[2].text, "Game files: SD card > Emulation > roms, found for 2 of 4");
        assert_eq!((l[2].state, l[3].state), (State::Done, State::Done));
        assert_eq!(l[3].text, "Steam: your ports are in Steam");
        assert_eq!((l[4].state, l[4].text.as_str()), (State::Pending, "Old copies: 4 kept, 2 ready to remove"));
        assert_eq!(pending(&l), [Step::OldCopies]);
    }

    #[test]
    fn each_step_reads_done_or_pending_from_the_app_and_the_record() {
        let fresh = Facts { ports_folder_is_default: true, ports_place: Some(Kind::Internal), game_files_needed: 0, ..Facts::default() };
        let l = lines(&fresh, &Progress::default());
        assert_eq!(texts(&l)[..2], [(Step::PortsFolder, State::Pending, "Ports folder: not chosen yet"), (Step::YourPorts, State::Pending, "Your ports: not looked for yet")]);
        // Keeping the default counts once the question was answered.
        let l = lines(&fresh, &Progress { ports_folder: true, ..Progress::default() });
        assert_eq!((l[0].state, l[0].text.as_str()), (State::Done, "Ports folder: internal storage"));
        // Looked for and nothing found, or skipped.
        assert_eq!(lines(&fresh, &Progress { your_ports: Some(true), ..Progress::default() })[1].state, State::Done);
        assert_eq!(texts(&lines(&fresh, &Progress { your_ports: Some(false), ..Progress::default() }))[1], (Step::YourPorts, State::Pending, "Your ports: skipped"));
        // Game files: none needed; a folder never scanned; skipped.
        assert_eq!((l[2].state, l[2].text.as_str()), (State::Done, "Game files: none of your ports need any"));
        let needs = Facts { game_files_needed: 2, rom_folder: Some("Home > ROMs".into()), ..fresh.clone() };
        assert_eq!(texts(&lines(&needs, &Progress::default()))[2], (Step::GameFiles, State::Pending, "Game files: Home > ROMs, not looked at yet"));
        let skipped = Facts { rom_folder: None, ..needs.clone() };
        assert_eq!(lines(&skipped, &Progress { game_files_skipped: true, ..Progress::default() })[2].text, "Game files: skipped, no folder set");
        // Steam: a problem, ports to add, Game Mode wording, nothing installed.
        let s = Facts { installed: 2, steam_to_add: 2, ..fresh.clone() };
        assert_eq!(texts(&lines(&s, &Progress::default()))[3], (Step::Steam, State::Pending, "Steam: 2 ports not in Steam yet"));
        assert_eq!(lines(&Facts { steam_problem: true, ..s.clone() }, &Progress::default())[3].text, "Steam: not set up (start Steam once and sign in)");
        assert_eq!(lines(&Facts { steam_waiting: 3, game_mode: true, ..s.clone() }, &Progress::default())[3].text, "Steam: 3 waiting for Desktop Mode");
        assert_eq!(texts(&lines(&fresh, &Progress::default()))[3], (Step::Steam, State::Done, "Steam: no ports to add yet"));
        // Old copies: none.
        assert_eq!(texts(&lines(&fresh, &Progress::default()))[4], (Step::OldCopies, State::Done, "Old copies: none kept"));
    }

    #[test]
    fn a_run_records_what_it_did_and_the_record_survives_the_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fork-settings.json");
        let mut settings = ForkSettings::load(&path);
        let mut p = Progress::load(&settings);
        assert_eq!(p, Progress::default());
        let mut s = Summary { imported: Some(3), ..Summary::default() };
        record_step(&mut p, Step::PortsFolder, &s, 1);
        record_step(&mut p, Step::YourPorts, &s, 1);
        s.game_files = Some(Results { total: 4, found: 2, ..Default::default() });
        record_step(&mut p, Step::GameFiles, &s, 1);
        s.skipped.push(Step::Steam);
        record_step(&mut p, Step::Steam, &s, 1);
        assert_eq!(p, Progress { ports_folder: true, your_ports: Some(true), imported: 4, game_files_skipped: false, game_files_found: Some((2, 4)), steam_skipped: true });
        p.save(&mut settings).unwrap();
        let again = Progress::load(&ForkSettings::load(&path));
        assert_eq!(again, p);
        // Other keys are kept.
        settings.set("setup_done", Value::Bool(true)).unwrap();
        assert_eq!(Progress::load(&ForkSettings::load(&path)), p);
        // Skipping Your ports without moving anything is "skipped".
        let mut q = Progress::default();
        record_step(&mut q, Step::YourPorts, &Summary { skipped: vec![Step::YourPorts], ..Summary::default() }, 0);
        assert_eq!((q.your_ports, q.imported), (Some(false), 0));
    }
}
