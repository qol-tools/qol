use qol_plugin_api::manifest::PluginManifest;

#[test]
fn manifest_declares_on_demand_actions_and_one_binary() {
    let toml = include_str!("../plugin.toml");
    let m: PluginManifest = toml::from_str(toml).expect("parse plugin.toml");
    m.validate().expect("valid plugin.toml");

    assert_eq!(m.plugin.name, "CLI Sessions");
    assert_eq!(
        m.plugin.platforms.as_deref(),
        Some(["linux".to_string(), "macos".to_string()].as_slice())
    );

    let runtime = m.runtime.expect("runtime");
    assert_eq!(runtime.command, "cli-sessions");
    let actions = runtime.actions.expect("actions");
    assert!(actions.contains_key("open"));
    assert!(
        actions.contains_key("next"),
        "the jump-to-next-attention hotkey resolves to a runtime action"
    );

    assert!(
        m.daemon.is_none(),
        "CLI Sessions is opened on demand through runtime actions, not autostarted"
    );

    let shortcut_actions: Vec<&str> = m.shortcuts.iter().map(|s| s.action.as_str()).collect();
    assert_eq!(
        shortcut_actions,
        ["open", "next"],
        "both the open and next-alert actions are bindable shortcuts"
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
