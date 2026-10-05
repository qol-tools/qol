use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

use super::super::helpers::validate_plugin_id;
use super::super::plugin_services;
use super::super::types::AppState;
use super::http_json;
use super::update_handlers::{action_error, action_ok, CoreActionRequest};
use crate::features::plugin_store::source::{builtin_sources, default_source_repos};
use crate::features::plugin_store::user_sources;
use crate::updates::jobs::{self, JobState, UpdateJob};

pub(super) const PLUGINS_QUERY: &str = "plugins";

static INSTALL_WORKER_RUNNING: AtomicBool = AtomicBool::new(false);

type HttpResult<T> = Result<T, Box<Response>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum PluginState {
    NotInstalled,
    Installed,
    Queued,
    Installing,
    Failed,
    Removing,
}

struct PluginView {
    id: String,
    name: String,
    description: String,
    available: Option<String>,
    installed: Option<String>,
    job: Option<UpdateJob>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct PluginsPayload {
    revalidating: bool,
    sources: Vec<SourcePayload>,
    plugins: Vec<PluginPayload>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct SourcePayload {
    repo: String,
    builtin: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct PluginPayload {
    id: String,
    name: String,
    description: String,
    available: Option<String>,
    installed: Option<String>,
    state: PluginState,
    error: Option<String>,
}

pub(super) async fn get_plugins(state: AppState) -> Response {
    http_json::blocking("plugins query", move || {
        let payload = collect_payload(&state)?;
        let json = http_json::encode_json(&payload, "Failed to serialize plugins")?;
        Ok(http_json::json_response(json))
    })
    .await
}

pub(super) fn post_plugins_action(
    state: &AppState,
    action: &str,
    request: &CoreActionRequest,
) -> Option<Response> {
    let response = match action {
        "install" => install_action(state, request.id.as_deref()),
        "cancel_install" => cancel_install_action(request.id.as_deref()),
        "uninstall" => uninstall_action(state, request.id.as_deref()),
        "add_source" => add_source_action(state, request.repo.as_deref()),
        "remove_source" => remove_source_action(state, request.repo.as_deref()),
        _ => return None,
    };
    Some(response)
}

fn install_action(state: &AppState, id: Option<&str>) -> Response {
    let id = match plugin_id(id) {
        Ok(id) => id,
        Err(response) => return *response,
    };
    let installed = match installed_ids(state) {
        Ok(installed) => installed,
        Err(response) => return *response,
    };
    let refusal = install_refusal(
        installed.contains(&id),
        jobs::get(&id).map(|job| job.state),
        updates_queued(&installed, &jobs::snapshot()),
    );
    if let Some(message) = refusal {
        return action_error(StatusCode::CONFLICT, message);
    }
    jobs::queue(&id);
    spawn_install_worker(state.clone());
    action_ok("Install queued")
}

fn cancel_install_action(id: Option<&str>) -> Response {
    let id = match plugin_id(id) {
        Ok(id) => id,
        Err(response) => return *response,
    };
    if !jobs::remove_queued(&id) {
        return action_error(StatusCode::CONFLICT, "The plugin is not waiting to install");
    }
    action_ok("Install cancelled")
}

fn uninstall_action(state: &AppState, id: Option<&str>) -> Response {
    let id = match plugin_id(id) {
        Ok(id) => id,
        Err(response) => return *response,
    };
    match installed_ids(state) {
        Ok(installed) if installed.contains(&id) => {}
        Ok(_) => return action_error(StatusCode::NOT_FOUND, format!("Unknown plugin: {id}")),
        Err(response) => return *response,
    }
    if let Err(message) = jobs::start_removing(&id) {
        return action_error(StatusCode::CONFLICT, message);
    }
    let worker_state = state.clone();
    tokio::spawn(async move {
        let result = plugin_services::uninstall_plugin(&worker_state, &id).await;
        if result.success {
            jobs::finish(&id);
        } else {
            log::warn!("Uninstall for {} failed: {}", id, result.message);
            jobs::fail(&id, result.message);
        }
    });
    action_ok("Removing")
}

fn add_source_action(state: &AppState, repo: Option<&str>) -> Response {
    let Some(repo) = repo else {
        return action_error(StatusCode::BAD_REQUEST, "A repository is required");
    };
    match user_sources::add(repo, &default_source_repos()) {
        Ok(repo) => {
            revalidate_catalog(state);
            action_ok(&format!("Added {repo}"))
        }
        Err(message) => action_error(StatusCode::BAD_REQUEST, message),
    }
}

fn remove_source_action(state: &AppState, repo: Option<&str>) -> Response {
    let Some(repo) = repo else {
        return action_error(StatusCode::BAD_REQUEST, "A repository is required");
    };
    match user_sources::remove(repo, &default_source_repos()) {
        Ok(()) => {
            revalidate_catalog(state);
            action_ok("Source removed")
        }
        Err(message) => action_error(StatusCode::BAD_REQUEST, message),
    }
}

fn revalidate_catalog(state: &AppState) {
    if let Err((_, message)) = plugin_services::list_plugins(state, true) {
        log::warn!("Plugin catalog revalidation could not start: {}", message);
    }
}

fn spawn_install_worker(state: AppState) {
    if INSTALL_WORKER_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async move {
        loop {
            drain_install_queue(&state).await;
            INSTALL_WORKER_RUNNING.store(false, Ordering::SeqCst);
            if next_install(&state).is_none() || INSTALL_WORKER_RUNNING.swap(true, Ordering::SeqCst)
            {
                break;
            }
        }
    });
}

async fn drain_install_queue(state: &AppState) {
    while let Some(id) = next_install(state) {
        if !jobs::take_queued(&id) {
            continue;
        }
        match plugin_services::install_plugin(state, &id).await {
            Ok(_) => jobs::finish(&id),
            Err((_, message)) => {
                log::warn!("Install for {} failed: {}", id, message);
                let error = anyhow::anyhow!(message);
                jobs::fail(&id, crate::updates::plain_update_failure(&error));
            }
        }
    }
}

fn next_install(state: &AppState) -> Option<String> {
    let id = jobs::next_queued()?;
    let installed = installed_ids(state).ok()?;
    (!installed.contains(&id)).then_some(id)
}

fn install_refusal(
    installed: bool,
    job: Option<JobState>,
    updates_queued: bool,
) -> Option<&'static str> {
    match job {
        Some(JobState::Queued) => return Some("The plugin is already waiting to install"),
        Some(JobState::Updating) => return Some("The plugin is already installing"),
        Some(JobState::Removing) => return Some("The plugin is being removed"),
        Some(JobState::Failed) | None => {}
    }
    if installed {
        return Some("The plugin is already installed");
    }
    if updates_queued {
        return Some(crate::updates::UPDATE_ALREADY_RUNNING);
    }
    None
}

fn updates_queued(installed: &HashSet<String>, jobs: &HashMap<String, UpdateJob>) -> bool {
    jobs.iter()
        .any(|(id, job)| job.state == JobState::Queued && installed.contains(id))
}

fn plugin_id(id: Option<&str>) -> HttpResult<String> {
    let Some(id) = id.map(str::trim).filter(|id| !id.is_empty()) else {
        return Err(Box::new(action_error(
            StatusCode::BAD_REQUEST,
            "A plugin id is required",
        )));
    };
    validate_plugin_id(id)
        .map_err(|message| Box::new(action_error(StatusCode::BAD_REQUEST, message)))?;
    Ok(id.to_string())
}

fn installed_ids(state: &AppState) -> HttpResult<HashSet<String>> {
    let installed = plugin_services::list_installed(state).map_err(|_| {
        Box::new(action_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Plugin list unavailable",
        ))
    })?;
    Ok(installed
        .plugins
        .iter()
        .map(|plugin| plugin.id.as_str().to_string())
        .collect())
}

fn collect_payload(state: &AppState) -> HttpResult<PluginsPayload> {
    let catalog = plugin_services::list_plugins(state, false)
        .map_err(|(status, message)| Box::new((status, message).into_response()))?;
    let installed = plugin_services::list_installed(state)
        .map_err(|status| Box::new(status.into_response()))?;
    let jobs = jobs::snapshot();
    let catalog_ids: HashSet<String> = catalog
        .plugins
        .iter()
        .map(|plugin| plugin.id.clone())
        .collect();
    let installed_versions: HashMap<&str, &str> = installed
        .plugins
        .iter()
        .map(|plugin| (plugin.id.as_str(), plugin.version.as_str()))
        .collect();
    let mut views: Vec<PluginView> = catalog
        .plugins
        .iter()
        .map(|plugin| PluginView {
            id: plugin.id.clone(),
            name: plugin.name.clone(),
            description: plugin.description.clone(),
            available: Some(plugin.version.clone()),
            installed: plugin.installed_version.clone().or_else(|| {
                installed_versions
                    .get(plugin.id.as_str())
                    .map(|version| version.to_string())
            }),
            job: jobs.get(&plugin.id).cloned(),
        })
        .collect();
    views.extend(
        installed
            .plugins
            .iter()
            .filter(|plugin| !catalog_ids.contains(plugin.id.as_str()))
            .map(|plugin| PluginView {
                id: plugin.id.as_str().to_string(),
                name: plugin.name.clone(),
                description: plugin.description.clone(),
                available: None,
                installed: Some(plugin.version.clone()),
                job: jobs.get(plugin.id.as_str()).cloned(),
            }),
    );
    let defaults = default_source_repos();
    let sources = builtin_sources()
        .into_iter()
        .map(|source| SourcePayload {
            builtin: defaults.contains(&source.repo),
            repo: source.repo,
        })
        .collect();
    Ok(build_payload(catalog.revalidating, sources, views))
}

fn build_payload(
    revalidating: bool,
    sources: Vec<SourcePayload>,
    mut views: Vec<PluginView>,
) -> PluginsPayload {
    views.sort_by(|left, right| {
        queue_position(left)
            .cmp(&queue_position(right))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.id.cmp(&right.id))
    });
    PluginsPayload {
        revalidating,
        sources,
        plugins: views.into_iter().map(plugin_payload).collect(),
    }
}

fn queue_position(view: &PluginView) -> (bool, u64) {
    match &view.job {
        Some(job) if matches!(job.state, JobState::Queued | JobState::Updating) => {
            (false, job.order)
        }
        _ => (true, 0),
    }
}

fn plugin_payload(view: PluginView) -> PluginPayload {
    let state = plugin_state(&view);
    PluginPayload {
        error: if state == PluginState::Failed {
            view.job.and_then(|job| job.reason)
        } else {
            None
        },
        id: view.id,
        name: view.name,
        description: view.description,
        available: view.available,
        installed: view.installed,
        state,
    }
}

fn plugin_state(view: &PluginView) -> PluginState {
    match view.job.as_ref().map(|job| job.state) {
        Some(JobState::Queued) => PluginState::Queued,
        Some(JobState::Updating) => PluginState::Installing,
        Some(JobState::Failed) => PluginState::Failed,
        Some(JobState::Removing) => PluginState::Removing,
        None if view.installed.is_some() => PluginState::Installed,
        None => PluginState::NotInstalled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(id: &str, name: &str, installed: Option<&str>) -> PluginView {
        PluginView {
            id: id.to_string(),
            name: name.to_string(),
            description: String::new(),
            available: Some("1.0.0".to_string()),
            installed: installed.map(str::to_string),
            job: None,
        }
    }

    fn job(state: JobState, order: u64, reason: Option<&str>) -> Option<UpdateJob> {
        Some(UpdateJob {
            state,
            progress: None,
            reason: reason.map(str::to_string),
            order,
        })
    }

    #[test]
    fn wire_query_name_is_stable() {
        assert_eq!(PLUGINS_QUERY, "plugins");
    }

    #[test]
    fn plugin_state_maps_jobs_before_the_installed_version() {
        let cases: &[(Option<JobState>, Option<&str>, PluginState)] = &[
            (None, None, PluginState::NotInstalled),
            (None, Some("1.0.0"), PluginState::Installed),
            (Some(JobState::Queued), None, PluginState::Queued),
            (Some(JobState::Updating), None, PluginState::Installing),
            (Some(JobState::Failed), None, PluginState::Failed),
            (
                Some(JobState::Removing),
                Some("1.0.0"),
                PluginState::Removing,
            ),
        ];
        for (job_state, installed, expected) in cases {
            let mut plugin = view("qol-voice", "Voice", *installed);
            plugin.job = job_state.and_then(|state| job(state, 0, None));
            assert_eq!(plugin_state(&plugin), *expected, "job: {job_state:?}");
        }
    }

    #[test]
    fn payload_reports_every_field_in_the_wire_shape() {
        let mut failed = view("qol-voice", "Voice", None);
        failed.description = "Talk to your terminal".to_string();
        failed.job = job(JobState::Failed, 3, Some("No connection to GitHub"));
        let mut removing = view("qol-lights", "Lights", Some("0.9.0"));
        removing.available = None;
        removing.job = job(JobState::Removing, 4, Some("ignored"));
        let payload = build_payload(
            true,
            vec![SourcePayload {
                repo: "qol-tools/qol".to_string(),
                builtin: true,
            }],
            vec![failed, removing],
        );
        assert_eq!(
            serde_json::to_value(&payload).unwrap(),
            serde_json::json!({
                "revalidating": true,
                "sources": [{ "repo": "qol-tools/qol", "builtin": true }],
                "plugins": [
                    {
                        "id": "qol-lights",
                        "name": "Lights",
                        "description": "",
                        "available": null,
                        "installed": "0.9.0",
                        "state": "removing",
                        "error": null
                    },
                    {
                        "id": "qol-voice",
                        "name": "Voice",
                        "description": "Talk to your terminal",
                        "available": "1.0.0",
                        "installed": null,
                        "state": "failed",
                        "error": "No connection to GitHub"
                    }
                ]
            })
        );
    }

    #[test]
    fn queued_and_installing_plugins_come_first_in_queue_order() {
        let mut queued_late = view("plugin-a", "Alpha", None);
        queued_late.job = job(JobState::Queued, 9, None);
        let mut installing = view("plugin-z", "Zed", None);
        installing.job = job(JobState::Updating, 2, None);
        let mut queued_early = view("plugin-m", "Mid", None);
        queued_early.job = job(JobState::Queued, 5, None);
        let mut failed = view("plugin-b", "beta", None);
        failed.job = job(JobState::Failed, 1, Some("No connection to GitHub"));
        let installed = view("plugin-c", "Charlie", Some("1.0.0"));
        let available = view("plugin-0", "Alpha", None);

        let payload = build_payload(
            false,
            Vec::new(),
            vec![
                installed,
                queued_late,
                available,
                failed,
                queued_early,
                installing,
            ],
        );
        let ids: Vec<&str> = payload.plugins.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            ["plugin-z", "plugin-m", "plugin-a", "plugin-0", "plugin-b", "plugin-c"]
        );
    }

    #[test]
    fn install_refusal_table() {
        let running = Some(crate::updates::UPDATE_ALREADY_RUNNING);
        let cases: &[(bool, Option<JobState>, bool, Option<&str>)] = &[
            (false, None, false, None),
            (false, Some(JobState::Failed), false, None),
            (
                false,
                Some(JobState::Queued),
                false,
                Some("The plugin is already waiting to install"),
            ),
            (
                false,
                Some(JobState::Updating),
                false,
                Some("The plugin is already installing"),
            ),
            (
                true,
                Some(JobState::Removing),
                false,
                Some("The plugin is being removed"),
            ),
            (true, None, false, Some("The plugin is already installed")),
            (
                true,
                Some(JobState::Failed),
                false,
                Some("The plugin is already installed"),
            ),
            (false, None, true, running),
        ];
        for (installed, job_state, queued, expected) in cases {
            assert_eq!(
                install_refusal(*installed, *job_state, *queued),
                *expected,
                "installed={installed} job={job_state:?} updates_queued={queued}"
            );
        }
    }

    #[test]
    fn only_a_queued_installed_plugin_counts_as_a_pending_update() {
        let installed: HashSet<String> = ["qol-launcher".to_string()].into();
        let jobs_with = |id: &str, state: JobState| -> HashMap<String, UpdateJob> {
            [(id.to_string(), job(state, 0, None).unwrap())].into()
        };
        assert!(updates_queued(
            &installed,
            &jobs_with("qol-launcher", JobState::Queued)
        ));
        assert!(!updates_queued(
            &installed,
            &jobs_with("qol-launcher", JobState::Updating)
        ));
        assert!(!updates_queued(
            &installed,
            &jobs_with("qol-voice", JobState::Queued)
        ));
        assert!(!updates_queued(&installed, &HashMap::new()));
    }

    #[test]
    fn plugin_id_requires_a_valid_id() {
        for input in [None, Some(""), Some("  "), Some("../bad"), Some("bad/path")] {
            let response = plugin_id(input).unwrap_err();
            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "input: {input:?}"
            );
        }
        assert_eq!(
            plugin_id(Some(" qol-voice ")).ok().as_deref(),
            Some("qol-voice")
        );
    }
}
