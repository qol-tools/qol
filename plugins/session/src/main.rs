use std::process::ExitCode;

fn main() -> ExitCode {
    qol_session::cli::exit_code(std::env::args().skip(1))
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::PluginManifest;

    qol_plugin_api::assert_plugin_toml_valid!();

    #[test]
    fn live_manifest_declares_the_lock_action_and_doctor() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        assert!(manifest.capabilities.doctor);
        assert_eq!(
            manifest.catalog_runtime_args("lock"),
            Some(vec!["lock".to_string()])
        );
    }
}
