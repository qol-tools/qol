use std::process::ExitCode;

fn main() -> ExitCode {
    qol_log::init_stderr();
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() && std::env::var_os(qol_conventions::ENV_DAEMON_SOCKET).is_some() {
        return match qol_clipboard::app::run_daemon() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                log::error!("{error:#}");
                ExitCode::FAILURE
            }
        };
    }
    qol_clipboard::cli::exit_code(args)
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::launcher_flows::{FlowEnter, FlowPanel};
    use qol_plugin_api::manifest::PluginManifest;

    qol_plugin_api::assert_plugin_toml_valid!();

    #[test]
    fn live_manifest_declares_the_headless_contract() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        assert!(manifest.capabilities.doctor);
        for action in ["open", "copy", "pin", "forget", "clear", "settings"] {
            assert_eq!(
                manifest.catalog_runtime_args(action),
                Some(vec![action.to_string()]),
                "action: {action}"
            );
        }
    }

    #[test]
    fn live_manifest_declares_the_clipboard_flow() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let launcher = manifest.launcher.as_ref().expect("launcher flow declared");
        assert_eq!(launcher.query.as_deref(), Some("rows"));
        assert_eq!(launcher.enter, Some(FlowEnter::Copy));
        assert_eq!(launcher.panel, Some(FlowPanel::Preview));
        assert_eq!(launcher.detail.as_deref(), Some("detail"));
        let keys: Vec<_> = launcher
            .row_actions
            .iter()
            .map(|action| (action.action.as_str(), action.key.as_deref()))
            .collect();
        assert_eq!(
            keys,
            vec![
                ("pin", Some("ctrl+p")),
                ("forget", Some("delete")),
                ("copy", None)
            ]
        );
        let runtime: qol_config::contract::RuntimeSpec =
            toml::from_str(include_str!("../qol-runtime.toml")).expect("qol-runtime.toml invalid");
        qol_plugin_api::manifest::validate_launcher_runtime(&manifest, Some(&runtime))
            .expect("launcher queries must be declared in qol-runtime.toml");
    }

    #[test]
    fn live_manifest_declares_the_resident_daemon() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let daemon = manifest.daemon.as_ref().expect("daemon must be declared");
        assert!(daemon.enabled);
        assert_eq!(daemon.socket.as_deref(), Some("/tmp/qol-clipboard.sock"));
    }
}
