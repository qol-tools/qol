mod cli;
mod daemon;
mod doctor;

use std::process::ExitCode;

fn main() -> ExitCode {
    cli::exit_code(std::env::args().skip(1))
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
            .expect("Task Runner runtime must be declared");

        assert_eq!(runtime.command, "task-runner");
        assert!(manifest.capabilities.doctor);
        assert_eq!(
            manifest.catalog_runtime_args("status"),
            Some(vec!["status".to_string()])
        );
    }

    #[test]
    fn live_manifest_declares_the_native_settings_action() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let settings = manifest
            .actions
            .get("settings")
            .expect("settings action must be declared");

        assert!(manifest.capabilities.gpui);
        assert_eq!(
            settings.kind,
            qol_plugin_api::manifest::ActionType::Settings
        );
        assert_eq!(settings.label, "Settings");
        assert_eq!(
            manifest.catalog_runtime_args("settings"),
            Some(vec!["settings".to_string()])
        );
    }
}
