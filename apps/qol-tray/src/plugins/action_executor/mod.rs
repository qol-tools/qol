use super::manager::PluginManager;
use crate::plugins::action_transport::DaemonActionDispatch;
use crate::plugins::PluginId;
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

pub(crate) use resolution::daemon_socket;
pub(crate) use tracking::ProcessTracker;

const DAEMON_READY_TIMEOUT: Duration = Duration::from_secs(3);
const DAEMON_READY_INTERVAL: Duration = Duration::from_millis(25);
const QUERY_DAEMON_READY_PROBE_TIMEOUT: Duration = Duration::from_millis(100);
/// Queries are reads that feed a settings row, so they get an interactive
/// budget instead of the action transport's 10s ceiling. A daemon that cannot
/// answer inside it is reported unavailable rather than stalling the panel.
const QUERY_DISPATCH_TIMEOUT: Duration = Duration::from_millis(750);

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
    if plugin_id == qol_conventions::CORE_PANEL_ID && action_id == "settings" {
        return match crate::settings_surface::request(plugin_id) {
            Ok(true) => {
                qol_runtime::probe!(
                    "SURFACE_ACTIVATION",
                    "plugin={plugin_id} phase=route outcome=hosted"
                );
                Ok(None)
            }
            Ok(false) => Err(ActionExecutionError::SpawnFailed(
                "the native settings host is unavailable on this platform".to_string(),
            )),
            Err(error) => Err(ActionExecutionError::SpawnFailed(format!("{error:#}"))),
        };
    }
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
    let tracker = process_tracker(plugin_manager)?;
    execution::execute_resolved_action(&tracker, &resolved, &input)
}

fn process_tracker(
    plugin_manager: &Arc<Mutex<PluginManager>>,
) -> Result<Arc<ProcessTracker>, ActionExecutionError> {
    plugin_manager
        .lock()
        .map(|manager| manager.process_tracker())
        .map_err(|_| ActionExecutionError::PluginManagerPoisoned)
}

pub fn dispatch_query(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
    query_name: &str,
) -> Result<serde_json::Value, ActionExecutionError> {
    let socket_path = resolve_plugin_daemon_socket(plugin_manager, plugin_id)?;
    let initial_dispatch = crate::plugins::action_transport::dispatch_daemon_action_with_timeout(
        &socket_path,
        query_name,
        QUERY_DISPATCH_TIMEOUT,
    );
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

    let retry_dispatch = crate::plugins::action_transport::dispatch_daemon_action_with_timeout(
        &socket_path,
        query_name,
        QUERY_DISPATCH_TIMEOUT,
    );
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
    let (identity, socket) = {
        let mut plugins = plugin_manager
            .lock()
            .map_err(|_| ActionExecutionError::PluginManagerPoisoned)?;
        plugins
            .reconcile_profile_generation()
            .map_err(|error| ActionExecutionError::SpawnFailed(error.to_string()))?;
        let plugin = plugins
            .get(plugin_id)
            .ok_or_else(|| ActionExecutionError::PluginNotFound(PluginId::new(plugin_id)))?;
        let socket = plugin
            .manifest
            .daemon
            .as_ref()
            .and_then(|daemon| daemon.socket.as_ref())
            .map(|socket| crate::dev_generation::daemon_socket_path(socket));
        (
            crate::plugins::config::manifest_identity(&plugin.manifest),
            socket,
        )
    };
    materialize_plugin_runtime_config(plugin_manager, plugin_id, &identity)?;
    socket.ok_or_else(|| ActionExecutionError::NoExecutionTarget {
        plugin_id: PluginId::new(plugin_id),
        action_id: "<query>".to_string(),
    })
}

fn resolve_plugin_action(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
    action_id: &str,
) -> Result<resolution::ResolvedAction, ActionExecutionError> {
    let (identity, resolved) = {
        let mut plugins = plugin_manager
            .lock()
            .map_err(|_| ActionExecutionError::PluginManagerPoisoned)?;
        plugins
            .reconcile_profile_generation()
            .map_err(|error| ActionExecutionError::SpawnFailed(error.to_string()))?;
        let plugin = plugins
            .get(plugin_id)
            .ok_or_else(|| ActionExecutionError::PluginNotFound(PluginId::new(plugin_id)))?;
        let resolved = resolution::resolve_action(plugin, action_id)?;
        (
            crate::plugins::config::manifest_identity(&plugin.manifest),
            resolved,
        )
    };
    materialize_plugin_runtime_config(plugin_manager, resolved.plugin_id.as_str(), &identity)?;
    Ok(resolved)
}

fn materialize_plugin_runtime_config(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
    identity: &crate::plugins::config::ManifestIdentity,
) -> Result<(), ActionExecutionError> {
    let manager = crate::plugins::PluginConfigManager::new().map_err(|error| {
        ActionExecutionError::SpawnFailed(format!(
            "failed to initialize runtime config for {}: {error:#}",
            plugin_id
        ))
    })?;
    #[cfg(debug_assertions)]
    let started = Instant::now();
    #[cfg(not(debug_assertions))]
    let started = ();
    let cache_status =
        crate::plugins::config::runtime_config_cache_status(&manager, plugin_id, identity);
    if !matches!(
        cache_status,
        crate::plugins::config::ActionCacheStatus::Miss
    ) {
        trace_runtime_cache_hit(plugin_id, started, cache_status);
        return Ok(());
    }
    let manifest = {
        let plugins = plugin_manager
            .lock()
            .map_err(|_| ActionExecutionError::PluginManagerPoisoned)?;
        plugins
            .get(plugin_id)
            .ok_or_else(|| ActionExecutionError::PluginNotFound(PluginId::new(plugin_id)))?
            .manifest
            .clone()
    };
    manager
        .ensure_runtime_config_for_manifest(plugin_id, &manifest)
        .map(drop)
        .map_err(|error| {
            ActionExecutionError::SpawnFailed(format!(
                "failed to materialize runtime config for {}: {error:#}",
                plugin_id
            ))
        })?;
    Ok(())
}

fn trace_runtime_cache_hit(
    plugin_id: &str,
    #[cfg(debug_assertions)] started: Instant,
    #[cfg(not(debug_assertions))] started: (),
    cache_status: crate::plugins::config::ActionCacheStatus,
) {
    let fields = runtime_cache_hit_fields(cache_status);
    #[cfg(debug_assertions)]
    {
        let (cache, last_known_good, mutation_scope) = fields;
        if !crate::plugins::config::should_sample_runtime_cache_hit() {
            return;
        }
        qol_runtime::probe!(
            "PROFILE_CONFIG_MATERIALIZE",
            "plugin={:?} cache={} last_known_good={} mutation_scope={} wait_us=0 timeout=false generation={} validation_us={}",
            plugin_id,
            cache,
            last_known_good,
            mutation_scope,
            crate::plugins::config::runtime_config_generation(),
            started.elapsed().as_micros()
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = (plugin_id, started, fields);
}

fn runtime_cache_hit_fields(
    cache_status: crate::plugins::config::ActionCacheStatus,
) -> (&'static str, bool, &'static str) {
    match cache_status {
        crate::plugins::config::ActionCacheStatus::Fresh => ("hit", false, "none"),
        crate::plugins::config::ActionCacheStatus::LastKnownGood { mutation_scope } => {
            ("last_known_good", true, mutation_scope)
        }
        crate::plugins::config::ActionCacheStatus::Miss => ("miss", false, "none"),
    }
}

fn ensure_daemon_ready_for_action(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    resolved: &resolution::ResolvedAction,
) -> Result<(), ActionExecutionError> {
    let Some(socket_path) = resolved.daemon_socket.as_deref() else {
        return Ok(());
    };
    #[cfg(debug_assertions)]
    let started = Instant::now();
    #[cfg(not(debug_assertions))]
    let started = ();
    trace_daemon_ready(
        "start",
        resolved.plugin_id.as_str(),
        &resolved.action_id,
        &started,
    );
    let result = ensure_daemon_ready(
        plugin_manager,
        resolved.plugin_id.as_str(),
        &resolved.action_id,
        socket_path,
        daemon_socket_ready,
    );
    trace_daemon_ready(
        "done",
        resolved.plugin_id.as_str(),
        &resolved.action_id,
        &started,
    );
    result
}

#[cfg(debug_assertions)]
fn trace_daemon_ready(phase: &str, plugin_id: &str, action_id: &str, started: &Instant) {
    qol_runtime::probe!(
        "ACTION_READY",
        "plugin={plugin_id} action={action_id} phase={phase} elapsed_ms={}",
        started.elapsed().as_millis()
    );
}

#[cfg(not(debug_assertions))]
fn trace_daemon_ready(phase: &str, plugin_id: &str, action_id: &str, started: &()) {
    let _ = (phase, plugin_id, action_id, started);
}

fn ensure_daemon_ready(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
    request_id: &str,
    socket_path: &Path,
    readiness_probe: fn(&Path) -> bool,
) -> Result<(), ActionExecutionError> {
    {
        let mut plugins = plugin_manager
            .lock()
            .map_err(|_| ActionExecutionError::PluginManagerPoisoned)?;
        plugins
            .ensure_plugin_daemon_running(plugin_id)
            .map_err(|error| ActionExecutionError::SpawnFailed(error.to_string()))?;
    }

    if readiness_probe(socket_path) {
        return Ok(());
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
