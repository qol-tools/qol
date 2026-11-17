pub fn state_path(plugin_id: &str) -> Option<std::path::PathBuf> {
    Some(plugin_data_dir(plugin_id)?.join("sessions.json"))
}

pub fn anomalies_dir(plugin_id: &str) -> Option<std::path::PathBuf> {
    Some(plugin_data_dir(plugin_id)?.join("anomalies"))
}

pub fn snapshots_dir(plugin_id: &str) -> Option<std::path::PathBuf> {
    Some(plugin_data_dir(plugin_id)?.join("snapshots"))
}

fn plugin_data_dir(plugin_id: &str) -> Option<std::path::PathBuf> {
    Some(qol_config::data_dir()?.join("plugins").join(plugin_id))
}
