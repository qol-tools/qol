use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;

use super::types::AppState;
use crate::daemon::DaemonEvent;

type BuildInfoErrorResponse = (StatusCode, Json<serde_json::Value>);

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(qol_conventions::dev_routes::ENABLED, get(dev_enabled))
        .route("/build-info", get(get_build_info))
        .route("/version", get(get_version))
        .route("/check-update", get(check_update))
        .route("/self-update", post(self_update))
        .route("/navigate", post(navigate))
        .route(qol_conventions::SHUTDOWN_ROUTE, post(shutdown))
}

#[derive(Deserialize)]
pub(super) struct NavigateBody {
    pub route: String,
}

/// A navigate request is only delivered when at least one UI tab is
/// subscribed to the event stream; otherwise the caller (e.g. `qol-tray
/// open`) falls back to opening a fresh browser tab. Delivery is best-effort:
/// the subscriber count is a snapshot, so a tab reconnecting in the gap may
/// miss the event. The worst case is one extra browser tab, never a crash.
fn decide_delivery(subscriber_count: usize) -> bool {
    subscriber_count > 0
}

fn route_is_valid(route: &str) -> bool {
    !route.trim().is_empty()
}

pub(super) async fn navigate(
    State(state): State<AppState>,
    Json(body): Json<NavigateBody>,
) -> impl IntoResponse {
    if !route_is_valid(&body.route) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "route required" })),
        );
    }
    let delivered = decide_delivery(state.daemon.events.subscriber_count());
    if delivered {
        state
            .daemon
            .events
            .send(DaemonEvent::Navigate { route: body.route });
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({ "delivered": delivered })),
    )
}

pub(super) async fn dev_enabled() -> Json<bool> {
    Json(super::boot::current_dev().await)
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
        executable,
    }))
}

fn build_info_error(message: &str) -> BuildInfoErrorResponse {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({ "error": message })),
    )
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

    #[test]
    fn decide_delivery_requires_a_subscriber() {
        assert!(!decide_delivery(0));
        assert!(decide_delivery(1));
        assert!(decide_delivery(5));
    }

    #[test]
    fn route_validity_rejects_blank() {
        assert!(!route_is_valid(""));
        assert!(!route_is_valid("   "));
        assert!(route_is_valid("shortcuts"));
        assert!(route_is_valid("shortcuts/add?type=url"));
    }

    #[tokio::test]
    async fn build_info_fails_closed_when_the_binary_did_not_register_identity() {
        let (status, Json(body)) = get_build_info().await.unwrap_err();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"], "running build identity is unavailable");
    }

    #[tokio::test]
    async fn navigate_event_with_route_reaches_a_live_subscriber() {
        use crate::daemon::EventBus;
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        assert!(decide_delivery(bus.subscriber_count()));
        if decide_delivery(bus.subscriber_count()) {
            bus.send(DaemonEvent::Navigate {
                route: "shortcuts/add?type=url".to_string(),
            });
        }
        match rx.try_recv() {
            Ok(DaemonEvent::Navigate { route }) => assert_eq!(route, "shortcuts/add?type=url"),
            other => panic!("expected a Navigate event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn navigate_is_not_sent_without_subscribers() {
        use crate::daemon::EventBus;
        let bus = EventBus::new();
        assert!(!decide_delivery(bus.subscriber_count()));
        let mut rx = bus.subscribe();
        assert!(rx.try_recv().is_err());
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
