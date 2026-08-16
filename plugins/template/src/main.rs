use std::process::ExitCode;

fn main() -> ExitCode {
    plugin_template::cli::exit_code(std::env::args().skip(1))
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::PluginManifest;

    qol_plugin_api::assert_plugin_toml_valid!();

    #[test]
    fn live_template_manifest_declares_the_headless_contract() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let runtime = manifest
            .runtime
            .as_ref()
            .expect("template runtime must be declared");

        assert_eq!(runtime.command, "plugin-template");
        assert!(manifest.capabilities.doctor);
        assert_eq!(
            manifest.catalog_runtime_args("run"),
            Some(vec!["run".to_string()])
        );
        assert_eq!(
            manifest.catalog_runtime_args("settings"),
            Some(vec!["settings".to_string()])
        );
    }
}
