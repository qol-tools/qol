mod app;
mod cli;
mod config;
mod diagnostics;
mod doctor;
mod glide;
mod platform;
mod restore;

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
            .expect("Window Actions runtime must be declared");

        assert_eq!(runtime.command, "window-actions");
        assert!(manifest.capabilities.doctor);
        assert_eq!(
            manifest.catalog_runtime_args("snap-left"),
            Some(vec!["snap-left".to_string()])
        );
        assert_eq!(
            manifest.catalog_runtime_args("glide-left"),
            Some(vec!["glide-left".to_string()])
        );
        assert_eq!(
            manifest.catalog_runtime_args("settings"),
            Some(vec!["settings".to_string()])
        );
        assert!(manifest.capabilities.gpui);
    }
}
