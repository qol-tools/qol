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

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/dev/enabled", get(dev_enabled))
        .route("/version", get(get_version))
        .route("/check-update", get(check_update))
        .route("/self-update", post(self_update))
        .route("/navigate", post(navigate))
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
    async fn navigate_event_with_route_reaches_a_live_subscriber() {
        use crate::daemon::EventBus;
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        assert!(decide_delivery(bus.subscriber_count()));
        // Mirror the handler: deliver only when a subscriber is present.
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
        // A late subscriber must not receive an event that the handler skipped.
        let mut rx = bus.subscribe();
        assert!(rx.try_recv().is_err());
    }

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
