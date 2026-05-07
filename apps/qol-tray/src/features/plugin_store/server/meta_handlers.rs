use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};

use super::types::AppState;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/dev/enabled", get(dev_enabled))
        .route("/version", get(get_version))
        .route("/check-update", get(check_update))
        .route("/self-update", post(self_update))
}

pub(super) async fn dev_enabled() -> Json<bool> {
    let mode_is_dev = crate::mode::ModeConfig::load()
        .map(|c| c.is_dev())
        .unwrap_or(false);
    Json(cfg!(feature = "dev") && mode_is_dev)
}

pub(super) async fn get_version() -> &'static str {
    #[cfg(feature = "dev")]
    if let Some(v) = crate::version::test_version_override() {
        return v;
    }
    env!("CARGO_PKG_VERSION")
}

pub(super) async fn check_update() -> Json<serde_json::Value> {
    let available = crate::updates::check_for_updates().await.unwrap_or(false);
    let latest = crate::updates::latest_version().map(String::from);
    Json(serde_json::json!({ "available": available, "latest": latest }))
}

pub(super) async fn self_update(State(state): State<AppState>) -> impl IntoResponse {
    let events = state.daemon.events.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::updates::download_and_install(events.clone()).await {
            log::error!("Self-update failed: {}", e);
            events.send(crate::daemon::DaemonEvent::UpdateFailed {
                message: e.to_string(),
            });
        }
    });
    StatusCode::ACCEPTED
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::{ModeConfig, ModeFlag};
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

    #[tokio::test]
    async fn dev_enabled_defaults_false_when_mode_config_missing() {
        let (_guard, _tmp, _path_guard) = isolated_env().await;

        let Json(enabled) = dev_enabled().await;
        assert!(!enabled);
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
