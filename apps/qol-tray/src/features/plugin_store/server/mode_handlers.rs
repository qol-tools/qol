use std::path::{Path, PathBuf};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;

use super::types::AppState;

#[derive(Deserialize)]
pub(super) struct SwitchRequest {
    target: SwitchTarget,
    path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum SwitchTarget {
    Dev,
    Prod,
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/mode/validate", post(validate_path))
        .route("/mode/switch", post(switch_mode))
}

async fn validate_path(Json(req): Json<SwitchRequest>) -> impl IntoResponse {
    let path = Path::new(&req.path);
    let valid = match req.target {
        SwitchTarget::Dev => path.join("Cargo.toml").is_file(),
        SwitchTarget::Prod => path.is_file() && is_executable(path),
    };
    Json(valid)
}

async fn switch_mode(
    State(state): State<AppState>,
    Json(req): Json<SwitchRequest>,
) -> impl IntoResponse {
    let path = PathBuf::from(&req.path);
    match req.target {
        SwitchTarget::Dev => switch_to_dev(state, path).into_response(),
        SwitchTarget::Prod => switch_to_prod(state, path).into_response(),
    }
}

fn switch_to_dev(state: AppState, repo_path: PathBuf) -> impl IntoResponse {
    if !repo_path.join("Cargo.toml").is_file() {
        return (StatusCode::BAD_REQUEST, "Invalid dev repo path").into_response();
    }

    let events = state.daemon.events.clone();
    let plugin_manager = state.plugin_manager.clone();

    tokio::spawn(async move {
        let events_clone = events.clone();
        let repo = repo_path.clone();
        let result = tokio::task::spawn_blocking(move || {
            super::mode_build::build_dev_binary(&repo, events_clone)
        })
        .await;

        match result {
            Ok(Ok(binary)) => {
                events.send(crate::daemon::DaemonEvent::ModeSwitchComplete);
                plugin_manager.lock().unwrap().shutdown();
                exec_restart(&binary);
            }
            Ok(Err(e)) => {
                log::error!("Mode switch build failed: {}", e);
                events.send(crate::daemon::DaemonEvent::ModeSwitchFailed {
                    message: e.to_string(),
                });
            }
            Err(e) => {
                log::error!("Mode switch task failed: {}", e);
                events.send(crate::daemon::DaemonEvent::ModeSwitchFailed {
                    message: e.to_string(),
                });
            }
        }
    });

    (StatusCode::ACCEPTED, "").into_response()
}

#[cfg(feature = "dev")]
fn switch_to_prod(state: AppState, binary_path: PathBuf) -> impl IntoResponse {
    if !binary_path.is_file() || !is_executable(&binary_path) {
        return (StatusCode::BAD_REQUEST, "Invalid prod binary path").into_response();
    }

    let events = state.daemon.events.clone();
    let plugin_manager = state.plugin_manager.clone();

    tokio::spawn(async move {
        events.send(crate::daemon::DaemonEvent::ModeSwitchProgress {
            percent: 50,
            phase: "Switching to production".into(),
        });
        plugin_manager.lock().unwrap().shutdown();
        events.send(crate::daemon::DaemonEvent::ModeSwitchComplete);
        exec_restart(&binary_path);
    });

    (StatusCode::ACCEPTED, "").into_response()
}

#[cfg(not(feature = "dev"))]
fn switch_to_prod(_state: AppState, _binary_path: PathBuf) -> impl IntoResponse {
    (StatusCode::BAD_REQUEST, "Already in prod mode")
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

fn exec_restart(binary: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
        let error = std::process::Command::new(binary).args(&args).exec();
        log::error!("exec_restart failed: {}", error);
    }
    #[cfg(not(unix))]
    {
        let _ = binary;
        log::error!("exec_restart not supported on this platform");
    }
}
