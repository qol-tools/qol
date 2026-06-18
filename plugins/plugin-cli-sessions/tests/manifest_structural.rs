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
        m.daemon.is_none(),
        "CLI Sessions is opened on demand through runtime actions, not autostarted"
    );

    assert_eq!(m.shortcuts.len(), 1);
    assert_eq!(m.shortcuts[0].id, "open");
    assert_eq!(m.shortcuts[0].name, "CLI Sessions");
    assert_eq!(m.shortcuts[0].action, "open");
    assert!(m.shortcuts[0].export_to_launcher);

    let bins = m.dependencies.expect("deps").binaries;
    assert_eq!(
        bins.len(),
        1,
        "single binary keeps store release discovery working"
    );
    assert_eq!(bins[0].name, "cli-sessions");

    assert!(m.capabilities.gpui);
}
