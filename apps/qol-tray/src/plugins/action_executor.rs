use super::{manager::PluginManager, Plugin};
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

static ACTION_PROCESSES: OnceLock<Mutex<HashMap<String, Vec<u32>>>> = OnceLock::new();
static RUNNING_ACTIONS: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();

struct ResolvedAction {
    plugin_id: String,
    action_id: String,
    plugin_dir: PathBuf,
    daemon_socket: Option<PathBuf>,
    command_path: Option<PathBuf>,
    args: Vec<String>,
    runtime_fallback_allowed: bool,
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

fn get_running_actions() -> &'static Mutex<HashMap<String, u32>> {
    RUNNING_ACTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn action_key(plugin_id: &str, action_id: &str) -> String {
    format!("{plugin_id}::{action_id}")
}

pub fn kill_all_plugin_processes() {
    crate::process_utils::reap_children_nonblocking();

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
    if let Ok(mut running) = get_running_actions().lock() {
        running.clear();
    }

    crate::process_utils::reap_children_nonblocking();
}

fn kill_process(pid: u32, plugin_id: &str) {
    let pid = pid as i32;
    if crate::process_utils::is_pid_alive(pid) {
        log::info!("Killing action process {} for plugin {}", pid, plugin_id);
        crate::process_utils::terminate_pid(pid, std::time::Duration::from_millis(100));
    }
}

fn reserve_runtime_spawn(plugin_id: &str, action_id: &str) -> bool {
    let key = action_key(plugin_id, action_id);
    let mut running = match get_running_actions().lock() {
        Ok(guard) => guard,
        Err(_) => return false,
    };

    let Some(existing_pid) = running.get(&key).copied() else {
        running.insert(key, 0);
        return true;
    };
    if existing_pid == 0 {
        log::info!(
            "Plugin action spawn already in progress, skipping: {}::{}",
            plugin_id,
            action_id
        );
        return false;
    }
    if crate::process_utils::is_pid_alive(existing_pid as i32) {
        log::info!(
            "Plugin action already running, skipping spawn: {}::{} (pid: {})",
            plugin_id,
            action_id,
            existing_pid
        );
        return false;
    }

    running.insert(key, 0);
    true
}

fn clear_runtime_spawn_reservation(plugin_id: &str, action_id: &str) {
    let key = action_key(plugin_id, action_id);
    if let Ok(mut running) = get_running_actions().lock() {
        if running.get(&key).copied() == Some(0) {
            running.remove(&key);
        }
    }
}

fn track_action_process(plugin_id: &str, action_id: &str, pid: u32) {
    let mut processes = match get_action_processes().lock() {
        Ok(p) => p,
        Err(_) => return,
    };
    processes
        .entry(plugin_id.to_string())
        .or_default()
        .push(pid);

    if let Ok(mut running) = get_running_actions().lock() {
        running.insert(action_key(plugin_id, action_id), pid);
    }
}

fn untrack_action_process(plugin_id: &str, action_id: &str, pid: u32) {
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

    let key = action_key(plugin_id, action_id);
    if let Ok(mut running) = get_running_actions().lock() {
        if running.get(&key).copied() == Some(pid) {
            running.remove(&key);
        }
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
    let runtime_fallback_allowed = allow_runtime_fallback(plugin, daemon_socket.as_ref(), command_path.as_ref());

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
        runtime_fallback_allowed,
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
    let Some(daemon) = plugin.manifest.daemon.as_ref().filter(|daemon| daemon.enabled) else {
        return true;
    };
    let Some(daemon_command_path) = super::resolve_plugin_command_path(&plugin.path, &daemon.command) else {
        return true;
    };

    if !paths_match(runtime_command_path, &daemon_command_path) {
        return true;
    }

    daemon_socket.is_some_and(|socket_path| !is_daemon_socket_reachable(socket_path))
}

fn paths_match(left: &std::path::Path, right: &std::path::Path) -> bool {
    let left = std::fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = std::fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    left == right
}

#[cfg(unix)]
fn is_daemon_socket_reachable(socket_path: &std::path::Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket_path).is_ok()
}

#[cfg(not(unix))]
fn is_daemon_socket_reachable(_socket_path: &std::path::Path) -> bool {
    false
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
        return execute_via_daemon(resolved, socket_path);
    }

    execute_via_runtime(resolved)
}

fn execute_via_daemon(
    resolved: &ResolvedAction,
    socket_path: &std::path::Path,
) -> Result<(), ActionExecutionError> {
    match super::action_transport::dispatch_daemon_action(socket_path, &resolved.action_id) {
        super::action_transport::DaemonActionDispatch::Handled => {
            log::info!(
                "Plugin action handled via daemon: {}::{}",
                resolved.plugin_id,
                resolved.action_id
            );
            Ok(())
        }
        super::action_transport::DaemonActionDispatch::Fallback => {
            log::warn!(
                "Daemon rejected action {}::{} with fallback",
                resolved.plugin_id,
                resolved.action_id
            );
            if resolved.runtime_fallback_allowed {
                return execute_via_runtime(resolved);
            }
            Err(ActionExecutionError::SpawnFailed(format!(
                "daemon rejected action {}::{}",
                resolved.plugin_id, resolved.action_id
            )))
        }
        super::action_transport::DaemonActionDispatch::Unavailable => {
            log::warn!(
                "Daemon unavailable for {}::{}",
                resolved.plugin_id,
                resolved.action_id
            );
            if resolved.runtime_fallback_allowed {
                return execute_via_runtime(resolved);
            }
            Err(ActionExecutionError::SpawnFailed(format!(
                "daemon unavailable for {}::{}",
                resolved.plugin_id, resolved.action_id
            )))
        }
        super::action_transport::DaemonActionDispatch::Error(message) => {
            log::warn!(
                "Daemon error for {}::{}: {}",
                resolved.plugin_id,
                resolved.action_id,
                message
            );
            Err(ActionExecutionError::SpawnFailed(format!(
                "daemon error for {}::{}: {}",
                resolved.plugin_id, resolved.action_id, message
            )))
        }
    }
}

fn execute_via_runtime(resolved: &ResolvedAction) -> Result<(), ActionExecutionError> {
    let command_path =
        resolved
            .command_path
            .as_ref()
            .ok_or_else(|| ActionExecutionError::NoExecutionTarget {
                plugin_id: resolved.plugin_id.clone(),
                action_id: resolved.action_id.clone(),
            })?;

    if !reserve_runtime_spawn(&resolved.plugin_id, &resolved.action_id) {
        return Ok(());
    }

    log::info!(
        "Executing runtime action: {:?} {:?}",
        command_path,
        resolved.args
    );
    let mut command = std::process::Command::new(command_path);
    command
        .args(&resolved.args)
        .current_dir(&resolved.plugin_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(socket_path) = &resolved.daemon_socket {
        command.env("QOL_TRAY_DAEMON_SOCKET", socket_path);
    }
    let mut child = command.spawn().map_err(|e| {
        clear_runtime_spawn_reservation(&resolved.plugin_id, &resolved.action_id);
        ActionExecutionError::SpawnFailed(e.to_string())
    })?;

    let pid = child.id();
    track_action_process(&resolved.plugin_id, &resolved.action_id, pid);
    let plugin_id = resolved.plugin_id.clone();
    let action_id = resolved.action_id.clone();
    std::thread::spawn(move || {
        let _ = child.wait();
        untrack_action_process(&plugin_id, &action_id, pid);
    });
    log::info!("Runtime action started (pid: {})", pid);
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
        get_running_actions().lock().unwrap().clear();
    }

    fn tracked_processes() -> HashMap<String, Vec<u32>> {
        get_action_processes().lock().unwrap().clone()
    }

    fn tracked_running_actions() -> HashMap<String, u32> {
        get_running_actions().lock().unwrap().clone()
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
    fn resolve_action_allows_runtime_fallback_when_daemon_and_runtime_share_binary_but_socket_unreachable() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("launcher"), "").unwrap();

        let plugin = make_plugin(
            &dir,
            "open",
            Some(RuntimeConfig {
                command: "launcher".to_string(),
                actions: None,
            }),
            Some(DaemonConfig {
                enabled: true,
                command: "launcher".to_string(),
                socket: Some("/tmp/qol-test.sock".to_string()),
            }),
        );

        let resolved = resolve_action(&plugin, "open").unwrap();
        assert!(resolved.runtime_fallback_allowed);
    }

    #[cfg(unix)]
    #[test]
    fn resolve_action_disables_runtime_fallback_when_daemon_and_runtime_share_binary_and_socket_reachable() {
        use std::os::unix::net::UnixListener;

        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("launcher"), "").unwrap();
        let socket_path = dir.path().join("daemon.sock");
        let _listener = UnixListener::bind(&socket_path).unwrap();

        let plugin = make_plugin(
            &dir,
            "open",
            Some(RuntimeConfig {
                command: "launcher".to_string(),
                actions: None,
            }),
            Some(DaemonConfig {
                enabled: true,
                command: "launcher".to_string(),
                socket: Some(socket_path.to_string_lossy().to_string()),
            }),
        );

        let resolved = resolve_action(&plugin, "open").unwrap();
        assert!(!resolved.runtime_fallback_allowed);
    }

    #[test]
    fn resolve_action_keeps_runtime_fallback_when_daemon_and_runtime_differ() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("launcher"), "").unwrap();
        fs::write(dir.path().join("launcher-cli"), "").unwrap();

        let plugin = make_plugin(
            &dir,
            "open",
            Some(RuntimeConfig {
                command: "launcher-cli".to_string(),
                actions: None,
            }),
            Some(DaemonConfig {
                enabled: true,
                command: "launcher".to_string(),
                socket: Some("/tmp/qol-test.sock".to_string()),
            }),
        );

        let resolved = resolve_action(&plugin, "open").unwrap();
        assert!(resolved.runtime_fallback_allowed);
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
            track_action_process("plugin-a", "open", 101);
            track_action_process("plugin-a", "close", 102);
            track_action_process("plugin-b", "open", 201);

            let tracked = tracked_processes();
            assert_eq!(tracked.get("plugin-a"), Some(&vec![101, 102]));
            assert_eq!(tracked.get("plugin-b"), Some(&vec![201]));
            let tracked_running = tracked_running_actions();
            assert_eq!(
                tracked_running.get("plugin-a::open"),
                Some(&101)
            );
            assert_eq!(
                tracked_running.get("plugin-a::close"),
                Some(&102)
            );

            untrack_action_process("plugin-a", "open", 101);
            let tracked = tracked_processes();
            assert_eq!(tracked.get("plugin-a"), Some(&vec![102]));
            let tracked_running = tracked_running_actions();
            assert!(!tracked_running.contains_key("plugin-a::open"));

            untrack_action_process("plugin-a", "close", 102);
            let tracked = tracked_processes();
            assert!(!tracked.contains_key("plugin-a"));
            assert_eq!(tracked.get("plugin-b"), Some(&vec![201]));
        });
    }

    #[test]
    fn kill_all_plugin_processes_clears_tracking_map() {
        with_process_tracking_lock(|| {
            track_action_process("plugin-a", "open", 999_001);
            track_action_process("plugin-b", "open", 999_002);
            kill_all_plugin_processes();

            let tracked = tracked_processes();
            assert!(tracked.is_empty());
            let tracked_running = tracked_running_actions();
            assert!(tracked_running.is_empty());
        });
    }
}
