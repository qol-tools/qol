mod platform;

pub(crate) use platform::{allow, check};

use crate::config::ServerConfig;

fn ports_label() -> String {
    format!(
        "UDP ports {} and {}",
        ServerConfig::DISCOVERY_PORT,
        ServerConfig::COMMAND_PORT
    )
}

fn details(state: &str) -> serde_json::Value {
    serde_json::json!({
        "state": state,
        "ports": [ServerConfig::DISCOVERY_PORT, ServerConfig::COMMAND_PORT],
        "inspection": "read_only",
        "rule_changed": false,
    })
}
