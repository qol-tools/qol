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
    RuntimeCommandNotFound { plugin_id: String, command: String },
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
            Self::RuntimeCommandNotFound { plugin_id, command } => write!(
                f,
                "runtime command not found for {}: {}",
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
    reap_zombie_children();

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

    reap_zombie_children();
}

#[cfg(unix)]
fn kill_process(pid: u32, plugin_id: &str) {
    unsafe {
        let pid = pid as i32;
        let mut status = 0;
        if libc::waitpid(pid, &mut status, libc::WNOHANG) == pid {
            return;
        }
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

#[cfg(unix)]
fn reap_zombie_children() {
    unsafe {
        loop {
            let mut status = 0;
            let reaped = libc::waitpid(-1, &mut status, libc::WNOHANG);
            if reaped <= 0 {
                break;
            }
        }
    }
}

#[cfg(not(unix))]
fn reap_zombie_children() {}

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

fn untrack_action_process(plugin_id: &str, pid: u32) {
    let mut processes = match get_action_processes().lock() {
        Ok(p) => p,
        Err(_) => return,
    };
    let remove_entry = if let Some(pids) = processes.get_mut(plugin_id) {
        pids.retain(|tracked| *tracked != pid);
        pids.is_empty()
    } else {
        false
    };
    if remove_entry {
        processes.remove(plugin_id);
    }
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

    let (command_path, args) = resolve_runtime_target(plugin, action_id)?;

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

fn resolve_runtime_target(
    plugin: &Plugin,
    action_id: &str,
) -> Result<(Option<PathBuf>, Vec<String>), ActionExecutionError> {
    let Some(runtime) = plugin.manifest.runtime.as_ref() else {
        return Ok((None, vec![]));
    };

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

    let command_path = super::resolve_plugin_command_path(&plugin.path, &runtime.command)
        .ok_or_else(|| ActionExecutionError::RuntimeCommandNotFound {
            plugin_id: plugin.id.clone(),
            command: runtime.command.clone(),
        })?;

    let args = match &runtime.actions {
        Some(map) => map
            .get(action_id)
            .cloned()
            .ok_or_else(|| ActionExecutionError::MissingActionMapping {
                plugin_id: plugin.id.clone(),
                action_id: action_id.to_string(),
            })?,
        None => vec![action_id.to_string()],
    };

    Ok((Some(command_path), args))
}

fn execute_resolved_action(resolved: &ResolvedAction) -> Result<(), ActionExecutionError> {
    if let Some(socket_path) = &resolved.daemon_socket {
        match super::action_transport::dispatch_daemon_action(socket_path, &resolved.action_id) {
            #[cfg(unix)]
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
            #[cfg(unix)]
            super::action_transport::DaemonActionDispatch::Error(message) => {
                log::warn!(
                    "Daemon returned error for {}::{}: {}",
                    resolved.plugin_id,
                    resolved.action_id,
                    message
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
    let mut child = std::process::Command::new(command_path)
        .args(&resolved.args)
        .current_dir(&resolved.plugin_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| ActionExecutionError::SpawnFailed(e.to_string()))?;

    let pid = child.id();
    track_action_process(&resolved.plugin_id, pid);
    let plugin_id = resolved.plugin_id.clone();
    std::thread::spawn(move || {
        let _ = child.wait();
        untrack_action_process(&plugin_id, pid);
    });
    log::info!("Plugin fallback action started (pid: {})", pid);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::manifest::{
        ActionType, DaemonConfig, MenuConfig, MenuItem, PluginInfo, PluginManifest,
        RuntimeConfig,
        CURRENT_MANIFEST_VERSION,
    };
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, OnceLock};
    use tempfile::TempDir;

    fn with_process_tracking_lock<T>(run: impl FnOnce() -> T) -> T {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _guard = LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        clear_tracked_processes();
        let result = run();
        clear_tracked_processes();
        result
    }

    fn clear_tracked_processes() {
        get_action_processes().lock().unwrap().clear();
    }

    fn tracked_processes() -> HashMap<String, Vec<u32>> {
        get_action_processes().lock().unwrap().clone()
    }

    fn make_plugin(
        dir: &TempDir,
        action_id: &str,
        runtime: Option<RuntimeConfig>,
        daemon: Option<DaemonConfig>,
    ) -> Plugin {
        let manifest = PluginManifest {
            manifest_version: CURRENT_MANIFEST_VERSION,
            plugin: PluginInfo {
                name: "Test".to_string(),
                description: "Test".to_string(),
                version: "1.0.0".to_string(),
                author: None,
                platforms: None,
            },
            menu: MenuConfig {
                label: "Test".to_string(),
                icon: None,
                items: vec![MenuItem::Action {
                    id: action_id.to_string(),
                    label: "Action".to_string(),
                    action: ActionType::Run,
                    config_key: None,
                }],
            },
            daemon,
            dependencies: None,
            runtime,
        };

        Plugin::new(
            "plugin-test".to_string(),
            manifest,
            dir.path().to_path_buf(),
        )
    }

    #[test]
    fn resolve_action_rejects_invalid_action_id() {
        let dir = TempDir::new().unwrap();
        let plugin = make_plugin(&dir, "open", None, None);

        let err = resolve_action(&plugin, "--bad").err().unwrap();
        assert!(matches!(err, ActionExecutionError::InvalidActionId(_)));
    }

    #[test]
    fn resolve_action_rejects_runtime_command_escape() {
        let dir = TempDir::new().unwrap();
        let plugin = make_plugin(
            &dir,
            "open",
            Some(RuntimeConfig {
                command: "../outside".to_string(),
                actions: None,
            }),
            None,
        );

        let err = resolve_action(&plugin, "open").err().unwrap();
        assert!(matches!(
            err,
            ActionExecutionError::RuntimeCommandEscapesPluginDir { .. }
        ));
    }

    #[test]
    fn resolve_action_rejects_missing_runtime_command_binary() {
        let dir = TempDir::new().unwrap();
        let plugin = make_plugin(
            &dir,
            "open",
            Some(RuntimeConfig {
                command: "launcher".to_string(),
                actions: None,
            }),
            None,
        );

        let err = resolve_action(&plugin, "open").err().unwrap();
        assert!(matches!(
            err,
            ActionExecutionError::RuntimeCommandNotFound { .. }
        ));
    }

    #[test]
    fn resolve_action_uses_passthrough_args_without_runtime_map() {
        let dir = TempDir::new().unwrap();
        let binary = dir.path().join("launcher");
        fs::write(&binary, "").unwrap();

        let plugin = make_plugin(
            &dir,
            "open",
            Some(RuntimeConfig {
                command: "launcher".to_string(),
                actions: None,
            }),
            None,
        );

        let resolved = resolve_action(&plugin, "open").unwrap();
        assert_eq!(resolved.command_path, Some(binary));
        assert_eq!(resolved.args, vec!["open".to_string()]);
    }

    #[test]
    fn resolve_action_rejects_missing_runtime_action_mapping() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("launcher"), "").unwrap();

        let mut actions = HashMap::new();
        actions.insert("other".to_string(), vec!["show".to_string()]);

        let plugin = make_plugin(
            &dir,
            "open",
            Some(RuntimeConfig {
                command: "launcher".to_string(),
                actions: Some(actions),
            }),
            None,
        );

        let err = resolve_action(&plugin, "open").err().unwrap();
        assert!(matches!(
            err,
            ActionExecutionError::MissingActionMapping { .. }
        ));
    }

    #[test]
    fn resolve_action_accepts_daemon_only_target() {
        let dir = TempDir::new().unwrap();
        let plugin = make_plugin(
            &dir,
            "open",
            None,
            Some(DaemonConfig {
                enabled: true,
                command: "daemon".to_string(),
                socket: Some("/tmp/qol-test.sock".to_string()),
            }),
        );

        let resolved = resolve_action(&plugin, "open").unwrap();
        assert!(resolved.command_path.is_none());
        assert_eq!(
            resolved.daemon_socket,
            Some(PathBuf::from("/tmp/qol-test.sock"))
        );
        assert!(resolved.args.is_empty());
    }

    #[test]
    fn try_execute_action_returns_plugin_not_found() {
        let manager = Arc::new(Mutex::new(PluginManager::new()));
        let err = try_execute_action(&manager, "missing", "open").err().unwrap();

        assert!(matches!(err, ActionExecutionError::PluginNotFound(_)));
    }

    #[test]
    fn process_tracking_adds_and_removes_entries() {
        with_process_tracking_lock(|| {
            track_action_process("plugin-a", 101);
            track_action_process("plugin-a", 102);
            track_action_process("plugin-b", 201);

            let tracked = tracked_processes();
            assert_eq!(tracked.get("plugin-a"), Some(&vec![101, 102]));
            assert_eq!(tracked.get("plugin-b"), Some(&vec![201]));

            untrack_action_process("plugin-a", 101);
            let tracked = tracked_processes();
            assert_eq!(tracked.get("plugin-a"), Some(&vec![102]));

            untrack_action_process("plugin-a", 102);
            let tracked = tracked_processes();
            assert!(!tracked.contains_key("plugin-a"));
            assert_eq!(tracked.get("plugin-b"), Some(&vec![201]));
        });
    }

    #[test]
    fn kill_all_plugin_processes_clears_tracking_map() {
        with_process_tracking_lock(|| {
            track_action_process("plugin-a", 999_001);
            track_action_process("plugin-b", 999_002);
            kill_all_plugin_processes();

            let tracked = tracked_processes();
            assert!(tracked.is_empty());
        });
    }
}
