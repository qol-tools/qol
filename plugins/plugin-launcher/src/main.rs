fn main() -> std::process::ExitCode {
    launcher::cli::exit_code(std::env::args().skip(1))
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::PluginManifest;

    qol_plugin_api::assert_plugin_toml_valid!();

    #[test]
    fn live_manifest_preserves_runtime_and_enables_doctor() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let runtime = manifest
            .runtime
            .as_ref()
            .expect("Launcher runtime must be declared");
        let daemon = manifest
            .daemon
            .as_ref()
            .expect("Launcher daemon must be declared");

        assert_eq!(runtime.command, "launcher");
        assert_eq!(
            manifest.catalog_runtime_args("open"),
            Some(vec!["--show".into()])
        );
        assert_eq!(
            manifest.catalog_runtime_args("settings"),
            Some(vec!["--settings".into()])
        );
        assert!(manifest.capabilities.gpui);
        assert!(manifest.capabilities.doctor);
        assert!(daemon.enabled);
        assert_eq!(daemon.command, "launcher");
        assert_eq!(daemon.socket.as_deref(), Some("/tmp/qol-launcher.sock"));
    }
}
