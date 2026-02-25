use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    pub max_columns: usize,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            max_columns: 6,
        }
    }
}

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

pub fn load_alt_tab_config() -> AltTabConfig {
    for path in config_paths() {
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };
        match serde_json::from_str::<AltTabConfig>(&contents) {
            Ok(config) => {
                println!(
                    "Loaded config from {}: action_mode={:?} max_columns={} reset_selection_on_open={} open_behavior={:?}",
                    path.display(),
                    config.action_mode,
                    config.display.max_columns,
                    config.reset_selection_on_open,
                    config.open_behavior
                );
                return config;
            }
            Err(e) => {
                println!("Failed to parse config at {}: {}", path.display(), e);
            }
        }
    }
    println!("Using default config");
    AltTabConfig::default()
}

pub(crate) fn config_paths() -> Vec<PathBuf> {
    const RELATIVE_PATHS: [&str; 2] = [
        "plugins/plugin-alt-tab/config.json",
        "plugins/alt-tab/config.json",
    ];

    let mut paths = Vec::new();
    for root in config_roots() {
        for relative in RELATIVE_PATHS {
            let candidate = root.join(relative);
            if !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
    }
    paths
}

fn config_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let Some(base) = base_data_dir() else {
        return roots;
    };
    if let Some(id) = install_id_from_env() {
        roots.push(base.join("installs").join(id));
    }
    if let Some(id) = install_id_from_active_file(&base) {
        let candidate = base.join("installs").join(id);
        if !roots.contains(&candidate) {
            roots.push(candidate);
        }
    }
    if !roots.contains(&base) {
        roots.push(base);
    }
    roots
}

pub(crate) fn base_data_dir() -> Option<PathBuf> {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .map(|path| path.join("qol-tray"))
}

pub(crate) fn install_id_from_env() -> Option<String> {
    let value = std::env::var("QOL_TRAY_INSTALL_ID").ok()?;
    let trimmed = value.trim();
    if valid_install_id(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

pub(crate) fn install_id_from_active_file(base_data_dir: &std::path::Path) -> Option<String> {
    let content = fs::read_to_string(base_data_dir.join("active-install-id")).ok()?;
    let trimmed = content.trim();
    if valid_install_id(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

pub(crate) fn valid_install_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
