use super::{manager::PluginManager, Plugin};
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

static ACTION_PROCESSES: OnceLock<Mutex<HashMap<String, Vec<u32>>>> = OnceLock::new();

struct ResolvedAction {
    plugin_id: String,
    action_id: String,
    plugin_dir: PathBuf,
    daemon_socket: Option<PathBuf>,
    command_path: Option<PathBuf>,
    args: Vec<String>,
}

#[derive(Debug)]
pub enum ActionExecutionError {
    PluginManagerPoisoned,
    PluginNotFound(String),
    InvalidActionId(String),
    RuntimeCommandEscapesPluginDir { plugin_id: String, command: String },
    MissingActionMapping { plugin_id: String, action_id: String },
    NoExecutionTarget { plugin_id: String, action_id: String },
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
            Self::MissingActionMapping {
                plugin_id,
                action_id,
            } => {
                write!(
                    f,
                    "missing action mapping for {}::{}",
                    plugin_id, action_id
                )
            }
            Self::NoExecutionTarget {
                plugin_id,
                action_id,
            } => {
                write!(f, "no execution target for {}::{}", plugin_id, action_id)
            }
            Self::SpawnFailed(error) => write!(f, "spawn failed: {}", error),
        }
    }
}

impl std::error::Error for ActionExecutionError {}

fn get_action_processes() -> &'static Mutex<HashMap<String, Vec<u32>>> {
    ACTION_PROCESSES.get_or_init(|| Mutex::new(HashMap::new()))
}

#[allow(dead_code)]
pub fn kill_plugin_processes(plugin_id: &str) {
    let mut processes = match get_action_processes().lock() {
        Ok(p) => p,
        Err(e) => {
            log::error!("Failed to lock action processes: {}", e);
            return;
        }
    };

    if let Some(pids) = processes.remove(plugin_id) {
        for pid in pids {
            kill_process(pid, plugin_id);
        }
    }
}

pub fn kill_all_plugin_processes() {
    let mut processes = match get_action_processes().lock() {
        Ok(p) => p,
        Err(e) => {
            log::error!("Failed to lock action processes: {}", e);
            return;
        }
    };

    for (plugin_id, pids) in processes.drain() {
        for pid in pids {
            kill_process(pid, &plugin_id);
        }
    }
}

#[cfg(unix)]
fn kill_process(pid: u32, plugin_id: &str) {
    unsafe {
        let pid = pid as i32;
        if libc::kill(pid, 0) == 0 {
            log::info!("Killing action process {} for plugin {}", pid, plugin_id);
            libc::kill(pid, libc::SIGTERM);
            std::thread::sleep(std::time::Duration::from_millis(100));
            if libc::kill(pid, 0) == 0 {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

#[cfg(not(unix))]
fn kill_process(_pid: u32, _plugin_id: &str) {}

fn track_action_process(plugin_id: &str, pid: u32) {
    let mut processes = match get_action_processes().lock() {
        Ok(p) => p,
        Err(_) => return,
    };
    processes
        .entry(plugin_id.to_string())
        .or_default()
        .push(pid);
}

pub fn execute_action(plugin_manager: &Arc<Mutex<PluginManager>>, plugin_id: &str, action_id: &str) {
    if let Err(error) = try_execute_action(plugin_manager, plugin_id, action_id) {
        log::warn!(
            "Plugin action execution failed for {}::{}: {}",
            plugin_id,
            action_id,
            error
        );
    }
}

pub fn try_execute_action(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
    action_id: &str,
) -> Result<(), ActionExecutionError> {
    let resolved = {
        let plugins = plugin_manager
            .lock()
            .map_err(|_| ActionExecutionError::PluginManagerPoisoned)?;

        let plugin = plugins
            .get(plugin_id)
            .ok_or_else(|| ActionExecutionError::PluginNotFound(plugin_id.to_string()))?;

        resolve_action(plugin, action_id)?
    };

    execute_resolved_action(&resolved)
}

fn resolve_action(plugin: &Plugin, action_id: &str) -> Result<ResolvedAction, ActionExecutionError> {
    if !crate::plugins::manifest::is_valid_action_id(action_id) {
        return Err(ActionExecutionError::InvalidActionId(action_id.to_string()));
    }

    let daemon_socket = plugin
        .manifest
        .daemon
        .as_ref()
        .and_then(|d| if d.enabled { d.socket.as_ref() } else { None })
        .map(PathBuf::from);

    let (command_path, args) = match plugin.manifest.runtime.as_ref() {
        Some(runtime) => {
            let command = std::path::Path::new(&runtime.command);
            let has_traversal = command.is_absolute()
                || command
                    .components()
                    .any(|c| c == std::path::Component::ParentDir);
            if has_traversal {
                return Err(ActionExecutionError::RuntimeCommandEscapesPluginDir {
                    plugin_id: plugin.id.clone(),
                    command: runtime.command.clone(),
                });
            }

            let command_path = plugin.path.join(command);
            let args = match &runtime.actions {
                Some(map) => map.get(action_id).cloned().ok_or_else(|| {
                    ActionExecutionError::MissingActionMapping {
                        plugin_id: plugin.id.clone(),
                        action_id: action_id.to_string(),
                    }
                })?,
                None => vec![action_id.to_string()],
            };
            (Some(command_path), args)
        }
        None => (None, vec![]),
    };

    if daemon_socket.is_none() && command_path.is_none() {
        return Err(ActionExecutionError::NoExecutionTarget {
            plugin_id: plugin.id.clone(),
            action_id: action_id.to_string(),
        });
    }

    Ok(ResolvedAction {
        plugin_id: plugin.id.clone(),
        action_id: action_id.to_string(),
        plugin_dir: plugin.path.clone(),
        daemon_socket,
        command_path,
        args,
    })
}

fn execute_resolved_action(resolved: &ResolvedAction) -> Result<(), ActionExecutionError> {
    if let Some(socket_path) = &resolved.daemon_socket {
        match super::action_transport::dispatch_daemon_action(socket_path, &resolved.action_id) {
            super::action_transport::DaemonActionDispatch::Handled => {
                log::info!(
                    "Plugin action handled via daemon socket: {}::{}",
                    resolved.plugin_id,
                    resolved.action_id
                );
                return Ok(());
            }
            super::action_transport::DaemonActionDispatch::Fallback => {
                log::info!(
                    "Daemon requested runtime fallback for {}::{}",
                    resolved.plugin_id,
                    resolved.action_id
                );
            }
            super::action_transport::DaemonActionDispatch::Unavailable => {
                log::debug!(
                    "Daemon socket unavailable for {}::{}",
                    resolved.plugin_id,
                    resolved.action_id
                );
            }
        }
    }

    let command_path =
        resolved
            .command_path
            .as_ref()
            .ok_or_else(|| ActionExecutionError::NoExecutionTarget {
                plugin_id: resolved.plugin_id.clone(),
                action_id: resolved.action_id.clone(),
            })?;

    log::info!(
        "Executing runtime fallback: {:?} {:?}",
        command_path,
        resolved.args
    );
    let child = std::process::Command::new(command_path)
        .args(&resolved.args)
        .current_dir(&resolved.plugin_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| ActionExecutionError::SpawnFailed(e.to_string()))?;

    let pid = child.id();
    track_action_process(&resolved.plugin_id, pid);
    log::info!("Plugin fallback action started (pid: {})", pid);
    Ok(())
}
