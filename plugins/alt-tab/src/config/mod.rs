use serde::{Deserialize, Serialize};

pub const DEFAULT_CARD_BACKGROUND_COLOR: &str = "#202322";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    pub max_columns: usize,
    pub card_scale: f32,
    pub dynamic_card_scale: bool,
    pub card_padding: f32,
    pub transparent_background: bool,
    pub card_background_color: String,
    pub card_background_brightness: f32,
    pub card_background_opacity: f32,
    pub icon_position: PreviewIconPosition,
    pub show_minimized: bool,
    pub show_debug_overlay: bool,
    pub show_hotkey_hints: bool,
    pub ghost_opacity: Option<f32>,
    pub ghost_debug_color: Option<String>,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            max_columns: 6,
            card_scale: crate::picker::layout::DEFAULT_CARD_SCALE,
            dynamic_card_scale: true,
            card_padding: crate::picker::layout::DEFAULT_CARD_PADDING,
            transparent_background: false,
            card_background_color: DEFAULT_CARD_BACKGROUND_COLOR.to_string(),
            card_background_brightness: 1.0,
            card_background_opacity: 0.85,
            icon_position: PreviewIconPosition::default(),
            show_minimized: true,
            show_debug_overlay: false,
            show_hotkey_hints: true,
            ghost_opacity: None,
            ghost_debug_color: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PreviewIconPosition {
    TopLeft,
    #[default]
    TopRight,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LabelSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl LabelSize {
    pub fn factor(&self) -> f32 {
        match self {
            LabelSize::Small => 0.8,
            LabelSize::Medium => 0.92,
            LabelSize::Large => 1.25,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LabelConfig {
    pub show_app_name: bool,
    pub show_window_title: bool,
    pub size: LabelSize,
}

impl Default for LabelConfig {
    fn default() -> Self {
        Self {
            show_app_name: false,
            show_window_title: true,
            size: LabelSize::default(),
        }
    }
}

pub(crate) fn capitalize_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActionMode {
    Sticky,
    #[default]
    HoldToSwitch,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OpenBehavior {
    #[default]
    CycleOnce,
    ShowOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AltTabConfig {
    pub display: DisplayConfig,
    pub action_mode: ActionMode,
    #[serde(default = "default_reset_selection_on_open")]
    pub reset_selection_on_open: bool,
    #[serde(default)]
    pub open_behavior: OpenBehavior,
    #[serde(default)]
    pub label: LabelConfig,
}

impl Default for AltTabConfig {
    fn default() -> Self {
        Self {
            display: DisplayConfig::default(),
            action_mode: ActionMode::default(),
            reset_selection_on_open: default_reset_selection_on_open(),
            open_behavior: OpenBehavior::default(),
            label: LabelConfig::default(),
        }
    }
}

fn default_reset_selection_on_open() -> bool {
    true
}

pub(crate) const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();
pub(crate) type ConfigInspection = qol_config::PluginConfigInspection<AltTabConfig>;

pub(crate) fn contract() -> &'static str {
    CONFIG_CONTRACT
}

pub fn load_alt_tab_config() -> AltTabConfig {
    let config: AltTabConfig =
        qol_config::load_plugin_config_from_env_with_contract(PLUGIN_ID, CONFIG_CONTRACT);
    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab] config: action_mode={:?} max_columns={} card_scale={} dynamic_card_scale={} card_padding={} icon_position={:?} reset_selection_on_open={} open_behavior={:?}",
        config.action_mode,
        config.display.max_columns,
        config.display.card_scale,
        config.display.dynamic_card_scale,
        config.display.card_padding,
        config.display.icon_position,
        config.reset_selection_on_open,
        config.open_behavior,
    );
    config
}

pub(crate) fn inspect() -> Result<ConfigInspection, qol_config::PluginConfigInspectionError> {
    qol_config::inspect_plugin_config_from_env_with_contract(PLUGIN_ID, CONFIG_CONTRACT)
}

#[cfg(test)]
fn contract_defaults() -> AltTabConfig {
    qol_config::typed_defaults_from_contract(CONFIG_CONTRACT).expect("contract defaults must parse")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_defaults_match_runtime_type() {
        qol_config::validate_contract_defaults_match_type::<AltTabConfig>(CONFIG_CONTRACT).unwrap();

        let defaults = contract_defaults();
        assert_eq!(defaults.display.ghost_opacity, Some(0.0));
        assert_eq!(
            defaults.display.ghost_debug_color.as_deref(),
            Some("ff0000")
        );
    }

    #[test]
    fn dynamic_card_scale_defaults_on() {
        assert!(DisplayConfig::default().dynamic_card_scale);
        let defaults = contract_defaults();
        assert!(defaults.display.dynamic_card_scale);
    }
}
