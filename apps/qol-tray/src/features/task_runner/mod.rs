use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::RwLock;
use regex::Regex;

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
struct TaskRunnerState {
    config: Arc<RwLock<TaskRunnerConfig>>,
    config_path: PathBuf,
}

#[derive(Serialize)]
struct ActionInfo {
    id: String,
    name: String,
    description: String,
}

#[derive(Serialize)]
struct ActionsResponse {
    actions: Vec<ActionInfo>,
}

#[derive(Deserialize)]
struct ExecuteRequest {
    action: String,
    #[serde(default)]
    params: HashMap<String, String>,
}

#[derive(Serialize)]
struct ExecuteResponse {
    success: bool,
    stdout: String,
    stderr: String,
    #[serde(rename = "exitCode")]
    exit_code: i32,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

pub fn router() -> Router {
    let config_path = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("qol-tray")
        .join(CONFIG_FILENAME);

    let config = load_config(&config_path).unwrap_or_default();

    let state = TaskRunnerState {
        config: Arc::new(RwLock::new(config)),
        config_path,
    };

    Router::new()
        .route("/actions", get(list_actions))
        .route("/execute", post(execute_action))
        .route("/config", get(get_config))
        .route("/config", axum::routing::put(set_config))
        .with_state(state)
}

fn load_config(path: &PathBuf) -> Option<TaskRunnerConfig> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

async fn list_actions(
    State(state): State<TaskRunnerState>,
) -> Json<ActionsResponse> {
    let config = state.config.read().await;
    let actions = config
        .actions
        .iter()
        .map(|(id, action)| ActionInfo {
            id: id.clone(),
            name: action.name.clone(),
            description: action.description.clone(),
        })
        .collect();

    Json(ActionsResponse { actions })
}

async fn execute_action(
    State(state): State<TaskRunnerState>,
    Json(req): Json<ExecuteRequest>,
) -> Result<Json<ExecuteResponse>, (StatusCode, Json<ErrorResponse>)> {
    let config = state.config.read().await;

    let action = config.actions.get(&req.action).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Unknown action: {}", req.action),
            }),
        )
    })?;

    let command = interpolate(&action.command, &req.params);
    let cwd = action.cwd.as_ref().map(|c| interpolate(c, &req.params));
    let timeout = action.timeout;

    log::info!("[task-runner] {}: {}", req.action, command);

    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(&command);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout),
        cmd.output(),
    )
    .await;

    match result {
        Ok(Ok(output)) => {
            let exit_code = output.status.code().unwrap_or(-1);
            Ok(Json(ExecuteResponse {
                success: output.status.success(),
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                exit_code,
            }))
        }
        Ok(Err(e)) => {
            log::error!("[task-runner] Command failed: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Command failed: {}", e),
                }),
            ))
        }
        Err(_) => {
            log::error!("[task-runner] Command timed out after {}s", timeout);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Timeout after {}s", timeout),
                }),
            ))
        }
    }
}

fn interpolate(template: &str, params: &HashMap<String, String>) -> String {
    let re = Regex::new(r"\{\{(\w+)\}\}").unwrap();
    re.replace_all(template, |caps: &regex::Captures| {
        let key = &caps[1];
        params.get(key).cloned().unwrap_or_default()
    })
    .to_string()
}

async fn get_config(
    State(state): State<TaskRunnerState>,
) -> Json<TaskRunnerConfig> {
    let config = state.config.read().await;
    Json(config.clone())
}

async fn set_config(
    State(state): State<TaskRunnerState>,
    Json(new_config): Json<TaskRunnerConfig>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    if let Some(parent) = state.config_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to create config dir: {}", e),
                }),
            )
        })?;
    }

    let content = serde_json::to_string_pretty(&new_config).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to serialize config: {}", e),
            }),
        )
    })?;

    std::fs::write(&state.config_path, content).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to write config: {}", e),
            }),
        )
    })?;

    let mut config = state.config.write().await;
    *config = new_config;

    log::info!("[task-runner] Config saved");
    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolate_single_param() {
        let cases = [
            ("echo {{msg}}", &[("msg", "hello")], "echo hello"),
            ("{{x}}", &[("x", "value")], "value"),
            ("prefix {{a}} suffix", &[("a", "mid")], "prefix mid suffix"),
            ("{{foo}}bar", &[("foo", "baz")], "bazbar"),
            ("bar{{foo}}", &[("foo", "baz")], "barbaz"),
        ];

        for (template, params, expected) in cases {
            let map: HashMap<String, String> = params.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            assert_eq!(interpolate(template, &map), expected, "template: {:?}", template);
        }
    }

    #[test]
    fn interpolate_multiple_params() {
        let cases: &[(&str, &[(&str, &str)], &str)] = &[
            ("{{a}} {{b}}", &[("a", "x"), ("b", "y")], "x y"),
            ("{{x}}{{y}}{{z}}", &[("x", "1"), ("y", "2"), ("z", "3")], "123"),
            ("git checkout {{branch}} && cd {{dir}}", &[("branch", "main"), ("dir", "/a/b")], "git checkout main && cd /a/b"),
            ("{{a}}-{{b}}-{{a}}", &[("a", "X"), ("b", "Y")], "X-Y-X"),
        ];

        for (template, params, expected) in cases {
            let map: HashMap<String, String> = params.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            assert_eq!(interpolate(template, &map), *expected, "template: {:?}", template);
        }
    }

    #[test]
    fn interpolate_missing_params() {
        let cases: &[(&str, &[(&str, &str)], &str)] = &[
            ("{{missing}}", &[], ""),
            ("hello {{name}}", &[], "hello "),
            ("{{a}} {{b}}", &[("a", "x")], "x "),
            ("{{a}}{{missing}}{{b}}", &[("a", "1"), ("b", "2")], "12"),
        ];

        for (template, params, expected) in cases {
            let map: HashMap<String, String> = params.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            assert_eq!(interpolate(template, &map), *expected, "template: {:?}", template);
        }
    }

    #[test]
    fn interpolate_no_placeholders() {
        let cases = [
            ("no placeholders here", "no placeholders here"),
            ("", ""),
            ("echo hello world", "echo hello world"),
            ("{ not a placeholder }", "{ not a placeholder }"),
            ("{single}", "{single}"),
            ("{{}", "{{}"),
            ("}}", "}}"),
        ];

        let empty: HashMap<String, String> = HashMap::new();
        for (template, expected) in cases {
            assert_eq!(interpolate(template, &empty), expected, "template: {:?}", template);
        }
    }

    #[test]
    fn interpolate_special_values() {
        let cases = [
            ("{{path}}", &[("path", "/a/b/c")], "/a/b/c"),
            ("{{url}}", &[("url", "https://example.com?q=1&x=2")], "https://example.com?q=1&x=2"),
            ("{{json}}", &[("json", r#"{"key": "value"}"#)], r#"{"key": "value"}"#),
            ("{{empty}}", &[("empty", "")], ""),
            ("{{spaces}}", &[("spaces", "  a b c  ")], "  a b c  "),
            ("{{unicode}}", &[("unicode", "日本語")], "日本語"),
            ("{{emoji}}", &[("emoji", "🚀")], "🚀"),
            ("{{newline}}", &[("newline", "a\nb\nc")], "a\nb\nc"),
            ("{{tab}}", &[("tab", "a\tb")], "a\tb"),
        ];

        for (template, params, expected) in cases {
            let map: HashMap<String, String> = params.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            assert_eq!(interpolate(template, &map), expected, "template: {:?}", template);
        }
    }

    #[test]
    fn interpolate_invalid_syntax_unchanged() {
        let cases = [
            "{{}}",
            "{{ spaces }}",
            "{{with-dash}}",
            "{{with.dot}}",
            "{{with/slash}}",
            "{single}",
            "{ {double} }",
            "{{nested{{inner}}}}",
            "{{123starts_with_num}}",
        ];

        let params: HashMap<String, String> = [
            ("spaces", "x"),
            ("with-dash", "x"),
            ("with.dot", "x"),
        ].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();

        for template in cases {
            let result = interpolate(template, &params);
            assert!(
                !result.contains("x") || template.contains("x"),
                "invalid placeholder should not be replaced: {:?} -> {:?}",
                template,
                result
            );
        }
    }

    #[test]
    fn interpolate_valid_identifiers() {
        let cases = [
            ("{{a}}", "a"),
            ("{{A}}", "A"),
            ("{{abc}}", "abc"),
            ("{{ABC}}", "ABC"),
            ("{{a1}}", "a1"),
            ("{{var_name}}", "var_name"),
            ("{{CamelCase}}", "CamelCase"),
            ("{{_underscore}}", "_underscore"),
            ("{{a123b456}}", "a123b456"),
        ];

        for (template, key) in cases {
            let map: HashMap<String, String> = [(key.to_string(), "REPLACED".to_string())].into_iter().collect();
            assert_eq!(interpolate(template, &map), "REPLACED", "key {:?} should be valid", key);
        }
    }

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
}
