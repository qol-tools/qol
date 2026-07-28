mod app;
mod cli;
mod config;
mod cursor;
mod doctor;
mod settings;
mod theme;

fn main() -> std::process::ExitCode {
    cli::exit_code(std::env::args().skip(1))
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::PluginManifest;

    qol_plugin_api::assert_plugin_toml_valid!();

    #[test]
    fn manifest_preserves_runtime_and_enables_doctor() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");

        assert_eq!(
            manifest
                .runtime
                .as_ref()
                .map(|runtime| runtime.command.as_str()),
            Some("plugin-os-themes")
        );
        assert!(manifest.capabilities.gpui);
        assert!(manifest.capabilities.doctor);

        let daemon = manifest.daemon.as_ref().expect("daemon contract missing");
        assert!(daemon.enabled);
        assert_eq!(daemon.command, "plugin-os-themes");
        assert_eq!(daemon.socket.as_deref(), Some("/tmp/qol-os-themes.sock"));
    }
}
