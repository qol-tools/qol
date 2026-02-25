use std::path::{Path, PathBuf};

use crate::frecency::FrequencyData;

pub fn default_path() -> PathBuf {
    qol_plugin_api::frecency::default_store_path("qol-launcher")
}

pub fn load(path: &Path) -> FrequencyData {
    qol_plugin_api::frecency::load(path)
}

pub fn save(path: &Path, data: &FrequencyData) {
    qol_plugin_api::frecency::save(path, data)
}
