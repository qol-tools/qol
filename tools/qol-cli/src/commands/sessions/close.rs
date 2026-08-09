use std::ffi::OsString;

use anyhow::{anyhow, bail, Context, Result};
use qol_terminal_sessions::{SessionBinding, SessionInventory, TerminalSessionService};
use serde::Serialize;

use super::bridge::PendingBridgeStore;

#[derive(Debug, Serialize)]
pub(super) struct CloseOutcome {
    pub(super) session: String,
    pub(super) key: String,
    pub(super) tool: String,
    pub(super) closed: bool,
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

pub(super) fn execute(
    terminals: &TerminalSessionService,
    pending: &PendingBridgeStore,
    binding: &SessionBinding,
) -> Result<CloseOutcome> {
    if terminals.is_current(binding).unwrap_or(false) {
        bail!("refusing to close the calling terminal `{binding}`");
    }
    let facts = terminals
        .discover()
        .context("session discovery failed")?
        .into_iter()
        .find(|session| session.id == *binding.session_id())
        .ok_or_else(|| anyhow!("close target `{binding}` is not a live session"))?;
    let Some(identity) = facts.spawn_identity.clone() else {
        bail!(
            "`{binding}` was not spawned by the session workflow; only spawned implementation sessions can be closed"
        );
    };
    if let Some(round) = pending.pending_round(binding)? {
        bail!(
            "session `{binding}` still has an open feature loop (marker {}); call session_loop_close first",
            round.completion_marker
        );
    }
    terminals.close(binding).context("close failed")?;
    qol_runtime::probe!(
        "CLI_SESSION_SPAWN",
        "event=close key={} tool={}",
        identity.key,
        identity.tool
    );
    Ok(CloseOutcome {
        session: binding.token(),
        key: identity.key.to_string(),
        tool: identity.tool.to_string(),
        closed: true,
    })
}
