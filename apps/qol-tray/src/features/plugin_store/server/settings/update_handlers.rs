use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

use super::super::helpers::{is_newer_version, validate_plugin_id};
use super::super::plugin_services;
use super::super::types::{AppState, PluginUpdateJob, PluginUpdateState, MAX_CONFIG_SIZE};
use super::http_json;

pub(super) const UPDATES_QUERY: &str = "updates";
pub(super) const ATTENTION_QUERY: &str = "attention";
pub(super) const HOST_PLUGIN_ID: &str = "qol-tray";
const ATTENTION_AFTER_SECS: u64 = 24 * 60 * 60;

type HttpResult<T> = Result<T, Box<Response>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum RowState {
    UpToDate,
    Available,
    Queued,
    Updating,
    Failed,
    DevBuild,
    DevLinked,
}

struct UpdatesView {
    checking: bool,
    checks_enabled: bool,
    last_attempt: Option<SystemTime>,
    last_success: Option<SystemTime>,
    check_error: Option<String>,
    tray_start: SystemTime,
    host: HostView,
    plugins: Vec<PluginView>,
}

struct HostView {
    current: String,
    latest: Option<String>,
    dev_build: bool,
    queued: bool,
    running: bool,
    progress: Option<u8>,
    error: Option<String>,
    updated_from: Option<String>,
}

struct PluginView {
    id: String,
    name: String,
    current: String,
    latest: Option<String>,
    dev_linked: bool,
    job: Option<PluginUpdateJob>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct UpdatesPayload {
    checking: bool,
    checks_enabled: bool,
    last_checked_secs: Option<u64>,
    last_success_secs: Option<u64>,
    check_error: Option<String>,
    running: bool,
    host: HostPayload,
    plugins: Vec<PluginPayload>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct HostPayload {
    id: &'static str,
    name: &'static str,
    current: String,
    latest: Option<String>,
    state: RowState,
    progress: Option<u8>,
    error: Option<String>,
    updated_from: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct PluginPayload {
    id: String,
    name: String,
    current: String,
    latest: Option<String>,
    state: RowState,
    progress: Option<u8>,
    error: Option<String>,
}

#[derive(Serialize)]
struct AttentionPayload {
    #[serde(rename = "__core-updates")]
    core_updates: bool,
}

#[derive(Default, Deserialize)]
struct CoreActionRequest {
    #[serde(default)]
    id: Option<String>,
}

#[derive(Serialize)]
struct CoreActionResponse {
    success: bool,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UpdateTarget {
    Host,
    Plugin(String),
}

pub(super) async fn get_updates(state: AppState) -> Response {
    http_json::blocking("updates query", move || {
        let view = collect_view(&state)?;
        let payload = build_updates_payload(&view, SystemTime::now());
        let json = http_json::encode_json(&payload, "Failed to serialize updates")?;
        Ok(http_json::json_response(json))
    })
    .await
}

pub(super) async fn get_attention(state: AppState) -> Response {
    http_json::blocking("attention query", move || {
        let view = collect_view(&state)?;
        let now = SystemTime::now();
        let payload = build_updates_payload(&view, now);
        let tray_started_secs = secs_since(now, Some(view.tray_start)).unwrap_or(0);
        let body = AttentionPayload {
            core_updates: updates_attention(&payload, tray_started_secs),
        };
        let json = http_json::encode_json(&body, "Failed to serialize attention")?;
        Ok(http_json::json_response(json))
    })
    .await
}

pub(super) async fn post_core_action(
    Path(action): Path<String>,
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> Response {
    let request: CoreActionRequest = if body.is_empty() {
        CoreActionRequest::default()
    } else {
        match http_json::parse_json_body(body, MAX_CONFIG_SIZE) {
            Ok(request) => request,
            Err(response) => return *response,
        }
    };
    match action.as_str() {
        "check_updates" => {
            spawn_refresh(state);
            action_ok("Checking for updates")
        }
        "update" => start_update_action(&state, request.id.as_deref()),
        "update_all" => start_update_all_action(&state),
        _ => action_error(StatusCode::NOT_FOUND, "Unknown action"),
    }
}

fn spawn_refresh(state: AppState) {
    tokio::spawn(async move {
        super::super::refresh_updates(&state, true).await;
    });
}

fn start_update_action(state: &AppState, id: Option<&str>) -> Response {
    let target = match update_target(id) {
        Ok(target) => target,
        Err((status, message)) => return action_error(status, message),
    };
    match target {
        UpdateTarget::Host => start_host_update(state),
        UpdateTarget::Plugin(id) => start_plugin_update(state, id),
    }
}

fn start_host_update(state: &AppState) -> Response {
    let available = is_available(
        crate::updates::current_version(),
        crate::updates::latest_version().as_deref(),
    );
    if let Some(message) = host_update_refusal(
        crate::updates::checks_enabled(),
        crate::updates::host_update_status().running,
        available,
    ) {
        return action_error(StatusCode::CONFLICT, message);
    }
    match super::super::meta_handlers::start_self_update(state, true) {
        Ok(()) => action_ok("Update started"),
        Err(message) => action_error(StatusCode::CONFLICT, message),
    }
}

fn start_plugin_update(state: &AppState, id: String) -> Response {
    let installed = match plugin_services::list_installed(state) {
        Ok(installed) => installed,
        Err(_) => {
            return action_error(StatusCode::INTERNAL_SERVER_ERROR, "Plugin list unavailable")
        }
    };
    if !installed
        .plugins
        .iter()
        .any(|plugin| plugin.id.as_str() == id.as_str())
    {
        return action_error(StatusCode::NOT_FOUND, format!("Unknown plugin: {id}"));
    }
    if let Err(message) = state.begin_plugin_update(&id) {
        return action_error(StatusCode::CONFLICT, message);
    }
    let worker_state = state.clone();
    tokio::spawn(async move {
        let result = plugin_services::run_plugin_update(&worker_state, &id).await;
        if !result.success {
            log::warn!("Update for {} failed: {}", id, result.message);
        }
    });
    action_ok("Update started")
}

fn start_update_all_action(state: &AppState) -> Response {
    let plugins = match plugin_views(state) {
        Ok(plugins) => plugins,
        Err(_) => {
            return action_error(StatusCode::INTERNAL_SERVER_ERROR, "Plugin list unavailable")
        }
    };
    let host_available = crate::updates::checks_enabled()
        && is_available(
            crate::updates::current_version(),
            crate::updates::latest_version().as_deref(),
        );
    let plan = update_all_plan(&plugins, host_available);
    let running = crate::updates::host_update_status().running || state.any_plugin_update_active();
    if let Some(message) = update_all_refusal(running, &plan) {
        return action_error(StatusCode::CONFLICT, message);
    }
    for target in &plan {
        if let UpdateTarget::Plugin(id) = target {
            state.mark_plugin_queued(id);
        }
    }
    if plan.contains(&UpdateTarget::Host) {
        crate::updates::mark_host_update_queued();
    }
    let worker_state = state.clone();
    tokio::spawn(async move {
        run_update_all(worker_state, plan).await;
    });
    action_ok("Update started")
}

async fn run_update_all(state: AppState, plan: Vec<UpdateTarget>) {
    let host_queued = plan.contains(&UpdateTarget::Host);
    let mut plugin_failed = false;
    for target in plan {
        let UpdateTarget::Plugin(id) = target else {
            continue;
        };
        if let Err(message) = state.begin_queued_plugin_update(&id) {
            log::warn!("Update all skipped {}: {}", id, message);
            state.fail_plugin_update(&id, message);
            plugin_failed = true;
            continue;
        }
        let result = plugin_services::run_plugin_update(&state, &id).await;
        if !result.success {
            log::warn!("Update all could not update {}: {}", id, result.message);
            plugin_failed = true;
        }
    }
    if !host_queued {
        return;
    }
    if plugin_failed {
        crate::updates::clear_host_update_queued();
        return;
    }
    if let Err(message) = super::super::meta_handlers::start_self_update(&state, true) {
        log::warn!(
            "Update all could not start the qol-tray update: {}",
            message
        );
        crate::updates::clear_host_update_queued();
    }
}

fn collect_view(state: &AppState) -> HttpResult<UpdatesView> {
    let health = crate::updates::check_health();
    let host = crate::updates::host_update_status();
    let checks_enabled = crate::updates::checks_enabled();
    let plugins = plugin_views(state).map_err(|status| Box::new(status.into_response()))?;
    Ok(UpdatesView {
        checking: health.checking,
        checks_enabled,
        last_attempt: health.last_attempt,
        last_success: health.last_success,
        check_error: health.last_error,
        tray_start: crate::updates::tray_start(),
        host: HostView {
            current: crate::updates::current_version().to_string(),
            latest: crate::updates::latest_version(),
            dev_build: !checks_enabled,
            queued: host.queued,
            running: host.running,
            progress: host.progress,
            error: host.error,
            updated_from: crate::updates::updated_from(),
        },
        plugins,
    })
}

fn plugin_views(state: &AppState) -> Result<Vec<PluginView>, StatusCode> {
    let installed = plugin_services::list_installed(state)?;
    let jobs = state.plugin_update_jobs();
    Ok(installed
        .plugins
        .into_iter()
        .map(|plugin| PluginView {
            id: plugin.id.as_str().to_string(),
            name: plugin.name,
            current: plugin.version,
            latest: plugin.available_version,
            dev_linked: plugin.source == Some("dev_linked"),
            job: jobs.get(plugin.id.as_str()).cloned(),
        })
        .collect())
}

fn build_updates_payload(view: &UpdatesView, now: SystemTime) -> UpdatesPayload {
    let mut plugins: Vec<&PluginView> = view.plugins.iter().collect();
    plugins.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    UpdatesPayload {
        checking: view.checking,
        checks_enabled: view.checks_enabled,
        last_checked_secs: secs_since(now, view.last_attempt),
        last_success_secs: secs_since(now, view.last_success),
        check_error: view.check_error.clone(),
        running: view.host.running
            || view.plugins.iter().any(|plugin| {
                matches!(
                    plugin_row_state(plugin),
                    RowState::Queued | RowState::Updating
                )
            }),
        host: host_payload(view),
        plugins: plugins.into_iter().map(plugin_payload).collect(),
    }
}

fn host_payload(view: &UpdatesView) -> HostPayload {
    let state = host_row_state(view);
    HostPayload {
        id: HOST_PLUGIN_ID,
        name: HOST_PLUGIN_ID,
        current: view.host.current.clone(),
        latest: view.host.latest.clone(),
        state,
        progress: if state == RowState::Updating {
            view.host.progress
        } else {
            None
        },
        error: if state == RowState::Failed {
            view.host.error.clone()
        } else {
            None
        },
        updated_from: view.host.updated_from.clone(),
    }
}

fn plugin_payload(plugin: &PluginView) -> PluginPayload {
    let state = plugin_row_state(plugin);
    PluginPayload {
        id: plugin.id.clone(),
        name: plugin.name.clone(),
        current: plugin.current.clone(),
        latest: plugin.latest.clone(),
        state,
        progress: None,
        error: if state == RowState::Failed {
            plugin.job.as_ref().and_then(|job| job.reason.clone())
        } else {
            None
        },
    }
}

fn host_row_state(view: &UpdatesView) -> RowState {
    if view.host.dev_build {
        return RowState::DevBuild;
    }
    if view.host.running {
        return RowState::Updating;
    }
    if view.host.queued {
        return RowState::Queued;
    }
    if view.host.error.is_some() {
        return RowState::Failed;
    }
    if is_available(&view.host.current, view.host.latest.as_deref()) {
        RowState::Available
    } else {
        RowState::UpToDate
    }
}

fn plugin_row_state(plugin: &PluginView) -> RowState {
    if let Some(job) = &plugin.job {
        match job.state {
            PluginUpdateState::Queued => return RowState::Queued,
            PluginUpdateState::Updating => return RowState::Updating,
            PluginUpdateState::Failed => return RowState::Failed,
        }
    }
    if plugin.dev_linked {
        return RowState::DevLinked;
    }
    if is_available(&plugin.current, plugin.latest.as_deref()) {
        RowState::Available
    } else {
        RowState::UpToDate
    }
}

fn updates_attention(payload: &UpdatesPayload, tray_started_secs: u64) -> bool {
    let needs_attention = |state: RowState| matches!(state, RowState::Available | RowState::Failed);
    if needs_attention(payload.host.state) {
        return true;
    }
    if payload
        .plugins
        .iter()
        .any(|plugin| needs_attention(plugin.state))
    {
        return true;
    }
    if !payload.checks_enabled {
        return false;
    }
    let idle_secs = match payload.last_success_secs {
        Some(since_success) => since_success.min(tray_started_secs),
        None => tray_started_secs,
    };
    idle_secs > ATTENTION_AFTER_SECS
}

fn secs_since(now: SystemTime, moment: Option<SystemTime>) -> Option<u64> {
    moment.map(|moment| {
        now.duration_since(moment)
            .map(|age| age.as_secs())
            .unwrap_or(0)
    })
}

fn is_available(current: &str, latest: Option<&str>) -> bool {
    let Some(latest) = latest else {
        return false;
    };
    current != "unknown" && is_newer_version(latest, current)
}

fn update_target(id: Option<&str>) -> Result<UpdateTarget, (StatusCode, &'static str)> {
    let Some(id) = id.map(str::trim).filter(|id| !id.is_empty()) else {
        return Err((StatusCode::BAD_REQUEST, "A plugin id is required"));
    };
    if id == HOST_PLUGIN_ID {
        return Ok(UpdateTarget::Host);
    }
    validate_plugin_id(id).map_err(|message| (StatusCode::BAD_REQUEST, message))?;
    Ok(UpdateTarget::Plugin(id.to_string()))
}

fn host_update_refusal(
    checks_enabled: bool,
    running: bool,
    available: bool,
) -> Option<&'static str> {
    if !checks_enabled {
        return Some("Development builds update through Recompile");
    }
    if running {
        return Some(crate::updates::UPDATE_ALREADY_RUNNING);
    }
    if !available {
        return Some("qol-tray is up to date");
    }
    None
}

fn update_all_refusal(running: bool, plan: &[UpdateTarget]) -> Option<&'static str> {
    if running {
        Some(crate::updates::UPDATE_ALREADY_RUNNING)
    } else if plan.is_empty() {
        Some("Nothing to update")
    } else {
        None
    }
}

fn update_all_plan(plugins: &[PluginView], host_available: bool) -> Vec<UpdateTarget> {
    let mut candidates: Vec<&PluginView> = plugins
        .iter()
        .filter(|plugin| {
            plugin
                .job
                .as_ref()
                .is_some_and(|job| job.state == PluginUpdateState::Failed)
                || is_available(&plugin.current, plugin.latest.as_deref())
        })
        .collect();
    candidates.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut plan: Vec<UpdateTarget> = candidates
        .into_iter()
        .map(|plugin| UpdateTarget::Plugin(plugin.id.clone()))
        .collect();
    if host_available {
        plan.push(UpdateTarget::Host);
    }
    plan
}

fn action_ok(message: &str) -> Response {
    Json(CoreActionResponse {
        success: true,
        message: message.to_string(),
    })
    .into_response()
}

fn action_error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(CoreActionResponse {
            success: false,
            message: message.into(),
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn plugin(id: &str, name: &str, current: &str, latest: Option<&str>) -> PluginView {
        PluginView {
            id: id.to_string(),
            name: name.to_string(),
            current: current.to_string(),
            latest: latest.map(str::to_string),
            dev_linked: false,
            job: None,
        }
    }

    fn job(state: PluginUpdateState, reason: Option<&str>) -> Option<PluginUpdateJob> {
        Some(PluginUpdateJob {
            state,
            reason: reason.map(str::to_string),
        })
    }

    fn host(current: &str, latest: Option<&str>) -> HostView {
        HostView {
            current: current.to_string(),
            latest: latest.map(str::to_string),
            dev_build: false,
            queued: false,
            running: false,
            progress: None,
            error: None,
            updated_from: None,
        }
    }

    fn view(host: HostView, plugins: Vec<PluginView>) -> UpdatesView {
        UpdatesView {
            checking: false,
            checks_enabled: true,
            last_attempt: None,
            last_success: None,
            check_error: None,
            tray_start: SystemTime::UNIX_EPOCH,
            host,
            plugins,
        }
    }

    #[test]
    fn wire_query_names_are_stable() {
        assert_eq!(UPDATES_QUERY, "updates");
        assert_eq!(ATTENTION_QUERY, "attention");
    }

    #[test]
    fn payload_reports_every_field_in_the_wire_shape() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let mut state = view(host("3.66.1", Some("3.67.0")), vec![]);
        state.checking = true;
        state.last_attempt = Some(now - Duration::from_secs(5));
        state.last_success = Some(now - Duration::from_secs(10));
        let payload = build_updates_payload(&state, now);
        assert_eq!(
            serde_json::to_value(&payload).unwrap(),
            serde_json::json!({
                "checking": true,
                "checks_enabled": true,
                "last_checked_secs": 5,
                "last_success_secs": 10,
                "check_error": null,
                "running": false,
                "host": {
                    "id": "qol-tray",
                    "name": "qol-tray",
                    "current": "3.66.1",
                    "latest": "3.67.0",
                    "state": "available",
                    "progress": null,
                    "error": null,
                    "updated_from": null
                },
                "plugins": []
            })
        );
    }

    #[test]
    fn payload_omits_detail_unless_the_row_carries_it() {
        let mut state = view(host("3.66.1", Some("3.67.0")), vec![]);
        state.checks_enabled = false;
        state.host.dev_build = true;
        state.check_error = Some("GitHub returned 500".to_string());
        let payload = build_updates_payload(&state, SystemTime::UNIX_EPOCH);
        assert_eq!(payload.host.state, RowState::DevBuild);
        assert_eq!(payload.host.error, None);
        assert_eq!(payload.host.progress, None);

        state.checks_enabled = true;
        state.host.dev_build = false;
        state.host.error = Some("The update could not be installed".to_string());
        let payload = build_updates_payload(&state, SystemTime::UNIX_EPOCH);
        assert_eq!(payload.host.state, RowState::Failed);
        assert_eq!(
            payload.host.error.as_deref(),
            Some("The update could not be installed")
        );

        state.host.error = None;
        state.host.running = true;
        state.host.progress = Some(42);
        let payload = build_updates_payload(&state, SystemTime::UNIX_EPOCH);
        assert_eq!(payload.host.state, RowState::Updating);
        assert_eq!(payload.host.progress, Some(42));
        assert!(payload.running);
    }

    #[test]
    fn host_row_state_precedence() {
        let mut state = view(host("3.66.1", Some("3.67.0")), vec![]);
        assert_eq!(host_row_state(&state), RowState::Available);

        state.host.latest = None;
        assert_eq!(host_row_state(&state), RowState::UpToDate);

        state.host.error = Some("No connection to GitHub".to_string());
        assert_eq!(host_row_state(&state), RowState::Failed);

        state.host.queued = true;
        assert_eq!(host_row_state(&state), RowState::Queued);

        state.host.running = true;
        assert_eq!(host_row_state(&state), RowState::Updating);

        state.host.dev_build = true;
        assert_eq!(host_row_state(&state), RowState::DevBuild);
    }

    #[test]
    fn plugin_row_states_cover_jobs_dev_links_and_versions() {
        let mut released = plugin("plugin-launcher", "Launcher", "1.63.1", Some("1.64.0"));
        assert_eq!(plugin_row_state(&released), RowState::Available);

        released.latest = Some("1.63.1".to_string());
        assert_eq!(plugin_row_state(&released), RowState::UpToDate);

        released.dev_linked = true;
        assert_eq!(plugin_row_state(&released), RowState::DevLinked);

        released.job = job(PluginUpdateState::Queued, None);
        assert_eq!(plugin_row_state(&released), RowState::Queued);
        released.job = job(PluginUpdateState::Updating, None);
        assert_eq!(plugin_row_state(&released), RowState::Updating);
        released.job = job(PluginUpdateState::Failed, Some("No connection to GitHub"));
        assert_eq!(plugin_row_state(&released), RowState::Failed);
    }

    #[test]
    fn payload_sorts_plugins_by_name_case_insensitively_and_marks_running() {
        let mut alpha = plugin("plugin-alpha", "Alpha", "1.0.0", None);
        alpha.job = job(PluginUpdateState::Queued, None);
        let mut beta = plugin("plugin-beta", "beta", "1.0.0", None);
        beta.job = job(PluginUpdateState::Failed, Some("No connection to GitHub"));
        let zed = plugin("plugin-zed", "Zed", "1.0.0", None);
        let state = view(host("3.66.1", None), vec![zed, beta, alpha]);
        let payload = build_updates_payload(&state, SystemTime::UNIX_EPOCH);
        let ids: Vec<&str> = payload.plugins.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["plugin-alpha", "plugin-beta", "plugin-zed"]);
        assert_eq!(payload.plugins[0].state, RowState::Queued);
        assert_eq!(payload.plugins[1].state, RowState::Failed);
        assert_eq!(
            payload.plugins[1].error.as_deref(),
            Some("No connection to GitHub")
        );
        assert_eq!(payload.plugins[2].state, RowState::UpToDate);
        assert_eq!(payload.plugins[0].progress, None);
        assert!(payload.running);
    }

    #[test]
    fn secs_since_is_zero_for_future_moments_and_none_without_a_moment() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        assert_eq!(secs_since(now, None), None);
        assert_eq!(secs_since(now, Some(now)), Some(0));
        assert_eq!(
            secs_since(now, Some(now - Duration::from_secs(30))),
            Some(30)
        );
        assert_eq!(
            secs_since(now, Some(now + Duration::from_secs(30))),
            Some(0)
        );
    }

    #[test]
    fn availability_requires_a_newer_known_latest() {
        let cases = [
            ("1.0.0", Some("1.0.1"), true),
            ("1.0.0", Some("1.0.0"), false),
            ("1.0.0", None, false),
            ("unknown", Some("1.0.1"), false),
        ];
        for (current, latest, expected) in cases {
            assert_eq!(
                is_available(current, latest),
                expected,
                "current={current} latest={latest:?}"
            );
        }
    }

    #[test]
    fn attention_rule_table() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let day = 24 * 60 * 60;
        type Case = (&'static str, bool, bool, bool, Option<u64>, u64, bool);
        let cases: &[Case] = &[
            ("host update available", true, false, true, Some(1), 1, true),
            ("plugin update failed", false, true, true, Some(1), 1, true),
            ("nothing to show", false, false, true, Some(1), 1, false),
            (
                "success under a day ago",
                false,
                false,
                true,
                Some(day - 1),
                day * 5,
                false,
            ),
            (
                "success over a day ago",
                false,
                false,
                true,
                Some(day + 1),
                day + 1,
                true,
            ),
            (
                "success over a day ago but the tray just started",
                false,
                false,
                true,
                Some(day + 1),
                1,
                false,
            ),
            (
                "no success since the tray started over a day ago",
                false,
                false,
                true,
                None,
                day + 1,
                true,
            ),
            (
                "no success and the tray just started",
                false,
                false,
                true,
                None,
                1,
                false,
            ),
            (
                "checks disabled never raises on age",
                false,
                false,
                false,
                None,
                day * 5,
                false,
            ),
        ];
        for (
            name,
            host_available,
            plugin_failed,
            checks_enabled,
            last_success_secs,
            tray_started_secs,
            expected,
        ) in cases
        {
            let mut state = view(
                host("3.66.1", host_available.then_some("3.67.0")),
                if *plugin_failed {
                    let mut failed = plugin("plugin-a", "A", "1.0.0", None);
                    failed.job = job(PluginUpdateState::Failed, Some("No connection to GitHub"));
                    vec![failed]
                } else {
                    Vec::new()
                },
            );
            state.checks_enabled = *checks_enabled;
            state.last_success = last_success_secs.map(|secs| now - Duration::from_secs(secs));
            let payload = build_updates_payload(&state, now);
            assert_eq!(
                updates_attention(&payload, *tray_started_secs),
                *expected,
                "case: {name}"
            );
        }
    }

    #[test]
    fn attention_payload_keys_the_core_updates_source() {
        let body = serde_json::to_value(AttentionPayload { core_updates: true }).unwrap();
        assert_eq!(body, serde_json::json!({ "__core-updates": true }));
    }

    #[test]
    fn update_target_validation_table() {
        let cases: &[(Option<&str>, Result<UpdateTarget, StatusCode>)] = &[
            (None, Err(StatusCode::BAD_REQUEST)),
            (Some(""), Err(StatusCode::BAD_REQUEST)),
            (Some("   "), Err(StatusCode::BAD_REQUEST)),
            (Some("../bad"), Err(StatusCode::BAD_REQUEST)),
            (Some("bad/path"), Err(StatusCode::BAD_REQUEST)),
            (
                Some("plugin-launcher"),
                Ok(UpdateTarget::Plugin("plugin-launcher".to_string())),
            ),
            (Some("qol-tray"), Ok(UpdateTarget::Host)),
        ];
        for (input, expected) in cases {
            let actual = update_target(*input).map_err(|(status, _)| status);
            assert_eq!(&actual, expected, "input: {input:?}");
        }
    }

    #[test]
    fn host_update_refusal_table() {
        let cases = [
            (
                false,
                false,
                true,
                Some("Development builds update through Recompile"),
            ),
            (true, true, true, Some("An update is already running")),
            (true, true, false, Some("An update is already running")),
            (true, false, false, Some("qol-tray is up to date")),
            (true, false, true, None),
        ];
        for (checks_enabled, running, available, expected) in cases {
            assert_eq!(
                host_update_refusal(checks_enabled, running, available),
                expected,
                "checks_enabled={checks_enabled} running={running} available={available}"
            );
        }
    }

    #[test]
    fn update_all_refusal_table() {
        let plan = [UpdateTarget::Plugin("plugin-a".to_string())];
        let empty: [UpdateTarget; 0] = [];
        let cases: &[(bool, &[UpdateTarget], Option<&str>)] = &[
            (true, plan.as_slice(), Some("An update is already running")),
            (false, &empty, Some("Nothing to update")),
            (false, plan.as_slice(), None),
        ];
        for (running, plan, expected) in cases {
            assert_eq!(update_all_refusal(*running, plan), *expected);
        }
    }

    #[test]
    fn update_all_plan_orders_plugins_by_name_then_the_host_last() {
        let mut failed = plugin("plugin-alt-tab", "Alt Tab", "1.0.0", None);
        failed.job = job(PluginUpdateState::Failed, Some("No connection to GitHub"));
        let launcher = plugin("plugin-launcher", "Launcher", "1.0.0", Some("1.1.0"));
        let gamma = plugin("plugin-gamma", "Gamma", "1.0.0", Some("1.0.0"));
        let zed = plugin("plugin-zed", "Zed", "1.0.0", Some("2.0.0"));
        let mut linked = plugin("plugin-linked", "Linked", "1.0.0", None);
        linked.dev_linked = true;
        let plugins = vec![zed, gamma, linked, failed, launcher];

        assert_eq!(
            update_all_plan(&plugins, true),
            vec![
                UpdateTarget::Plugin("plugin-alt-tab".to_string()),
                UpdateTarget::Plugin("plugin-launcher".to_string()),
                UpdateTarget::Plugin("plugin-zed".to_string()),
                UpdateTarget::Host,
            ]
        );
        assert_eq!(
            update_all_plan(&plugins, false),
            vec![
                UpdateTarget::Plugin("plugin-alt-tab".to_string()),
                UpdateTarget::Plugin("plugin-launcher".to_string()),
                UpdateTarget::Plugin("plugin-zed".to_string()),
            ]
        );
        assert_eq!(update_all_plan(&[], false), Vec::new());
    }

    #[test]
    fn action_response_serializes_success_and_message() {
        let body = serde_json::to_value(CoreActionResponse {
            success: true,
            message: "Update started".to_string(),
        })
        .unwrap();
        assert_eq!(
            body,
            serde_json::json!({"success": true, "message": "Update started"})
        );
    }
}
