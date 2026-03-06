mod codesign;
mod plugin_build;
mod self_build;
mod spawn;

use std::path::Path;

use super::types::BuildResult;
use crate::dev::adapters::traits::CargoPluginBuilder;

pub(crate) struct CargoCommandPluginBuilder;

impl CargoPluginBuilder for CargoCommandPluginBuilder {
    fn build_plugin_with_progress(
        &self,
        plugin_id: &str,
        path: &Path,
        on_progress: &mut dyn FnMut(u8, String),
    ) -> BuildResult {
        plugin_build::build_cargo_plugin_with_progress(plugin_id, path, on_progress)
    }
}

pub fn build_qol_tray_self_with_progress<F>(on_progress: F) -> BuildResult
where
    F: FnMut(u8, String),
{
    self_build::build_qol_tray_self_with_progress(on_progress)
}
