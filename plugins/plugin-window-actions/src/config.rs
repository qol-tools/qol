use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CenterMode {
    Pixels,
    Percent,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowActionsConfig {
    pub center_mode: CenterMode,
    pub center_width_px: f64,
    pub center_height_px: f64,
    pub center_width_percent: f64,
    pub center_height_percent: f64,
    pub snap_fraction: f64,
    pub reveal_taskbar_after_move: bool,
}

impl Default for WindowActionsConfig {
    fn default() -> Self {
        Self {
            center_mode: CenterMode::Pixels,
            center_width_px: 1152.0,
            center_height_px: 892.0,
            center_width_percent: 0.64,
            center_height_percent: 0.79,
            snap_fraction: 0.5,
            reveal_taskbar_after_move: true,
        }
    }
}

pub fn load_config() -> WindowActionsConfig {
    let Some(path) = config_path() else {
        return WindowActionsConfig::default();
    };
    let Ok(raw) = std::fs::read_to_string(path) else {
        return WindowActionsConfig::default();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return WindowActionsConfig::default();
    };
    WindowActionsConfig::from_json(&value)
}

impl WindowActionsConfig {
    fn from_json(value: &serde_json::Value) -> Self {
        let defaults = Self::default();
        Self {
            center_mode: read_string(value, &["center_mode"])
                .and_then(parse_center_mode)
                .unwrap_or(defaults.center_mode),
            center_width_px: read_number(value, &["center_width_px"])
                .or_else(|| read_number(value, &["center", "width"]))
                .unwrap_or(defaults.center_width_px)
                .max(1.0),
            center_height_px: read_number(value, &["center_height_px"])
                .or_else(|| read_number(value, &["center", "height"]))
                .unwrap_or(defaults.center_height_px)
                .max(1.0),
            center_width_percent: read_number(value, &["center_width_percent"])
                .or_else(|| read_number(value, &["center", "width_percent"]))
                .unwrap_or(defaults.center_width_percent)
                .clamp(0.1, 1.0),
            center_height_percent: read_number(value, &["center_height_percent"])
                .or_else(|| read_number(value, &["center", "height_percent"]))
                .unwrap_or(defaults.center_height_percent)
                .clamp(0.1, 1.0),
            snap_fraction: read_number(value, &["snap_fraction"])
                .or_else(|| read_number(value, &["snap", "left_fraction"]))
                .or_else(|| read_number(value, &["snap", "right_fraction"]))
                .or_else(|| read_number(value, &["snap", "bottom_fraction"]))
                .unwrap_or(defaults.snap_fraction)
                .clamp(0.1, 1.0),
            reveal_taskbar_after_move: read_bool(value, &["reveal_taskbar_after_move"])
                .or_else(|| read_bool(value, &["monitor_move", "reveal_taskbar_on_change"]))
                .unwrap_or(defaults.reveal_taskbar_after_move),
        }
    }

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

fn config_path() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok();
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));

    cwd.into_iter()
        .chain(exe_dir)
        .map(|dir| dir.join("config.json"))
        .find(|path| path.is_file())
}

fn read_number(value: &serde_json::Value, path: &[&str]) -> Option<f64> {
    read_value(value, path)?.as_f64()
}

fn read_string<'a>(value: &'a serde_json::Value, path: &[&str]) -> Option<&'a str> {
    read_value(value, path)?.as_str()
}

fn read_bool(value: &serde_json::Value, path: &[&str]) -> Option<bool> {
    read_value(value, path)?.as_bool()
}

fn read_value<'a>(value: &'a serde_json::Value, path: &[&str]) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

fn parse_center_mode(value: &str) -> Option<CenterMode> {
    if value == "pixels" {
        return Some(CenterMode::Pixels);
    }
    if value == "percent" {
        return Some(CenterMode::Percent);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{CenterMode, WindowActionsConfig};
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_new_contract_values_are_respected(
            use_percent in any::<bool>(),
            width in 1.0f64..4000.0,
            height in 1.0f64..4000.0,
            width_percent in 0.1f64..1.0,
            height_percent in 0.1f64..1.0,
            snap in 0.1f64..1.0,
            reveal in any::<bool>()
        ) {
            let value = serde_json::json!({
                "center_mode": if use_percent { "percent" } else { "pixels" },
                "center_width_px": width,
                "center_height_px": height,
                "center_width_percent": width_percent,
                "center_height_percent": height_percent,
                "snap_fraction": snap,
                "reveal_taskbar_after_move": reveal
            });

            let config = WindowActionsConfig::from_json(&value);

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
        fn prop_legacy_nested_values_are_respected(
            width in 1.0f64..4000.0,
            height in 1.0f64..4000.0,
            width_percent in 0.1f64..1.0,
            height_percent in 0.1f64..1.0,
            snap in 0.1f64..1.0,
            reveal in any::<bool>()
        ) {
            let value = serde_json::json!({
                "center": {
                    "width": width,
                    "height": height,
                    "width_percent": width_percent,
                    "height_percent": height_percent
                },
                "snap": {
                    "left_fraction": snap
                },
                "monitor_move": {
                    "reveal_taskbar_on_change": reveal
                }
            });

            let config = WindowActionsConfig::from_json(&value);

            prop_assert_eq!(config.center_mode, CenterMode::Pixels);
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
