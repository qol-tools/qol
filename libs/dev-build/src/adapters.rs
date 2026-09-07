use crate::types::BuildResult;
use std::collections::HashMap;
use std::path::Path;

pub trait CoreEventSink: Send + Sync {
    fn publish(&self, event: crate::core::CoreEvent);
}

pub trait CargoPluginBuilder: Send + Sync {
    fn build_plugin_with_progress(
        &self,
        plugin_id: &str,
        path: &Path,
        on_progress: &mut dyn FnMut(u8, String),
    ) -> BuildResult;

    fn build_plugins_with_progress(
        &self,
        plugins: &[(&str, &Path)],
        on_progress: &mut dyn FnMut(&str, u8, String),
    ) -> Option<Vec<BuildResult>> {
        let _ = (plugins, on_progress);
        None
    }
}

pub trait BuildFingerprintStore: Send + Sync {
    fn load(&self, config_dir: &Path) -> HashMap<String, String>;
    fn save(&self, config_dir: &Path, fingerprints: &HashMap<String, String>)
        -> Result<(), String>;
}
