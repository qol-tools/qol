use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};

use crate::dev::state::DiscoveryStatus;
use crate::dev::DevConfig;

use super::dev_services;
use super::dev_validation::sanitize_monitored_plugin_ids;
use super::types::{
    AppState, BuildStateResponse, DiscoveryStateResponse, SetPluginCpuMonitoringRequest,
    ToolingGhAccountPayload,
};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/dev/discover", post(trigger_discovery))
        .route("/dev/discovery-state", get(get_discovery_state))
        .route("/dev/build-state", get(get_build_state))
        .route("/dev/plugin-cpu", get(get_plugin_cpu))
        .route(
            "/dev/plugin-cpu/monitoring",
            axum::routing::put(set_plugin_cpu_monitoring),
        )
        .route(
            "/dev/tooling-gh-account",
            get(get_tooling_gh_account).post(set_tooling_gh_account),
        )
}

pub(super) async fn get_tooling_gh_account() -> Json<ToolingGhAccountPayload> {
    let value = tokio::task::spawn_blocking(|| {
        DevConfig::load()
            .map(|c| c.tooling_gh_account)
            .unwrap_or(None)
    })
    .await
    .unwrap_or(None);
    Json(ToolingGhAccountPayload { value })
}

pub(super) async fn set_tooling_gh_account(
    Json(payload): Json<ToolingGhAccountPayload>,
) -> impl IntoResponse {
    let result =
        tokio::task::spawn_blocking(move || DevConfig::set_tooling_gh_account(payload.value)).await;
    match result {
        Ok(Ok(())) => StatusCode::OK.into_response(),
        Ok(Err(error)) => {
            log::error!("Failed to write tooling_gh_account: {error:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to persist tooling gh account",
            )
                .into_response()
        }
        Err(error) => {
            log::error!("set_tooling_gh_account join error: {}", error);
            (StatusCode::INTERNAL_SERVER_ERROR, "Handler crashed").into_response()
        }
    }
}

pub(super) async fn get_discovery_state(
    State(state): State<AppState>,
) -> Json<DiscoveryStateResponse> {
    let guard = match state.dev_state.discovery.read() {
        Ok(guard) => guard,
        Err(error) => {
            log::error!("Discovery state lock poisoned: {}", error);
            return Json(DiscoveryStateResponse {
                status: "idle".to_string(),
                plugins: Vec::new(),
            });
        }
    };
    let status = match guard.status {
        DiscoveryStatus::Idle => "idle",
        DiscoveryStatus::Discovering => "discovering",
        DiscoveryStatus::Complete => "complete",
    };
    Json(DiscoveryStateResponse {
        status: status.to_string(),
        plugins: guard.plugins.clone(),
    })
}

pub(super) async fn get_build_state(State(state): State<AppState>) -> Json<BuildStateResponse> {
    Json(state.runtime.build_state_snapshot())
}

pub(super) async fn get_plugin_cpu(
    State(state): State<AppState>,
) -> Json<super::dev_plugin_cpu::PluginCpuResponse> {
    Json(state.plugin_cpu.snapshot())
}

pub(super) async fn set_plugin_cpu_monitoring(
    State(state): State<AppState>,
    Json(req): Json<SetPluginCpuMonitoringRequest>,
) -> impl IntoResponse {
    let plugin_ids = match sanitize_monitored_plugin_ids(req.plugin_ids) {
        Ok(plugin_ids) => plugin_ids,
        Err(message) => return (StatusCode::BAD_REQUEST, message).into_response(),
    };
    state.plugin_cpu.set_monitored_plugins(plugin_ids);
    StatusCode::OK.into_response()
}

pub(super) async fn trigger_discovery(State(state): State<AppState>) -> impl IntoResponse {
    log::info!("Discovery refresh requested");
    dev_services::refresh_discovery(&state);
    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn isolated_env() -> (
        tokio::sync::MutexGuard<'static, ()>,
        TempDir,
        crate::paths::TestPathRootGuard,
    ) {
        let guard = crate::test_support::env_lock().lock().await;
        let tmp = TempDir::new().unwrap();
        let path_guard = crate::paths::push_test_path_root(tmp.path());
        (guard, tmp, path_guard)
    }

    fn extract_status(response: axum::response::Response) -> StatusCode {
        response.status()
    }

    #[tokio::test]
    async fn get_returns_none_when_unset() {
        let (_guard, _tmp, _path_guard) = isolated_env().await;

        let Json(payload) = get_tooling_gh_account().await;
        assert_eq!(payload.value, None);
    }

    #[tokio::test]
    async fn post_then_get_round_trips_value() {
        let (_guard, _tmp, _path_guard) = isolated_env().await;

        let response = set_tooling_gh_account(Json(ToolingGhAccountPayload {
            value: Some("KMRH47".to_string()),
        }))
        .await;
        assert_eq!(extract_status(response.into_response()), StatusCode::OK);

        let Json(payload) = get_tooling_gh_account().await;
        assert_eq!(payload.value.as_deref(), Some("KMRH47"));
    }

    #[tokio::test]
    async fn post_with_null_clears_value() {
        let (_guard, _tmp, _path_guard) = isolated_env().await;

        set_tooling_gh_account(Json(ToolingGhAccountPayload {
            value: Some("KMRH47".to_string()),
        }))
        .await;

        let response = set_tooling_gh_account(Json(ToolingGhAccountPayload { value: None })).await;
        assert_eq!(extract_status(response.into_response()), StatusCode::OK);

        let Json(payload) = get_tooling_gh_account().await;
        assert_eq!(payload.value, None);
    }

    #[tokio::test]
    async fn post_trims_and_normalizes_whitespace() {
        let (_guard, _tmp, _path_guard) = isolated_env().await;

        set_tooling_gh_account(Json(ToolingGhAccountPayload {
            value: Some("  octocat  ".to_string()),
        }))
        .await;

        let Json(payload) = get_tooling_gh_account().await;
        assert_eq!(payload.value.as_deref(), Some("octocat"));
    }

    #[tokio::test]
    async fn post_with_empty_string_clears() {
        let (_guard, _tmp, _path_guard) = isolated_env().await;

        set_tooling_gh_account(Json(ToolingGhAccountPayload {
            value: Some("KMRH47".to_string()),
        }))
        .await;

        set_tooling_gh_account(Json(ToolingGhAccountPayload {
            value: Some(String::new()),
        }))
        .await;

        let Json(payload) = get_tooling_gh_account().await;
        assert_eq!(payload.value, None);
    }

    #[test]
    fn payload_deserialization_table() {
        let cases: &[(&str, Option<&str>)] = &[
            (r#"{"value":"KMRH47"}"#, Some("KMRH47")),
            (r#"{"value":null}"#, None),
            (r#"{}"#, None),
            (r#"{"value":"octocat"}"#, Some("octocat")),
        ];
        for (raw, expected) in cases {
            let payload: ToolingGhAccountPayload =
                serde_json::from_str(raw).unwrap_or_else(|e| panic!("failed to parse {raw}: {e}"));
            assert_eq!(payload.value.as_deref(), *expected, "for input {raw}");
        }
    }

    #[test]
    fn payload_serializes_null_explicitly() {
        let none_value = ToolingGhAccountPayload { value: None };
        let serialized = serde_json::to_string(&none_value).unwrap();
        assert_eq!(serialized, r#"{"value":null}"#);

        let some_value = ToolingGhAccountPayload {
            value: Some("KMRH47".to_string()),
        };
        let serialized = serde_json::to_string(&some_value).unwrap();
        assert_eq!(serialized, r#"{"value":"KMRH47"}"#);
    }
}
