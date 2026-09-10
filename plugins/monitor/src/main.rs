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

    fn modes_row_actions(
        row: &plugin_monitor::monitor::layout::ModeRow,
    ) -> Vec<qol_config::contract::ResolvedRowAction> {
        let spec =
            qol_config::contract::parse_spec("qol-config.toml").expect("qol-config.toml invalid");
        let field = spec.field("modes").expect("the modes field must exist");
        assert_eq!(field.row_label.as_deref(), Some("{connector} {label}"));
        let row_json = serde_json::to_value(row).unwrap();
        qol_config::contract::resolve_row_actions(
            field.row_action.as_ref(),
            &field.row_actions,
            &row_json,
        )
    }

    fn modes_row_action_input(
        row: &plugin_monitor::monitor::layout::ModeRow,
    ) -> qol_config::contract::ResolvedRowAction {
        modes_row_actions(row)
            .into_iter()
            .find(|action| action.action == "set_mode")
            .expect("the modes row action must be set_mode")
    }

    #[test]
    fn modes_row_action_round_trips_into_the_daemon_set_mode_command() {
        let row = plugin_monitor::monitor::layout::ModeRow {
            id: "id-alpha#11".into(),
            display_id: "id-alpha".into(),
            connector: "card0-DP-1".into(),
            token: 11,
            width: 1280,
            height: 720,
            refresh_hz: 75,
            label: "1280x720@75".into(),
            detail: "available mode".into(),
            current: false,
            writable: true,
            selectable: true,
        };
        let action = modes_row_action_input(&row);
        assert_eq!(action.label, "Set");
        let request = qol_runtime::protocol::DaemonRequest {
            action: action.action.clone(),
            input: action.input.clone(),
        };
        match plugin_monitor::daemon::parse_request(&request) {
            qol_plugin_daemon::daemon::ReadResult::Command(
                plugin_monitor::daemon::Command::SetMode {
                    display,
                    token,
                    width,
                    height,
                    refresh,
                },
            ) => {
                assert_eq!(display, "id-alpha");
                assert_eq!(token, Some(11));
                assert_eq!((width, height, refresh), (1280, 720, Some(75)));
            }
            _ => panic!("the modes row action must parse as set_mode"),
        }
    }

    #[test]
    fn mode_rows_address_each_row_by_its_own_token() {
        use qol_windowing::display::{DisplayHandle, DisplayMode, DisplaySnapshot};
        use std::collections::BTreeMap;

        let snapshot = DisplaySnapshot {
            handle: DisplayHandle::new("id-alpha".into(), "card0-DP-1".into(), None, false),
            bounds: qol_windowing::MonitorBounds {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            },
            primary: true,
            mode: Some(DisplayMode {
                token: 10,
                width: 1920,
                height: 1080,
                refresh_hz: 60,
            }),
        };
        let modes = BTreeMap::from([(
            "card0-DP-1".to_string(),
            vec![
                DisplayMode {
                    token: 10,
                    width: 1920,
                    height: 1080,
                    refresh_hz: 60,
                },
                DisplayMode {
                    token: 11,
                    width: 1920,
                    height: 1080,
                    refresh_hz: 60,
                },
            ],
        )]);
        let rows = plugin_monitor::monitor::layout::mode_rows(&[snapshot], &modes, true);
        assert_eq!(rows.len(), 2);
        assert_ne!(rows[0].id, rows[1].id, "every mode row needs a unique id");
        assert_eq!(
            rows[0].label, rows[1].label,
            "colliding labels stay distinct"
        );
        assert_eq!(rows.iter().filter(|row| row.selectable).count(), 1);
        for row in &rows {
            if !row.selectable {
                assert!(
                    modes_row_actions(row).is_empty(),
                    "the non-selectable current mode row must offer no row action"
                );
                continue;
            }
            let action = modes_row_action_input(row);
            assert_eq!(
                action.input["id"],
                serde_json::json!(row.display_id),
                "{}",
                row.id
            );
            assert_eq!(action.input["token"], serde_json::json!(row.token));
            let request = qol_runtime::protocol::DaemonRequest {
                action: action.action.clone(),
                input: action.input.clone(),
            };
            match plugin_monitor::daemon::parse_request(&request) {
                qol_plugin_daemon::daemon::ReadResult::Command(
                    plugin_monitor::daemon::Command::SetMode { token, .. },
                ) => assert_eq!(token, Some(row.token), "row {}", row.id),
                _ => panic!("row {} must parse as set_mode", row.id),
            }
        }
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
        let apply_layout = config
            .field("apply_layout")
            .and_then(|field| field.label.clone())
            .expect("the apply_layout field needs a label");
        assert_eq!(
            apply_layout, manifest.actions["apply_layout"].label,
            "the apply_layout label must match across plugin.toml and qol-config.toml"
        );
        assert!(
            runtime.actions["apply_layout"]
                .description
                .contains("positions"),
            "the runtime description must keep the positions wording"
        );
    }
}
