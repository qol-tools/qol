use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::daemon::ConfigKind;
use crate::hotkeys::{get_registration_errors, HotkeyConfig, HotkeyManager};

use super::super::types::{AppState, MAX_CONFIG_SIZE};
use super::http_json;

type HttpResult<T> = Result<T, Box<Response>>;

#[derive(Serialize)]
pub(in super::super) struct HotkeyRecordingStatus {
    native: bool,
}

#[derive(Serialize)]
pub(in super::super) struct HotkeyCaptureResult {
    native: bool,
    key: Option<String>,
    canceled: bool,
}

pub(in super::super) async fn get_hotkeys() -> impl IntoResponse {
    blocking(get_hotkeys_inner).await
}

pub(in super::super) async fn set_hotkeys(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    blocking(move || set_hotkeys_inner(&state, body)).await
}

pub(in super::super) async fn open_hotkeys_file() -> impl IntoResponse {
    let path = crate::paths::hotkeys_path();
    blocking_open(move || open_config_file(path)).await
}

pub(in super::super) async fn open_shortcuts_file() -> impl IntoResponse {
    let path = crate::paths::shortcuts_path();
    blocking_open(move || open_config_file(path)).await
}

pub(in super::super) async fn start_hotkey_recording(
    Path(session_id): Path<u64>,
    State(state): State<AppState>,
) -> axum::Json<HotkeyRecordingStatus> {
    axum::Json(HotkeyRecordingStatus {
        native: crate::hotkeys::start_recording(session_id, state.daemon.events.clone()),
    })
}

pub(in super::super) async fn cancel_hotkey_recording(Path(session_id): Path<u64>) -> StatusCode {
    crate::hotkeys::cancel_recording(session_id);
    StatusCode::NO_CONTENT
}

pub(in super::super) async fn capture_hotkey_recording(
    Path(session_id): Path<u64>,
    State(state): State<AppState>,
) -> axum::Json<HotkeyCaptureResult> {
    let mut events = state.daemon.events.subscribe();
    if !crate::hotkeys::start_recording(session_id, state.daemon.events.clone()) {
        return axum::Json(HotkeyCaptureResult {
            native: false,
            key: None,
            canceled: false,
        });
    }
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        wait_for_recording(&mut events, session_id),
    )
    .await
    .unwrap_or(None);
    crate::hotkeys::cancel_recording(session_id);
    let canceled = outcome.is_none();
    axum::Json(HotkeyCaptureResult {
        native: true,
        key: outcome,
        canceled,
    })
}

async fn wait_for_recording(
    events: &mut tokio::sync::broadcast::Receiver<crate::daemon::DaemonEvent>,
    session_id: u64,
) -> Option<String> {
    loop {
        match events.recv().await {
            Ok(crate::daemon::DaemonEvent::HotkeyRecorded {
                session_id: recorded,
                key,
            }) if recorded == session_id => return Some(key),
            Ok(crate::daemon::DaemonEvent::HotkeyRecordingCanceled {
                session_id: canceled,
            }) if canceled == session_id => return None,
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
        }
    }
}

async fn blocking<F>(work: F) -> Response
where
    F: FnOnce() -> HttpResult<Response> + Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(Ok(response)) => response,
        Ok(Err(boxed)) => *boxed,
        Err(error) => {
            log::error!("hotkey handler join error: {}", error);
            (StatusCode::INTERNAL_SERVER_ERROR, "Handler crashed").into_response()
        }
    }
}

async fn blocking_open<F>(work: F) -> Response
where
    F: FnOnce() -> Response + Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(response) => response,
        Err(error) => {
            log::error!("hotkey open handler join error: {}", error);
            (StatusCode::INTERNAL_SERVER_ERROR, "Handler crashed").into_response()
        }
    }
}

fn open_config_file(path: anyhow::Result<std::path::PathBuf>) -> Response {
    let Ok(path) = path else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "Path unavailable").into_response();
    };
    match crate::features::profile::sync::open_path(&path) {
        Ok(()) => StatusCode::OK.into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{error}")).into_response(),
    }
}

fn get_hotkeys_inner() -> HttpResult<Response> {
    let manager = hotkey_manager()?;
    let config = manager
        .load_config()
        .map_err(|_| Box::new(load_failed_response()))?;
    Ok(hotkeys_json_response(&config))
}

fn set_hotkeys_inner(state: &AppState, body: axum::body::Bytes) -> HttpResult<Response> {
    let config = parse_hotkeys(body)?;
    let manager = hotkey_manager()?;
    manager
        .save_config(&config)
        .map_err(|_| Box::new(save_failed_response()))?;
    state.daemon.config.config_changed(ConfigKind::Hotkeys);
    Ok(hotkeys_saved_response())
}

fn hotkey_manager() -> HttpResult<HotkeyManager> {
    HotkeyManager::new().map_err(|_| Box::new(load_failed_response()))
}

fn parse_hotkeys(body: axum::body::Bytes) -> HttpResult<HotkeyConfig> {
    let config: HotkeyConfig = http_json::parse_json_body(body, MAX_CONFIG_SIZE)?;
    if let Some(key) = crate::hotkeys::duplicate_enabled_chord(&config) {
        return Err(Box::new(
            (
                StatusCode::BAD_REQUEST,
                format!("Duplicate enabled hotkey chord: {key}"),
            )
                .into_response(),
        ));
    }
    Ok(config)
}

fn hotkeys_json_response(config: &HotkeyConfig) -> Response {
    let Ok(json) = encode_hotkeys_json(config) else {
        return serialize_failed_response();
    };
    http_json::json_response(json)
}

fn encode_hotkeys_json(config: &HotkeyConfig) -> HttpResult<Vec<u8>> {
    http_json::encode_json(config, "Failed to serialize hotkeys")
}

fn hotkeys_saved_response() -> Response {
    (StatusCode::OK, "Hotkeys saved").into_response()
}

fn load_failed_response() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "Failed to load hotkeys").into_response()
}

fn save_failed_response() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save hotkeys").into_response()
}

pub(in super::super) async fn get_hotkey_errors() -> impl IntoResponse {
    axum::Json(get_registration_errors())
}

fn serialize_failed_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to serialize hotkeys",
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_enabled_chords_are_rejected_before_persistence() {
        let body = axum::body::Bytes::from_static(
            br#"{"hotkeys":[
                {"id":"first","key":"F12","plugin_uid":"plugin-a","action":"open","enabled":true},
                {"id":"second","key":"f12","plugin_uid":"plugin-a","action":"settings","enabled":true}
            ]}"#,
        );

        let response = parse_hotkeys(body).expect_err("duplicate chord must be rejected");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn disabled_binding_may_share_an_enabled_chord() {
        let body = axum::body::Bytes::from_static(
            br#"{"hotkeys":[
                {"id":"enabled","key":"F12","plugin_uid":"plugin-a","action":"open","enabled":true},
                {"id":"disabled","key":"f12","plugin_uid":"plugin-a","action":"settings","enabled":false}
            ]}"#,
        );

        let config = parse_hotkeys(body).expect("disabled binding does not compete for the chord");
        assert_eq!(config.hotkeys.len(), 2);
    }

    #[tokio::test]
    async fn native_capture_waits_for_its_own_session() {
        let bus = crate::daemon::EventBus::new();
        let mut events = bus.subscribe();
        bus.send(crate::daemon::DaemonEvent::HotkeyRecorded {
            session_id: 8,
            key: "F8".to_string(),
        });
        bus.send(crate::daemon::DaemonEvent::HotkeyRecorded {
            session_id: 9,
            key: "Ctrl+F9".to_string(),
        });

        assert_eq!(
            wait_for_recording(&mut events, 9).await,
            Some("Ctrl+F9".to_string())
        );
    }

    #[tokio::test]
    async fn native_capture_cancel_is_terminal() {
        let bus = crate::daemon::EventBus::new();
        let mut events = bus.subscribe();
        bus.send(crate::daemon::DaemonEvent::HotkeyRecordingCanceled { session_id: 11 });

        assert_eq!(wait_for_recording(&mut events, 11).await, None);
    }
}
