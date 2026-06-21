use qol_plugin_api::manifest::PluginManifest;

#[test]
fn manifest_prewarms_daemon_and_declares_actions() {
    let toml = include_str!("../plugin.toml");
    let m: PluginManifest = toml::from_str(toml).expect("parse plugin.toml");
    m.validate().expect("valid plugin.toml");

    assert_eq!(m.plugin.name, "CLI Sessions");
    assert_eq!(
        m.plugin.platforms.as_deref(),
        Some(["linux".to_string(), "macos".to_string()].as_slice())
    );

    let runtime = m.runtime.as_ref().expect("runtime");
    assert_eq!(runtime.command, "cli-sessions");
    assert!(runtime.actions.is_none());
    let actions = m.executable_action_ids();
    assert!(actions.contains("open"));
    assert!(
        actions.contains("next"),
        "the jump-to-next-attention hotkey resolves to a catalog action"
    );
    assert!(
        actions.contains("snapshot"),
        "the snapshot-all hotkey resolves to a catalog action"
    );
    assert_eq!(
        m.catalog_runtime_args("open"),
        Some(vec!["open".to_string()])
    );
    assert_eq!(
        m.catalog_runtime_args("next"),
        Some(vec!["next".to_string()])
    );
    assert_eq!(
        m.catalog_runtime_args("snapshot"),
        Some(vec!["snapshot".to_string()])
    );

    let daemon = m.daemon.as_ref().expect(
        "CLI Sessions pre-warms a hidden daemon so opening it is instant and focuses before \
         keystrokes can leak to the terminal underneath (Linux gpui cold-start race)",
    );
    assert!(daemon.enabled);
    assert_eq!(daemon.command, "cli-sessions");
    assert_eq!(daemon.socket.as_deref(), Some("/tmp/qol-cli-sessions.sock"));

    let shortcut_actions: Vec<&str> = m.shortcuts.iter().map(|s| s.action.as_str()).collect();
    assert_eq!(
        shortcut_actions,
        ["open", "next", "snapshot"],
        "open, next-alert, and snapshot-all are bindable shortcuts"
    );
    assert!(m.shortcuts.iter().all(|s| s.export_to_launcher));

    let bins = m.dependencies.expect("deps").binaries;
    assert_eq!(
        bins.len(),
        1,
        "single binary keeps store release discovery working"
    );
    assert_eq!(bins[0].name, "cli-sessions");

    assert!(m.capabilities.gpui);
}
