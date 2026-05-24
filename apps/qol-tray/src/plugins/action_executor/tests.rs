use super::resolution::resolve_action;
use super::tracking::{
    action_processes_snapshot, clear_tracking, running_actions_snapshot, track_action_process,
    untrack_action_process,
};
use super::*;
use crate::plugins::manifest::{
    ActionType, BuildInfo, Capabilities, DaemonConfig, MenuConfig, MenuItem, PluginInfo,
    PluginManifest, RuntimeConfig, CURRENT_MANIFEST_VERSION,
};
use crate::plugins::{Plugin, PluginId};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use tempfile::TempDir;

fn with_process_tracking_lock<T>(run: impl FnOnce() -> T) -> T {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
    clear_tracking();
    let result = run();
    clear_tracking();
    result
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
        capabilities: Capabilities::default(),
        build: BuildInfo::default(),
        traits: None,
        config: Default::default(),
    };

    Plugin::new(
        PluginId::new("plugin-test"),
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
fn resolve_action_allows_runtime_fallback_when_daemon_and_runtime_share_binary_but_socket_unreachable(
) {
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
fn resolve_action_disables_runtime_fallback_when_daemon_and_runtime_share_binary_and_socket_reachable(
) {
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
    let err = try_execute_action(&manager, "missing", "open")
        .err()
        .unwrap();

    assert!(matches!(err, ActionExecutionError::PluginNotFound(_)));
}

#[test]
fn process_tracking_adds_and_removes_entries() {
    with_process_tracking_lock(|| {
        track_action_process("plugin-a", "open", 101);
        track_action_process("plugin-a", "close", 102);
        track_action_process("plugin-b", "open", 201);

        let tracked = action_processes_snapshot();
        assert_eq!(tracked.get("plugin-a"), Some(&vec![101, 102]));
        assert_eq!(tracked.get("plugin-b"), Some(&vec![201]));
        let tracked_running = running_actions_snapshot();
        assert_eq!(tracked_running.get("plugin-a::open"), Some(&101));
        assert_eq!(tracked_running.get("plugin-a::close"), Some(&102));

        untrack_action_process("plugin-a", "open", 101);
        let tracked = action_processes_snapshot();
        assert_eq!(tracked.get("plugin-a"), Some(&vec![102]));
        let tracked_running = running_actions_snapshot();
        assert!(!tracked_running.contains_key("plugin-a::open"));

        untrack_action_process("plugin-a", "close", 102);
        let tracked = action_processes_snapshot();
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

        let tracked = action_processes_snapshot();
        assert!(tracked.is_empty());
        let tracked_running = running_actions_snapshot();
        assert!(tracked_running.is_empty());
    });
}
