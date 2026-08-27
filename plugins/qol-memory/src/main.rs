use std::process::ExitCode;

fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() && std::env::var_os(qol_conventions::ENV_DAEMON_SOCKET).is_some() {
        return match qol_memory::app::run_daemon() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error:#}");
                ExitCode::FAILURE
            }
        };
    }
    qol_memory::cli::exit_code(args)
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
            .expect("QoL Memory runtime must be declared");

        assert_eq!(runtime.command, "qol-memory");
        assert!(manifest.capabilities.doctor);
        assert_eq!(
            manifest.catalog_runtime_args("status"),
            Some(vec!["status".to_string()])
        );
    }

    #[test]
    fn live_manifest_declares_the_resident_daemon() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let daemon = manifest.daemon.as_ref().expect("daemon must be declared");

        assert_eq!(manifest.daemon.as_ref().map(|d| d.enabled), Some(true));
        assert_eq!(daemon.command, "qol-memory");
        assert_eq!(daemon.socket.as_deref(), Some("/tmp/qol-memory.sock"));
    }
}
