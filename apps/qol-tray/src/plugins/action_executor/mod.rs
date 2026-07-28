use super::manager::PluginManager;
use crate::plugins::action_transport::DaemonActionDispatch;
use crate::plugins::{Plugin, PluginId};
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod execution;
mod platform;
mod resolution;
mod tracking;

#[cfg(test)]
mod tests;

pub use tracking::kill_all_plugin_processes;
pub(crate) use tracking::kill_plugin_processes;

const DAEMON_READY_TIMEOUT: Duration = Duration::from_secs(3);
const DAEMON_READY_INTERVAL: Duration = Duration::from_millis(25);
const QUERY_DAEMON_READY_PROBE_TIMEOUT: Duration = Duration::from_millis(100);

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
    execute_action_with_input(
        plugin_manager,
        plugin_id,
        action_id,
        serde_json::Value::Null,
    );
}

pub fn execute_action_with_input(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
    action_id: &str,
    input: serde_json::Value,
) {
    if let Err(error) = try_execute_action_with_input(plugin_manager, plugin_id, action_id, input) {
        log::error!(
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
    try_execute_action_with_input(
        plugin_manager,
        plugin_id,
        action_id,
        serde_json::Value::Null,
    )
}

pub fn try_execute_action_with_input(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
    action_id: &str,
    input: serde_json::Value,
) -> Result<(), ActionExecutionError> {
    try_execute_action_with_input_result(plugin_manager, plugin_id, action_id, input).map(drop)
}

pub fn try_execute_action_with_input_result(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
    action_id: &str,
    input: serde_json::Value,
) -> Result<Option<serde_json::Value>, ActionExecutionError> {
    let resolved = resolve_plugin_action(plugin_manager, plugin_id, action_id)?;
    if resolved.hosted_settings {
        match crate::settings_surface::request(plugin_id) {
            Ok(true) => {
                qol_runtime::probe!(
                    "SURFACE_ACTIVATION",
                    "plugin={plugin_id} phase=route outcome=hosted"
                );
                return Ok(None);
            }
            Ok(false) => qol_runtime::probe!(
                "SURFACE_ACTIVATION",
                "plugin={plugin_id} phase=route outcome=platform_fallback"
            ),
            Err(error) => {
                log::warn!(
                    "Native settings host failed for {}::{}: {:#}; using plugin fallback",
                    plugin_id,
                    action_id,
                    error
                );
                qol_runtime::probe!(
                    "SURFACE_ACTIVATION",
                    "plugin={plugin_id} phase=route outcome=spawn_failed error={error}"
                );
            }
        }
    }
    ensure_daemon_ready_for_action(plugin_manager, &resolved)?;
    execution::execute_resolved_action(&resolved, &input)
}

pub fn dispatch_query(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
    query_name: &str,
) -> Result<serde_json::Value, ActionExecutionError> {
    let socket_path = resolve_plugin_daemon_socket(plugin_manager, plugin_id)?;
    let initial_dispatch =
        crate::plugins::action_transport::dispatch_daemon_action(&socket_path, query_name);
    #[cfg(debug_assertions)]
    trace_query_dispatch(plugin_id, query_name, "initial", &initial_dispatch);
    if !matches!(&initial_dispatch, DaemonActionDispatch::Unavailable) {
        return query_dispatch_result(initial_dispatch, plugin_id, query_name);
    }

    let readiness = ensure_daemon_ready(
        plugin_manager,
        plugin_id,
        query_name,
        &socket_path,
        query_daemon_socket_ready,
    );
    #[cfg(debug_assertions)]
    if let Err(error) = &readiness {
        qol_runtime::probe!(
            "QUERY_DISPATCH",
            "plugin={} query={} attempt=recovery outcome=not_ready error={}",
            plugin_id,
            query_name,
            error
        );
    }
    readiness?;

    let retry_dispatch =
        crate::plugins::action_transport::dispatch_daemon_action(&socket_path, query_name);
    #[cfg(debug_assertions)]
    trace_query_dispatch(plugin_id, query_name, "retry", &retry_dispatch);
    query_dispatch_result(retry_dispatch, plugin_id, query_name)
}

fn query_dispatch_result(
    dispatch: DaemonActionDispatch,
    plugin_id: &str,
    query_name: &str,
) -> Result<serde_json::Value, ActionExecutionError> {
    match dispatch {
        DaemonActionDispatch::Handled { payload } => Ok(payload.unwrap_or(serde_json::Value::Null)),
        DaemonActionDispatch::Fallback => Err(ActionExecutionError::ActionRejected(format!(
            "query {query_name} rejected by {plugin_id} daemon"
        ))),
        DaemonActionDispatch::Error(message) => Err(ActionExecutionError::ActionRejected(message)),
        DaemonActionDispatch::Unavailable => Err(ActionExecutionError::ActionRejected(format!(
            "daemon unavailable for {plugin_id}"
        ))),
    }
}

#[cfg(debug_assertions)]
fn trace_query_dispatch(
    plugin_id: &str,
    query_name: &str,
    attempt: &str,
    dispatch: &DaemonActionDispatch,
) {
    qol_runtime::probe!(
        "QUERY_DISPATCH",
        "plugin={} query={} attempt={} outcome={}",
        plugin_id,
        query_name,
        attempt,
        dispatch_outcome(dispatch)
    );
}

#[cfg(debug_assertions)]
fn dispatch_outcome(dispatch: &DaemonActionDispatch) -> &'static str {
    match dispatch {
        DaemonActionDispatch::Handled { .. } => "handled",
        DaemonActionDispatch::Fallback => "fallback",
        DaemonActionDispatch::Unavailable => "unavailable",
        DaemonActionDispatch::Error(_) => "error",
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
    materialize_plugin_runtime_config(plugin)?;
    let socket = plugin
        .manifest
        .daemon
        .as_ref()
        .and_then(|daemon| daemon.socket.as_ref())
        .ok_or_else(|| ActionExecutionError::NoExecutionTarget {
            plugin_id: PluginId::new(plugin_id),
            action_id: "<query>".to_string(),
        })?;
    Ok(crate::dev_generation::daemon_socket_path(socket))
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
    materialize_plugin_runtime_config(plugin)?;
    resolution::resolve_action(plugin, action_id)
}

fn materialize_plugin_runtime_config(plugin: &Plugin) -> Result<(), ActionExecutionError> {
    crate::plugins::PluginConfigManager::new()
        .and_then(|manager| {
            manager
                .materialize_runtime_config_for_manifest(plugin.id.as_str(), &plugin.manifest)
                .map(|_| ())
        })
        .map_err(|error| {
            ActionExecutionError::SpawnFailed(format!(
                "failed to materialize runtime config for {}: {error:#}",
                plugin.id
            ))
        })
}

fn ensure_daemon_ready_for_action(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    resolved: &resolution::ResolvedAction,
) -> Result<(), ActionExecutionError> {
    let Some(socket_path) = resolved.daemon_socket.as_deref() else {
        return Ok(());
    };

    ensure_daemon_ready(
        plugin_manager,
        resolved.plugin_id.as_str(),
        &resolved.action_id,
        socket_path,
        daemon_socket_ready,
    )
}

fn ensure_daemon_ready(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
    request_id: &str,
    socket_path: &Path,
    readiness_probe: fn(&Path) -> bool,
) -> Result<(), ActionExecutionError> {
    if readiness_probe(socket_path) {
        return Ok(());
    }

    {
        let mut plugins = plugin_manager
            .lock()
            .map_err(|_| ActionExecutionError::PluginManagerPoisoned)?;
        plugins
            .ensure_plugin_daemon_running(plugin_id)
            .map_err(|error| ActionExecutionError::SpawnFailed(error.to_string()))?;
    }

    wait_for_daemon_socket(socket_path, readiness_probe)
        .then_some(())
        .ok_or_else(|| {
            ActionExecutionError::SpawnFailed(format!(
                "daemon socket did not become ready for {}::{}",
                plugin_id, request_id
            ))
        })
}

fn wait_for_daemon_socket(socket_path: &Path, readiness_probe: fn(&Path) -> bool) -> bool {
    let deadline = Instant::now() + DAEMON_READY_TIMEOUT;
    loop {
        if readiness_probe(socket_path) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(DAEMON_READY_INTERVAL);
    }
}

fn daemon_socket_ready(socket_path: &Path) -> bool {
    matches!(
        crate::plugins::action_transport::dispatch_daemon_action(socket_path, "ping"),
        DaemonActionDispatch::Handled { .. }
    )
}

fn query_daemon_socket_ready(socket_path: &Path) -> bool {
    matches!(
        crate::plugins::action_transport::dispatch_daemon_action_with_timeout(
            socket_path,
            "ping",
            QUERY_DAEMON_READY_PROBE_TIMEOUT,
        ),
        DaemonActionDispatch::Handled { .. }
    )
}
