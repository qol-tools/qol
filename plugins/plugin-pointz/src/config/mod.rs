use serde::{Deserialize, Serialize};

const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();

pub struct ServerConfig;

impl ServerConfig {
    pub const DAEMON_SOCKET: &'static str = "/tmp/qol-pointz.sock";
    pub const DISCOVERY_PORT: u16 = 45454;
    pub const COMMAND_PORT: u16 = 45455;
    pub const DISCOVER_MESSAGE: &'static str = "DISCOVER";
    pub const DISCOVERY_BUFFER_SIZE: usize = 1024;
    pub const COMMAND_BUFFER_SIZE: usize = 4096;
    pub const UNKNOWN_HOSTNAME: &'static str = "Unknown";

    pub const MOUSE_CLICK_DELAY_MS: u64 = 10;
    pub const FALLBACK_SCREEN_WIDTH: f64 = 1920.0;
    pub const FALLBACK_SCREEN_HEIGHT: f64 = 1080.0;
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PluginConfig {}

pub(crate) type ConfigInspection = qol_config::PluginConfigInspection<PluginConfig>;

pub(crate) fn inspect() -> Result<ConfigInspection, qol_config::PluginConfigInspectionError> {
    qol_config::inspect_plugin_config_from_env_with_contract(env!("QOL_PLUGIN_ID"), CONFIG_CONTRACT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_defaults_match_the_typed_empty_config() {
        qol_config::validate_contract_defaults_match_type::<PluginConfig>(CONFIG_CONTRACT).unwrap();
    }
}
