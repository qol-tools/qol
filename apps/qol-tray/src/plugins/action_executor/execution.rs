use super::resolution::ResolvedAction;
use super::tracking::{
    clear_runtime_spawn_reservation, reserve_runtime_spawn, track_action_process,
    track_unreserved_action_process, untrack_action_process,
};
use super::ActionExecutionError;
use crate::plugins::action_transport::DaemonActionDispatch;
use std::path::Path;
#[cfg(debug_assertions)]
use std::time::Instant;

pub(super) fn execute_resolved_action(
    resolved: &ResolvedAction,
    input: &serde_json::Value,
) -> Result<Option<serde_json::Value>, ActionExecutionError> {
    if let Some(socket_path) = &resolved.daemon_socket {
        return execute_via_daemon(resolved, socket_path, input);
    }

    if has_action_input(input) {
        return Err(ActionExecutionError::ActionRejected(
            "action input requires a daemon-backed plugin".into(),
        ));
    }

    execute_via_runtime(resolved).map(|()| None)
}

fn execute_via_daemon(
    resolved: &ResolvedAction,
    socket_path: &Path,
    input: &serde_json::Value,
) -> Result<Option<serde_json::Value>, ActionExecutionError> {
    #[cfg(debug_assertions)]
    let started = Instant::now();
    let dispatch = crate::plugins::action_transport::dispatch_daemon_action_with_input(
        socket_path,
        &resolved.action_id,
        input,
    );
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "ACTION_DISPATCH",
        "plugin={} action={} outcome={}",
        resolved.plugin_id,
        resolved.action_id,
        super::dispatch_outcome(&dispatch)
    );
    #[cfg(debug_assertions)]
    trace_window_action_dispatch(resolved, input, started.elapsed(), &dispatch);
    if let DaemonActionDispatch::Handled { payload } = &dispatch {
        log::info!(
            "Plugin action handled via daemon: {}::{}",
            resolved.plugin_id,
            resolved.action_id
        );
        return Ok(payload.clone());
    }
    let reason = daemon_failure_reason(resolved, &dispatch)?;
    log::warn!("{} {}::{}", reason, resolved.plugin_id, resolved.action_id);
    if resolved.runtime_fallback_allowed {
        if has_action_input(input) {
            return Err(ActionExecutionError::ActionRejected(
                "action input requires an available plugin daemon".into(),
            ));
        }
        return execute_via_runtime(resolved).map(|()| None);
    }
    Err(ActionExecutionError::SpawnFailed(format!(
        "{} {}::{}",
        reason, resolved.plugin_id, resolved.action_id
    )))
}

#[cfg(debug_assertions)]
fn trace_window_action_dispatch(
    resolved: &ResolvedAction,
    input: &serde_json::Value,
    elapsed: std::time::Duration,
    dispatch: &DaemonActionDispatch,
) {
    let Some(direction) = resolved.action_id.strip_prefix("glide-") else {
        return;
    };
    if resolved.plugin_id.as_str() != "plugin-window-actions" {
        return;
    }
    let phase = input
        .get("phase")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    if phase == "heartbeat" && matches!(dispatch, DaemonActionDispatch::Handled { .. }) {
        return;
    }
    qol_runtime::probe!(
        "WINACT_DISPATCH",
        "session={} seq={} source={} phase={} direction={} transport_us={} outcome={}",
        input
            .get("trace_session")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        input
            .get("trace_seq")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        input
            .get("trace_source")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown"),
        phase,
        direction,
        elapsed.as_micros(),
        super::dispatch_outcome(dispatch)
    );
}

fn has_action_input(input: &serde_json::Value) -> bool {
    !input.is_null() && !input.as_object().is_some_and(serde_json::Map::is_empty)
}

fn daemon_failure_reason(
    resolved: &ResolvedAction,
    dispatch: &DaemonActionDispatch,
) -> Result<&'static str, ActionExecutionError> {
    match dispatch {
        DaemonActionDispatch::Fallback => Ok("daemon rejected action"),
        DaemonActionDispatch::Unavailable => Ok("daemon unavailable for"),
        DaemonActionDispatch::Error(message) => daemon_dispatch_error(resolved, message),
        DaemonActionDispatch::Handled { .. } => unreachable!(),
    }
}

fn daemon_dispatch_error(
    resolved: &ResolvedAction,
    message: &str,
) -> Result<&'static str, ActionExecutionError> {
    log::warn!(
        "Daemon error for {}::{}: {}",
        resolved.plugin_id,
        resolved.action_id,
        message
    );
    Err(ActionExecutionError::ActionRejected(message.to_string()))
}

fn execute_via_runtime(resolved: &ResolvedAction) -> Result<(), ActionExecutionError> {
    let command_path = runtime_command_path(resolved)?;
    if resolved.dedupe_runtime_spawn
        && !reserve_runtime_spawn(resolved.plugin_id.as_str(), &resolved.action_id)
    {
        return Ok(());
    }

    log::info!(
        "Executing runtime action: {:?} {:?}",
        command_path,
        resolved.args
    );
    #[cfg(feature = "dev")]
    let mut command = runtime_command(resolved, command_path);
    #[cfg(not(feature = "dev"))]
    let command = runtime_command(resolved, command_path);
    #[cfg(feature = "dev")]
    let relay_patterns = configure_action_log_relay(resolved, &mut command);
    #[cfg(feature = "dev")]
    let mut child = spawn_runtime_command(resolved, command)?;
    #[cfg(not(feature = "dev"))]
    let child = spawn_runtime_command(resolved, command)?;
    #[cfg(feature = "dev")]
    crate::logging::relay::attach(
        resolved.plugin_id.as_str(),
        None::<std::process::ChildStdout>,
        child.stderr.take(),
        relay_patterns,
    );
    let pid = child.id();
    if resolved.dedupe_runtime_spawn {
        track_action_process(resolved.plugin_id.as_str(), &resolved.action_id, pid);
    } else {
        track_unreserved_action_process(resolved.plugin_id.as_str(), pid);
    }
    spawn_wait_untracker(resolved, child, pid);
    log::info!("Runtime action started (pid: {})", pid);
    Ok(())
}

fn runtime_command_path(resolved: &ResolvedAction) -> Result<&Path, ActionExecutionError> {
    resolved
        .command_path
        .as_deref()
        .ok_or_else(|| ActionExecutionError::NoExecutionTarget {
            plugin_id: resolved.plugin_id.clone(),
            action_id: resolved.action_id.clone(),
        })
}

fn spawn_runtime_command(
    resolved: &ResolvedAction,
    mut command: std::process::Command,
) -> Result<std::process::Child, ActionExecutionError> {
    command.spawn().map_err(|error| {
        if resolved.dedupe_runtime_spawn {
            clear_runtime_spawn_reservation(resolved.plugin_id.as_str(), &resolved.action_id);
        }
        ActionExecutionError::SpawnFailed(error.to_string())
    })
}

// Same constraint as daemon spawns: the tray's stderr is a pipe into qol dev
// that dies at every generation handoff, and an action inheriting it
// EPIPE-panics on its first write. Pipe stderr into the relay instead.
#[cfg(feature = "dev")]
fn configure_action_log_relay(
    resolved: &ResolvedAction,
    command: &mut std::process::Command,
) -> Vec<String> {
    let log_control =
        crate::logging::load_plugin_control_from_shared_config(resolved.plugin_id.as_str());
    if log_control.muted {
        command.stderr(std::process::Stdio::null());
        return Vec::new();
    }
    command.stderr(std::process::Stdio::piped());
    log_control.suppress_patterns
}

fn runtime_command(resolved: &ResolvedAction, command_path: &Path) -> std::process::Command {
    let mut command = std::process::Command::new(command_path);
    command
        .args(&resolved.args)
        .current_dir(&resolved.plugin_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .env(qol_conventions::ENV_PLUGIN_ID, resolved.plugin_id.as_str())
        .env(
            qol_conventions::ENV_STATE_SOCKET,
            crate::dev_generation::state_socket_path(),
        );
    crate::features::theme::apply_accent_env(&mut command);
    crate::features::theme::apply_theme_name_env(&mut command);
    if let Some(socket_path) = &resolved.daemon_socket {
        command.env(qol_conventions::ENV_DAEMON_SOCKET, socket_path);
    }
    command
}

fn spawn_wait_untracker(resolved: &ResolvedAction, mut child: std::process::Child, pid: u32) {
    let plugin_id = resolved.plugin_id.to_string();
    let action_id = resolved.action_id.clone();
    std::thread::spawn(move || {
        let _ = child.wait();
        untrack_action_process(&plugin_id, &action_id, pid);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_plugin_api::manifest::PluginId;
    use std::ffi::OsStr;
    use std::path::PathBuf;

    fn resolved() -> ResolvedAction {
        ResolvedAction {
            plugin_id: PluginId::new("plugin-cli-sessions"),
            action_id: "open".to_string(),
            plugin_dir: PathBuf::from("/tmp"),
            daemon_socket: None,
            command_path: Some(PathBuf::from("/bin/true")),
            args: vec!["open".to_string()],
            runtime_fallback_allowed: true,
            dedupe_runtime_spawn: false,
            hosted_settings: false,
        }
    }

    #[cfg(feature = "dev")]
    #[test]
    fn dev_action_spawns_pipe_stderr_instead_of_inheriting_the_tray_pipe() {
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let resolved = resolved();
        let mut command = runtime_command(&resolved, Path::new("/usr/bin/true"));
        let _patterns = configure_action_log_relay(&resolved, &mut command);

        let mut child = command.spawn().unwrap();

        let stderr = child.stderr.take();
        let _ = child.wait();
        assert!(
            stderr.is_some(),
            "dev runtime actions must pipe stderr; inheriting the tray's qol dev \
             pipe makes the action EPIPE-panic after a generation handoff",
        );
    }

    #[test]
    fn runtime_command_arms_host_death_watchdog_via_state_socket() {
        let command = runtime_command(&resolved(), Path::new("/bin/true"));
        let entry = command
            .get_envs()
            .find(|(key, _)| *key == OsStr::new(qol_conventions::ENV_STATE_SOCKET));
        let (_, value) =
            entry.expect("action spawns must set the state-socket env var so the watchdog arms");
        assert_eq!(
            value,
            Some(OsStr::new(crate::paths::STATE_SOCKET_PATH)),
            "watchdog lifeline must point at the host state socket",
        );
    }

    #[test]
    fn runtime_command_injects_saved_theme_accent() {
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        crate::features::theme::save_selected_accent_key("blue").unwrap();

        let command = runtime_command(&resolved(), Path::new("/bin/true"));
        let (_, value) = command
            .get_envs()
            .find(|(key, _)| *key == OsStr::new(qol_conventions::ENV_THEME_ACCENT))
            .expect("runtime action spawns must inherit the selected tray accent");

        assert_eq!(value, Some(OsStr::new("blue")));
    }
}
