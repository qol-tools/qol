use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

const CONFIG_FILENAME: &str = "task-runner.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionConfig {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub command: String,
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    #[serde(default)]
    pub cwd: Option<String>,
}

fn default_timeout() -> u64 {
    60
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TaskRunnerConfig {
    #[serde(default)]
    pub actions: HashMap<String, ActionConfig>,
}

#[derive(Clone)]
pub(super) struct TaskRunnerState {
    pub(super) config: Arc<RwLock<TaskRunnerConfig>>,
    pub(super) config_path: PathBuf,
}

pub(super) fn load_state() -> TaskRunnerState {
    let config_path = config_path();
    let config = load_config(&config_path);
    TaskRunnerState {
        config: Arc::new(RwLock::new(config)),
        config_path,
    }
}

fn config_path() -> PathBuf {
    crate::paths::task_runner_config_path().unwrap_or_else(|_| fallback_config_path())
}

fn fallback_config_path() -> PathBuf {
    qol_config::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(CONFIG_FILENAME)
}

fn load_config(path: &Path) -> TaskRunnerConfig {
    read_config(path)
        .and_then(|content| parse_config(&content))
        .unwrap_or_default()
}

fn read_config(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn parse_config(content: &str) -> Option<TaskRunnerConfig> {
    serde_json::from_str(content).ok()
}

pub(super) fn persist_config(
    state: &TaskRunnerState,
    new_config: &TaskRunnerConfig,
) -> Result<(), String> {
    ensure_config_dir(&state.config_path)?;
    let content = serialize_config(new_config)?;
    write_config(&state.config_path, &content)
}

fn ensure_config_dir(path: &Path) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent).map_err(|error| format!("Failed to create config dir: {error}"))
}

fn serialize_config(new_config: &TaskRunnerConfig) -> Result<String, String> {
    serde_json::to_string_pretty(new_config)
        .map_err(|error| format!("Failed to serialize config: {error}"))
}

fn write_config(path: &Path, content: &str) -> Result<(), String> {
    crate::file_io::atomic_write(path, content.as_bytes())
        .map_err(|error| format!("Failed to write config: {error}"))
}

pub(super) async fn replace_config(state: &TaskRunnerState, new_config: TaskRunnerConfig) {
    let mut config = state.config.write().await;
    *config = new_config;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_timeout() {
        assert_eq!(default_timeout(), 60);
    }

    #[test]
    fn config_deserialize_with_defaults() {
        let json = r#"{
            "name": "Test",
            "command": "echo hello"
        }"#;

        let config: ActionConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.name, "Test");
        assert_eq!(config.command, "echo hello");
        assert_eq!(config.description, "");
        assert_eq!(config.timeout, 60);
        assert_eq!(config.cwd, None);
    }

    #[test]
    fn config_deserialize_full() {
        let json = r#"{
            "name": "Full Action",
            "description": "A full config",
            "command": "ls -la",
            "timeout": 120,
            "cwd": "/tmp"
        }"#;

        let config: ActionConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.name, "Full Action");
        assert_eq!(config.description, "A full config");
        assert_eq!(config.command, "ls -la");
        assert_eq!(config.timeout, 120);
        assert_eq!(config.cwd, Some("/tmp".to_string()));
    }

    #[test]
    fn config_serialize_roundtrip() {
        let original = ActionConfig {
            name: "Test".to_string(),
            description: "Desc".to_string(),
            command: "echo {{msg}}".to_string(),
            timeout: 30,
            cwd: Some("/a/b".to_string()),
        };

        let json = serde_json::to_string(&original).unwrap();
        let parsed: ActionConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.name, original.name);
        assert_eq!(parsed.description, original.description);
        assert_eq!(parsed.command, original.command);
        assert_eq!(parsed.timeout, original.timeout);
        assert_eq!(parsed.cwd, original.cwd);
    }

    #[test]
    fn task_runner_config_empty() {
        let config = TaskRunnerConfig::default();
        assert!(config.actions.is_empty());
    }

    #[test]
    fn task_runner_config_deserialize() {
        let json = r#"{
            "actions": {
                "my-action": {
                    "name": "My Action",
                    "command": "echo test"
                }
            }
        }"#;

        let config: TaskRunnerConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.actions.len(), 1);
        assert!(config.actions.contains_key("my-action"));
        assert_eq!(config.actions["my-action"].name, "My Action");
    }

    #[test]
    fn task_runner_config_multiple_actions() {
        let json = r#"{
            "actions": {
                "action1": { "name": "First", "command": "cmd1" },
                "action2": { "name": "Second", "command": "cmd2", "timeout": 10 },
                "action3": { "name": "Third", "command": "cmd3", "cwd": "/x" }
            }
        }"#;

        let config: TaskRunnerConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.actions.len(), 3);
        assert_eq!(config.actions["action1"].timeout, 60);
        assert_eq!(config.actions["action2"].timeout, 10);
        assert_eq!(config.actions["action3"].cwd, Some("/x".to_string()));
    }

    #[test]
    fn fallback_config_path_uses_qol_config_namespace_when_available() {
        let path = fallback_config_path();
        assert!(
            path.ends_with(CONFIG_FILENAME),
            "expected {CONFIG_FILENAME} leaf, got {path:?}"
        );
        if path != PathBuf::from(".").join(CONFIG_FILENAME) {
            let parent = path.parent().expect("config path has a parent");
            assert!(
                parent.ends_with(qol_config::NAMESPACE),
                "expected parent under {} namespace, got {parent:?}",
                qol_config::NAMESPACE
            );
        }
    }
}
