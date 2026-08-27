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
    binding: &SessionBinding,
) -> Result<CloseOutcome> {
    if let Some(round) = pending.pending_round(binding)? {
        bail!(
            "session `{binding}` still has an open feature loop (marker {}); call session_loop_close first",
            round.completion_marker
        );
    }
    let outcome = close_spawned_terminal(terminals, binding)?;
    if outcome.terminal_state == TerminalCloseState::AlreadyGone {
        bail!("close target `{binding}` is not a live session");
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

pub(super) fn reap_orphaned_rounds(
    terminals: &TerminalSessionService,
    pending: &PendingBridgeStore,
) -> Result<Vec<String>> {
    let mut reaped = Vec::new();
    for round in pending.pending_rounds()? {
        if round.completed {
            continue;
        }
        let Ok(binding) = round.session.parse::<SessionBinding>() else {
            continue;
        };
        if !super::watch::session_gone(terminals, &binding) {
            continue;
        }
        pending.discard(&binding)?;
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
    use std::sync::Arc;

    use qol_terminal_sessions::{
        BackendId, DeliveryMode, SessionCapabilities, SessionFacts, SessionFocus, SessionId,
        TerminalBackend, TerminalError, TerminalSnapshot, TextInput,
    };

    struct InventoryOnlyBackend {
        id: BackendId,
        live: Vec<SessionFacts>,
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
            _target: &SessionBinding,
            _text: &str,
            _mode: DeliveryMode,
        ) -> Result<(), TerminalError> {
            Ok(())
        }

        fn send_key(&self, _target: &SessionBinding, _key: &str) -> Result<(), TerminalError> {
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
            spawn_identity: None,
        }
    }

    fn terminals(live: Vec<SessionFacts>) -> TerminalSessionService {
        TerminalSessionService::from_backends([Arc::new(InventoryOnlyBackend {
            id: BackendId::new("fake").unwrap(),
            live,
        }) as Arc<dyn TerminalBackend>])
        .unwrap()
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

        let reaped = reap_orphaned_rounds(&service, &store(&root)).unwrap();

        assert_eq!(reaped, [orphan.token()]);
        assert!(store(&root).pending_round(&orphan).unwrap().is_none());
    }

    #[test]
    fn reap_leaves_an_open_round_with_a_live_terminal_untouched() {
        let root = tempfile::TempDir::new().unwrap();
        let alive = open_round(&root, "1", 100);
        let service = terminals(vec![facts("1", 100)]);

        let reaped = reap_orphaned_rounds(&service, &store(&root)).unwrap();

        assert!(reaped.is_empty());
        assert!(store(&root).pending_round(&alive).unwrap().is_some());
    }

    #[test]
    fn reap_leaves_a_completed_round_untouched_even_when_its_terminal_is_gone() {
        let root = tempfile::TempDir::new().unwrap();
        let finished = open_round(&root, "1", 100);
        store(&root)
            .observe(&finished, "QOL_BRIDGE_DONE_1", true)
            .unwrap();
        let service = terminals(Vec::new());

        let reaped = reap_orphaned_rounds(&service, &store(&root)).unwrap();

        assert!(reaped.is_empty());
        assert!(store(&root).pending_round(&finished).unwrap().is_some());
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

        let reaped = reap_orphaned_rounds(&service, &store(&root)).unwrap();

        assert_eq!(reaped, [gone_first.token(), gone_second.token()]);
        assert!(store(&root).pending_round(&gone_first).unwrap().is_none());
        assert!(store(&root).pending_round(&gone_second).unwrap().is_none());
        assert!(store(&root).pending_round(&finished).unwrap().is_some());
        assert!(store(&root).pending_round(&alive).unwrap().is_some());
    }
}
