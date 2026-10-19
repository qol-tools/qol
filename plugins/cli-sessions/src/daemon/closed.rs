use std::collections::HashMap;
use std::path::PathBuf;

use qol_terminal_sessions::cli::{CliSessionDescriptor, CliSessionInterpreter};
use qol_terminal_sessions::closed::{self, ClosedRecord};
use qol_terminal_sessions::SessionId;

use crate::host::Pane;
use crate::session::registry::{is_master_key, meaningful_name};

const MODE_REFRESH_SECS: u64 = 30;

struct Sighting {
    record: ClosedRecord,
    mode_read_at: u64,
}

#[derive(Default)]
pub struct ClosedLedger {
    dir: Option<PathBuf>,
    live: HashMap<SessionId, Sighting>,
}

impl ClosedLedger {
    pub fn with_dir(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            live: HashMap::new(),
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
                self.close(previous.record, now);
            }
            return;
        };
        let mode_read_at = match previous {
            Some(previous) if previous.record.external_id == next.external_id => {
                next.permission_mode = previous.record.permission_mode;
                if now < previous.mode_read_at.saturating_add(MODE_REFRESH_SECS) {
                    previous.mode_read_at
                } else {
                    if let Some(mode) = interpreter.permission_mode(pane) {
                        next.permission_mode = Some(mode);
                    }
                    now
                }
            }
            previous => {
                if let Some(previous) = previous {
                    self.close(previous.record, now);
                }
                next.permission_mode = interpreter.permission_mode(pane);
                self.forget(&next.external_id);
                now
            }
        };
        self.live.insert(
            pane.id.clone(),
            Sighting {
                record: next,
                mode_read_at,
            },
        );
    }

    pub fn retire_missing(&mut self, panes: &[Pane], now: u64) {
        let gone: Vec<SessionId> = self
            .live
            .keys()
            .filter(|id| !panes.iter().any(|pane| &pane.id == *id))
            .cloned()
            .collect();
        for id in gone {
            if let Some(sighting) = self.live.remove(&id) {
                self.close(sighting.record, now);
            }
        }
    }

    fn close(&self, record: ClosedRecord, now: u64) {
        let Some(dir) = self.dir.as_deref() else {
            return;
        };
        let record = ClosedRecord {
            closed_at: now,
            ..record
        };
        if let Err(error) = closed::write(dir, &record) {
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
        closed_at: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use qol_terminal_sessions::cli::{
        claude_tool, generic_tool, CliSessionEvidence, CliSessionStrategy, CliTool,
    };
    use qol_terminal_sessions::{
        SessionCapabilities, SessionFacts, SpawnIdentity, SpawnKey, SpawnSurface,
    };

    struct ModeStrategy {
        tool: CliTool,
        mode: Mutex<Option<String>>,
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

        fn permission_mode(&self, _session: &SessionFacts) -> Option<String> {
            self.mode.lock().unwrap().clone()
        }
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

    fn ledger(root: &tempfile::TempDir) -> ClosedLedger {
        ClosedLedger::with_dir(Some(root.path().join("closed")))
    }

    fn closed_ids(root: &tempfile::TempDir) -> Vec<String> {
        closed::records(&root.path().join("closed"))
            .unwrap()
            .into_iter()
            .map(|record| record.external_id)
            .collect()
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
        assert!(closed_ids(&root).is_empty());

        ledger.retire_missing(&[], 200);
        let record = closed::find(&root.path().join("closed"), "abc").unwrap();
        assert_eq!(record.tool, "claude");
        assert_eq!(record.cwd, "/project");
        assert_eq!(record.title.as_deref(), Some("Old chat"));
        assert_eq!(record.key, None);
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
        let record = closed::find(&root.path().join("closed"), "abc").unwrap();
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
        ledger.retire_missing(&[], 200);
        assert!(closed_ids(&root).is_empty());
    }

    #[test]
    fn the_permission_mode_is_refreshed_while_the_session_lives() {
        let root = tempfile::tempdir().unwrap();
        let strategy = Arc::new(ModeStrategy {
            tool: claude_tool(),
            mode: Mutex::new(Some("default".to_owned())),
        });
        let interpreter = CliSessionInterpreter::from_strategies([
            strategy.clone() as Arc<dyn CliSessionStrategy>
        ])
        .unwrap();
        let mut ledger = ledger(&root);
        let first = pane(1);
        let seen = descriptor(claude_tool(), Some("abc"));
        ledger.observe(&interpreter, &first, &seen, 100);
        *strategy.mode.lock().unwrap() = Some("bypassPermissions".to_owned());
        ledger.observe(&interpreter, &first, &seen, 100 + MODE_REFRESH_SECS - 1);
        assert_eq!(
            ledger.live[&first.id].record.permission_mode.as_deref(),
            Some("default")
        );
        ledger.observe(&interpreter, &first, &seen, 100 + MODE_REFRESH_SECS);
        assert_eq!(
            ledger.live[&first.id].record.permission_mode.as_deref(),
            Some("bypassPermissions")
        );
        *strategy.mode.lock().unwrap() = None;
        ledger.observe(&interpreter, &first, &seen, 100 + 2 * MODE_REFRESH_SECS);
        ledger.retire_missing(&[], 300);
        assert_eq!(
            closed::find(&root.path().join("closed"), "abc")
                .unwrap()
                .permission_mode
                .as_deref(),
            Some("bypassPermissions")
        );
    }

    #[test]
    fn a_ledger_without_a_directory_records_nothing() {
        let interpreter = CliSessionInterpreter::system();
        let mut ledger = ClosedLedger::default();
        ledger.observe(
            &interpreter,
            &pane(1),
            &descriptor(claude_tool(), Some("abc")),
            100,
        );
        ledger.retire_missing(&[], 200);
        assert!(ledger.live.is_empty());
    }
}
