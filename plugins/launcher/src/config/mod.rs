use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DisplayConfig {
    pub ghost_opacity: Option<f32>,
    pub ghost_debug_color: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LauncherConfig {
    pub display: DisplayConfig,
    #[serde(default)]
    pub extra_file_scan_roots: Vec<PathBuf>,
}

const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();
const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");

pub(crate) fn contract() -> &'static str {
    CONFIG_CONTRACT
}

pub(crate) fn plugin_id() -> &'static str {
    PLUGIN_ID
}

pub fn load_launcher_config() -> LauncherConfig {
    qol_config::load_plugin_config_from_env_with_contract(PLUGIN_ID, CONFIG_CONTRACT)
}

pub(crate) fn inspect_launcher_config() -> Result<
    qol_config::PluginConfigInspection<LauncherConfig>,
    qol_config::PluginConfigInspectionError,
> {
    qol_config::inspect_plugin_config_from_env_with_contract(PLUGIN_ID, CONFIG_CONTRACT)
}

#[cfg(test)]
fn contract_defaults() -> LauncherConfig {
    qol_config::typed_defaults_from_contract(CONFIG_CONTRACT).expect("contract defaults must parse")
}

pub fn apply_ghost_debug() {
    #[cfg(debug_assertions)]
    {
        let config = load_launcher_config();
        qol_gpui::popup_window::set_ghost_debug(
            config.display.ghost_opacity,
            config.display.ghost_debug_color.as_deref(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_defaults_match_runtime_type() {
        qol_config::validate_contract_defaults_match_type::<LauncherConfig>(CONFIG_CONTRACT)
            .unwrap();

        let defaults = contract_defaults();
        assert_eq!(defaults.display.ghost_opacity, Some(0.0));
        assert_eq!(
            defaults.display.ghost_debug_color.as_deref(),
            Some("00ff00")
        );
        assert!(defaults.extra_file_scan_roots.is_empty());
    }
}
