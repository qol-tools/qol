use super::types::AppState;
use crate::updates::jobs::Operation;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};

type BuildInfoErrorResponse = (StatusCode, Json<serde_json::Value>);

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(qol_conventions::dev_routes::ENABLED, get(dev_enabled))
        .route("/build-info", get(get_build_info))
        .route("/version", get(get_version))
        .route("/check-update", get(check_update))
        .route("/self-update", post(self_update))
        .route(qol_conventions::SHUTDOWN_ROUTE, post(shutdown))
}

pub(super) async fn dev_enabled() -> Json<bool> {
    Json(current_dev().await)
}

async fn current_dev() -> bool {
    let mode_is_dev = tokio::task::spawn_blocking(|| {
        crate::mode::ModeConfig::load().unwrap_or_default().is_dev()
    })
    .await
    .unwrap_or(false);
    cfg!(feature = "dev") && mode_is_dev
}

pub(super) async fn get_version() -> &'static str {
    #[cfg(feature = "dev")]
    if let Some(v) = crate::version::test_version_override() {
        return v;
    }
    env!("CARGO_PKG_VERSION")
}

pub(super) async fn get_build_info(
) -> Result<Json<qol_conventions::artifact::RunningBuildInfo>, BuildInfoErrorResponse> {
    let identity = qol_conventions::artifact::current()
        .cloned()
        .ok_or_else(|| build_info_error("running build identity is unavailable"))?;
    let executable = std::env::current_exe()
        .map_err(|error| build_info_error(&format!("cannot resolve executable: {error}")))?;
    Ok(Json(qol_conventions::artifact::RunningBuildInfo {
        identity,
        executable: qol_conventions::artifact::normalized_executable(executable),
    }))
}

fn build_info_error(message: &str) -> BuildInfoErrorResponse {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({ "error": message })),
    )
}

pub(super) async fn check_update() -> Json<serde_json::Value> {
    let available = crate::updates::check_for_updates_force()
        .await
        .unwrap_or(false);
    let latest = crate::updates::latest_version();
    Json(serde_json::json!({ "available": available, "latest": latest }))
}

pub(super) async fn self_update() -> impl IntoResponse {
    let operation = Operation::Host {
        confirm_after_restart: false,
    };
    match crate::updates::jobs::push(crate::updates::jobs::HOST_ID, operation) {
        Ok(()) => StatusCode::ACCEPTED,
        Err(message) => {
            log::warn!("Self-update refused: {}", message);
            StatusCode::CONFLICT
        }
    }
}

async fn shutdown(State(state): State<AppState>) -> StatusCode {
    log::info!("[lifecycle] graceful shutdown requested by local API");
    crate::tray::platform::request_shutdown(&state.shutdown_tx);
    StatusCode::ACCEPTED
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::{ModeConfig, ModeFlag};
    use tempfile::TempDir;

    #[cfg(feature = "dev")]
    type TestRootGuard = crate::paths::TestEnvPathRootGuard;
    #[cfg(not(feature = "dev"))]
    type TestRootGuard = crate::paths::TestPathRootGuard;

    #[tokio::test]
    async fn build_info_fails_closed_when_the_binary_did_not_register_identity() {
        let (status, Json(body)) = get_build_info().await.unwrap_err();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"], "running build identity is unavailable");
    }

    async fn isolated_env() -> (tokio::sync::MutexGuard<'static, ()>, TempDir, TestRootGuard) {
        let guard = crate::test_support::env_lock().lock().await;
        let tmp = TempDir::new().unwrap();
        #[cfg(feature = "dev")]
        let path_guard = crate::paths::push_test_env_path_root(tmp.path());
        #[cfg(not(feature = "dev"))]
        let path_guard = crate::paths::push_test_path_root(tmp.path());
        (guard, tmp, path_guard)
    }

    #[tokio::test]
    async fn dev_enabled_default_matches_capability_when_mode_config_missing() {
        let (_guard, _tmp, _path_guard) = isolated_env().await;

        let Json(enabled) = dev_enabled().await;
        assert_eq!(enabled, cfg!(feature = "dev"));
    }

    #[cfg(feature = "dev")]
    #[tokio::test]
    async fn dev_enabled_true_when_dev_feature_and_mode_dev() {
        let (_guard, _tmp, _path_guard) = isolated_env().await;

        ModeConfig::set(ModeFlag::Dev).unwrap();

        let Json(enabled) = dev_enabled().await;
        assert!(enabled);
    }

    #[cfg(not(feature = "dev"))]
    #[tokio::test]
    async fn dev_enabled_false_when_mode_dev_without_dev_feature() {
        let (_guard, _tmp, _path_guard) = isolated_env().await;

        ModeConfig::set(ModeFlag::Dev).unwrap();

        let Json(enabled) = dev_enabled().await;
        assert!(!enabled);
    }
}
