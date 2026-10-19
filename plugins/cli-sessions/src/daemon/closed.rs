use std::collections::HashMap;
use std::path::PathBuf;

use qol_terminal_sessions::cli::{CliSessionDescriptor, CliSessionInterpreter};
use qol_terminal_sessions::closed::{self, ClosedRecord};
use qol_terminal_sessions::SessionId;

use crate::host::Pane;
use crate::session::registry::{is_master_key, meaningful_name};

#[derive(Default)]
pub struct ClosedLedger {
    dir: Option<PathBuf>,
    live: HashMap<SessionId, ClosedRecord>,
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
                self.close(previous, now);
            }
            return;
        };
        match previous {
            Some(previous) if previous.external_id == next.external_id => {
                next.permission_mode = previous.permission_mode;
            }
            previous => {
                if let Some(previous) = previous {
                    self.close(previous, now);
                }
                next.permission_mode = interpreter.permission_mode(pane);
                self.forget(&next.external_id);
            }
        }
        self.live.insert(pane.id.clone(), next);
    }

    pub fn retire_missing(&mut self, panes: &[Pane], now: u64) {
        let gone: Vec<SessionId> = self
            .live
            .keys()
            .filter(|id| !panes.iter().any(|pane| &pane.id == *id))
            .cloned()
            .collect();
        for id in gone {
            if let Some(record) = self.live.remove(&id) {
                self.close(record, now);
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
    use qol_terminal_sessions::cli::{claude_tool, generic_tool, CliSessionEvidence, CliTool};
    use qol_terminal_sessions::{SessionCapabilities, SpawnIdentity, SpawnKey, SpawnSurface};

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
