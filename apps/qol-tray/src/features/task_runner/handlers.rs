use super::config::{self, ActionConfig, TaskRunnerConfig, TaskRunnerState};
use super::execution::{self, ExecutionRequest, ExecutionResult};
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
struct ExecuteRequestBody {
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

type HandlerError = (StatusCode, Json<ErrorResponse>);
type HandlerResult<T> = Result<T, HandlerError>;

pub(super) fn router(state: TaskRunnerState) -> Router {
    Router::new()
        .route("/actions", get(list_actions))
        .route("/execute", post(execute_action))
        .route("/config", get(get_config))
        .route("/config", axum::routing::put(set_config))
        .with_state(state)
}

async fn list_actions(State(state): State<TaskRunnerState>) -> Json<ActionsResponse> {
    let config = state.config.read().await;
    Json(actions_response(&config))
}

fn actions_response(config: &TaskRunnerConfig) -> ActionsResponse {
    let actions = config
        .actions
        .iter()
        .map(|(id, action)| action_info(id, action))
        .collect();
    ActionsResponse { actions }
}

fn action_info(id: &str, action: &ActionConfig) -> ActionInfo {
    ActionInfo {
        id: id.to_string(),
        name: action.name.clone(),
        description: action.description.clone(),
    }
}

async fn execute_action(
    State(state): State<TaskRunnerState>,
    Json(req): Json<ExecuteRequestBody>,
) -> HandlerResult<Json<ExecuteResponse>> {
    let action = configured_action(&state, &req.action).await?;
    let request = ExecutionRequest::new(&req.action, &action, &req.params);
    let result = execution::execute(request).await.map_err(internal_error)?;
    Ok(Json(ExecuteResponse::from(result)))
}

async fn configured_action(
    state: &TaskRunnerState,
    action_id: &str,
) -> HandlerResult<ActionConfig> {
    let config = state.config.read().await;
    config
        .actions
        .get(action_id)
        .cloned()
        .ok_or_else(|| bad_request(format!("Unknown action: {action_id}")))
}

impl From<ExecutionResult> for ExecuteResponse {
    fn from(result: ExecutionResult) -> Self {
        Self {
            success: result.success,
            stdout: result.stdout,
            stderr: result.stderr,
            exit_code: result.exit_code,
        }
    }
}

async fn get_config(State(state): State<TaskRunnerState>) -> Json<TaskRunnerConfig> {
    let config = state.config.read().await;
    Json(config.clone())
}

async fn set_config(
    State(state): State<TaskRunnerState>,
    Json(new_config): Json<TaskRunnerConfig>,
) -> HandlerResult<StatusCode> {
    config::persist_config(&state, &new_config).map_err(internal_error)?;
    config::replace_config(&state, new_config).await;
    log::info!("[task-runner] Config saved");
    Ok(StatusCode::OK)
}

fn bad_request(error: String) -> HandlerError {
    (StatusCode::BAD_REQUEST, Json(ErrorResponse { error }))
}

fn internal_error(error: String) -> HandlerError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse { error }),
    )
}
