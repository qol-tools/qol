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
