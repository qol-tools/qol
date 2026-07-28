use std::process::ExitCode;

fn main() -> ExitCode {
    plugin_cli_sessions::cli::exit_code(std::env::args().skip(1))
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::PluginManifest;

    qol_plugin_api::assert_plugin_toml_valid!();

    #[test]
    fn live_manifest_declares_the_headless_doctor_contract() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let runtime = manifest
            .runtime
            .as_ref()
            .expect("CLI Sessions runtime must be declared");

        assert_eq!(runtime.command, "cli-sessions");
        assert!(manifest.capabilities.doctor);
        for (action, command) in [("open", "open"), ("next", "next"), ("snapshot", "snapshot")] {
            assert_eq!(
                manifest.catalog_runtime_args(action),
                Some(vec![command.to_string()])
            );
        }
    }
}
