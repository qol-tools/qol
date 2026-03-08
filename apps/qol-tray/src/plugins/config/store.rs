use super::PluginConfigs;
use crate::file_io;
use anyhow::Result;
use std::path::Path;

pub(super) fn load_configs(config_path: &Path) -> Result<PluginConfigs> {
    file_io::load_json_or_default(config_path)
}

pub(super) fn save_configs(config_path: &Path, configs: &PluginConfigs) -> Result<()> {
    file_io::write_pretty_json(config_path, configs)
}

pub(super) fn load_plugin_config(plugin_path: &Path) -> Result<serde_json::Value> {
    file_io::read_json(plugin_path)
}

pub(super) fn write_plugin_config(plugin_path: &Path, config: &serde_json::Value) -> Result<()> {
    file_io::write_pretty_json(plugin_path, config)
}
