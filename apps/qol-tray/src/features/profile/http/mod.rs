mod import_export;
mod sync;

use axum::{
    body::Bytes,
    extract::FromRef,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::de::DeserializeOwned;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const MAX_PROFILE_REQUEST_SIZE: usize = 1024 * 1024;

#[derive(Clone)]
pub(crate) struct ProfileHttpState {
    pub(crate) plugins_dir: PathBuf,
    pub(crate) plugin_manager: Arc<Mutex<crate::plugins::PluginManager>>,
    pub(crate) daemon: crate::daemon::Daemon,
    pub(crate) sync_service: Arc<crate::features::profile::sync::SyncService>,
}

pub(crate) fn routes<S>() -> Router<S>
where
    ProfileHttpState: FromRef<S>,
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/config/export", get(import_export::export_config))
        .route("/config/import", post(import_export::import_config))
        .route("/sync/providers", get(sync::get_sync_providers))
        .route("/sync/status", get(sync::get_sync_status))
        .route(
            "/sync/github/branches",
            post(sync::list_sync_github_branches),
        )
        .route("/sync/connect", post(sync::connect_sync))
        .route("/sync/pull", post(sync::pull_sync))
        .route("/sync/push", post(sync::push_sync))
        .route("/sync/disconnect", post(sync::disconnect_sync))
        .route("/sync/acknowledge", post(sync::acknowledge_sync))
        .route("/sync/backups", get(sync::list_sync_backups))
        .route("/sync/backups/open-dir", post(sync::open_sync_backups_dir))
        .route("/sync/backups/{file_name}", get(sync::preview_sync_backup))
}

fn parse_json_body<T: DeserializeOwned>(body: Bytes) -> Result<T, Box<Response>> {
    if body.len() > MAX_PROFILE_REQUEST_SIZE {
        return Err(Box::new(config_too_large_response()));
    }
    serde_json::from_slice(&body).map_err(|_| Box::new(invalid_json_response()))
}

fn reload_after_profile_apply(state: &ProfileHttpState) {
    let mut manager = match state.plugin_manager.lock() {
        Ok(manager) => manager,
        Err(error) => {
            log::error!("Plugin manager mutex poisoned: {}", error);
            return;
        }
    };
    let reload_ok = match manager.reload_plugins() {
        Ok(()) => true,
        Err(error) => {
            log::error!("Failed to reload plugins: {}", error);
            false
        }
    };
    if reload_ok {
        crate::features::launcher_apps::trigger_full_sync();
    }
    drop(manager);
    crate::hotkeys::trigger_reload();
    state.daemon.events.send_plugins_changed();
}

fn invalid_json_response() -> Response {
    (StatusCode::BAD_REQUEST, "Invalid JSON").into_response()
}

fn config_too_large_response() -> Response {
    (StatusCode::PAYLOAD_TOO_LARGE, "Config too large").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Payload {
        value: String,
    }

    #[test]
    fn parse_json_body_rejects_oversize_requests() {
        let body = Bytes::from(vec![b'x'; MAX_PROFILE_REQUEST_SIZE + 1]);
        let result = parse_json_body::<Payload>(body);
        assert!(result.is_err());
    }

    #[test]
    fn parse_json_body_accepts_valid_json_within_limit() {
        let body = Bytes::from_static(br#"{"value":"ok"}"#);
        let result = parse_json_body::<Payload>(body).unwrap();
        assert_eq!(result.value, "ok");
    }
}
