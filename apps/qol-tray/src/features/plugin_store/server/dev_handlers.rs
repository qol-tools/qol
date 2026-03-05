#![cfg(feature = "dev")]

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};

use crate::dev::state::DiscoveryStatus;

use super::dev_services;
use super::dev_validation::sanitize_monitored_plugin_ids;
use super::types::{
    AppState, BuildStateResponse, DiscoveryStateResponse, SetPluginCpuMonitoringRequest,
};

pub(super) async fn reload_plugins(State(state): State<AppState>) -> impl IntoResponse {
    if let Err(message) = dev_services::queue_reload(&state) {
        log::warn!("Developer reload requested, but a build is already in progress");
        return (StatusCode::CONFLICT, message).into_response();
    }
    (StatusCode::OK, "Reload queued").into_response()
}

pub(super) async fn recompile_self(State(state): State<AppState>) -> impl IntoResponse {
    if let Err(message) = dev_services::queue_self_recompile(&state) {
        return (StatusCode::CONFLICT, message).into_response();
    }
    StatusCode::ACCEPTED.into_response()
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
