use axum::{
    body::Body,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};

use super::dev_services;
use super::types::{AppState, MockTargetInfo, DEFAULT_UI_SERVER_PORT};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/dev/mock-check-update", get(mock_check_update))
        .route("/dev/mock-targets", get(list_mock_targets))
        .route("/dev/mock-targets/start", post(start_mock_targets))
        .route("/dev/mock-targets/stop", post(stop_mock_targets))
        .route("/dev/mock-plugin-build", post(mock_plugin_build))
        .route("/dev/mock-plugin-build/stop", post(stop_mock_plugin_build))
        .route("/dev/mock-self-recompile", post(mock_self_recompile))
        .route(
            "/dev/mock-self-recompile/stop",
            post(stop_mock_self_recompile),
        )
        .route("/dev/mock-self-update", post(mock_self_update))
        .route("/dev/mock-self-update/stop", post(stop_mock_self_update))
        .route("/dev/update-fixture.tar.gz", get(serve_update_fixture))
        .route("/dev/test-self-update", post(test_self_update))
}

pub(super) async fn mock_check_update() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "available": true, "latest": "99.0.0" }))
}

pub(super) async fn list_mock_targets(State(state): State<AppState>) -> Json<Vec<MockTargetInfo>> {
    Json(state.runtime.list_mock_targets())
}

pub(super) async fn start_mock_targets(State(state): State<AppState>) -> impl IntoResponse {
    let started = match dev_services::start_mock_targets(&state) {
        Ok(started) => started,
        Err(message) => return (StatusCode::CONFLICT, message).into_response(),
    };
    mock_targets_response(
        StatusCode::ACCEPTED,
        "started",
        started,
        state.runtime.list_mock_targets(),
    )
}

pub(super) async fn stop_mock_targets(State(state): State<AppState>) -> impl IntoResponse {
    let stopped = dev_services::stop_mock_targets(&state);
    if stopped.is_empty() {
        return mock_targets_response(
            StatusCode::OK,
            "stopped",
            stopped,
            state.runtime.list_mock_targets(),
        );
    }
    mock_targets_response(
        StatusCode::ACCEPTED,
        "stopped",
        stopped,
        state.runtime.list_mock_targets(),
    )
}

pub(super) async fn mock_self_update(State(state): State<AppState>) -> impl IntoResponse {
    mock_start_response(
        state
            .runtime
            .start_mock_self_update(state.daemon.events.clone()),
        "Mock update queued",
    )
}

pub(super) async fn stop_mock_self_update(State(state): State<AppState>) -> impl IntoResponse {
    mock_stop_response(
        state.runtime.stop_mock_self_update(),
        "Stopping mock update",
        "No mock update in progress",
    )
}

pub(super) async fn mock_self_recompile(State(state): State<AppState>) -> impl IntoResponse {
    mock_start_response(
        state
            .runtime
            .start_mock_self_recompile(state.daemon.events.clone()),
        "Mock recompile queued",
    )
}

pub(super) async fn stop_mock_self_recompile(State(state): State<AppState>) -> impl IntoResponse {
    mock_stop_response(
        state.runtime.stop_mock_self_recompile(),
        "Stopping mock recompile",
        "No mock recompile in progress",
    )
}

pub(super) async fn stop_mock_plugin_build(State(state): State<AppState>) -> impl IntoResponse {
    mock_stop_response(
        state.runtime.stop_mock_plugin_build(),
        "Stopping mock build",
        "No mock build in progress",
    )
}

pub(super) async fn mock_plugin_build(State(state): State<AppState>) -> impl IntoResponse {
    mock_start_response(
        dev_services::queue_mock_plugin_build(&state),
        "Mock build queued",
    )
}

fn mock_start_response(
    result: Result<(), &'static str>,
    queued_message: &'static str,
) -> axum::response::Response {
    match result {
        Ok(()) => (StatusCode::ACCEPTED, queued_message).into_response(),
        Err(message) => (StatusCode::CONFLICT, message).into_response(),
    }
}

fn mock_stop_response(
    stopped: bool,
    stopping_message: &'static str,
    idle_message: &'static str,
) -> axum::response::Response {
    if stopped {
        return (StatusCode::ACCEPTED, stopping_message).into_response();
    }
    (StatusCode::OK, idle_message).into_response()
}

fn mock_targets_response(
    status: StatusCode,
    key: &'static str,
    ids: Vec<&'static str>,
    targets: Vec<MockTargetInfo>,
) -> axum::response::Response {
    (
        status,
        Json(serde_json::json!({ key: ids, "targets": targets })),
    )
        .into_response()
}

async fn serve_update_fixture() -> impl IntoResponse {
    match build_update_fixture() {
        Ok(bytes) => (
            StatusCode::OK,
            [
                ("content-type", "application/gzip"),
                ("content-length", &bytes.len().to_string()),
            ],
            Body::from(bytes),
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

fn platform_bundle_name() -> String {
    let os = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    };
    format!("qol-tray-{os}-{arch}")
}

fn build_update_fixture() -> anyhow::Result<Vec<u8>> {
    let current_exe = std::env::current_exe()?;
    let bundle_name = platform_bundle_name();

    let mut binary_data = std::fs::read(&current_exe)?;
    patch_test_version(&mut binary_data);

    let buf = Vec::new();
    let encoder = flate2::write::GzEncoder::new(buf, flate2::Compression::fast());
    let mut tar = tar::Builder::new(encoder);

    let mut header = tar::Header::new_gnu();
    header.set_size(binary_data.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    tar.append_data(
        &mut header,
        format!("{bundle_name}/qol-tray"),
        binary_data.as_slice(),
    )?;

    let encoder = tar.into_inner()?;
    Ok(encoder.finish()?)
}

fn patch_test_version(binary: &mut [u8]) {
    let full_static: [u8; 32] = *b"@@QOL_TEST_VER@@\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
    let version = format!("{}-test", env!("CARGO_PKG_VERSION"));
    let Some(pos) = binary.windows(32).position(|w| w == full_static) else {
        log::warn!("Test version static not found in binary");
        return;
    };
    let patch = &mut binary[pos..pos + 32];
    patch.fill(0);
    let bytes = version.as_bytes();
    patch[..bytes.len().min(31)].copy_from_slice(&bytes[..bytes.len().min(31)]);
}

async fn test_self_update(
    query: axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    if query.get("live").is_some() {
        return live_self_update().await.into_response();
    }
    dry_run_self_update().await.into_response()
}

#[allow(clippy::unused_async)]
async fn live_self_update() -> impl IntoResponse {
    let fixture_url = format!(
        "http://127.0.0.1:{}/api/dev/update-fixture.tar.gz",
        DEFAULT_UI_SERVER_PORT
    );
    std::env::set_var("QOL_TRAY_DEV_UPDATE_URL", &fixture_url);
    (StatusCode::OK, "Fixture URL configured")
}

async fn dry_run_self_update() -> impl IntoResponse {
    let fixture_url = format!(
        "http://127.0.0.1:{}/api/dev/update-fixture.tar.gz",
        DEFAULT_UI_SERVER_PORT
    );

    let mut steps: Vec<serde_json::Value> = Vec::new();
    let mut ok = true;

    let dest = std::env::temp_dir().join("qol-tray-test-update.tar.gz");
    let download_result = async {
        let client = reqwest::Client::new();
        let response = client.get(&fixture_url).send().await?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("HTTP {status}");
        }
        let bytes = response.bytes().await?;
        std::fs::write(&dest, &bytes)?;
        Ok::<usize, anyhow::Error>(bytes.len())
    }
    .await;

    match &download_result {
        Ok(size) => steps.push(serde_json::json!({
            "step": "download",
            "ok": true,
            "detail": format!("{} bytes from {fixture_url}", size)
        })),
        Err(e) => {
            ok = false;
            steps.push(serde_json::json!({
                "step": "download",
                "ok": false,
                "detail": e.to_string()
            }));
        }
    }

    if ok {
        match extract_and_verify(&dest) {
            Ok((binary_path, binary_size)) => {
                steps.push(serde_json::json!({
                    "step": "extract",
                    "ok": true,
                    "detail": format!("found qol-tray ({binary_size} bytes) at {}", binary_path.display())
                }));

                match verify_binary_matches(&binary_path) {
                    Ok(detail) => steps.push(serde_json::json!({
                        "step": "verify",
                        "ok": true,
                        "detail": detail
                    })),
                    Err(e) => {
                        ok = false;
                        steps.push(serde_json::json!({
                            "step": "verify",
                            "ok": false,
                            "detail": e.to_string()
                        }));
                    }
                }
            }
            Err(e) => {
                ok = false;
                steps.push(serde_json::json!({
                    "step": "extract",
                    "ok": false,
                    "detail": e.to_string()
                }));
            }
        }
    }

    let _ = std::fs::remove_file(&dest);
    let _ = std::fs::remove_dir_all(dest.with_extension("extracted"));

    Json(serde_json::json!({ "ok": ok, "steps": steps }))
}

fn extract_and_verify(archive: &std::path::Path) -> anyhow::Result<(std::path::PathBuf, u64)> {
    let tar_gz = std::fs::File::open(archive)?;
    let tar = flate2::read::GzDecoder::new(tar_gz);
    let mut archive_reader = tar::Archive::new(tar);

    let extract_dir = archive.with_extension("extracted");
    if extract_dir.exists() {
        let _ = std::fs::remove_dir_all(&extract_dir);
    }
    std::fs::create_dir_all(&extract_dir)?;
    archive_reader.unpack(&extract_dir)?;

    for entry in walkdir::WalkDir::new(&extract_dir).max_depth(2) {
        let entry = entry?;
        if entry.file_type().is_file() && entry.file_name().to_string_lossy() == "qol-tray" {
            let size = entry.metadata()?.len();
            return Ok((entry.into_path(), size));
        }
    }
    anyhow::bail!("qol-tray binary not found in archive")
}

fn verify_binary_matches(extracted: &std::path::Path) -> anyhow::Result<String> {
    let current_exe = std::env::current_exe()?;
    let current_size = std::fs::metadata(&current_exe)?.len();
    let extracted_size = std::fs::metadata(extracted)?.len();

    if current_size != extracted_size {
        anyhow::bail!("size mismatch: running={current_size}, extracted={extracted_size}");
    }

    let extracted_bytes = std::fs::read(extracted)?;
    let patched_version = read_patched_version(&extracted_bytes);
    let version_info = patched_version
        .map(|v| format!(", patched version: {v}"))
        .unwrap_or_default();

    Ok(format!(
        "matches running binary ({current_size} bytes) at {}{version_info}",
        current_exe.display()
    ))
}

fn read_patched_version(binary: &[u8]) -> Option<String> {
    let sentinel = b"@@QOL_TEST_VER@@";
    let pos = binary.windows(sentinel.len()).position(|w| w == sentinel);
    if pos.is_some() {
        return None; // sentinel intact, not patched
    }
    let current = env!("CARGO_PKG_VERSION");
    let test_prefix = format!("{current}-test");
    let start = binary
        .windows(test_prefix.len())
        .position(|w| w == test_prefix.as_bytes())?;
    let end = binary[start..]
        .iter()
        .position(|&b| b == 0)
        .map(|i| start + i)
        .unwrap_or(start + 31);
    std::str::from_utf8(&binary[start..end])
        .ok()
        .map(String::from)
}
