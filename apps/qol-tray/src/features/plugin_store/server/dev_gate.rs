use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};

pub(super) async fn require_dev_mode(request: Request, next: Next) -> Response {
    if dev_routes_enabled() {
        return next.run(request).await;
    }
    StatusCode::NOT_FOUND.into_response()
}

fn dev_routes_enabled() -> bool {
    crate::mode::ModeConfig::load().unwrap_or_default().is_dev()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::{ModeConfig, ModeFlag};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    #[test]
    fn dev_routes_default_matches_capability_when_mode_file_missing() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        assert_eq!(dev_routes_enabled(), cfg!(feature = "dev"));
    }

    #[test]
    fn dev_routes_follow_mode_config() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        ModeConfig::set(ModeFlag::Dev).unwrap();
        assert!(dev_routes_enabled());

        ModeConfig::set(ModeFlag::Prod).unwrap();
        assert!(!dev_routes_enabled());
    }

    fn gated_router() -> Router {
        Router::new()
            .route("/probe", get(|| async { "ok" }))
            .route_layer(axum::middleware::from_fn(require_dev_mode))
    }

    async fn probe_status(router: Router) -> StatusCode {
        router
            .oneshot(Request::get("/probe").body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn route_layer_404s_in_prod_mode() {
        let _guard = crate::test_support::env_lock().lock().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        ModeConfig::set(ModeFlag::Prod).unwrap();

        assert_eq!(probe_status(gated_router()).await, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn route_layer_passes_in_dev_mode() {
        let _guard = crate::test_support::env_lock().lock().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        ModeConfig::set(ModeFlag::Dev).unwrap();

        assert_eq!(probe_status(gated_router()).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn route_layer_returns_404_for_unmatched_paths() {
        let _guard = crate::test_support::env_lock().lock().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        ModeConfig::set(ModeFlag::Dev).unwrap();

        let status = gated_router()
            .oneshot(Request::get("/missing").body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status();
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
