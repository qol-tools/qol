use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CenterMode {
    Pixels,
    Percent,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct WindowActionsConfig {
    #[serde(default = "default_center_mode")]
    pub center_mode: CenterMode,
    #[serde(default = "default_center_width_px")]
    pub center_width_px: f64,
    #[serde(default = "default_center_height_px")]
    pub center_height_px: f64,
    #[serde(default = "default_center_width_percent")]
    pub center_width_percent: f64,
    #[serde(default = "default_center_height_percent")]
    pub center_height_percent: f64,
    #[serde(default = "default_snap_fraction")]
    pub snap_fraction: f64,
    #[serde(default = "default_reveal_taskbar")]
    pub reveal_taskbar_after_move: bool,
}

fn default_center_mode() -> CenterMode {
    CenterMode::Pixels
}
fn default_center_width_px() -> f64 {
    1152.0
}
fn default_center_height_px() -> f64 {
    892.0
}
fn default_center_width_percent() -> f64 {
    0.64
}
fn default_center_height_percent() -> f64 {
    0.79
}
fn default_snap_fraction() -> f64 {
    0.5
}
fn default_reveal_taskbar() -> bool {
    true
}

impl Default for WindowActionsConfig {
    fn default() -> Self {
        Self {
            center_mode: default_center_mode(),
            center_width_px: default_center_width_px(),
            center_height_px: default_center_height_px(),
            center_width_percent: default_center_width_percent(),
            center_height_percent: default_center_height_percent(),
            snap_fraction: default_snap_fraction(),
            reveal_taskbar_after_move: default_reveal_taskbar(),
        }
    }
}

pub fn load_config() -> WindowActionsConfig {
    qol_config::load_plugin_config(&["plugin-window-actions"])
}

impl WindowActionsConfig {
    #[cfg(any(target_os = "macos", test))]
    pub fn center_size_for_monitor(&self, monitor_width: f64, monitor_height: f64) -> (f64, f64) {
        let width = self.resolve_center_width(monitor_width);
        let height = self.resolve_center_height(monitor_height);
        (
            width.clamp(1.0, monitor_width),
            height.clamp(1.0, monitor_height),
        )
    }

    #[cfg(any(target_os = "macos", test))]
    fn resolve_center_width(&self, monitor_width: f64) -> f64 {
        if self.center_mode == CenterMode::Percent {
            return monitor_width * self.center_width_percent;
        }
        self.center_width_px
    }

    #[cfg(any(target_os = "macos", test))]
    fn resolve_center_height(&self, monitor_height: f64) -> f64 {
        if self.center_mode == CenterMode::Percent {
            return monitor_height * self.center_height_percent;
        }
        self.center_height_px
    }
}

#[cfg(test)]
mod tests {
    use super::{CenterMode, WindowActionsConfig};
    use proptest::prelude::*;

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
            reveal in any::<bool>()
        ) {
            let json = serde_json::json!({
                "center_mode": if use_percent { "percent" } else { "pixels" },
                "center_width_px": width,
                "center_height_px": height,
                "center_width_percent": width_percent,
                "center_height_percent": height_percent,
                "snap_fraction": snap,
                "reveal_taskbar_after_move": reveal
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
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_percent_center_size_tracks_monitor_dimensions(
            monitor_width in 100.0f64..6000.0,
            monitor_height in 100.0f64..4000.0,
            width_percent in 0.1f64..1.0,
            height_percent in 0.1f64..1.0
        ) {
            let config = WindowActionsConfig {
                center_mode: CenterMode::Percent,
                center_width_px: 1152.0,
                center_height_px: 892.0,
                center_width_percent: width_percent,
                center_height_percent: height_percent,
                snap_fraction: 0.5,
                reveal_taskbar_after_move: true,
            };

            let (width, height) = config.center_size_for_monitor(monitor_width, monitor_height);

            prop_assert_eq!(width, monitor_width * width_percent);
            prop_assert_eq!(height, monitor_height * height_percent);
        }
    }
}
