use super::{
    read_hotkeys_list, read_plugin_configs, read_shortcuts_list, read_task_runner_value,
    PluginLockEntry, ProfileExportBundle, CURRENT_PROFILE_VERSION,
};
use anyhow::Result;

pub fn build_export_bundle(
    exported_at: String,
    plugins: Vec<PluginLockEntry>,
) -> Result<ProfileExportBundle> {
    Ok(ProfileExportBundle {
        version: CURRENT_PROFILE_VERSION,
        exported_at,
        hotkeys: read_hotkeys_list(),
        shortcuts: read_shortcuts_list(),
        task_runner: read_task_runner_value(),
        plugin_configs: read_plugin_configs()?,
        plugins,
    })
}

pub fn build_export_bundle_json(
    exported_at: String,
    plugins: Vec<PluginLockEntry>,
) -> Result<String> {
    serde_json::to_string_pretty(&build_export_bundle(exported_at, plugins)?).map_err(Into::into)
}
