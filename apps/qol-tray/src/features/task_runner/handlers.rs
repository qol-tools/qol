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

#[cfg(test)]
mod tests {
    use super::*;

    fn action(name: &str, description: &str, command: &str) -> ActionConfig {
        ActionConfig {
            name: name.to_string(),
            description: description.to_string(),
            command: command.to_string(),
            timeout: 60,
            cwd: None,
        }
    }

    #[test]
    fn action_info_serializes_id_name_and_description() {
        let info = action_info("build", &action("Build", "Run build script", "make build"));
        let value = serde_json::to_value(&info).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "id": "build",
                "name": "Build",
                "description": "Run build script",
            }),
            "ActionInfo serialization is the public API the browser extension consumes",
        );
    }

    #[test]
    fn actions_response_lists_every_configured_action() {
        let mut actions = HashMap::new();
        actions.insert("build".to_string(), action("Build", "", "make build"));
        actions.insert("test".to_string(), action("Test", "", "make test"));
        actions.insert("lint".to_string(), action("Lint", "", "make lint"));
        let config = TaskRunnerConfig { actions };

        let response = actions_response(&config);

        assert_eq!(response.actions.len(), 3);
        let mut ids: Vec<_> = response.actions.iter().map(|a| a.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["build", "lint", "test"]);
    }

    #[test]
    fn actions_response_yields_empty_list_for_empty_config() {
        let response = actions_response(&TaskRunnerConfig::default());
        assert!(response.actions.is_empty());
    }

    #[test]
    fn execute_response_serializes_with_camelcase_exit_code() {
        let result = ExecutionResult {
            success: false,
            stdout: "out".to_string(),
            stderr: "err".to_string(),
            exit_code: 42,
        };
        let response: ExecuteResponse = result.into();
        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "success": false,
                "stdout": "out",
                "stderr": "err",
                "exitCode": 42,
            }),
            "exitCode (camelCase) is the contract the browser extension reads",
        );
    }

    #[test]
    fn execute_request_body_defaults_to_empty_params() {
        let body: ExecuteRequestBody = serde_json::from_str(r#"{"action":"build"}"#).unwrap();
        assert_eq!(body.action, "build");
        assert!(body.params.is_empty());
    }

    #[test]
    fn execute_request_body_accepts_explicit_params_map() {
        let body: ExecuteRequestBody = serde_json::from_str(
            r#"{"action":"deploy","params":{"target":"staging","tag":"v1.2.3"}}"#,
        )
        .unwrap();
        assert_eq!(body.action, "deploy");
        assert_eq!(
            body.params.get("target").map(String::as_str),
            Some("staging")
        );
        assert_eq!(body.params.get("tag").map(String::as_str), Some("v1.2.3"));
    }

    #[test]
    fn bad_request_produces_400_status_with_error_body() {
        let (status, Json(body)) = bad_request("Unknown action: foo".to_string());
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.error, "Unknown action: foo");
    }

    #[test]
    fn internal_error_produces_500_status_with_error_body() {
        let (status, Json(body)) = internal_error("disk full".to_string());
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body.error, "disk full");
    }
}
