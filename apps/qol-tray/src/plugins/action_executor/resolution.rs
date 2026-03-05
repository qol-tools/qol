use super::ActionExecutionError;
use crate::plugins::Plugin;
use std::path::{Component, Path, PathBuf};

pub(super) struct ResolvedAction {
    pub(super) plugin_id: String,
    pub(super) action_id: String,
    pub(super) plugin_dir: PathBuf,
    pub(super) daemon_socket: Option<PathBuf>,
    pub(super) command_path: Option<PathBuf>,
    pub(super) args: Vec<String>,
    pub(super) runtime_fallback_allowed: bool,
}

pub(super) fn resolve_action(
    plugin: &Plugin,
    action_id: &str,
) -> Result<ResolvedAction, ActionExecutionError> {
    validate_action_id(action_id)?;
    let daemon_socket = daemon_socket(plugin);
    let (command_path, args) = resolve_runtime_target(plugin, action_id)?;
    let runtime_fallback_allowed =
        allow_runtime_fallback(plugin, daemon_socket.as_ref(), command_path.as_ref());
    ensure_execution_target(plugin, action_id, &daemon_socket, &command_path)?;
    Ok(ResolvedAction {
        plugin_id: plugin.id.clone(),
        action_id: action_id.to_string(),
        plugin_dir: plugin.path.clone(),
        daemon_socket,
        command_path,
        args,
        runtime_fallback_allowed,
    })
}

fn validate_action_id(action_id: &str) -> Result<(), ActionExecutionError> {
    if crate::plugins::manifest::is_valid_action_id(action_id) {
        return Ok(());
    }

    Err(ActionExecutionError::InvalidActionId(action_id.to_string()))
}

fn daemon_socket(plugin: &Plugin) -> Option<PathBuf> {
    plugin
        .manifest
        .daemon
        .as_ref()
        .and_then(|daemon| enabled_socket(daemon))
        .map(PathBuf::from)
}

fn enabled_socket(daemon: &crate::plugins::manifest::DaemonConfig) -> Option<&str> {
    if daemon.enabled {
        return daemon.socket.as_deref();
    }

    None
}

fn ensure_execution_target(
    plugin: &Plugin,
    action_id: &str,
    daemon_socket: &Option<PathBuf>,
    command_path: &Option<PathBuf>,
) -> Result<(), ActionExecutionError> {
    if daemon_socket.is_some() || command_path.is_some() {
        return Ok(());
    }

    Err(ActionExecutionError::NoExecutionTarget {
        plugin_id: plugin.id.clone(),
        action_id: action_id.to_string(),
    })
}

fn allow_runtime_fallback(
    plugin: &Plugin,
    daemon_socket: Option<&PathBuf>,
    runtime_command_path: Option<&PathBuf>,
) -> bool {
    if daemon_socket.is_none() {
        return runtime_command_path.is_some();
    }

    let Some(runtime_command_path) = runtime_command_path else {
        return false;
    };
    let Some(daemon_command_path) = daemon_command_path(plugin) else {
        return true;
    };
    if !paths_match(runtime_command_path, &daemon_command_path) {
        return true;
    }

    daemon_socket.is_some_and(|socket_path| !is_daemon_socket_reachable(socket_path))
}

fn daemon_command_path(plugin: &Plugin) -> Option<PathBuf> {
    let daemon = plugin
        .manifest
        .daemon
        .as_ref()
        .filter(|daemon| daemon.enabled)?;
    super::super::resolve_plugin_command_path(&plugin.path, &daemon.command)
}

fn paths_match(left: &Path, right: &Path) -> bool {
    let left = std::fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = std::fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    left == right
}

#[cfg(unix)]
fn is_daemon_socket_reachable(socket_path: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket_path).is_ok()
}

#[cfg(not(unix))]
fn is_daemon_socket_reachable(_socket_path: &Path) -> bool {
    false
}

fn resolve_runtime_target(
    plugin: &Plugin,
    action_id: &str,
) -> Result<(Option<PathBuf>, Vec<String>), ActionExecutionError> {
    let Some(runtime) = plugin.manifest.runtime.as_ref() else {
        return Ok((None, Vec::new()));
    };

    validate_runtime_command_path(plugin, &runtime.command)?;
    let command_path = runtime_command_path(plugin, &runtime.command)?;
    let args = runtime_args(plugin, action_id, runtime)?;
    Ok((Some(command_path), args))
}

fn validate_runtime_command_path(
    plugin: &Plugin,
    command: &str,
) -> Result<(), ActionExecutionError> {
    if !runtime_command_escapes_plugin_dir(command) {
        return Ok(());
    }

    Err(ActionExecutionError::RuntimeCommandEscapesPluginDir {
        plugin_id: plugin.id.clone(),
        command: command.to_string(),
    })
}

fn runtime_command_escapes_plugin_dir(command: &str) -> bool {
    let command = Path::new(command);
    command.is_absolute()
        || command
            .components()
            .any(|component| component == Component::ParentDir)
}

fn runtime_command_path(plugin: &Plugin, command: &str) -> Result<PathBuf, ActionExecutionError> {
    super::super::resolve_plugin_command_path(&plugin.path, command).ok_or_else(|| {
        ActionExecutionError::RuntimeCommandNotFound {
            plugin_id: plugin.id.clone(),
            command: command.to_string(),
        }
    })
}

fn runtime_args(
    plugin: &Plugin,
    action_id: &str,
    runtime: &crate::plugins::manifest::RuntimeConfig,
) -> Result<Vec<String>, ActionExecutionError> {
    let Some(actions) = runtime.actions.as_ref() else {
        return Ok(vec![action_id.to_string()]);
    };

    actions
        .get(action_id)
        .cloned()
        .ok_or_else(|| ActionExecutionError::MissingActionMapping {
            plugin_id: plugin.id.clone(),
            action_id: action_id.to_string(),
        })
}
