use std::process::ExitCode;

fn main() -> ExitCode {
    plugin_monitor::cli::exit_code(std::env::args().skip(1))
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::PluginManifest;

    qol_plugin_api::assert_plugin_toml_valid!();

    #[test]
    fn live_manifest_declares_the_headless_contract() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let runtime = manifest
            .runtime
            .as_ref()
            .expect("monitor runtime must be declared");

        assert_eq!(runtime.command, "plugin-monitor");
        assert_eq!(
            manifest.plugin.uid.as_ref().map(|uid| uid.as_str()),
            Some(plugin_monitor::hotkeys::PLUGIN_UID),
            "the doctor and the host hotkey config must agree on the tray-written uid"
        );
        assert!(manifest.capabilities.doctor);
        assert!(manifest.capabilities.gpui);
        assert_eq!(
            manifest.catalog_runtime_args("brightness-up"),
            Some(vec!["up".to_string()])
        );
        assert_eq!(
            manifest.catalog_runtime_args("brightness-down"),
            Some(vec!["down".to_string()])
        );
        assert_eq!(
            manifest.catalog_runtime_args("settings"),
            Some(vec!["settings".to_string()])
        );
        for (action, label, verb) in [
            ("set_mode", "Set Display Mode", "set-mode"),
            ("set_primary", "Set Primary Display", "primary"),
            ("arrange", "Arrange Displays", "arrange"),
            ("apply_layout", "Apply positions", "apply-layout"),
        ] {
            assert_eq!(
                manifest.catalog_runtime_args(action),
                Some(vec![verb.to_string()]),
                "action: {action}"
            );
            assert_eq!(manifest.actions[action].label, label, "action: {action}");
        }
        assert!(manifest.actions["brightness-up"].continuous);
        assert!(manifest.actions["brightness-down"].continuous);
        assert!(!manifest.actions["settings"].continuous);

        let daemon = manifest
            .daemon
            .as_ref()
            .expect("continuous actions require a daemon transport");
        assert!(daemon.enabled);
        assert_eq!(daemon.command, "plugin-monitor");
        assert!(daemon.socket.is_some());

        let expected = [
            "apply",
            "apply_layout",
            "arrange",
            "brightness-down",
            "brightness-up",
            "night_off",
            "night_on",
            "night_toggle",
            "reload",
            "set_brightness",
            "set_mode",
            "set_primary",
            "settings",
        ]
        .map(str::to_string);
        assert_eq!(
            manifest.executable_action_ids(),
            std::collections::BTreeSet::from(expected)
        );
    }

    #[test]
    fn every_runtime_action_is_reachable_through_the_manifest() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let runtime = qol_config::contract::parse_runtime_spec("qol-runtime.toml")
            .expect("qol-runtime.toml invalid");
        let declared = manifest.executable_action_ids();
        let unreachable: Vec<&String> = runtime
            .actions
            .keys()
            .filter(|action| !declared.contains(*action))
            .collect();
        assert!(
            unreachable.is_empty(),
            "runtime actions the host cannot dispatch without a plugin.toml [action.*] entry: {unreachable:?}"
        );
    }

    #[test]
    fn arrangement_actions_align_labels_and_verbs_across_the_contracts() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let config =
            qol_config::contract::parse_spec("qol-config.toml").expect("qol-config.toml invalid");
        let runtime = qol_config::contract::parse_runtime_spec("qol-runtime.toml")
            .expect("qol-runtime.toml invalid");
        qol_config::contract::validate_contracts(&config, Some(&runtime))
            .expect("qol-config.toml and qol-runtime.toml must validate together");
        let arrangement = config
            .field("arrangement")
            .expect("the arrangement field must exist");
        assert_eq!(arrangement.label.as_deref(), Some("Arrangement"));
        assert_eq!(arrangement.query.as_deref(), Some("layout"));
        assert_eq!(arrangement.active_query.as_deref(), Some("modes"));
        assert_eq!(arrangement.action.as_deref(), Some("arrange"));
        assert_eq!(arrangement.active_action.as_deref(), Some("set_mode"));
        for query in ["layout", "modes"] {
            assert!(
                runtime.queries.contains_key(query),
                "query: {query} must stay declared in qol-runtime.toml"
            );
        }
        for action in ["arrange", "set_mode"] {
            assert!(
                runtime.actions.contains_key(action),
                "action: {action} must stay declared in qol-runtime.toml"
            );
        }
        for (action, label) in [
            ("set_mode", "Set Display Mode"),
            ("set_primary", "Set Primary Display"),
            ("arrange", "Arrange Displays"),
        ] {
            assert_eq!(manifest.actions[action].label, label, "action: {action}");
            assert!(
                runtime.actions.contains_key(action),
                "action: {action} must stay declared in qol-runtime.toml"
            );
        }
        assert!(manifest.actions.contains_key("apply_layout"));
        assert!(
            runtime.actions["apply_layout"]
                .description
                .contains("positions"),
            "the runtime description must keep the positions wording"
        );
    }
}
