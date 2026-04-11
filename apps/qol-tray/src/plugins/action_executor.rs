use super::manager::PluginManager;
use crate::plugins::PluginId;
use std::fmt::{Display, Formatter};
use std::sync::{Arc, Mutex};

mod execution;
mod resolution;
mod tracking;

#[cfg(test)]
mod tests;

pub use tracking::kill_all_plugin_processes;

#[cfg(feature = "dev")]
pub fn action_processes_snapshot() -> std::collections::HashMap<String, Vec<u32>> {
    tracking::action_processes_snapshot()
}

#[derive(Debug)]
pub enum ActionExecutionError {
    PluginManagerPoisoned,
    PluginNotFound(PluginId),
    InvalidActionId(String),
    RuntimeCommandEscapesPluginDir {
        plugin_id: PluginId,
        command: String,
    },
    RuntimeCommandNotFound {
        plugin_id: PluginId,
        command: String,
    },
    MissingActionMapping {
        plugin_id: PluginId,
        action_id: String,
    },
    NoExecutionTarget {
        plugin_id: PluginId,
        action_id: String,
    },
    ActionRejected(String),
    SpawnFailed(String),
}

impl Display for ActionExecutionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PluginManagerPoisoned => write!(f, "plugin manager lock failed"),
            Self::PluginNotFound(plugin_id) => write!(f, "plugin not found: {}", plugin_id),
            Self::InvalidActionId(action_id) => write!(f, "invalid action id: {}", action_id),
            Self::RuntimeCommandEscapesPluginDir { plugin_id, command } => write!(
                f,
                "runtime command escapes plugin dir for {}: {}",
                plugin_id, command
            ),
            Self::RuntimeCommandNotFound { plugin_id, command } => write!(
                f,
                "runtime command not found for {}: {}",
                plugin_id, command
            ),
            Self::MissingActionMapping {
                plugin_id,
                action_id,
            } => {
                write!(f, "missing action mapping for {}::{}", plugin_id, action_id)
            }
            Self::NoExecutionTarget {
                plugin_id,
                action_id,
            } => {
                write!(f, "no execution target for {}::{}", plugin_id, action_id)
            }
            Self::ActionRejected(message) => write!(f, "{}", message),
            Self::SpawnFailed(error) => write!(f, "spawn failed: {}", error),
        }
    }
}

impl std::error::Error for ActionExecutionError {}

pub fn execute_action(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
    action_id: &str,
) {
    if let Err(error) = try_execute_action(plugin_manager, plugin_id, action_id) {
        log::warn!(
            "Plugin action execution failed for {}::{}: {}",
            plugin_id,
            action_id,
            error
        );
        #[cfg(feature = "dev")]
        {
            eprintln!(
                "[\x1b[31mACTION ERROR\x1b[0m] {}::{} failed: {}",
                plugin_id, action_id, error
            );
        }
    }
}

pub fn try_execute_action(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
    action_id: &str,
) -> Result<(), ActionExecutionError> {
    let resolved = resolve_plugin_action(plugin_manager, plugin_id, action_id)?;
    execution::execute_resolved_action(&resolved)
}

pub fn dispatch_query(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
    query_name: &str,
) -> Result<serde_json::Value, ActionExecutionError> {
    let socket_path = resolve_plugin_daemon_socket(plugin_manager, plugin_id)?;
    let dispatch =
        crate::plugins::action_transport::dispatch_daemon_action(&socket_path, query_name);
    match dispatch {
        crate::plugins::action_transport::DaemonActionDispatch::Handled { payload } => {
            Ok(payload.unwrap_or(serde_json::Value::Null))
        }
        crate::plugins::action_transport::DaemonActionDispatch::Fallback => {
            Err(ActionExecutionError::ActionRejected(format!(
                "query {query_name} rejected by {plugin_id} daemon"
            )))
        }
        crate::plugins::action_transport::DaemonActionDispatch::Error(message) => {
            Err(ActionExecutionError::ActionRejected(message))
        }
        crate::plugins::action_transport::DaemonActionDispatch::Unavailable => Err(
            ActionExecutionError::ActionRejected(format!("daemon unavailable for {plugin_id}")),
        ),
    }
}

fn resolve_plugin_daemon_socket(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
) -> Result<std::path::PathBuf, ActionExecutionError> {
    let plugins = plugin_manager
        .lock()
        .map_err(|_| ActionExecutionError::PluginManagerPoisoned)?;
    let plugin = plugins
        .get(plugin_id)
        .ok_or_else(|| ActionExecutionError::PluginNotFound(PluginId::new(plugin_id)))?;
    let socket = plugin
        .manifest
        .daemon
        .as_ref()
        .and_then(|daemon| daemon.socket.as_ref())
        .ok_or_else(|| ActionExecutionError::NoExecutionTarget {
            plugin_id: PluginId::new(plugin_id),
            action_id: "<query>".to_string(),
        })?;
    Ok(std::path::PathBuf::from(socket))
}

pub fn dispatch_action_by_name(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
    action_name: &str,
    _input: serde_json::Value,
) -> Result<serde_json::Value, ActionExecutionError> {
    try_execute_action(plugin_manager, plugin_id, action_name)?;
    Ok(serde_json::Value::Null)
}

fn resolve_plugin_action(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
    action_id: &str,
) -> Result<resolution::ResolvedAction, ActionExecutionError> {
    let plugins = plugin_manager
        .lock()
        .map_err(|_| ActionExecutionError::PluginManagerPoisoned)?;
    let plugin = plugins
        .get(plugin_id)
        .ok_or_else(|| ActionExecutionError::PluginNotFound(PluginId::new(plugin_id)))?;
    resolution::resolve_action(plugin, action_id)
}
