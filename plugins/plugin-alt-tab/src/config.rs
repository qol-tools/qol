use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    pub max_columns: usize,
    pub transparent_background: bool,
    pub card_background_color: String,
    pub card_background_opacity: f32,
    pub show_minimized: bool,
    pub show_debug_overlay: bool,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            max_columns: 6,
            transparent_background: false,
            card_background_color: "1a1e2a".to_string(),
            card_background_opacity: 0.85,
            show_minimized: true,
            show_debug_overlay: false,
        }
    }
}

pub use qol_plugin_api::color::parse_hex_color;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LabelConfig {
    pub show_app_name: bool,
    pub show_window_title: bool,
}

impl Default for LabelConfig {
    fn default() -> Self {
        Self {
            show_app_name: true,
            show_window_title: true,
        }
    }
}

impl LabelConfig {
    pub fn format(&self, app_name: &str, title: &str) -> String {
        let show_app = self.show_app_name && !app_name.is_empty();
        let show_title = self.show_window_title && !title.is_empty();
        match (show_app, show_title) {
            (true, true) => format!("{} - {}", capitalize_first(app_name), title),
            (true, false) => capitalize_first(app_name),
            (false, true) => title.to_string(),
            (false, false) => String::new(),
        }
    }
}

fn capitalize_first(s: &str) -> String {
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

const PLUGIN_NAMES: &[&str] = &["plugin-alt-tab", "alt-tab"];

pub fn load_alt_tab_config() -> AltTabConfig {
    let config: AltTabConfig = qol_plugin_api::config::load_plugin_config(PLUGIN_NAMES);
    eprintln!(
        "[alt-tab] config: action_mode={:?} max_columns={} reset_selection_on_open={} open_behavior={:?}",
        config.action_mode,
        config.display.max_columns,
        config.reset_selection_on_open,
        config.open_behavior,
    );
    config
}

