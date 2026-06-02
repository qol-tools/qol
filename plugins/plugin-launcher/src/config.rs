use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct DisplayConfig {
    pub ghost_opacity: Option<f32>,
    pub ghost_debug_color: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct LauncherConfig {
    pub display: DisplayConfig,
}

const PLUGIN_ID: &str = "plugin-launcher";

pub fn load_launcher_config() -> LauncherConfig {
    qol_config::load_plugin_config_from_env(PLUGIN_ID)
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
