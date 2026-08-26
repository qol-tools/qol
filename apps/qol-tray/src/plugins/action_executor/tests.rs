use super::resolution::resolve_action;
use super::*;
use crate::plugins::manifest::{
    ActionDeclaration, ActionType, BuildInfo, Capabilities, DaemonConfig, MenuConfig, MenuItem,
    PluginInfo, PluginManifest, RuntimeConfig, CURRENT_MANIFEST_VERSION,
};
use crate::plugins::{Plugin, PluginId};
use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

fn make_plugin(
    dir: &TempDir,
    action_id: &str,
    runtime: Option<RuntimeConfig>,
    daemon: Option<DaemonConfig>,
) -> Plugin {
    let manifest = PluginManifest {
        manifest_version: CURRENT_MANIFEST_VERSION,
        plugin: PluginInfo {
            id: Some("test-plugin".into()),
            uid: None,
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
        actions: Default::default(),
        capabilities: Capabilities::default(),
        build: BuildInfo::default(),
        traits: None,
        shortcuts: Vec::new(),
        config: Default::default(),
    };

    Plugin::new(
        PluginId::new("plugin-test"),
        manifest,
        dir.path().to_path_buf(),
    )
}

fn make_catalog_plugin(
    dir: &TempDir,
    action_id: &str,
    args: Vec<String>,
    runtime: Option<RuntimeConfig>,
) -> Plugin {
    let mut plugin = make_plugin(dir, action_id, runtime, None);
    plugin.manifest.actions.insert(
        action_id.to_string(),
        ActionDeclaration {
            label: "Action".to_string(),
            kind: ActionType::Run,
            continuous: false,
            args: Some(args),
            config_key: None,
            checked: false,
        },
    );
    plugin
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
fn resolve_action_routes_to_daemon_when_runtime_binary_is_missing() {
    let dir = TempDir::new().unwrap();
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
            port: None,
            extra_ports: Vec::new(),
        }),
    );

    let resolved = resolve_action(&plugin, "open").unwrap();
    assert!(
        resolved.command_path.is_none(),
        "missing binary must not block daemon-served actions"
    );
    assert_eq!(
        resolved.daemon_socket,
        Some(crate::dev_generation::daemon_socket_path(
            "/tmp/qol-test.sock"
        ))
    );
    assert!(!resolved.runtime_fallback_allowed);
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
fn resolve_action_uses_catalog_args_without_runtime_action_map() {
    let dir = TempDir::new().unwrap();
    let binary = dir.path().join("launcher");
    fs::write(&binary, "").unwrap();

    let plugin = make_catalog_plugin(
        &dir,
        "open",
        vec!["show".to_string(), "--foreground".to_string()],
        Some(RuntimeConfig {
            command: "launcher".to_string(),
            actions: None,
        }),
    );

    let resolved = resolve_action(&plugin, "open").unwrap();

    assert_eq!(resolved.command_path, Some(binary));
    assert_eq!(resolved.args, vec!["show", "--foreground"]);
}

#[test]
fn resolve_action_hosts_only_contract_driven_gpui_settings() {
    let cases = [
        (true, ActionType::Settings, true, true),
        (false, ActionType::Settings, true, false),
        (true, ActionType::Run, true, false),
        (true, ActionType::Settings, false, false),
    ];
    for (gpui, kind, has_contract, expected) in cases {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("launcher"), "").unwrap();
        if has_contract {
            fs::write(dir.path().join("qol-config.toml"), "schema_version = 1").unwrap();
        }
        let mut plugin = make_catalog_plugin(
            &dir,
            "settings",
            vec!["settings".to_string()],
            Some(RuntimeConfig {
                command: "launcher".to_string(),
                actions: None,
            }),
        );
        plugin.manifest.capabilities.gpui = gpui;
        plugin.manifest.actions.get_mut("settings").unwrap().kind = kind;

        let resolved = resolve_action(&plugin, "settings").unwrap();
        assert_eq!(
            resolved.hosted_settings, expected,
            "gpui={gpui} kind={kind:?} has_contract={has_contract}"
        );
    }
}

#[test]
fn hosted_settings_contract_is_a_complete_execution_target() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("qol-config.toml"), "schema_version = 1").unwrap();
    let mut plugin = make_catalog_plugin(&dir, "settings", vec![], None);
    plugin.manifest.capabilities.gpui = true;
    plugin.manifest.actions.get_mut("settings").unwrap().kind = ActionType::Settings;

    let resolved = resolve_action(&plugin, "settings").unwrap();
    assert!(resolved.hosted_settings);
    assert!(resolved.command_path.is_none());
    assert!(resolved.daemon_socket.is_none());
}

#[test]
fn resolve_action_rejects_non_catalog_runtime_action_when_catalog_exists() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("launcher"), "").unwrap();

    let mut actions = HashMap::new();
    actions.insert("hidden".to_string(), vec!["hidden".to_string()]);

    let plugin = make_catalog_plugin(
        &dir,
        "open",
        vec!["open".to_string()],
        Some(RuntimeConfig {
            command: "launcher".to_string(),
            actions: Some(actions),
        }),
    );

    let err = resolve_action(&plugin, "hidden").err().unwrap();
    assert!(matches!(
        err,
        ActionExecutionError::MissingActionMapping { .. }
    ));
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
            port: None,
            extra_ports: Vec::new(),
        }),
    );

    let resolved = resolve_action(&plugin, "open").unwrap();
    assert!(resolved.command_path.is_none());
    assert_eq!(
        resolved.daemon_socket,
        Some(crate::dev_generation::daemon_socket_path(
            "/tmp/qol-test.sock"
        ))
    );
    assert!(resolved.args.is_empty());
}

#[test]
fn resolve_action_disables_runtime_fallback_when_daemon_and_runtime_share_binary_even_if_socket_unreachable(
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
            port: None,
            extra_ports: Vec::new(),
        }),
    );

    let resolved = resolve_action(&plugin, "open").unwrap();
    assert!(!resolved.runtime_fallback_allowed);
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
            port: None,
            extra_ports: Vec::new(),
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
            port: None,
            extra_ports: Vec::new(),
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
    let tracker = ProcessTracker::default();
    tracker.track_action_process("plugin-a", "open", 101);
    tracker.track_action_process("plugin-a", "close", 102);
    tracker.track_action_process("plugin-b", "open", 201);

    let tracked = tracker.action_processes_snapshot();
    assert_eq!(tracked.get("plugin-a"), Some(&vec![101, 102]));
    assert_eq!(tracked.get("plugin-b"), Some(&vec![201]));
    let tracked_running = tracker.running_actions_snapshot();
    assert_eq!(tracked_running.get("plugin-a::open"), Some(&101));
    assert_eq!(tracked_running.get("plugin-a::close"), Some(&102));

    tracker.untrack_action_process("plugin-a", "open", 101);
    let tracked = tracker.action_processes_snapshot();
    assert_eq!(tracked.get("plugin-a"), Some(&vec![102]));
    let tracked_running = tracker.running_actions_snapshot();
    assert!(!tracked_running.contains_key("plugin-a::open"));

    tracker.untrack_action_process("plugin-a", "close", 102);
    let tracked = tracker.action_processes_snapshot();
    assert!(!tracked.contains_key("plugin-a"));
    assert_eq!(tracked.get("plugin-b"), Some(&vec![201]));
}

#[test]
fn a_second_manager_shutdown_leaves_this_managers_tracking_intact() {
    let root = TempDir::new().unwrap();
    let _guard = crate::paths::push_test_path_root(root.path());
    let tracker = PluginManager::new().process_tracker();
    tracker.track_action_process("plugin-a", "open", 101);

    PluginManager::new().shutdown();

    let tracked = tracker.action_processes_snapshot();
    assert_eq!(
        tracked.get("plugin-a"),
        Some(&vec![101]),
        "one manager's shutdown must not drain another manager's action processes",
    );
}

#[test]
fn kill_all_plugin_processes_clears_tracking_map() {
    let tracker = ProcessTracker::default();
    tracker.track_action_process("plugin-a", "open", 999_001);
    tracker.track_action_process("plugin-b", "open", 999_002);
    tracker.kill_all_plugin_processes();

    let tracked = tracker.action_processes_snapshot();
    assert!(tracked.is_empty());
    let tracked_running = tracker.running_actions_snapshot();
    assert!(tracked_running.is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn plugin_cleanup_does_not_reap_an_untracked_child() {
    let tracker = ProcessTracker::default();
    let mut child = std::process::Command::new("sh")
        .args(["-c", "exit 0"])
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while !qol_process::is_pid_zombie(child.id()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(qol_process::is_pid_zombie(child.id()));

    tracker.kill_all_plugin_processes();

    assert!(child.wait().unwrap().success());
}

#[test]
fn kill_plugin_processes_preserves_other_plugins_tracking() {
    let tracker = ProcessTracker::default();
    tracker.track_action_process("plugin-a", "open", 999_001);
    tracker.track_action_process("plugin-a", "close", 999_002);
    tracker.track_action_process("plugin-b", "open", 999_003);

    tracker.kill_plugin_processes("plugin-a");

    let tracked = tracker.action_processes_snapshot();
    assert!(!tracked.contains_key("plugin-a"));
    assert_eq!(tracked.get("plugin-b"), Some(&vec![999_003]));
    let tracked_running = tracker.running_actions_snapshot();
    assert!(!tracked_running.contains_key("plugin-a::open"));
    assert!(!tracked_running.contains_key("plugin-a::close"));
    assert_eq!(tracked_running.get("plugin-b::open"), Some(&999_003));
}

#[test]
fn on_demand_readiness_cannot_bypass_shutdown_reconciliation() {
    let plugin_manager = Arc::new(Mutex::new(PluginManager::new()));
    plugin_manager.lock().unwrap().shutdown();

    let error = ensure_daemon_ready(
        &plugin_manager,
        "plugin-shutdown",
        "action",
        std::path::Path::new("/tmp/qol-test-daemon.sock"),
        |_| true,
    )
    .expect_err("readiness must not bypass the lifecycle cancellation gate");
    assert!(error.to_string().contains("shutting down"));
}

#[test]
fn repeated_daemon_action_resolution_does_not_read_profile_state() {
    let dir = TempDir::new().unwrap();
    let plugin = make_plugin(
        &dir,
        "open",
        None,
        Some(DaemonConfig {
            enabled: true,
            command: "daemon".to_string(),
            socket: Some("/tmp/qol-test.sock".to_string()),
            port: None,
            extra_ports: Vec::new(),
        }),
    );
    let mut manager = PluginManager::new();
    manager.insert_plugin_for_test(plugin);
    let plugin_manager = Arc::new(Mutex::new(manager));

    crate::plugins::config::reset_profile_config_read_count();
    for _ in 0..5 {
        let resolved = resolve_plugin_action(&plugin_manager, "plugin-test", "open").unwrap();
        assert!(resolved.daemon_socket.is_some());
    }
    assert_eq!(crate::plugins::config::profile_config_read_count(), 0);
}

#[cfg(unix)]
#[test]
fn repeated_daemon_action_readiness_does_not_read_profile_state() {
    use std::os::unix::fs::PermissionsExt;

    let path_root = TempDir::new().unwrap();
    let _path = crate::paths::push_test_path_root(path_root.path());
    let plugin_dir = TempDir::new().unwrap();
    let daemon = plugin_dir.path().join("daemon");
    fs::write(&daemon, "#!/bin/sh\nexec sleep 30\n").unwrap();
    fs::set_permissions(&daemon, fs::Permissions::from_mode(0o755)).unwrap();
    let mut plugin = make_plugin(
        &plugin_dir,
        "open",
        None,
        Some(DaemonConfig {
            enabled: true,
            command: "daemon".to_string(),
            socket: Some("/tmp/qol-test.sock".to_string()),
            port: None,
            extra_ports: Vec::new(),
        }),
    );
    plugin.start_daemon().unwrap();

    let mut manager = PluginManager::new();
    manager.insert_plugin_for_test(plugin);
    let plugin_manager = Arc::new(Mutex::new(manager));
    crate::plugins::config::reset_profile_config_read_count();

    for _ in 0..5 {
        let resolved = resolve_plugin_action(&plugin_manager, "plugin-test", "open").unwrap();
        ensure_daemon_ready(
            &plugin_manager,
            "plugin-test",
            "open",
            resolved.daemon_socket.as_deref().unwrap(),
            |_| true,
        )
        .unwrap();
    }

    assert_eq!(crate::plugins::config::profile_config_read_count(), 0);
    plugin_manager.lock().unwrap().shutdown();
}

/// Queries feed settings rows, so they must fail fast rather than inherit the
/// action transport's ceiling. Without this bound, one wedged plugin daemon
/// stalls the settings rail for seconds per query.
#[test]
fn query_dispatch_uses_an_interactive_budget_not_the_action_ceiling() {
    let action_ceiling = crate::plugins::action_transport::default_io_timeout();
    assert!(
        super::QUERY_DISPATCH_TIMEOUT < action_ceiling,
        "query budget {:?} must be tighter than the action ceiling {action_ceiling:?}",
        super::QUERY_DISPATCH_TIMEOUT
    );
    assert!(
        super::QUERY_DISPATCH_TIMEOUT <= std::time::Duration::from_millis(1000),
        "a settings row read must stay interactive, got {:?}",
        super::QUERY_DISPATCH_TIMEOUT
    );
}
