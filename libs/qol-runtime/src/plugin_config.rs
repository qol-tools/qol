use serde::de::DeserializeOwned;
use serde::Serialize;

use qol_conventions::ENV_PLUGIN_ID;

use crate::client::PlatformStateClient;

pub fn load<T: DeserializeOwned + Default>() -> T {
    let Some(plugin_id) = plugin_id() else {
        return T::default();
    };
    match PlatformStateClient::from_env().get_plugin_config(&plugin_id) {
        Some(value) if !value.is_null() => serde_json::from_value(value).unwrap_or_default(),
        _ => T::default(),
    }
}

pub fn save<T: Serialize>(value: &T) -> bool {
    let Some(plugin_id) = plugin_id() else {
        eprintln!("[runtime/plugin_config] save skipped: {ENV_PLUGIN_ID} unset");
        return false;
    };
    let Ok(json) = serde_json::to_value(value) else {
        return false;
    };
    PlatformStateClient::from_env().set_plugin_config(&plugin_id, &json)
}

fn plugin_id() -> Option<String> {
    std::env::var(ENV_PLUGIN_ID)
        .ok()
        .filter(|id| !id.is_empty())
}
