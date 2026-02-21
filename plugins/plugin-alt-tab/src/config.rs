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
        Self { max_columns: 6 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActionMode {
    #[default]
    Sticky,
    HoldToSwitch,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AltTabConfig {
    pub display: DisplayConfig,
    pub action_mode: ActionMode,
}

pub fn load_alt_tab_config() -> AltTabConfig {
    for path in config_paths() {
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };
        match serde_json::from_str::<AltTabConfig>(&contents) {
            Ok(config) => {
                println!("Loaded config from {}: {:?}", path.display(), config);
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

fn config_paths() -> Vec<PathBuf> {
    const INSTALL_RELATIVE_CONFIG_PATHS: [&str; 2] = [
        "plugins/plugin-alt-tab/config.json",
        "plugins/alt-tab/config.json",
    ];
    const LEGACY_RELATIVE_CONFIG_PATHS: [&str; 2] = [
        "qol-tray/plugins/plugin-alt-tab/config.json",
        "qol-tray/plugins/alt-tab/config.json",
    ];

    let mut paths = Vec::new();

    for root in install_config_roots() {
        for relative in INSTALL_RELATIVE_CONFIG_PATHS {
            let candidate = root.join(relative);
            if !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
    }

    let mut roots = Vec::new();
    if let Ok(xdg_config_home) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg_config_home.trim().is_empty() {
            roots.push(PathBuf::from(xdg_config_home));
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.trim().is_empty() {
            roots.push(PathBuf::from(home).join(".config"));
        }
    }

    for root in roots {
        for relative in LEGACY_RELATIVE_CONFIG_PATHS {
            let candidate = root.join(relative);
            if !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
    }

    paths
}

fn install_config_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let Some(base_data_dir) = base_data_dir() else {
        return roots;
    };
    if let Some(install_id) = install_id_from_env() {
        let candidate = base_data_dir.join("installs").join(install_id);
        if !roots.contains(&candidate) {
            roots.push(candidate);
        }
    }
    if let Some(install_id) = install_id_from_active_file(&base_data_dir) {
        let candidate = base_data_dir.join("installs").join(install_id);
        if !roots.contains(&candidate) {
            roots.push(candidate);
        }
    }
    roots
}

fn base_data_dir() -> Option<PathBuf> {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .map(|path| path.join("qol-tray"))
}

fn install_id_from_env() -> Option<String> {
    let value = std::env::var("QOL_TRAY_INSTALL_ID").ok()?;
    let trimmed = value.trim();
    if valid_install_id(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn install_id_from_active_file(base_data_dir: &std::path::Path) -> Option<String> {
    let content = fs::read_to_string(base_data_dir.join("active-install-id")).ok()?;
    let trimmed = content.trim();
    if valid_install_id(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn valid_install_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
