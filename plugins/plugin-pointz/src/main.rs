mod app;
mod cli;
mod command;
mod config;
mod discovery;
mod doctor;
mod input;
mod network;
mod qol;
mod security;

fn main() -> std::process::ExitCode {
    cli::exit_code(std::env::args().skip(1))
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::{PluginManifest, PortProtocol};

    qol_plugin_api::assert_plugin_toml_valid!();

    #[test]
    fn manifest_preserves_runtime_actions_endpoints_and_enables_doctor() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let runtime = manifest
            .runtime
            .as_ref()
            .expect("PointZ runtime must be declared");
        let daemon = manifest
            .daemon
            .as_ref()
            .expect("PointZ daemon must be declared");

        assert_eq!(runtime.command, "pointzerver");
        assert!(manifest.capabilities.doctor);
        assert_eq!(
            manifest.catalog_runtime_args("settings"),
            Some(vec!["--action".to_string(), "settings".to_string()])
        );
        assert!(daemon.enabled);
        assert_eq!(daemon.command, "pointzerver");
        assert_eq!(
            daemon.socket.as_deref(),
            Some(crate::config::ServerConfig::DAEMON_SOCKET)
        );
        assert_eq!(daemon.extra_ports.len(), 2);
        assert!(daemon.extra_ports.iter().any(|port| {
            port.name == "discovery"
                && port.port == crate::config::ServerConfig::DISCOVERY_PORT
                && port.protocol == PortProtocol::Udp
        }));
        assert!(daemon.extra_ports.iter().any(|port| {
            port.name == "command"
                && port.port == crate::config::ServerConfig::COMMAND_PORT
                && port.protocol == PortProtocol::Udp
        }));
    }
}
