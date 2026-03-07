use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

const LOG_CONTROL_STATE_FILE: &str = "dev-plugin-log-controls.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct LogControl {
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub suppress_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PluginLogControlFile {
    #[serde(default)]
    plugins: HashMap<String, LogControl>,
}

pub fn load_all_plugin_controls(config_dir: &Path) -> HashMap<String, LogControl> {
    let state_path = config_dir.join(LOG_CONTROL_STATE_FILE);
    let Ok(content) = std::fs::read_to_string(&state_path) else {
        return HashMap::new();
    };

    serde_json::from_str::<PluginLogControlFile>(&content)
        .map(|state| state.plugins)
        .unwrap_or_default()
}

pub fn save_all_plugin_controls(
    config_dir: &Path,
    controls: &HashMap<String, LogControl>,
) -> Result<(), String> {
    let state = PluginLogControlFile {
        plugins: controls.clone(),
    };
    save_controls_file(config_dir, LOG_CONTROL_STATE_FILE, &state)
}

fn save_controls_file(
    config_dir: &Path,
    filename: &str,
    state: &impl Serialize,
) -> Result<(), String> {
    std::fs::create_dir_all(config_dir).map_err(|e| {
        format!(
            "Failed to create config directory {}: {}",
            config_dir.display(),
            e
        )
    })?;
    let path = config_dir.join(filename);
    let tmp_path = config_dir.join(format!(".{}.tmp", filename));
    let content = serde_json::to_string_pretty(state)
        .map_err(|e| format!("Failed to serialize {}: {}", filename, e))?;
    std::fs::write(&tmp_path, &content)
        .map_err(|e| format!("Failed to write {}: {}", filename, e))?;
    std::fs::rename(&tmp_path, &path).map_err(|e| format!("Failed to finalize {}: {}", filename, e))
}

pub fn load_plugin_control(config_dir: &Path, plugin_id: &str) -> LogControl {
    load_all_plugin_controls(config_dir)
        .remove(plugin_id)
        .unwrap_or_default()
}

pub fn load_plugin_control_from_shared_config(plugin_id: &str) -> LogControl {
    let Ok(config_dir) = crate::paths::shared_config_dir() else {
        return LogControl::default();
    };
    load_plugin_control(&config_dir, plugin_id)
}

pub fn upsert_plugin_control(
    config_dir: &Path,
    plugin_id: &str,
    control: LogControl,
) -> Result<(), String> {
    let mut controls = load_all_plugin_controls(config_dir);
    upsert_control_entry(&mut controls, plugin_id, control);
    save_all_plugin_controls(config_dir, &controls)
}

fn upsert_control_entry(
    controls: &mut HashMap<String, LogControl>,
    key: &str,
    mut control: LogControl,
) {
    control.suppress_patterns = normalize_patterns(control.suppress_patterns);
    if !control.muted && control.suppress_patterns.is_empty() {
        controls.remove(key);
        return;
    }
    controls.insert(key.to_string(), control);
}

fn normalize_patterns(patterns: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    patterns
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|p| p.chars().take(160).collect::<String>())
        .filter(|p| seen.insert(p.clone()))
        .collect()
}

#[cfg(feature = "dev")]
const CORE_LOG_CONTROL_STATE_FILE: &str = "dev-core-log-controls.json";

#[cfg(feature = "dev")]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CoreLogControlFile {
    #[serde(default)]
    sections: HashMap<String, LogControl>,
}

#[cfg(feature = "dev")]
pub(super) fn load_all_core_controls(config_dir: &Path) -> HashMap<String, LogControl> {
    let path = config_dir.join(CORE_LOG_CONTROL_STATE_FILE);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };
    serde_json::from_str::<CoreLogControlFile>(&content)
        .map(|state| state.sections)
        .unwrap_or_default()
}

#[cfg(feature = "dev")]
fn save_all_core_controls(
    config_dir: &Path,
    controls: &HashMap<String, LogControl>,
) -> Result<(), String> {
    let state = CoreLogControlFile {
        sections: controls.clone(),
    };
    save_controls_file(config_dir, CORE_LOG_CONTROL_STATE_FILE, &state)
}

#[cfg(feature = "dev")]
pub fn upsert_core_control(
    config_dir: &Path,
    section: &str,
    control: LogControl,
) -> Result<(), String> {
    let mut controls = load_all_core_controls(config_dir);
    upsert_control_entry(&mut controls, section, control);
    save_all_core_controls(config_dir, &controls)
}

pub(crate) fn matches_any_pattern(text: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| text.contains(p.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn upsert_plugin_control_roundtrip_and_clear() {
        let tmp = TempDir::new().unwrap();

        upsert_plugin_control(
            tmp.path(),
            "foo",
            LogControl {
                muted: false,
                suppress_patterns: vec![
                    " [spam] ".to_string(),
                    "".to_string(),
                    "[spam]".to_string(),
                ],
            },
        )
        .unwrap();

        let loaded = load_plugin_control(tmp.path(), "foo");
        assert!(!loaded.muted);
        assert_eq!(loaded.suppress_patterns, vec!["[spam]".to_string()]);

        upsert_plugin_control(
            tmp.path(),
            "foo",
            LogControl {
                muted: false,
                suppress_patterns: vec![],
            },
        )
        .unwrap();

        let loaded_after_clear = load_plugin_control(tmp.path(), "foo");
        assert_eq!(loaded_after_clear, LogControl::default());
    }

    #[cfg(feature = "dev")]
    #[test]
    fn upsert_core_control_roundtrip_and_clear() {
        let tmp = TempDir::new().unwrap();

        upsert_core_control(
            tmp.path(),
            "runtime",
            LogControl {
                muted: true,
                suppress_patterns: vec![],
            },
        )
        .unwrap();

        let loaded = load_all_core_controls(tmp.path());
        assert_eq!(loaded.len(), 1, "expected 1 section after upsert");
        assert!(loaded["runtime"].muted, "runtime should be muted");

        upsert_core_control(
            tmp.path(),
            "runtime",
            LogControl {
                muted: false,
                suppress_patterns: vec![],
            },
        )
        .unwrap();

        let cleared = load_all_core_controls(tmp.path());
        assert!(cleared.is_empty(), "expected empty after clearing");
    }
}
