use std::env;
use std::process::ExitCode;

use qol_removeapp::cli;

fn main() -> ExitCode {
    cli::exit_code(env::args().skip(1))
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::PluginManifest;

    qol_plugin_api::assert_plugin_toml_valid!();

    #[test]
    fn live_manifest_declares_the_headless_contract() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        assert!(manifest.capabilities.doctor);
        assert_eq!(
            manifest.catalog_runtime_args("open"),
            Some(vec!["open".to_string()])
        );
    }
}
