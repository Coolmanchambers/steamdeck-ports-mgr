//! The "Connect game files" step for one port: which found file feeds which slot, and how.
//!
//! Decisions and text only. The screen starts from the matcher's result, offers use-in-place, link or
//! copy as far as the port's access level allows, shows the extra space a copy costs, and builds the
//! requests the connect planner takes. Nothing here writes anything; `make_plan` only reads, and
//! `apply_plans` goes through the store, which refuses a copy without approval.

use super::report::PortRow;
use super::util::{ellipsize, file_name, free_bytes, human_bytes};
use crate::fork::rom_library::{
    check_mapping, plan_connect_set, Approval, ConnectPlan, ConnectRequest, FileMatch, Health, Mapping, Method, PlanAction, RomLibraryStore, RomSpec,
    SlotResult, SlotStatus,
};
use std::path::{Path, PathBuf};

/// A copy is refused when it would leave less than this free on the drive.
pub const FREE_SPACE_MARGIN: u64 = 1 << 30;

#[derive(Debug, Clone)]
struct SlotChoice {
    result: SlotResult,
    candidates: Vec<FileMatch>,
    chosen: Option<usize>,
    method: Method,
    /// Recorded earlier and still true on disk.
    connected: Option<Method>,
    /// Recorded earlier but no longer true on disk.
    broken: Option<Health>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    Intro,
    Slot(usize),
    Space,
    Connect,
    Back,
}

/// One entry in the "change this slot" list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    File(usize),
    Method(Method),
}

#[derive(Debug, Clone)]
pub struct ConnectScreen {
    pub port_name: String,
    port_key: String,
    port_dir: PathBuf,
    spec: RomSpec,
    slots: Vec<SlotChoice>,
}

fn health_text(h: Health) -> &'static str {
    match h {
        Health::Ok => "fine",
        Health::SourceMissing => "the game file is gone (drive unplugged?)",
        Health::SourceChanged => "the game file changed",
        Health::DestMissing => "the link or copy is gone",
        Health::DestChanged => "the link or copy changed",
    }
}

impl ConnectScreen {
    /// `port_dir` is where the port is installed. `mappings` are the recorded connections.
    pub fn new(row: &PortRow, port_dir: PathBuf, mappings: &[Mapping]) -> ConnectScreen {
        let slots = row
            .matched
            .slots
            .iter()
            .map(|result| {
                let (candidates, chosen) = match &result.status {
                    SlotStatus::Found(f) => (vec![f.clone()], Some(0)),
                    SlotStatus::Ambiguous(list) => (list.clone(), None),
                    SlotStatus::WrongVersion(_) | SlotStatus::Missing => (Vec::new(), None),
                };
                let ours: Vec<&Mapping> = mappings.iter().filter(|m| m.port_key == row.matched.port_key && m.slot == result.slot_id).collect();
                let worst = ours.iter().map(|m| check_mapping(m)).find(|h| *h != Health::Ok);
                let connected = (!ours.is_empty() && worst.is_none()).then(|| ours[0].method);
                SlotChoice { result: result.clone(), candidates, chosen, method: row.spec.access.recommended(), connected, broken: worst }
            })
            .collect();
        ConnectScreen { port_name: row.port.display_name().to_string(), port_key: row.matched.port_key.clone(), port_dir, spec: row.spec.clone(), slots }
    }

    fn connectable(&self, i: usize) -> bool {
        !self.slots[i].candidates.is_empty() && self.slots[i].connected.is_none()
    }

    pub fn lines(&self) -> Vec<Line> {
        let mut lines = vec![Line::Intro];
        lines.extend((0..self.slots.len()).map(Line::Slot));
        lines.extend([Line::Space, Line::Connect, Line::Back]);
        lines
    }

    fn methods(&self) -> Vec<Method> {
        [Method::UseInPlace, Method::Symlink, Method::Copy].into_iter().filter(|m| self.spec.access.allows(*m)).collect()
    }

    /// The size a copy of this slot's chosen file(s) would take.
    fn slot_bytes(&self, i: usize) -> u64 {
        let slot = &self.slots[i];
        let Some(f) = slot.chosen.and_then(|c| slot.candidates.get(c)) else { return 0 };
        f.all_paths().iter().map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(f.size)).sum()
    }

    pub fn copy_bytes(&self) -> u64 {
        (0..self.slots.len()).filter(|i| self.connectable(*i) && self.slots[*i].method == Method::Copy).map(|i| self.slot_bytes(i)).sum()
    }

    fn pending(&self) -> usize {
        (0..self.slots.len()).filter(|i| self.connectable(*i) && self.slots[*i].chosen.is_some()).count()
    }

    pub fn labels(&self) -> Vec<String> {
        self.lines()
            .into_iter()
            .map(|line| match line {
                Line::Intro => format!("Pick how each file reaches {}. Nothing is written yet.", self.port_name),
                Line::Slot(i) => self.slot_label(i),
                Line::Space => match self.copy_bytes() {
                    0 => "Extra disk space: none".to_string(),
                    n => format!("Extra disk space: {} for copies", human_bytes(n)),
                },
                Line::Connect if self.pending() == 0 => "Connect now (nothing to connect yet)".to_string(),
                Line::Connect => "Connect now".to_string(),
                Line::Back => "Back".to_string(),
            })
            .collect()
    }

    fn slot_label(&self, i: usize) -> String {
        let slot = &self.slots[i];
        let name = ellipsize(&slot.result.label, 40);
        let optional = if slot.result.optional { " (optional)" } else { "" };
        if let Some(method) = slot.connected {
            let file = slot.candidates.first().map(|f| ellipsize(&file_name(&f.path), 40)).unwrap_or_default();
            return format!("{name}{optional}: {file} - already connected ({})", method.label());
        }
        if slot.candidates.is_empty() {
            let why = match slot.result.status {
                SlotStatus::WrongVersion(_) => "only the wrong version was found",
                _ => "not found in your game folder",
            };
            return format!("{name}{optional}: {why}");
        }
        let Some(c) = slot.chosen else {
            return format!("{name}{optional}: {} files match, choose one", slot.candidates.len());
        };
        let f = &slot.candidates[c];
        let mut text = format!("{name}{optional}: {} - {}", ellipsize(&file_name(&f.path), 40), slot.method.label());
        if slot.method == Method::Copy {
            text.push_str(&format!(" (+{})", human_bytes(self.slot_bytes(i))));
        }
        if let Some(h) = slot.broken {
            text.push_str(&format!(" [earlier connection: {}]", health_text(h)));
        }
        if f.is_low_confidence() {
            text.push_str(" [matched by name, check this]");
        }
        if f.needs_byte_order_conversion() {
            text.push_str(" [N64 byte order differs]");
        }
        text
    }

    /// What the "change this slot" list offers: the candidate files (when there are several) and the
    /// methods this port allows.
    pub fn picks(&self, i: usize) -> Vec<Pick> {
        let mut picks = Vec::new();
        if self.slots[i].candidates.len() > 1 {
            picks.extend((0..self.slots[i].candidates.len()).map(Pick::File));
        }
        picks.extend(self.methods().into_iter().map(Pick::Method));
        picks
    }

    pub fn pick_labels(&self, i: usize) -> Vec<String> {
        let slot = &self.slots[i];
        self.picks(i)
            .into_iter()
            .map(|pick| match pick {
                Pick::File(c) => {
                    let f = &slot.candidates[c];
                    let mark = if slot.chosen == Some(c) { "(chosen) " } else { "" };
                    let dir = f.path.parent().and_then(Path::file_name).map(|n| format!("  in {}", n.to_string_lossy())).unwrap_or_default();
                    format!("File: {mark}{} ({}){dir}", file_name(&f.path), human_bytes(f.size))
                }
                Pick::Method(m) => {
                    let mark = if slot.method == m { "(chosen) " } else { "" };
                    let what = match m {
                        Method::UseInPlace => "the port reads it from your folder, nothing is written",
                        Method::Symlink => "a pointer in the port's folder, no extra space",
                        Method::Copy => "a second copy, uses extra space and asks first",
                    };
                    format!("Method: {mark}{} - {what}", m.label())
                }
            })
            .collect()
    }

    /// Whether choosing a slot's row opens the list (an already connected slot has nothing to change).
    pub fn is_editable(&self, i: usize) -> bool {
        self.connectable(i)
    }

    pub fn choose(&mut self, i: usize, pick: Pick) {
        match pick {
            Pick::File(c) if c < self.slots[i].candidates.len() => self.slots[i].chosen = Some(c),
            Pick::Method(m) if self.spec.access.allows(m) => self.slots[i].method = m,
            _ => {}
        }
    }

    /// The requests for every slot that has a file chosen and is not connected yet.
    pub fn requests(&self) -> Result<Vec<ConnectRequest>, String> {
        let mut out = Vec::new();
        for i in 0..self.slots.len() {
            if !self.connectable(i) {
                continue;
            }
            let slot = &self.slots[i];
            let Some(c) = slot.chosen else {
                return Err(format!("Choose a file for \"{}\" first.", slot.result.label));
            };
            let f = &slot.candidates[c];
            for (n, path) in f.all_paths().into_iter().enumerate() {
                out.push(ConnectRequest {
                    port_key: self.port_key.clone(),
                    slot: slot.result.slot_id.clone(),
                    source: path,
                    port_dir: self.port_dir.clone(),
                    dest_subdir: self.spec.dest_subdir.clone(),
                    dest_name: None,
                    method: slot.method,
                    access: self.spec.access,
                    byte_order: if n == 0 { f.byte_order } else { None },
                    known_crc32: if n == 0 { Some(f.crc32) } else { None },
                });
            }
        }
        if out.is_empty() {
            return Err("There is nothing to connect: every file that was found is already connected, or none was found.".into());
        }
        Ok(out)
    }
}

pub struct PlanOutput {
    pub plans: Vec<ConnectPlan>,
    pub extra_bytes: u64,
    pub free_bytes: Option<u64>,
}

impl PlanOutput {
    pub fn needs_copy_approval(&self) -> bool {
        self.plans.iter().any(|p| p.needs_confirmation)
    }

    /// Words for the confirmation before anything is written.
    pub fn confirm_text(&self, port_name: &str) -> String {
        let copies: Vec<&ConnectPlan> = self.plans.iter().filter(|p| p.needs_confirmation).collect();
        let mut text = format!(
            "{port_name} needs {} copied into its own folder.\n\nThis uses {} of disk space",
            if copies.len() == 1 { "1 file".to_string() } else { format!("{} files", copies.len()) },
            human_bytes(self.extra_bytes)
        );
        match self.free_bytes {
            Some(free) => text.push_str(&format!(" ({} free on that drive).", human_bytes(free))),
            None => text.push('.'),
        }
        text.push_str("\nYour originals are not moved, changed or deleted.\n");
        for p in copies.iter().take(4) {
            text.push_str(&format!("\n  {} ({})", file_name(&p.source), human_bytes(p.size)));
        }
        if copies.len() > 4 {
            text.push_str(&format!("\n  and {} more", copies.len() - 4));
        }
        text.push_str("\n\nA copy can take a while and cannot be cancelled once it starts.");
        for w in self.plans.iter().flat_map(|p| p.warnings.iter()) {
            text.push_str(&format!("\n\nNote: {w}"));
        }
        text
    }

    pub fn warnings(&self) -> Vec<String> {
        self.plans.iter().flat_map(|p| p.warnings.iter().map(move |w| format!("{}: {w}", file_name(&p.source)))).collect()
    }
}

/// Plan the requests (reads the disk, writes nothing) and refuse a copy that will not fit.
pub fn make_plan(requests: Vec<ConnectRequest>) -> Result<PlanOutput, String> {
    let plans = plan_connect_set(&requests).map_err(|e| e.to_string())?;
    let extra_bytes: u64 = plans.iter().map(|p| p.extra_bytes).sum();
    let free = plans.iter().find(|p| p.needs_confirmation).and_then(|p| p.dest.as_deref().and_then(Path::parent)).and_then(free_bytes);
    check_space(extra_bytes, free)?;
    Ok(PlanOutput { plans, extra_bytes, free_bytes: free })
}

/// A copy is refused when the drive is known to be too full. Unknown free space is not a reason to refuse.
pub fn check_space(extra_bytes: u64, free: Option<u64>) -> Result<(), String> {
    match free {
        Some(free) if extra_bytes > 0 && free < extra_bytes.saturating_add(FREE_SPACE_MARGIN) => Err(format!(
            "Not enough free space for the copy: it needs {} and the drive has {} free (the app keeps {} spare). Choose Link or Use original, or free some space.",
            human_bytes(extra_bytes),
            human_bytes(free),
            human_bytes(FREE_SPACE_MARGIN)
        )),
        _ => Ok(()),
    }
}

/// Apply the plans and record the result. Reloads the record from disk so it is never stale.
pub fn apply_plans(store_path: &Path, plans: &[ConnectPlan], approval: Approval) -> Result<Vec<Mapping>, String> {
    let mut store = RomLibraryStore::load(store_path).map_err(|e| e.to_string())?;
    store.apply_set_and_record(plans, approval).map_err(|e| e.to_string())
}

pub fn done_text(port_name: &str, plans: &[ConnectPlan], mappings: &[Mapping]) -> String {
    let mut text = format!("Connected game files for {port_name}.\n");
    for m in mappings {
        let name = file_name(&m.source);
        let was_there = plans.iter().any(|p| p.source == m.source && matches!(p.action, PlanAction::AlreadyConnected(_)));
        let how = match (m.method, was_there) {
            (_, true) => "was already in place".to_string(),
            (Method::UseInPlace, _) => format!("stays where it is: {}", ellipsize(&m.source.to_string_lossy(), 70)),
            (Method::Symlink, _) => "linked into the port's folder".to_string(),
            (Method::Copy, _) => "copied into the port's folder".to_string(),
        };
        text.push_str(&format!("\n  {name}: {how}"));
    }
    if mappings.iter().any(|m| m.method == Method::UseInPlace) {
        text.push_str("\n\nFor \"stays where it is\" files, point the port at that path if it asks for the game.");
    }
    text.push_str("\n\nYour originals were not changed.");
    text
}

#[cfg(test)]
mod tests {
    use super::super::report::{build_report, tests_support::*};
    use super::*;
    use crate::fork::rom_library::testutil::*;
    use std::fs;

    struct Fixture {
        _tmp: tempfile::TempDir,
        roms: PathBuf,
        store_path: PathBuf,
        port_dir: PathBuf,
        zelda: Vec<u8>,
    }

    fn fixture(access: &str) -> (Fixture, PortRow) {
        let tmp = temp_dir();
        let roms = tmp.path().join("roms");
        let zelda = n64_rom(7, 40_000);
        write(&roms.join("n64/Zelda renamed.z64"), &zelda);
        let port_dir = tmp.path().join("lib/ZeldaPort");
        fs::create_dir_all(&port_dir).unwrap();
        let local = tmp.path().join("ports.local.json");
        fs::write(
            &local,
            serde_json::json!({"ports": [{"name": "Zelda Port", "folder": "ZeldaPort", "rom_access": access, "source": "https://example.com/x.zip"}]}).to_string(),
        )
        .unwrap();
        let catalog = vec![port(
            "Zelda Port",
            "ZeldaPort",
            &["n64"],
            &format!("Required files: Zelda (USA).z64 (CRC32: {:08X})", digests_of(&zelda).crc32),
        )];
        let input = super::super::report::ReportInput::new(catalog, ["ZeldaPort".to_string()].into(), &local, vec![]);
        let report = build_report(&input, &scan_dir(&roms));
        let row = report.groups.into_iter().flat_map(|g| g.ports).next().unwrap();
        (Fixture { store_path: tmp.path().join("rom_library.json"), _tmp: tmp, roms, port_dir, zelda }, row)
    }

    #[test]
    fn a_found_file_defaults_to_the_cheapest_method_the_port_allows() {
        for (access, expect) in [("in_place", Method::UseInPlace), ("link", Method::Symlink), ("copy", Method::Copy)] {
            let (f, row) = fixture(access);
            let screen = ConnectScreen::new(&row, f.port_dir.clone(), &[]);
            let reqs = screen.requests().unwrap();
            assert_eq!(reqs.len(), 1);
            assert_eq!(reqs[0].method, expect, "{access}");
            assert!(screen.labels().iter().any(|l| l.contains(expect.label())));
        }
    }

    #[test]
    fn methods_offered_follow_the_access_level() {
        let (f, link_row) = fixture("link");
        let screen = ConnectScreen::new(&link_row, f.port_dir.clone(), &[]);
        assert_eq!(screen.picks(0), [Pick::Method(Method::Symlink), Pick::Method(Method::Copy)]);
        let (f2, copy_row) = fixture("copy");
        let screen = ConnectScreen::new(&copy_row, f2.port_dir.clone(), &[]);
        assert_eq!(screen.picks(0), [Pick::Method(Method::Copy)]);
        let (f3, open_row) = fixture("in_place");
        let screen = ConnectScreen::new(&open_row, f3.port_dir.clone(), &[]);
        assert_eq!(screen.picks(0).len(), 3);
    }

    #[test]
    fn copy_shows_its_cost_and_needs_approval() {
        let (f, row) = fixture("link");
        let mut screen = ConnectScreen::new(&row, f.port_dir.clone(), &[]);
        assert_eq!(screen.copy_bytes(), 0);
        assert!(screen.labels().contains(&"Extra disk space: none".to_string()));
        screen.choose(0, Pick::Method(Method::Copy));
        assert_eq!(screen.copy_bytes(), f.zelda.len() as u64);
        assert!(screen.labels().iter().any(|l| l.starts_with("Extra disk space: 40 KB")), "{:?}", screen.labels());
        let plan = make_plan(screen.requests().unwrap()).unwrap();
        assert!(plan.needs_copy_approval());
        assert!(plan.confirm_text("Zelda Port").contains("uses 40 KB"));
        assert!(matches!(apply_plans(&f.store_path, &plan.plans, Approval::None), Err(e) if e.contains("approval")));
        assert!(fs::read_dir(&f.port_dir).unwrap().next().is_none(), "nothing written without approval");
        assert!(!f.store_path.exists(), "nothing recorded without approval");
    }

    #[cfg(unix)]
    #[test]
    fn link_connects_records_and_survives_a_reload_and_originals_stay_byte_identical() {
        let (f, row) = fixture("link");
        let before = snapshot(&f.roms);
        let screen = ConnectScreen::new(&row, f.port_dir.clone(), &[]);
        let plan = make_plan(screen.requests().unwrap()).unwrap();
        assert!(!plan.needs_copy_approval());
        let mappings = apply_plans(&f.store_path, &plan.plans, Approval::None).unwrap();
        assert_eq!(mappings.len(), 1);
        assert!(fs::symlink_metadata(f.port_dir.join("Zelda renamed.z64")).unwrap().file_type().is_symlink());
        assert_eq!(snapshot(&f.roms), before);
        // After a restart the record is still there and the screen shows the slot as connected.
        let store = RomLibraryStore::load(&f.store_path).unwrap();
        let again = ConnectScreen::new(&row, f.port_dir.clone(), store.mappings());
        assert!(again.labels().iter().any(|l| l.contains("already connected (Link)")), "{:?}", again.labels());
        assert!(again.requests().is_err());
        assert!(!again.is_editable(0));
        assert!(done_text("Zelda Port", &plan.plans, &mappings).contains("linked into the port's folder"));
    }

    #[test]
    fn use_original_writes_nothing_and_says_where_the_file_is() {
        let (f, row) = fixture("in_place");
        let screen = ConnectScreen::new(&row, f.port_dir.clone(), &[]);
        let plan = make_plan(screen.requests().unwrap()).unwrap();
        let mappings = apply_plans(&f.store_path, &plan.plans, Approval::None).unwrap();
        assert!(fs::read_dir(&f.port_dir).unwrap().next().is_none());
        let text = done_text("Zelda Port", &plan.plans, &mappings);
        assert!(text.contains("stays where it is") && text.contains("Zelda renamed.z64"), "{text}");
    }

    #[test]
    fn an_ambiguous_slot_must_be_chosen_before_connecting() {
        let tmp = temp_dir();
        let roms = tmp.path().join("roms");
        let (a, b) = (n64_rom(11, 9_000), n64_rom(12, 9_000));
        write(&roms.join("n64/pd_usa.z64"), &a);
        write(&roms.join("n64/pd_eur.z64"), &b);
        let port_dir = tmp.path().join("lib/Both");
        fs::create_dir_all(&port_dir).unwrap();
        let text = format!("Required files: Perfect Dark (USA)/(Europe).z64 (CRC32: {:08X}/{:08X})", digests_of(&a).crc32, digests_of(&b).crc32);
        let catalog = vec![port("Both", "Both", &["n64"], &text)];
        let input = super::super::report::ReportInput { catalog, installed: ["Both".to_string()].into(), local_entries: Default::default(), mappings: vec![] };
        let report = build_report(&input, &scan_dir(&roms));
        let row = report.groups.into_iter().flat_map(|g| g.ports).next().unwrap();
        assert_eq!(row.status, crate::fork::rom_library::PortStatus::Ambiguous);
        let mut screen = ConnectScreen::new(&row, port_dir, &[]);
        assert!(screen.requests().unwrap_err().contains("Choose a file"));
        assert!(screen.labels().iter().any(|l| l.contains("2 files match, choose one")));
        assert!(matches!(screen.picks(0)[0], Pick::File(0)));
        screen.choose(0, Pick::File(1));
        assert_eq!(screen.requests().unwrap().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_slot_cannot_be_connected_and_a_broken_connection_is_offered_again() {
        let (f, row) = fixture("link");
        let screen = ConnectScreen::new(&row, f.port_dir.clone(), &[]);
        let plan = make_plan(screen.requests().unwrap()).unwrap();
        let mappings = apply_plans(&f.store_path, &plan.plans, Approval::None).unwrap();
        // The link is deleted by hand.
        fs::remove_file(f.port_dir.join("Zelda renamed.z64")).unwrap();
        let again = ConnectScreen::new(&row, f.port_dir.clone(), &mappings);
        assert!(again.is_editable(0));
        assert!(again.labels().iter().any(|l| l.contains("earlier connection: the link or copy is gone")), "{:?}", again.labels());
        assert!(again.requests().is_ok());
    }

    #[test]
    fn a_copy_that_does_not_fit_is_refused_before_it_starts() {
        assert!(check_space(5_000_000_000, Some(5_500_000_000)).unwrap_err().contains("Not enough free space"));
        assert!(check_space(5_000_000_000, Some(7_000_000_000)).is_ok());
        assert!(check_space(5_000_000_000, None).is_ok(), "unknown space does not block");
        assert!(check_space(0, Some(0)).is_ok(), "a link needs no space");
    }
}
