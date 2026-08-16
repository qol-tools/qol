use std::ffi::OsString;

use anyhow::{bail, Context, Result};
use qol_terminal_sessions::{
    ScreenReader, SessionBinding, SessionInventory, TerminalSessionService,
};
use serde::Serialize;

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
