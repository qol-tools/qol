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
        token(&row.project),
        token(row.name.as_deref().unwrap_or("")),
        token(row.branch.as_deref().unwrap_or(""))
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
            quoted(&error.to_string())
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

#[cfg(debug_assertions)]
fn token(value: &str) -> String {
    compact(value, 96)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':' | '@' | ',') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(debug_assertions)]
fn quoted(value: &str) -> String {
    compact(value, 160)
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' {
                c
            } else if c == ' ' {
                ' '
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(debug_assertions)]
fn compact(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}
