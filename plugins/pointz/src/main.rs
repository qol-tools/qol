mod app;
mod cli;
mod command;
mod config;
mod doctor;
mod input;
mod network;
mod qol;

fn main() -> std::process::ExitCode {
    qol_log::init_stderr();
    cli::exit_code(std::env::args().skip(1))
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::{PeerTrust, PluginManifest};

    qol_plugin_api::assert_plugin_toml_valid!();

    #[test]
    fn manifest_hands_pairing_and_ports_to_core_and_enables_doctor() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let daemon = manifest
            .daemon
            .as_ref()
            .expect("PointZ daemon must be declared");

        assert!(manifest.capabilities.doctor);
        assert_eq!(manifest.capabilities.peer_trust, Some(PeerTrust::PointzV1));
        assert_eq!(
            manifest.catalog_runtime_args("settings"),
            Some(vec!["--action".to_string(), "settings".to_string()])
        );
        assert!(daemon.enabled);
        assert_eq!(
            daemon.socket.as_deref(),
            Some(crate::config::ServerConfig::DAEMON_SOCKET)
        );
        assert!(daemon.extra_ports.is_empty());
    }
}
