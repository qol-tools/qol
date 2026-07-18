use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DisplayConfig {
    pub ghost_opacity: Option<f32>,
    pub ghost_debug_color: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LauncherConfig {
    pub display: DisplayConfig,
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

#[cfg(test)]
fn contract_defaults() -> LauncherConfig {
    qol_config::typed_defaults_from_contract(CONFIG_CONTRACT).expect("contract defaults must parse")
}

/// Reload the launcher config and push the ghost debug opacity/colour into the
/// shared popup layer. Called at boot (for the pre-created ghost) and on every
/// show, so changing the values in qol-tray takes effect without restarting the
/// daemon. The ghost debug visual is debug-only, so this compiles out (no config
/// IO) in release builds, keeping show a pure reveal.
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
    }
}
