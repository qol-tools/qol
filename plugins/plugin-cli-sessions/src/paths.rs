pub const PLUGIN_ID: &str = "plugin-cli-sessions";

pub fn state_path() -> Option<std::path::PathBuf> {
    let id = qol_config::plugin_id_from_env(PLUGIN_ID);
    Some(
        qol_config::data_dir()?
            .join("plugins")
            .join(id)
            .join("sessions.json"),
    )
}
