use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Result};
use qol_headless::OutputFormat;
use qol_terminal_sessions::{
    ScreenReader, SessionBinding, SessionInventory, TerminalSessionService,
};
use serde::Serialize;
use serde_json::Value;

use super::bridge::{PendingBridgeStore, Role};

const DISCARDED_ROUND_REASON: &str = "_(no report: the lane was closed with a discarded round)_";
const REAPED_ROUND_REASON: &str = "_(no report: the terminal exited before the round reported)_";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TerminalCloseState {
    Closed,
    AlreadyGone,
    CloseFailed,
}

#[derive(Debug, Serialize)]
pub(super) struct CloseOutcome {
    pub(super) session: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tool: Option<String>,
    pub(super) closed: bool,
    pub(super) terminal_state: TerminalCloseState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) close_detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) discarded_round: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct SiblingLaneClose {
    pub(super) session: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tool: Option<String>,
    pub(super) closed: bool,
    pub(super) terminal_state: TerminalCloseState,
    pub(super) report: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) close_detail: Option<String>,
}

pub(super) fn run(args: &[OsString]) -> Result<()> {
    let binding = super::single_binding(args, "qol sessions close <session>")?;
    let outcome = execute(
        &TerminalSessionService::system(),
        &PendingBridgeStore::system()?,
        &super::bridge::trace_dir(),
        &binding,
    )?;
    println!(
        "{}",
        serde_json::to_string(&outcome).context("failed to serialize close outcome")?
    );
    Ok(())
}

pub(super) fn close_spawned_terminal(
    terminals: &TerminalSessionService,
    binding: &SessionBinding,
) -> Result<CloseOutcome> {
    if terminals.is_current(binding).unwrap_or(false) {
        bail!("refusing to close the calling terminal `{binding}`");
    }
    let facts = terminals
        .discover()
        .context("session discovery failed")?
        .into_iter()
        .find(|session| session.id == *binding.session_id());
    let Some(facts) = facts else {
        return Ok(CloseOutcome {
            session: binding.token(),
            key: None,
            tool: None,
            closed: true,
            terminal_state: TerminalCloseState::AlreadyGone,
            close_detail: Some(format!(
                "terminal `{binding}` is no longer live; nothing left to close"
            )),
            discarded_round: None,
        });
    };
    let Some(identity) = facts.spawn_identity.clone() else {
        bail!(
            "`{binding}` was not spawned by the session workflow; only spawned implementation sessions can be closed"
        );
    };
    if let Err(error) = terminals.close(binding) {
        return Ok(CloseOutcome {
            session: binding.token(),
            key: Some(identity.key.to_string()),
            tool: Some(identity.tool.to_string()),
            closed: false,
            terminal_state: TerminalCloseState::CloseFailed,
            close_detail: Some(error.to_string()),
            discarded_round: None,
        });
    }
    qol_runtime::probe!(
        "CLI_SESSION_SPAWN",
        "event=close key={} tool={}",
        identity.key,
        identity.tool
    );
    Ok(CloseOutcome {
        session: binding.token(),
        key: Some(identity.key.to_string()),
        tool: Some(identity.tool.to_string()),
        closed: true,
        terminal_state: TerminalCloseState::Closed,
        close_detail: None,
        discarded_round: None,
    })
}

pub(super) fn close_loop_siblings(
    terminals: &TerminalSessionService,
    pending: &PendingBridgeStore,
    initiator: &str,
    named: &SessionBinding,
) -> Result<Vec<SiblingLaneClose>> {
    let mut siblings = Vec::new();
    for round in pending.pending_rounds()? {
        if round.session == named.token() || round.driver != initiator || !round.completed {
            continue;
        }
        let binding: SessionBinding = round.session.parse().with_context(|| {
            format!(
                "sibling checkpoint carries an invalid session token `{}`",
                round.session
            )
        })?;
        if pending.role(&binding)? != Role::Lane {
            continue;
        }
        let report = round
            .screen
            .filter(|screen| !screen.is_empty())
            .unwrap_or_else(|| terminals.read_screen(&binding).unwrap_or_default());
        let close = close_spawned_terminal(terminals, &binding)?;
        siblings.push(SiblingLaneClose {
            session: round.session,
            key: close.key,
            tool: close.tool,
            closed: close.closed,
            terminal_state: close.terminal_state,
            report,
            close_detail: close.close_detail,
        });
    }
    Ok(siblings)
}

pub(super) fn execute(
    terminals: &TerminalSessionService,
    pending: &PendingBridgeStore,
    trace_dir: &Path,
    binding: &SessionBinding,
) -> Result<CloseOutcome> {
    let hung = match pending.pending_round(binding)? {
        Some(round) if round.completed => bail!(
            "session `{binding}` holds a completed round awaiting review (marker {}); review it and call session_loop_close",
            round.completion_marker
        ),
        Some(round) => Some(round),
        None => None,
    };
    let mut outcome = close_spawned_terminal(terminals, binding)?;
    if let Some(round) = hung {
        let marker = round.completion_marker.clone();
        pending.discard(binding)?;
        outcome.discarded_round = Some(marker);
        super::watch::settle_orphaned_group_round(
            terminals,
            pending,
            trace_dir,
            &round,
            DISCARDED_ROUND_REASON,
        )?;
        qol_runtime::probe!(
            "CLI_SESSION_SPAWN",
            "event=close_discarded_hung_round session={}",
            binding.token()
        );
    }
    Ok(outcome)
}

const LOOP_CLOSE_USAGE: &str = "qol sessions loop-close <session> --completion-marker MARKER --outcome accepted|paused [--landed TEXT] [--before TEXT] [--now TEXT] [--verification TEXT] [--remaining TEXT]";
const NARRATIVE_PLACEHOLDER: &str = "(not provided)";

#[derive(Default)]
struct LoopCloseArgs {
    completion_marker: Option<String>,
    outcome: Option<String>,
    landed: Option<String>,
    before: Option<String>,
    now: Option<String>,
    verification: Option<String>,
    remaining: Option<String>,
}

pub(super) fn run_loop_close(args: &[OsString], output_format: OutputFormat) -> Result<()> {
    let reports_dir = qol_config::data_subdir("sessions").unwrap_or_else(|| PathBuf::from("."));
    let receipt = loop_close_from_args(
        &TerminalSessionService::system(),
        &PendingBridgeStore::system()?,
        &reports_dir,
        args,
    )?;
    print_loop_close_receipt(&receipt, output_format)
}

pub(super) fn loop_close_from_args(
    terminals: &TerminalSessionService,
    pending: &PendingBridgeStore,
    reports_dir: &Path,
    args: &[OsString],
) -> Result<Value> {
    let (session_token, parsed) = parse_loop_close_args(args)?;
    let binding = SessionBinding::from_str(&session_token)
        .map_err(|error| anyhow!("invalid session token `{session_token}`: {error}"))?;
    let completion_marker = parsed.completion_marker.as_deref().ok_or_else(|| {
        anyhow!("loop-close needs --completion-marker: pass the reviewed round's completion marker")
    })?;
    let outcome = parsed
        .outcome
        .as_deref()
        .ok_or_else(|| anyhow!("loop-close needs --outcome: pass `accepted` or `paused`"))?;
    if !matches!(outcome, "accepted" | "paused") {
        bail!("loop-close `--outcome` must be `accepted` or `paused`, not `{outcome}`");
    }
    let filled = [
        parsed
            .landed
            .unwrap_or_else(|| NARRATIVE_PLACEHOLDER.to_owned()),
        parsed
            .before
            .unwrap_or_else(|| NARRATIVE_PLACEHOLDER.to_owned()),
        parsed
            .now
            .unwrap_or_else(|| NARRATIVE_PLACEHOLDER.to_owned()),
        parsed
            .verification
            .unwrap_or_else(|| NARRATIVE_PLACEHOLDER.to_owned()),
        parsed
            .remaining
            .unwrap_or_else(|| NARRATIVE_PLACEHOLDER.to_owned()),
    ];
    super::mcp::execute_loop_close(
        terminals,
        pending,
        reports_dir,
        &super::mcp::LoopCloseCommand {
            binding: &binding,
            completion_marker,
            accepted: outcome == "accepted",
            narrative: super::mcp::LoopCloseNarrative::Filled(&filled),
        },
    )
}

fn print_loop_close_receipt(receipt: &Value, output_format: OutputFormat) -> Result<()> {
    match output_format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(receipt)
                .context("failed to serialize loop-close receipt")?
        ),
        OutputFormat::PlainText => {
            println!(
                "loop_closed={} outcome={}",
                receipt["loop_closed"].as_bool().unwrap_or_default(),
                receipt["outcome"].as_str().unwrap_or_default(),
            );
            if let Some(state) = receipt.get("terminal_state").and_then(Value::as_str) {
                println!(
                    "terminal_closed={} terminal_state={}",
                    receipt["terminal_closed"].as_bool().unwrap_or_default(),
                    state,
                );
            }
            if let Some(lanes) = receipt.get("sibling_lanes").and_then(Value::as_array) {
                println!("sibling lanes closed: {}", lanes.len());
            }
            println!();
            println!("{}", receipt["final_report"].as_str().unwrap_or_default());
        }
    }
    Ok(())
}

fn parse_loop_close_args(args: &[OsString]) -> Result<(String, LoopCloseArgs)> {
    let session = args
        .first()
        .and_then(|argument| argument.to_str())
        .ok_or_else(|| anyhow!("usage: {LOOP_CLOSE_USAGE}"))?
        .to_owned();
    let mut parsed = LoopCloseArgs::default();
    let mut index = 1;
    while index < args.len() {
        let argument = args[index]
            .to_str()
            .ok_or_else(|| anyhow!("loop-close arguments must be valid UTF-8"))?;
        match argument {
            "--completion-marker" => {
                parsed.completion_marker = Some(loop_close_value(args, index)?);
            }
            "--outcome" => parsed.outcome = Some(loop_close_value(args, index)?),
            "--landed" => parsed.landed = Some(loop_close_value(args, index)?),
            "--before" => parsed.before = Some(loop_close_value(args, index)?),
            "--now" => parsed.now = Some(loop_close_value(args, index)?),
            "--verification" => parsed.verification = Some(loop_close_value(args, index)?),
            "--remaining" => parsed.remaining = Some(loop_close_value(args, index)?),
            other => bail!("unknown loop-close flag `{other}`\nusage: {LOOP_CLOSE_USAGE}"),
        }
        index += 2;
    }
    Ok((session, parsed))
}

fn loop_close_value(args: &[OsString], index: usize) -> Result<String> {
    args.get(index + 1)
        .and_then(|value| value.to_str())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("usage: {LOOP_CLOSE_USAGE}"))
}

fn initiator_gone(terminals: &TerminalSessionService, driver: &str) -> bool {
    driver
        .parse::<SessionBinding>()
        .map(|binding| super::watch::session_gone(terminals, &binding))
        .unwrap_or(true)
}

pub(super) fn reap_orphaned_rounds(
    terminals: &TerminalSessionService,
    pending: &PendingBridgeStore,
    trace_dir: &Path,
) -> Result<Vec<String>> {
    let mut reaped = Vec::new();
    for round in pending.pending_rounds()? {
        let Ok(binding) = round.session.parse::<SessionBinding>() else {
            continue;
        };
        if !super::watch::session_gone(terminals, &binding) {
            continue;
        }
        if round.completed && !initiator_gone(terminals, &round.driver) {
            continue;
        }
        pending.discard(&binding)?;
        super::watch::settle_orphaned_group_round(
            terminals,
            pending,
            trace_dir,
            &round,
            REAPED_ROUND_REASON,
        )?;
        qol_runtime::probe!(
            "CLI_SESSION_BRIDGE",
            "event=reaped_orphan session={}",
            round.session
        );
        reaped.push(round.session);
    }
    Ok(reaped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use qol_terminal_sessions::{
        BackendId, DeliveryMode, SessionCapabilities, SessionCloser, SessionFacts, SessionFocus,
        SessionId, SpawnIdentity, SpawnKey, SpawnSurface, TerminalBackend, TerminalError,
        TerminalSnapshot, TextInput,
    };

    struct InventoryOnlyBackend {
        id: BackendId,
        live: Vec<SessionFacts>,
        sent: Mutex<Vec<(SessionBinding, String)>>,
        closed: Mutex<Vec<SessionBinding>>,
    }

    impl SessionInventory for InventoryOnlyBackend {
        fn discover(&self) -> Result<Vec<SessionFacts>, TerminalError> {
            Ok(self.live.clone())
        }
    }

    impl ScreenReader for InventoryOnlyBackend {
        fn read_screen(&self, target: &SessionBinding) -> Result<String, TerminalError> {
            Err(TerminalError::Unsupported {
                target: target.session_id().clone(),
                capability: "screen reading",
            })
        }
    }

    impl SessionFocus for InventoryOnlyBackend {
        fn focus(&self, _target: &SessionBinding) -> Result<(), TerminalError> {
            Ok(())
        }
    }

    impl TextInput for InventoryOnlyBackend {
        fn send_text(
            &self,
            target: &SessionBinding,
            text: &str,
            _mode: DeliveryMode,
        ) -> Result<(), TerminalError> {
            self.sent
                .lock()
                .unwrap()
                .push((target.clone(), text.to_owned()));
            Ok(())
        }

        fn send_key(&self, _target: &SessionBinding, _key: &str) -> Result<(), TerminalError> {
            Ok(())
        }
    }

    impl SessionCloser for InventoryOnlyBackend {
        fn close(&self, target: &SessionBinding) -> Result<(), TerminalError> {
            self.closed.lock().unwrap().push(target.clone());
            Ok(())
        }
    }

    impl TerminalBackend for InventoryOnlyBackend {
        fn read_screen_from_snapshot(
            &self,
            _snapshot: &TerminalSnapshot,
            target: &SessionBinding,
        ) -> Result<String, TerminalError> {
            Err(TerminalError::Unsupported {
                target: target.session_id().clone(),
                capability: "screen reading",
            })
        }

        fn id(&self) -> &BackendId {
            &self.id
        }

        fn closer(&self) -> Option<&dyn SessionCloser> {
            Some(self)
        }
    }

    fn facts(native: &str, root_pid: i32) -> SessionFacts {
        SessionFacts {
            id: SessionId::new(BackendId::new("fake").unwrap(), native).unwrap(),
            root_pid,
            cwd: "/work".to_owned(),
            title: "Terminal".to_owned(),
            at_prompt: true,
            reported_cmd: None,
            foreground_basenames: Vec::new(),
            foreground_pids: Vec::new(),
            capabilities: SessionCapabilities::ALL,
            spawn_identity: Some(SpawnIdentity {
                key: SpawnKey::new(format!("key-{native}")).unwrap(),
                tool: qol_terminal_sessions::cli::CliToolId::new("pi").unwrap(),
                surface: SpawnSurface::Tab,
            }),
        }
    }

    fn terminals(live: Vec<SessionFacts>) -> TerminalSessionService {
        terminals_with_backend(live).0
    }

    fn terminals_with_backend(
        live: Vec<SessionFacts>,
    ) -> (TerminalSessionService, Arc<InventoryOnlyBackend>) {
        let backend = Arc::new(InventoryOnlyBackend {
            id: BackendId::new("fake").unwrap(),
            live,
            sent: Mutex::new(Vec::new()),
            closed: Mutex::new(Vec::new()),
        });
        let service = TerminalSessionService::from_backends([
            Arc::clone(&backend) as Arc<dyn TerminalBackend>
        ])
        .unwrap();
        (service, backend)
    }

    fn store(root: &tempfile::TempDir) -> PendingBridgeStore {
        PendingBridgeStore::with_dir(root.path().to_path_buf())
    }

    fn open_round(root: &tempfile::TempDir, native: &str, root_pid: i32) -> SessionBinding {
        let binding = format!("v1:fake:{native}:{root_pid}")
            .parse::<SessionBinding>()
            .unwrap();
        store(root)
            .start(
                &binding,
                &format!("QOL_BRIDGE_DONE_{native}"),
                "v1:fake:initiator:9",
                false,
                None,
            )
            .unwrap();
        binding
    }

    #[test]
    fn reap_discards_an_open_round_whose_terminal_is_gone() {
        let root = tempfile::TempDir::new().unwrap();
        let orphan = open_round(&root, "1", 100);
        let service = terminals(Vec::new());

        let reaped = reap_orphaned_rounds(&service, &store(&root), root.path()).unwrap();

        assert_eq!(reaped, [orphan.token()]);
        assert!(store(&root).pending_round(&orphan).unwrap().is_none());
    }

    #[test]
    fn reap_leaves_an_open_round_with_a_live_terminal_untouched() {
        let root = tempfile::TempDir::new().unwrap();
        let alive = open_round(&root, "1", 100);
        let service = terminals(vec![facts("1", 100)]);

        let reaped = reap_orphaned_rounds(&service, &store(&root), root.path()).unwrap();

        assert!(reaped.is_empty());
        assert!(store(&root).pending_round(&alive).unwrap().is_some());
    }

    #[test]
    fn reap_leaves_a_completed_round_whose_initiator_is_still_live() {
        let root = tempfile::TempDir::new().unwrap();
        let finished = open_round(&root, "1", 100);
        store(&root)
            .observe(&finished, "QOL_BRIDGE_DONE_1", true)
            .unwrap();
        let service = terminals(vec![facts("initiator", 9)]);

        let reaped = reap_orphaned_rounds(&service, &store(&root), root.path()).unwrap();

        assert!(
            reaped.is_empty(),
            "the initiator can still loop-close this lane, so its report stays"
        );
        assert!(store(&root).pending_round(&finished).unwrap().is_some());
    }

    #[test]
    fn reap_discards_a_completed_round_whose_terminal_and_initiator_are_gone() {
        let root = tempfile::TempDir::new().unwrap();
        let finished = open_round(&root, "1", 100);
        store(&root)
            .observe(&finished, "QOL_BRIDGE_DONE_1", true)
            .unwrap();
        let service = terminals(Vec::new());

        let reaped = reap_orphaned_rounds(&service, &store(&root), root.path()).unwrap();

        assert_eq!(
            reaped,
            [finished.token()],
            "a completed round nobody can review or loop-close is not left as an orphan row"
        );
        assert!(store(&root).pending_round(&finished).unwrap().is_none());
    }

    #[test]
    fn reap_names_exactly_the_rounds_it_removed() {
        let root = tempfile::TempDir::new().unwrap();
        let gone_first = open_round(&root, "1", 100);
        let gone_second = open_round(&root, "2", 200);
        let finished = open_round(&root, "3", 300);
        store(&root)
            .observe(&finished, "QOL_BRIDGE_DONE_3", true)
            .unwrap();
        let alive = open_round(&root, "4", 400);
        let service = terminals(vec![facts("4", 400)]);

        let reaped = reap_orphaned_rounds(&service, &store(&root), root.path()).unwrap();

        assert_eq!(
            reaped,
            [gone_first.token(), gone_second.token(), finished.token()]
        );
        assert!(store(&root).pending_round(&gone_first).unwrap().is_none());
        assert!(store(&root).pending_round(&gone_second).unwrap().is_none());
        assert!(store(&root).pending_round(&finished).unwrap().is_none());
        assert!(store(&root).pending_round(&alive).unwrap().is_some());
    }

    #[test]
    fn closing_a_hung_group_member_settles_the_group_and_delivers_one_wake() {
        let root = tempfile::TempDir::new().unwrap();
        let group = "closed-member";
        let driver: SessionBinding = "v1:fake:driver:900".parse().unwrap();
        let lane: SessionBinding = "v1:fake:spawn-group-lane:200".parse().unwrap();
        let done: SessionBinding = "v1:fake:spawn-group-done:300".parse().unwrap();
        let pending = store(&root);
        pending
            .start_with_label(
                &lane,
                "QOL_BRIDGE_DONE_hung",
                &driver.token(),
                false,
                Some(group),
                Some("lane-hung"),
                false,
            )
            .unwrap();
        super::super::watch::register_group_member(
            root.path(),
            group,
            &lane.token(),
            Some("lane-hung"),
        )
        .unwrap();
        super::super::watch::register_group_member(
            root.path(),
            group,
            &done.token(),
            Some("lane-done"),
        )
        .unwrap();
        super::super::watch::settle_group_round(
            root.path(),
            group,
            &done.token(),
            Some("lane-done"),
            "finished report body",
        )
        .unwrap();
        let (service, backend) =
            terminals_with_backend(vec![facts("spawn-group-lane", 200), facts("driver", 900)]);

        let outcome = execute(&service, &pending, root.path(), &lane).unwrap();

        assert!(outcome.closed);
        assert_eq!(
            outcome.discarded_round.as_deref(),
            Some("QOL_BRIDGE_DONE_hung")
        );
        let member = super::super::watch::combined_report_path(root.path(), group)
            .with_file_name("members")
            .join("v1_fake_spawn-group-lane_200.json");
        let recorded = std::fs::read_to_string(&member).unwrap();
        assert!(
            recorded.contains("\"outcome\":\"discarded\""),
            "closing a hung grouped lane must record a terminal outcome: {recorded}"
        );
        let combined = std::fs::read_to_string(super::super::watch::combined_report_path(
            root.path(),
            group,
        ))
        .unwrap();
        assert!(
            combined.contains("finished report body"),
            "the combined report must carry the finished member: {combined}"
        );
        assert!(
            combined.contains(DISCARDED_ROUND_REASON),
            "the combined report must name the discarded round's reason: {combined}"
        );
        let sent = backend.sent.lock().unwrap();
        assert_eq!(sent.len(), 1, "exactly one combined wake: {sent:?}");
        assert_eq!(sent[0].0, driver, "the wake goes to the initiator");
        assert!(
            sent[0].1.contains(group) && sent[0].1.contains("closed with a discarded round"),
            "the wake must name the group and the discarded member: {:?}",
            sent[0].1
        );
        drop(sent);
        assert!(pending.pending_round(&lane).unwrap().is_none());

        let delivered = super::super::watch::maybe_deliver_group_combined(
            &pending,
            &service,
            root.path(),
            group,
            &lane.token(),
            &driver.token(),
            false,
            &mut |_| {},
        )
        .unwrap();
        assert!(delivered.is_none());
        assert_eq!(
            backend.sent.lock().unwrap().len(),
            1,
            "a settled group must never deliver a duplicate wake"
        );
    }

    #[test]
    fn reaping_a_gone_group_member_settles_its_group() {
        let root = tempfile::TempDir::new().unwrap();
        let group = "reaped-member";
        let driver: SessionBinding = "v1:fake:driver:900".parse().unwrap();
        let lane: SessionBinding = "v1:fake:spawn-group-lane:200".parse().unwrap();
        let done: SessionBinding = "v1:fake:spawn-group-done:300".parse().unwrap();
        let pending = store(&root);
        pending
            .start_with_label(
                &lane,
                "QOL_BRIDGE_DONE_gone",
                &driver.token(),
                false,
                Some(group),
                Some("lane-gone"),
                false,
            )
            .unwrap();
        super::super::watch::register_group_member(
            root.path(),
            group,
            &lane.token(),
            Some("lane-gone"),
        )
        .unwrap();
        super::super::watch::register_group_member(
            root.path(),
            group,
            &done.token(),
            Some("lane-done"),
        )
        .unwrap();
        super::super::watch::settle_group_round(
            root.path(),
            group,
            &done.token(),
            Some("lane-done"),
            "finished report body",
        )
        .unwrap();
        let (service, backend) = terminals_with_backend(vec![facts("driver", 900)]);

        let reaped = reap_orphaned_rounds(&service, &pending, root.path()).unwrap();

        assert_eq!(reaped, [lane.token()]);
        let member = super::super::watch::combined_report_path(root.path(), group)
            .with_file_name("members")
            .join("v1_fake_spawn-group-lane_200.json");
        let recorded = std::fs::read_to_string(&member).unwrap();
        assert!(
            recorded.contains("\"outcome\":\"discarded\""),
            "reaping a gone grouped lane must record a terminal outcome: {recorded}"
        );
        let combined = std::fs::read_to_string(super::super::watch::combined_report_path(
            root.path(),
            group,
        ))
        .unwrap();
        assert!(combined.contains("finished report body"));
        assert!(combined.contains(REAPED_ROUND_REASON));
        assert_eq!(
            backend.sent.lock().unwrap().len(),
            1,
            "the reaped member's group must deliver exactly one wake"
        );
    }
}
