use std::process::ExitCode;

fn main() -> ExitCode {
    plugin_monitor::cli::exit_code(std::env::args().skip(1))
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::PluginManifest;

    qol_plugin_api::assert_plugin_toml_valid!();

    #[test]
    fn live_manifest_declares_the_headless_contract() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let runtime = manifest
            .runtime
            .as_ref()
            .expect("monitor runtime must be declared");

        assert_eq!(runtime.command, "plugin-monitor");
        assert_eq!(
            manifest.plugin.uid.as_ref().map(|uid| uid.as_str()),
            Some(plugin_monitor::hotkeys::PLUGIN_UID),
            "the doctor and the host hotkey config must agree on the tray-written uid"
        );
        assert!(manifest.capabilities.doctor);
        assert!(manifest.capabilities.gpui);
        assert_eq!(
            manifest.catalog_runtime_args("brightness-up"),
            Some(vec!["up".to_string()])
        );
        assert_eq!(
            manifest.catalog_runtime_args("brightness-down"),
            Some(vec!["down".to_string()])
        );
        assert_eq!(
            manifest.catalog_runtime_args("settings"),
            Some(vec!["settings".to_string()])
        );
        assert!(manifest.actions["brightness-up"].continuous);
        assert!(manifest.actions["brightness-down"].continuous);
        assert!(!manifest.actions["settings"].continuous);

        let daemon = manifest
            .daemon
            .as_ref()
            .expect("continuous actions require a daemon transport");
        assert!(daemon.enabled);
        assert_eq!(daemon.command, "plugin-monitor");
        assert!(daemon.socket.is_some());

        let mut expected = std::collections::BTreeSet::new();
        expected.insert("brightness-up".to_string());
        expected.insert("brightness-down".to_string());
        expected.insert("settings".to_string());
        assert_eq!(manifest.executable_action_ids(), expected);
    }
}
