use super::HotkeyConfig;
use crate::file_io;
use anyhow::Result;
use std::path::Path;

pub(super) fn load_config(config_path: &Path) -> Result<HotkeyConfig> {
    file_io::load_json_or_default(config_path)
}

pub(super) fn save_config(config_path: &Path, config: &HotkeyConfig) -> Result<()> {
    file_io::write_pretty_json(config_path, config)
}
