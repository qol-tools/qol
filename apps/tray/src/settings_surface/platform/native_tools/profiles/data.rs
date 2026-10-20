use std::time::Duration;

use std::collections::BTreeMap;

use anyhow::Context;
use qol_runtime::local_http::Method;
use serde::{Deserialize, Serialize};

use super::super::data::{request_json, request_text, REQUEST_TIMEOUT};

const SYNC_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(super) struct Profile {
    pub(super) name: String,
    pub(super) active: bool,
    pub(super) plugins: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Health {
    NotConfigured,
    Healthy,
    Attention,
    Error,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(super) struct Incident {
    pub(super) message: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(super) struct SyncStatus {
    pub(super) configured: bool,
    #[serde(default)]
    pub(super) repo_url: Option<String>,
    pub(super) health: Health,
    pub(super) pull_on_launch: bool,
    pub(super) push_on_change: bool,
    pub(super) has_github_token: bool,
    #[serde(default)]
    pub(super) last_sync_at: Option<String>,
    #[serde(default)]
    pub(super) incident: Option<Incident>,
    #[serde(default)]
    pub(super) last_error: Option<String>,
    pub(super) backup_count: usize,
    #[serde(default)]
    pub(super) conflict_count: usize,
    #[serde(default)]
    pub(super) latest_backup_file: Option<String>,
    pub(super) github_connect: GitHubConnect,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(super) enum GitHubConnect {
    Idle,
    Waiting {
        user_code: String,
        verification_uri: String,
    },
    Connecting,
    Failed {
        message: String,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(super) struct Backup {
    pub(super) file_name: String,
    pub(super) size_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(super) struct Conflict {
    pub(super) file: String,
    #[serde(default)]
    pub(super) plugin: Option<String>,
    pub(super) key_path: String,
    pub(super) local: serde_json::Value,
    pub(super) remote: serde_json::Value,
    #[serde(default)]
    pub(super) local_edited: Option<String>,
    #[serde(default)]
    pub(super) remote_edited: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Side {
    Mine,
    Remote,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Review {
    pub(super) conflicts: Vec<Conflict>,
    pub(super) names: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct InstalledPlugin {
    id: String,
    uid: String,
    name: String,
}

#[derive(Deserialize)]
struct Installed {
    plugins: Vec<InstalledPlugin>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub(super) profiles: Vec<Profile>,
    pub(super) sync: SyncStatus,
}

pub(super) fn load() -> anyhow::Result<Snapshot> {
    Ok(Snapshot {
        profiles: request_json(Method::Get, "/api/profiles", None, REQUEST_TIMEOUT)?,
        sync: request_json(Method::Get, "/api/sync/status", None, REQUEST_TIMEOUT)?,
    })
}

pub(super) fn backups() -> anyhow::Result<Vec<Backup>> {
    request_json(Method::Get, "/api/sync/backups", None, REQUEST_TIMEOUT)
}

pub(super) fn review() -> anyhow::Result<Review> {
    let conflicts = request_json(Method::Get, "/api/sync/conflicts", None, REQUEST_TIMEOUT)?;
    let names = request_json::<Installed>(Method::Get, "/api/installed", None, REQUEST_TIMEOUT)
        .map(|installed| {
            installed
                .plugins
                .into_iter()
                .flat_map(|plugin| [(plugin.id, plugin.name.clone()), (plugin.uid, plugin.name)])
                .collect()
        })
        .unwrap_or_default();
    Ok(Review { conflicts, names })
}

pub(super) fn resolve(conflicts: &[Conflict], picks: &[Side]) -> anyhow::Result<Option<Review>> {
    let choices = conflicts
        .iter()
        .zip(picks)
        .map(|(conflict, side)| {
            serde_json::json!({
                "file": conflict.file,
                "key_path": conflict.key_path,
                "side": side,
            })
        })
        .collect::<Vec<_>>();
    let body = serde_json::json!({ "choices": choices }).to_string();
    let result: serde_json::Value = request_json(
        Method::Post,
        "/api/sync/conflicts/resolve",
        Some(&body),
        SYNC_TIMEOUT,
    )?;
    if let Some(error) = result["status"]["last_error"].as_str() {
        anyhow::bail!("{error}");
    }
    match result["status"]["conflict_count"].as_u64() {
        Some(count) if count > 0 => review().map(Some),
        _ => Ok(None),
    }
}

pub(super) fn use_profile(name: &str) -> anyhow::Result<()> {
    let body = serde_json::json!({ "profile": name }).to_string();
    request_text(
        Method::Put,
        "/api/core/config",
        Some(&body),
        REQUEST_TIMEOUT,
    )
    .map(drop)
}

pub(super) fn create_profile(name: &str) -> anyhow::Result<()> {
    let body = serde_json::json!({ "name": name }).to_string();
    request_text(Method::Post, "/api/profiles", Some(&body), REQUEST_TIMEOUT).map(drop)
}

pub(super) fn sync_now() -> anyhow::Result<()> {
    request_text(Method::Post, "/api/sync/now", Some("{}"), SYNC_TIMEOUT).map(drop)
}

pub(super) fn set_auto_sync(pull_on_launch: bool, push_on_change: bool) -> anyhow::Result<()> {
    let body = serde_json::json!({
        "pull_on_launch": pull_on_launch,
        "push_on_change": push_on_change,
    })
    .to_string();
    request_text(Method::Post, "/api/sync/auto", Some(&body), REQUEST_TIMEOUT).map(drop)
}

pub(super) fn disconnect() -> anyhow::Result<()> {
    request_text(
        Method::Post,
        "/api/sync/disconnect",
        Some("{}"),
        SYNC_TIMEOUT,
    )
    .map(drop)
}

pub(super) fn connect_github() -> anyhow::Result<GitHubConnect> {
    request_json(
        Method::Post,
        "/api/sync/github/connect",
        Some("{}"),
        REQUEST_TIMEOUT,
    )
}

pub(super) fn open_backup(file_name: &str) -> anyhow::Result<()> {
    let route = format!("/api/sync/backups/{file_name}/open");
    request_text(Method::Post, &route, Some("{}"), REQUEST_TIMEOUT).map(drop)
}

pub(super) fn export_to(path: &std::path::Path) -> anyhow::Result<()> {
    let bundle = request_text(Method::Get, "/api/config/export", None, REQUEST_TIMEOUT)?;
    std::fs::write(path, bundle).with_context(|| format!("could not write {}", path.display()))
}

pub(super) fn import_from(path: &std::path::Path) -> anyhow::Result<()> {
    let bundle = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    request_text(
        Method::Post,
        "/api/config/import",
        Some(&bundle),
        SYNC_TIMEOUT,
    )
    .map(drop)
}
