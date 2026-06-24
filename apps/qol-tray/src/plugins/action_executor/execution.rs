use super::resolution::ResolvedAction;
use super::tracking::{
    clear_runtime_spawn_reservation, reserve_runtime_spawn, track_action_process,
    track_unreserved_action_process, untrack_action_process,
};
use super::ActionExecutionError;
use crate::plugins::action_transport::DaemonActionDispatch;
use std::path::Path;

pub(super) fn execute_resolved_action(
    resolved: &ResolvedAction,
) -> Result<(), ActionExecutionError> {
    if let Some(socket_path) = &resolved.daemon_socket {
        return execute_via_daemon(resolved, socket_path);
    }

    execute_via_runtime(resolved)
}

fn execute_via_daemon(
    resolved: &ResolvedAction,
    socket_path: &Path,
) -> Result<(), ActionExecutionError> {
    let dispatch =
        crate::plugins::action_transport::dispatch_daemon_action(socket_path, &resolved.action_id);
    if matches!(dispatch, DaemonActionDispatch::Handled { .. }) {
        log::info!(
            "Plugin action handled via daemon: {}::{}",
            resolved.plugin_id,
            resolved.action_id
        );
        return Ok(());
    }
    let reason = daemon_failure_reason(resolved, &dispatch)?;
    log::warn!("{} {}::{}", reason, resolved.plugin_id, resolved.action_id);
    if resolved.runtime_fallback_allowed {
        return execute_via_runtime(resolved);
    }
    Err(ActionExecutionError::SpawnFailed(format!(
        "{} {}::{}",
        reason, resolved.plugin_id, resolved.action_id
    )))
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
    let child = spawn_runtime_command(resolved, command_path)?;
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
    command_path: &Path,
) -> Result<std::process::Child, ActionExecutionError> {
    runtime_command(resolved, command_path)
        .spawn()
        .map_err(|error| {
            if resolved.dedupe_runtime_spawn {
                clear_runtime_spawn_reservation(resolved.plugin_id.as_str(), &resolved.action_id);
            }
            ActionExecutionError::SpawnFailed(error.to_string())
        })
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
            qol_conventions::STATE_SOCKET_PATH,
        );
    if let Some(socket_path) = &resolved.daemon_socket {
        command.env("QOL_TRAY_DAEMON_SOCKET", socket_path);
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
        }
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
}
