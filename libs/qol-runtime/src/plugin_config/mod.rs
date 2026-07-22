use serde::de::DeserializeOwned;
use serde::Serialize;

use qol_conventions::ENV_PLUGIN_ID;

mod platform;

pub fn load<T: DeserializeOwned + Default>() -> T {
    match load_json() {
        Some(value) if !value.is_null() => serde_json::from_value(value).unwrap_or_default(),
        _ => T::default(),
    }
}

pub fn load_json() -> Option<serde_json::Value> {
    let plugin_id = plugin_id()?;
    platform::load_json(&plugin_id)
}

pub fn save<T: Serialize>(value: &T) -> bool {
    let Some(plugin_id) = plugin_id() else {
        eprintln!("[runtime/plugin_config] save skipped: {ENV_PLUGIN_ID} unset");
        return false;
    };
    let Ok(json) = serde_json::to_value(value) else {
        return false;
    };
    platform::save(&plugin_id, &json)
}

fn plugin_id() -> Option<String> {
    std::env::var(ENV_PLUGIN_ID)
        .ok()
        .filter(|id| !id.is_empty())
}
