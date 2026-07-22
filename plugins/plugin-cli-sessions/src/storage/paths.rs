pub const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");

pub fn state_path() -> Option<std::path::PathBuf> {
    Some(plugin_data_dir()?.join("sessions.json"))
}

pub fn anomalies_dir() -> Option<std::path::PathBuf> {
    Some(plugin_data_dir()?.join("anomalies"))
}

pub fn snapshots_dir() -> Option<std::path::PathBuf> {
    Some(plugin_data_dir()?.join("snapshots"))
}

fn plugin_data_dir() -> Option<std::path::PathBuf> {
    let id = qol_config::plugin_id_from_env(PLUGIN_ID);
    Some(qol_config::data_dir()?.join("plugins").join(id))
}
