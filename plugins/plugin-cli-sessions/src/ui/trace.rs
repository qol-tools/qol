use crate::registry::SessionState;

pub(super) fn jump_missing(reason: &'static str, index: usize, row_count: usize) {
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "CLI_SESSIONS_JUMP",
        "phase=missing reason={reason} index={index} rows={row_count}"
    );

    #[cfg(not(debug_assertions))]
    let _ = (reason, index, row_count);
}

pub(super) fn jump_target(
    reason: &'static str,
    index: usize,
    row_count: usize,
    row: &SessionState,
) {
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "CLI_SESSIONS_JUMP",
        "phase=target reason={reason} index={index} rows={row_count} wid={} status={:?} tool={:?} project={} name={} branch={}",
        row.window_id,
        row.status,
        row.tool,
        qol_runtime::probe::token(&row.project),
        qol_runtime::probe::token(row.name.as_deref().unwrap_or("")),
        qol_runtime::probe::token(row.branch.as_deref().unwrap_or(""))
    );

    #[cfg(not(debug_assertions))]
    let _ = (reason, index, row_count, row);
}

pub(super) fn dismiss(reason: &'static str, hidden: bool) {
    #[cfg(debug_assertions)]
    qol_runtime::probe!("CLI_SESSIONS_DISMISS", "reason={reason} hidden={hidden}");

    #[cfg(not(debug_assertions))]
    let _ = (reason, hidden);
}

pub(super) fn focus_start(reason: &'static str, window_id: u64) {
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "CLI_SESSIONS_FOCUS",
        "phase=start reason={reason} wid={window_id}"
    );

    #[cfg(not(debug_assertions))]
    let _ = (reason, window_id);
}

pub(super) fn focus_result(reason: &'static str, window_id: u64, result: &anyhow::Result<()>) {
    #[cfg(debug_assertions)]
    match result {
        Ok(()) => qol_runtime::probe!(
            "CLI_SESSIONS_FOCUS",
            "phase=done reason={reason} wid={window_id} ok=true"
        ),
        Err(error) => qol_runtime::probe!(
            "CLI_SESSIONS_FOCUS",
            "phase=done reason={reason} wid={window_id} ok=false err=\"{}\"",
            qol_runtime::probe::quoted(&error.to_string(), 160)
        ),
    }

    #[cfg(not(debug_assertions))]
    let _ = (reason, window_id, result);
}

pub(super) fn open_command(shown: bool) {
    #[cfg(debug_assertions)]
    qol_runtime::probe!("CLI_SESSIONS_OPEN", "shown={shown}");

    #[cfg(not(debug_assertions))]
    let _ = shown;
}
