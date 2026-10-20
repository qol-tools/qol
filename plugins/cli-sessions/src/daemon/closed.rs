use std::collections::HashMap;
use std::path::{Path, PathBuf};

use qol_terminal_sessions::cli::{CliResumeSettings, CliSessionDescriptor, CliSessionInterpreter};
use qol_terminal_sessions::closed::{self, ClosedRecord};
use qol_terminal_sessions::{SessionBinding, SessionId};

use crate::host::Pane;
use crate::session::registry::{is_master_key, meaningful_name};

const MISSING_TICKS: u8 = 3;
const LIVE_REFRESH_SECS: u64 = 60;
const LIVE_DIR: &str = "live";

struct Sighting {
    record: ClosedRecord,
    binding: Option<SessionBinding>,
    persisted_at: u64,
    missed: u8,
}

pub struct ClosedLedger {
    dir: Option<PathBuf>,
    live: HashMap<SessionId, Sighting>,
    restored: HashMap<String, ClosedRecord>,
    restored_missed: u8,
    is_lane: fn(&SessionBinding) -> bool,
}

impl Default for ClosedLedger {
    fn default() -> Self {
        Self::with_dir(None)
    }
}

impl ClosedLedger {
    pub fn with_dir(dir: Option<PathBuf>) -> Self {
        let restored = dir.as_deref().map(restore).unwrap_or_default();
        Self {
            dir,
            live: HashMap::new(),
            restored,
            restored_missed: 0,
            is_lane: qol_sessions_engine::is_lane,
        }
    }

    pub fn observe(
        &mut self,
        interpreter: &CliSessionInterpreter,
        pane: &Pane,
        descriptor: &CliSessionDescriptor,
        now: u64,
    ) {
        let previous = self.live.remove(&pane.id);
        let Some(mut next) = resumable(interpreter, pane, descriptor) else {
            if let Some(previous) = previous {
                self.close(previous, now);
            }
            return;
        };
        let previous = match previous {
            Some(previous) if previous.record.external_id == next.external_id => Some(previous),
            previous => {
                if let Some(previous) = previous {
                    self.close(previous, now);
                }
                None
            }
        };
        let settings = interpreter.resume_settings(pane);
        let sighting = match previous {
            Some(previous) => {
                next.lane = previous.record.lane;
                merge(&mut next, Some(&previous.record), settings);
                let persisted_at = if next == previous.record
                    && now < previous.persisted_at.saturating_add(LIVE_REFRESH_SECS)
                {
                    previous.persisted_at
                } else {
                    self.persist(&next, now);
                    now
                };
                Sighting {
                    record: next,
                    binding: previous.binding,
                    persisted_at,
                    missed: 0,
                }
            }
            None => {
                self.restored.remove(&next.external_id);
                self.live.retain(|_, sighting| {
                    sighting.missed == 0 || sighting.record.external_id != next.external_id
                });
                merge(&mut next, None, settings);
                let binding = pane.binding().ok();
                next.lane = self.lane(binding.as_ref());
                self.forget(&next.external_id);
                self.persist(&next, now);
                Sighting {
                    record: next,
                    binding,
                    persisted_at: now,
                    missed: 0,
                }
            }
        };
        self.live.insert(pane.id.clone(), sighting);
    }

    pub fn retire_missing(&mut self, panes: &[Pane], now: u64) {
        let mut gone = Vec::new();
        for (id, sighting) in &mut self.live {
            if panes.iter().any(|pane| &pane.id == id) {
                sighting.missed = 0;
                continue;
            }
            sighting.missed = sighting.missed.saturating_add(1);
            if sighting.missed >= MISSING_TICKS {
                gone.push(id.clone());
            }
        }
        for id in gone {
            if let Some(sighting) = self.live.remove(&id) {
                self.close(sighting, now);
            }
        }
        self.retire_restored(now);
    }

    fn retire_restored(&mut self, now: u64) {
        if self.restored.is_empty() {
            return;
        }
        self.restored_missed = self.restored_missed.saturating_add(1);
        if self.restored_missed < MISSING_TICKS {
            return;
        }
        for record in std::mem::take(&mut self.restored).into_values() {
            self.unpersist(&record.external_id);
            self.record(&record, now);
        }
    }

    fn close(&self, sighting: Sighting, now: u64) {
        let lane = sighting.record.lane || self.lane(sighting.binding.as_ref());
        let record = ClosedRecord {
            closed_at: now,
            lane,
            ..sighting.record
        };
        self.unpersist(&record.external_id);
        self.record(&record, now);
    }

    fn lane(&self, binding: Option<&SessionBinding>) -> bool {
        self.dir.is_some() && binding.is_some_and(self.is_lane)
    }

    fn record(&self, record: &ClosedRecord, now: u64) {
        let Some(dir) = self.dir.as_deref() else {
            return;
        };
        if let Err(error) = closed::write(dir, record) {
            log::warn!(
                "[cli-sessions] closed session {} not recorded: {error}",
                record.external_id
            );
            return;
        }
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=closed tool={} external_id={} key={:?}",
            record.tool,
            record.external_id,
            record.key
        );
        if let Err(error) = closed::prune(dir, now) {
            log::warn!("[cli-sessions] closed sessions not pruned: {error}");
        }
    }

    fn forget(&self, external_id: &str) {
        let Some(dir) = self.dir.as_deref() else {
            return;
        };
        if let Err(error) = closed::forget(dir, external_id) {
            log::warn!("[cli-sessions] closed session {external_id} not cleared: {error}");
        }
    }

    fn persist(&self, record: &ClosedRecord, now: u64) {
        let Some(dir) = self.dir.as_deref() else {
            return;
        };
        let record = ClosedRecord {
            closed_at: now,
            ..record.clone()
        };
        if let Err(error) = closed::write(&dir.join(LIVE_DIR), &record) {
            log::warn!(
                "[cli-sessions] live session {} not saved: {error}",
                record.external_id
            );
        }
    }

    fn unpersist(&self, external_id: &str) {
        if self
            .live
            .values()
            .any(|sighting| sighting.record.external_id == external_id)
        {
            return;
        }
        let Some(dir) = self.dir.as_deref() else {
            return;
        };
        if let Err(error) = closed::forget(&dir.join(LIVE_DIR), external_id) {
            log::warn!("[cli-sessions] live session {external_id} not cleared: {error}");
        }
    }
}

fn restore(dir: &Path) -> HashMap<String, ClosedRecord> {
    match closed::records(&dir.join(LIVE_DIR)) {
        Ok(records) => records
            .into_iter()
            .map(|record| (record.external_id.clone(), record))
            .collect(),
        Err(error) => {
            log::warn!("[cli-sessions] live sessions not restored: {error}");
            HashMap::new()
        }
    }
}

fn merge(next: &mut ClosedRecord, earlier: Option<&ClosedRecord>, settings: CliResumeSettings) {
    next.permission_mode = settings
        .permission_mode
        .or_else(|| earlier.and_then(|earlier| earlier.permission_mode.clone()));
    next.model = settings
        .model
        .or_else(|| earlier.and_then(|earlier| earlier.model.clone()));
    next.effort = settings
        .effort
        .or_else(|| earlier.and_then(|earlier| earlier.effort.clone()));
    next.claude_config_dir = settings
        .claude_config_dir
        .or_else(|| earlier.and_then(|earlier| earlier.claude_config_dir.clone()));
}

fn resumable(
    interpreter: &CliSessionInterpreter,
    pane: &Pane,
    descriptor: &CliSessionDescriptor,
) -> Option<ClosedRecord> {
    let external_id = descriptor
        .external_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())?;
    let key = pane
        .spawn_identity
        .as_ref()
        .map(|identity| identity.key.as_str().to_owned());
    if is_master_key(key.as_deref()) {
        return None;
    }
    interpreter.resume_args_for(&descriptor.tool.id, external_id)?;
    Some(ClosedRecord {
        external_id: external_id.to_owned(),
        tool: descriptor.tool.id.to_string(),
        cwd: pane.cwd.clone(),
        title: meaningful_name(descriptor.display_name.as_deref()).map(str::to_owned),
        key,
        permission_mode: None,
        model: None,
        effort: None,
        claude_config_dir: None,
        lane: false,
        closed_at: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use qol_terminal_sessions::cli::{
        claude_tool, generic_tool, CliSessionEvidence, CliSessionStrategy, CliTool,
    };
    use qol_terminal_sessions::{
        SessionCapabilities, SessionFacts, SpawnIdentity, SpawnKey, SpawnSurface,
    };

    struct ModeStrategy {
        tool: CliTool,
        settings: Mutex<CliResumeSettings>,
    }

    impl CliSessionStrategy for ModeStrategy {
        fn tool(&self) -> &CliTool {
            &self.tool
        }

        fn matches(&self, session: &SessionFacts) -> bool {
            session
                .foreground_basenames
                .iter()
                .any(|name| name == "claude")
        }

        fn describe(&self, _session: &SessionFacts) -> CliSessionDescriptor {
            descriptor(self.tool.clone(), None)
        }

        fn resume_args(&self, external_id: &str) -> Option<Vec<String>> {
            Some(vec!["--resume".to_owned(), external_id.to_owned()])
        }

        fn resume_settings(&self, _session: &SessionFacts) -> CliResumeSettings {
            self.settings.lock().unwrap().clone()
        }
    }

    static LANE: AtomicBool = AtomicBool::new(false);

    fn never_lane(_binding: &SessionBinding) -> bool {
        false
    }

    fn always_lane(_binding: &SessionBinding) -> bool {
        true
    }

    fn lane_flag(_binding: &SessionBinding) -> bool {
        LANE.load(Ordering::SeqCst)
    }

    fn pane(window: u64) -> Pane {
        Pane {
            id: crate::host::kitty_session_id(window),
            root_pid: 1,
            cwd: "/project".into(),
            title: "chat".into(),
            at_prompt: true,
            focused: false,
            reported_cmd: Some("claude".into()),
            foreground_basenames: vec!["claude".into()],
            foreground_pids: Vec::new(),
            capabilities: SessionCapabilities::ALL,
            spawn_identity: None,
        }
    }

    fn keyed(window: u64, key: &str) -> Pane {
        let mut pane = pane(window);
        pane.spawn_identity = Some(SpawnIdentity {
            key: SpawnKey::new(key).unwrap(),
            tool: claude_tool().id,
            surface: SpawnSurface::Tab,
        });
        pane
    }

    fn descriptor(tool: CliTool, external_id: Option<&str>) -> CliSessionDescriptor {
        CliSessionDescriptor {
            tool,
            display_name: Some("Old chat".to_owned()),
            external_id: external_id.map(str::to_owned),
            external_id_authoritative: false,
            has_activity: None,
            evidence: CliSessionEvidence::default(),
        }
    }

    fn ledger_with(dir: Option<PathBuf>, is_lane: fn(&SessionBinding) -> bool) -> ClosedLedger {
        ClosedLedger {
            is_lane,
            ..ClosedLedger::with_dir(dir)
        }
    }

    fn ledger(root: &tempfile::TempDir) -> ClosedLedger {
        ledger_with(Some(closed_dir(root)), never_lane)
    }

    fn closed_dir(root: &tempfile::TempDir) -> PathBuf {
        root.path().join("closed")
    }

    fn live_dir(root: &tempfile::TempDir) -> PathBuf {
        closed_dir(root).join(LIVE_DIR)
    }

    fn vanish(ledger: &mut ClosedLedger, now: u64) {
        for _ in 0..MISSING_TICKS {
            ledger.retire_missing(&[], now);
        }
    }

    fn closed_ids(root: &tempfile::TempDir) -> Vec<String> {
        closed::records(&closed_dir(root))
            .unwrap()
            .into_iter()
            .map(|record| record.external_id)
            .collect()
    }

    fn settings_interpreter(
        settings: CliResumeSettings,
    ) -> (Arc<ModeStrategy>, CliSessionInterpreter) {
        let strategy = Arc::new(ModeStrategy {
            tool: claude_tool(),
            settings: Mutex::new(settings),
        });
        let interpreter = CliSessionInterpreter::from_strategies([
            strategy.clone() as Arc<dyn CliSessionStrategy>
        ])
        .unwrap();
        (strategy, interpreter)
    }

    #[test]
    fn a_resumable_session_is_recorded_when_its_pane_goes_and_forgotten_when_it_is_seen_again() {
        let root = tempfile::tempdir().unwrap();
        let interpreter = CliSessionInterpreter::system();
        let mut ledger = ledger(&root);
        let first = pane(1);
        ledger.observe(
            &interpreter,
            &first,
            &descriptor(claude_tool(), Some("abc")),
            100,
        );
        ledger.retire_missing(std::slice::from_ref(&first), 101);
        ledger.retire_missing(&[], 102);
        ledger.retire_missing(std::slice::from_ref(&first), 103);
        ledger.retire_missing(&[], 104);
        ledger.retire_missing(&[], 105);
        assert!(closed_ids(&root).is_empty());

        ledger.retire_missing(&[], 200);
        let record = closed::find(&closed_dir(&root), "abc").unwrap();
        assert_eq!(record.tool, "claude");
        assert_eq!(record.cwd, "/project");
        assert_eq!(record.title.as_deref(), Some("Old chat"));
        assert_eq!(record.key, None);
        assert!(!record.lane);
        assert_eq!(record.closed_at, 200);

        ledger.observe(
            &interpreter,
            &pane(2),
            &descriptor(claude_tool(), Some("abc")),
            300,
        );
        assert!(closed_ids(&root).is_empty());
    }

    #[test]
    fn a_tool_that_exits_inside_a_living_pane_is_recorded_at_that_tick() {
        let root = tempfile::tempdir().unwrap();
        let interpreter = CliSessionInterpreter::system();
        let mut ledger = ledger(&root);
        let pane = keyed(1, "lane-7");
        ledger.observe(
            &interpreter,
            &pane,
            &descriptor(claude_tool(), Some("abc")),
            100,
        );
        ledger.observe(&interpreter, &pane, &descriptor(generic_tool(), None), 150);
        let record = closed::find(&closed_dir(&root), "abc").unwrap();
        assert_eq!(record.key.as_deref(), Some("lane-7"));
        assert_eq!(record.closed_at, 150);

        ledger.observe(
            &interpreter,
            &pane,
            &descriptor(claude_tool(), Some("def")),
            160,
        );
        ledger.observe(
            &interpreter,
            &pane,
            &descriptor(claude_tool(), Some("ghi")),
            170,
        );
        assert_eq!(closed_ids(&root), ["def", "abc"]);
    }

    #[test]
    fn the_switchboard_and_tools_without_a_resume_command_are_never_recorded() {
        let root = tempfile::tempdir().unwrap();
        let interpreter = CliSessionInterpreter::system();
        let mut ledger = ledger(&root);
        let switchboard = keyed(1, qol_sessions_engine::SWITCHBOARD_KEY);
        ledger.observe(
            &interpreter,
            &switchboard,
            &descriptor(claude_tool(), Some("sb")),
            100,
        );
        ledger.observe(
            &interpreter,
            &pane(2),
            &descriptor(generic_tool(), Some("gen")),
            100,
        );
        ledger.observe(
            &interpreter,
            &pane(3),
            &descriptor(claude_tool(), Some("  ")),
            100,
        );
        vanish(&mut ledger, 200);
        assert!(closed_ids(&root).is_empty());
    }

    #[test]
    fn resume_settings_are_read_every_tick_and_an_absent_read_keeps_the_last_known_value() {
        let root = tempfile::tempdir().unwrap();
        let (strategy, interpreter) = settings_interpreter(CliResumeSettings {
            permission_mode: Some("default".to_owned()),
            model: Some("opus".to_owned()),
            effort: Some("high".to_owned()),
            claude_config_dir: Some("/home/user/.claude-work".to_owned()),
        });
        let mut ledger = ledger(&root);
        let first = pane(1);
        let seen = descriptor(claude_tool(), Some("abc"));
        ledger.observe(&interpreter, &first, &seen, 100);
        strategy.settings.lock().unwrap().permission_mode = Some("bypassPermissions".to_owned());
        ledger.observe(&interpreter, &first, &seen, 101);
        assert_eq!(
            ledger.live[&first.id].record.permission_mode.as_deref(),
            Some("bypassPermissions")
        );
        *strategy.settings.lock().unwrap() = CliResumeSettings::default();
        ledger.observe(&interpreter, &first, &seen, 102);
        vanish(&mut ledger, 300);
        let record = closed::find(&closed_dir(&root), "abc").unwrap();
        assert_eq!(record.permission_mode.as_deref(), Some("bypassPermissions"));
        assert_eq!(record.model.as_deref(), Some("opus"));
        assert_eq!(record.effort.as_deref(), Some("high"));
        assert_eq!(
            record.claude_config_dir.as_deref(),
            Some("/home/user/.claude-work")
        );
    }

    #[test]
    fn the_lane_flag_comes_from_the_injected_check_at_start_and_at_close() {
        let interpreter = CliSessionInterpreter::system();
        let seen = descriptor(claude_tool(), Some("abc"));

        let root = tempfile::tempdir().unwrap();
        let mut ledger = ledger_with(Some(closed_dir(&root)), always_lane);
        ledger.observe(&interpreter, &pane(1), &seen, 100);
        assert!(ledger.live[&pane(1).id].record.lane);
        assert!(closed::find(&live_dir(&root), "abc").unwrap().lane);

        let root = tempfile::tempdir().unwrap();
        let mut ledger = ledger_with(Some(closed_dir(&root)), lane_flag);
        LANE.store(false, Ordering::SeqCst);
        ledger.observe(&interpreter, &pane(1), &seen, 100);
        assert!(!ledger.live[&pane(1).id].record.lane);
        LANE.store(true, Ordering::SeqCst);
        vanish(&mut ledger, 200);
        assert!(closed::find(&closed_dir(&root), "abc").unwrap().lane);
    }

    #[test]
    fn the_live_file_exists_while_a_sighting_is_open_and_goes_when_it_closes() {
        let root = tempfile::tempdir().unwrap();
        let interpreter = CliSessionInterpreter::system();
        let mut ledger = ledger(&root);
        let seen = descriptor(claude_tool(), Some("abc"));
        ledger.observe(&interpreter, &pane(1), &seen, 100);
        assert_eq!(
            closed::find(&live_dir(&root), "abc").map(|record| record.closed_at),
            Some(100)
        );
        assert!(closed_ids(&root).is_empty());

        ledger.observe(&interpreter, &pane(1), &seen, 100 + LIVE_REFRESH_SECS - 1);
        assert_eq!(
            closed::find(&live_dir(&root), "abc").map(|record| record.closed_at),
            Some(100)
        );
        ledger.observe(&interpreter, &pane(1), &seen, 100 + LIVE_REFRESH_SECS);
        assert_eq!(
            closed::find(&live_dir(&root), "abc").map(|record| record.closed_at),
            Some(100 + LIVE_REFRESH_SECS)
        );

        vanish(&mut ledger, 300);
        assert_eq!(closed::find(&live_dir(&root), "abc"), None);
        assert_eq!(closed_ids(&root), ["abc"]);
    }

    #[test]
    fn a_conversation_open_at_shutdown_is_recorded_at_its_last_sighting_after_a_restart() {
        let root = tempfile::tempdir().unwrap();
        let interpreter = CliSessionInterpreter::system();
        let seen = descriptor(claude_tool(), Some("abc"));
        let mut stopped = ledger(&root);
        stopped.observe(&interpreter, &pane(1), &seen, 100);
        stopped.observe(&interpreter, &pane(1), &seen, 100 + LIVE_REFRESH_SECS);
        drop(stopped);

        let mut ledger = ledger(&root);
        for _ in 1..MISSING_TICKS {
            ledger.retire_missing(&[], 500);
        }
        assert!(closed_ids(&root).is_empty());
        ledger.retire_missing(&[], 500);
        assert_eq!(
            closed::find(&closed_dir(&root), "abc").map(|record| record.closed_at),
            Some(100 + LIVE_REFRESH_SECS)
        );
        assert_eq!(closed::find(&live_dir(&root), "abc"), None);
        assert!(ledger.restored.is_empty());
    }

    #[test]
    fn a_conversation_sighted_again_in_the_first_tick_after_a_restart_is_never_recorded() {
        let root = tempfile::tempdir().unwrap();
        let interpreter = CliSessionInterpreter::system();
        let seen = descriptor(claude_tool(), Some("abc"));
        let first = pane(1);
        let mut stopped = ledger(&root);
        stopped.observe(&interpreter, &first, &seen, 100);
        drop(stopped);

        let mut ledger = ledger(&root);
        ledger.retire_missing(std::slice::from_ref(&first), 200);
        ledger.observe(&interpreter, &first, &seen, 200);
        for _ in 0..MISSING_TICKS {
            ledger.retire_missing(std::slice::from_ref(&first), 201);
        }
        assert!(closed_ids(&root).is_empty());
        assert!(ledger.restored.is_empty());
        assert_eq!(
            closed::find(&live_dir(&root), "abc").map(|record| record.closed_at),
            Some(200)
        );
    }

    #[test]
    fn a_conversation_that_moves_panes_drops_its_missing_old_pane_without_recording_it() {
        let root = tempfile::tempdir().unwrap();
        let interpreter = CliSessionInterpreter::system();
        let seen = descriptor(claude_tool(), Some("abc"));
        let mut ledger = ledger(&root);
        let panes = [pane(1), pane(2)];
        ledger.retire_missing(&panes, 100);
        ledger.observe(&interpreter, &panes[0], &seen, 100);
        ledger.observe(&interpreter, &panes[1], &seen, 100);
        assert_eq!(ledger.live.len(), 2);

        let panes = [pane(2), pane(3)];
        ledger.retire_missing(&panes, 101);
        ledger.observe(&interpreter, &panes[0], &seen, 101);
        ledger.observe(&interpreter, &panes[1], &seen, 101);
        assert_eq!(ledger.live.len(), 2);
        assert!(!ledger.live.contains_key(&pane(1).id));
        for _ in 0..MISSING_TICKS {
            ledger.retire_missing(&panes, 102);
        }
        assert!(closed_ids(&root).is_empty());
        assert!(closed::find(&live_dir(&root), "abc").is_some());
    }

    #[test]
    fn a_ledger_without_a_directory_tracks_sightings_and_retires_them_without_records() {
        let interpreter = CliSessionInterpreter::system();
        let mut ledger = ledger_with(None, never_lane);
        let first = pane(1);
        ledger.observe(
            &interpreter,
            &first,
            &descriptor(claude_tool(), Some("abc")),
            100,
        );
        assert_eq!(ledger.live[&first.id].record.external_id, "abc");
        assert!(ledger.restored.is_empty());
        for _ in 1..MISSING_TICKS {
            ledger.retire_missing(&[], 200);
        }
        assert!(ledger.live.contains_key(&first.id));
        ledger.retire_missing(&[], 200);
        assert!(ledger.live.is_empty());
        assert!(ledger.dir.is_none());
    }
}
