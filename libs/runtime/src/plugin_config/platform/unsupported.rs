pub(crate) fn load_json(_plugin_id: &str) -> Option<serde_json::Value> {
    None
}

pub(crate) fn save(_plugin_id: &str, _config: &serde_json::Value) -> bool {
    false
}
