use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

const LOG_CONTROL_STATE_FILE: &str = "dev-plugin-log-controls.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct PluginLogControl {
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub suppress_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PluginLogControlState {
    #[serde(default)]
    plugins: HashMap<String, PluginLogControl>,
}

pub fn load_all_controls(config_dir: &Path) -> HashMap<String, PluginLogControl> {
    let state_path = config_dir.join(LOG_CONTROL_STATE_FILE);
    let Ok(content) = std::fs::read_to_string(&state_path) else {
        return HashMap::new();
    };

    serde_json::from_str::<PluginLogControlState>(&content)
        .map(|state| state.plugins)
        .unwrap_or_default()
}

pub fn save_all_controls(
    config_dir: &Path,
    controls: &HashMap<String, PluginLogControl>,
) -> Result<(), String> {
    if let Err(e) = std::fs::create_dir_all(config_dir) {
        return Err(format!(
            "Failed to create config directory {}: {}",
            config_dir.display(),
            e
        ));
    }

    let state_path = config_dir.join(LOG_CONTROL_STATE_FILE);
    let tmp_path = config_dir.join(".dev-plugin-log-controls.tmp");
    let state = PluginLogControlState {
        plugins: controls.clone(),
    };
    let content = serde_json::to_string_pretty(&state)
        .map_err(|e| format!("Failed to serialize plugin log controls: {}", e))?;

    std::fs::write(&tmp_path, content)
        .map_err(|e| format!("Failed to write plugin log control temp file: {}", e))?;
    std::fs::rename(&tmp_path, &state_path)
        .map_err(|e| format!("Failed to finalize plugin log control file: {}", e))
}

pub fn load_control(config_dir: &Path, plugin_id: &str) -> PluginLogControl {
    load_all_controls(config_dir)
        .remove(plugin_id)
        .unwrap_or_default()
}

pub fn load_control_from_shared_config(plugin_id: &str) -> PluginLogControl {
    let Ok(config_dir) = crate::paths::shared_config_dir() else {
        return PluginLogControl::default();
    };
    load_control(&config_dir, plugin_id)
}

pub fn upsert_control(
    config_dir: &Path,
    plugin_id: &str,
    mut control: PluginLogControl,
) -> Result<(), String> {
    control.suppress_patterns = normalize_patterns(control.suppress_patterns);

    let mut controls = load_all_controls(config_dir);
    if control.muted || !control.suppress_patterns.is_empty() {
        controls.insert(plugin_id.to_string(), control);
    } else {
        controls.remove(plugin_id);
    }

    save_all_controls(config_dir, &controls)
}

fn normalize_patterns(patterns: Vec<String>) -> Vec<String> {
    use std::collections::HashSet;

    let mut seen = HashSet::new();
    let mut normalized = Vec::new();

    for pattern in patterns {
        let trimmed = pattern.trim();
        if trimmed.is_empty() {
            continue;
        }
        let capped = if trimmed.len() > 160 {
            trimmed[..160].to_string()
        } else {
            trimmed.to_string()
        };

        if seen.insert(capped.clone()) {
            normalized.push(capped);
        }
    }

    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn upsert_control_roundtrip_and_clear() {
        let tmp = TempDir::new().unwrap();

        upsert_control(
            tmp.path(),
            "plugin-a",
            PluginLogControl {
                muted: false,
                suppress_patterns: vec![
                    " [spam] ".to_string(),
                    "".to_string(),
                    "[spam]".to_string(),
                ],
            },
        )
        .unwrap();

        let loaded = load_control(tmp.path(), "plugin-a");
        assert!(!loaded.muted);
        assert_eq!(loaded.suppress_patterns, vec!["[spam]".to_string()]);

        upsert_control(
            tmp.path(),
            "plugin-a",
            PluginLogControl {
                muted: false,
                suppress_patterns: vec![],
            },
        )
        .unwrap();

        let loaded_after_clear = load_control(tmp.path(), "plugin-a");
        assert_eq!(loaded_after_clear, PluginLogControl::default());
    }
}
