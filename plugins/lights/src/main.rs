use std::process::ExitCode;

fn main() -> ExitCode {
    plugin_lights::cli::exit_code(std::env::args().skip(1))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use qol_plugin_api::manifest::PluginManifest;

    #[test]
    fn live_manifest_declares_the_headless_doctor_contract() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let runtime = manifest
            .runtime
            .as_ref()
            .expect("Lights runtime must be declared");
        let expected = plugin_lights::runtime::actions::ALL_ACTIONS
            .iter()
            .map(|action| (*action).to_string())
            .collect::<BTreeSet<_>>();

        assert_eq!(runtime.command, "plugin-lights");
        assert!(manifest.capabilities.doctor);
        assert_eq!(manifest.executable_action_ids(), expected);
        for action in expected {
            assert_eq!(
                manifest.catalog_runtime_args(&action),
                Some(vec![action.clone()]),
                "action={action}"
            );
        }
    }
}
