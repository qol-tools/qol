use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() && std::env::var_os(qol_conventions::ENV_DAEMON_SOCKET).is_some() {
        return qol_sound::app::run();
    }
    qol_sound::cli::exit_code(args)
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::PluginManifest;

    qol_plugin_api::assert_plugin_toml_valid!();

    #[test]
    fn the_manifest_declares_the_headless_contract() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        assert!(manifest.capabilities.doctor);
        assert!(manifest.capabilities.gpui);
        assert_eq!(
            manifest.catalog_runtime_args("settings"),
            Some(vec!["settings".to_string()])
        );
    }
}
