use crate::client::PlatformStateClient;

pub(crate) fn load_json(plugin_id: &str) -> Option<serde_json::Value> {
    PlatformStateClient::from_env().get_plugin_config(plugin_id)
}

pub(crate) fn save(plugin_id: &str, config: &serde_json::Value) -> bool {
    PlatformStateClient::from_env().set_plugin_config(plugin_id, config)
}
