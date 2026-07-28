use qol_config::{PluginConfigInspection, PluginConfigInspectionError};
use serde::{Deserialize, Serialize};

const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();
const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
pub(crate) type ConfigInspection = PluginConfigInspection<WindowActionsConfig>;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CenterMode {
    Pixels,
    Percent,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct WindowActionsConfig {
    pub center_mode: CenterMode,
    pub center_width_px: f64,
    pub center_height_px: f64,
    pub center_width_percent: f64,
    pub center_height_percent: f64,
    pub snap_fraction: f64,
    pub reveal_taskbar_after_move: bool,
    pub glide_speed_px_per_second: f64,
}

pub fn load_config() -> WindowActionsConfig {
    qol_config::load_plugin_config_from_env_with_contract(PLUGIN_ID, CONFIG_CONTRACT)
}

pub(crate) fn inspect_config() -> Result<ConfigInspection, PluginConfigInspectionError> {
    qol_config::inspect_plugin_config_from_env_with_contract(PLUGIN_ID, CONFIG_CONTRACT)
}

#[cfg(test)]
mod tests {
    use super::{CenterMode, WindowActionsConfig};
    use proptest::prelude::*;

    #[test]
    fn contract_defaults_match_runtime_type() {
        qol_config::validate_contract_defaults_match_type::<WindowActionsConfig>(
            super::CONFIG_CONTRACT,
        )
        .unwrap();

        let defaults: WindowActionsConfig =
            qol_config::typed_defaults_from_contract(super::CONFIG_CONTRACT).unwrap();
        assert_eq!(defaults.center_mode, CenterMode::Pixels);
        assert_eq!(defaults.center_width_px, 1152.0);
        assert_eq!(defaults.center_height_px, 892.0);
        assert_eq!(defaults.center_width_percent, 0.64);
        assert_eq!(defaults.center_height_percent, 0.79);
        assert_eq!(defaults.snap_fraction, 0.5);
        assert!(defaults.reveal_taskbar_after_move);
        assert_eq!(defaults.glide_speed_px_per_second, 1200.0);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_deserialized_values_are_respected(
            use_percent in any::<bool>(),
            width in 1.0f64..4000.0,
            height in 1.0f64..4000.0,
            width_percent in 0.1f64..1.0,
            height_percent in 0.1f64..1.0,
            snap in 0.1f64..1.0,
            reveal in any::<bool>(),
            glide_speed in 100.0f64..4000.0
        ) {
            let json = serde_json::json!({
                "center_mode": if use_percent { "percent" } else { "pixels" },
                "center_width_px": width,
                "center_height_px": height,
                "center_width_percent": width_percent,
                "center_height_percent": height_percent,
                "snap_fraction": snap,
                "reveal_taskbar_after_move": reveal,
                "glide_speed_px_per_second": glide_speed
            });

            let config: WindowActionsConfig = serde_json::from_value(json).unwrap();

            prop_assert_eq!(
                config.center_mode,
                if use_percent { CenterMode::Percent } else { CenterMode::Pixels }
            );
            prop_assert_eq!(config.center_width_px, width);
            prop_assert_eq!(config.center_height_px, height);
            prop_assert_eq!(config.center_width_percent, width_percent);
            prop_assert_eq!(config.center_height_percent, height_percent);
            prop_assert_eq!(config.snap_fraction, snap);
            prop_assert_eq!(config.reveal_taskbar_after_move, reveal);
            prop_assert_eq!(config.glide_speed_px_per_second, glide_speed);
        }
    }
}
